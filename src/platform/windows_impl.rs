//! Win32 implementation of the platform glue (see `mod.rs`).
//!
//! One hidden message-only window on a dedicated thread hosts:
//! - the clipboard format listener (`WM_CLIPBOARDUPDATE`),
//! - the "show yourself" named event waiter is a separate tiny
//!   thread (it blocks in WaitForSingleObject),
//! - the Ctrl+Alt+G hotkey (`WM_HOTKEY`).
//!
//! All three report through the same mpsc channel the GUI drains.

use super::{Platform, PlatformEvent, PlatformSetup};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
    OpenClipboard, RemoveClipboardFormatListener,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
    INFINITE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage, HWND_MESSAGE,
    MB_ICONERROR, MB_OK, MSG, WM_CLIPBOARDUPDATE, WM_DESTROY, WM_HOTKEY, WNDCLASSW, WS_OVERLAPPED,
};

const CF_UNICODETEXT: u32 = 13;
const HOTKEY_SHOW: i32 = 0xC11B;
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "ytTrayDL";
const MUTEX_NAME: &str = "Local\\ytTrayDL_SingleInstance_v1";
const SHOW_EVENT_NAME: &str = "Local\\ytTrayDL_ShowYourself_v1";

static SENDER: OnceLock<Sender<PlatformEvent>> = OnceLock::new();
static LISTENER_HWND: OnceLock<isize> = OnceLock::new();

pub struct Inner {
    mutex: HANDLE,
    /// Kept so the handle isn't dropped early; the listener thread
    /// is detached by design and runs for the process lifetime.
    _thread: Option<std::thread::JoinHandle<()>>,
}

// HANDLE is a raw pointer wrapper; the mutex is only ever closed on
// this struct's Drop, from the owning thread's process teardown.
unsafe impl Send for Inner {}

pub fn setup(show_on_launch: bool) -> PlatformSetup {
    let (tx, rx) = channel::<PlatformEvent>();
    let _ = SENDER.set(tx.clone());

    // Single instance: try to create the named mutex. If it already
    // exists, signal the running instance to show itself and report
    // `already_running` so the caller exits quietly.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, wide(MUTEX_NAME).as_ptr()) };
    let last_err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    const ERROR_ALREADY_EXISTS: u32 = 183;
    if !mutex.is_null() && last_err == ERROR_ALREADY_EXISTS {
        signal_show_event();
        return PlatformSetup {
            platform: Platform {
                inner: Inner {
                    mutex: std::ptr::null_mut(),
                    _thread: None,
                },
            },
            events: rx,
            already_running: true,
        };
    }

    // "Show yourself" event waiter thread.
    {
        let tx2 = tx.clone();
        std::thread::spawn(move || unsafe {
            let ev = CreateEventW(std::ptr::null(), 0, 0, wide(SHOW_EVENT_NAME).as_ptr());
            if ev.is_null() {
                return;
            }
            loop {
                if WaitForSingleObject(ev, INFINITE) != 0 {
                    break;
                }
                if tx2.send(PlatformEvent::ShowRequested).is_err() {
                    break;
                }
            }
        });
    }

    // Clipboard listener + hotkey on a message-window thread.
    let thread_tx = tx.clone();
    let thread = std::thread::spawn(move || unsafe {
        message_window_thread(thread_tx);
    });

    if show_on_launch {
        let _ = tx.send(PlatformEvent::ShowRequested);
    }

    PlatformSetup {
        platform: Platform {
            inner: Inner {
                mutex,
                _thread: Some(thread),
            },
        },
        events: rx,
        already_running: false,
    }
}

fn signal_show_event() {
    unsafe {
        let ev = OpenEventW(EVENT_MODIFY_STATE, 0, wide(SHOW_EVENT_NAME).as_ptr());
        if !ev.is_null() {
            SetEvent(ev);
            CloseHandle(ev);
        }
    }
}

unsafe fn message_window_thread(tx: Sender<PlatformEvent>) {
    let hinstance = GetModuleHandleW(std::ptr::null());
    let class_name = wide("ytTrayDLMessageWindow");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance,
        lpszClassName: class_name.as_ptr(),
        ..std::mem::zeroed()
    };
    RegisterClassW(&wc);
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        wide("ytTrayDL").as_ptr(),
        WS_OVERLAPPED,
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        return;
    }
    let _ = LISTENER_HWND.set(hwnd as isize);
    AddClipboardFormatListener(hwnd);
    // Ctrl+Alt+G — "G for Grab". If another app owns it, RegisterHotKey
    // simply fails and the other recovery paths still work.
    RegisterHotKey(hwnd, HOTKEY_SHOW, MOD_CONTROL | MOD_ALT, 'G' as u32);
    let _ = tx; // the window proc sends via the static SENDER

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
        if r <= 0 {
            break;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            if let Some(text) = read_clipboard_text_with_retry() {
                if let Some(tx) = SENDER.get() {
                    let _ = tx.send(PlatformEvent::ClipboardText(text));
                }
            }
            0
        }
        WM_HOTKEY => {
            if wparam as i32 == HOTKEY_SHOW {
                if let Some(tx) = SENDER.get() {
                    let _ = tx.send(PlatformEvent::ShowRequested);
                }
            }
            0
        }
        WM_DESTROY => {
            RemoveClipboardFormatListener(hwnd);
            UnregisterHotKey(hwnd, HOTKEY_SHOW);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Read CF_UNICODETEXT. The clipboard can be momentarily locked by
/// the app that just set it, so retry briefly (5 × 15 ms) — the
/// sequence-number check keeps us from emitting the same content
/// twice when several updates land back-to-back.
unsafe fn read_clipboard_text_with_retry() -> Option<String> {
    static LAST_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = GetClipboardSequenceNumber();
    if seq != 0 && LAST_SEQ.swap(seq, std::sync::atomic::Ordering::SeqCst) == seq {
        return None;
    }
    for _ in 0..5 {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(15));
            continue;
        }
        let data = GetClipboardData(CF_UNICODETEXT);
        let result = if data.is_null() {
            None
        } else {
            let ptr = GlobalLock(data) as *const u16;
            if ptr.is_null() {
                None
            } else {
                let mut len = 0usize;
                while *ptr.add(len) != 0 && len < 1_000_000 {
                    len += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                GlobalUnlock(data);
                Some(s)
            }
        };
        CloseClipboard();
        return result;
    }
    None
}

impl Drop for Platform {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if let Some(&hwnd) = LISTENER_HWND.get() {
                if hwnd != 0 {
                    PostMessageW(hwnd as HWND, 0x0010 /* WM_CLOSE */, 0, 0);
                    DestroyWindow(hwnd as HWND);
                }
            }
            if !self.inner.mutex.is_null() {
                CloseHandle(self.inner.mutex);
            }
        }
    }
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        );
        if rc != 0 {
            return Err(format!("opening Run key failed ({rc})"));
        }
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            // `--background`: autostart goes straight to the tray;
            // only manual launches open the control panel.
            let value = format!("\"{}\" --background", exe.display());
            let bytes = wide(&value);
            let rc = RegSetValueExW(
                hkey,
                wide(RUN_VALUE).as_ptr(),
                0,
                REG_SZ,
                bytes.as_ptr() as *const u8,
                (bytes.len() * 2) as u32,
            );
            if rc == 0 {
                Ok(())
            } else {
                Err(format!("setting Run value failed ({rc})"))
            }
        } else {
            let rc = RegDeleteValueW(hkey, wide(RUN_VALUE).as_ptr());
            if rc == 0 || rc == 2 {
                Ok(()) // 2 = value already absent
            } else {
                Err(format!("removing Run value failed ({rc})"))
            }
        };
        RegCloseKey(hkey);
        result
    }
}

pub fn autostart_enabled() -> bool {
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        ) != 0
        {
            return false;
        }
        let mut ty = 0u32;
        let mut size = 0u32;
        let rc = RegQueryValueExW(
            hkey,
            wide(RUN_VALUE).as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            std::ptr::null_mut(),
            &mut size,
        );
        RegCloseKey(hkey);
        rc == 0
    }
}

pub fn open_path(path: &Path) -> Result<(), String> {
    shell_execute(&path.to_string_lossy())
}

pub fn open_url(url: &str) -> Result<(), String> {
    shell_execute(url)
}

fn shell_execute(target: &str) -> Result<(), String> {
    unsafe {
        let rc = ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(target).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            5, // SW_SHOW
        );
        // ShellExecute returns a value > 32 on success.
        if (rc as isize) > 32 {
            Ok(())
        } else {
            Err(format!("ShellExecute failed ({})", rc as isize))
        }
    }
}

pub fn fatal_error_box(title: &str, message: &str) {
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(message).as_ptr(),
            wide(title).as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub fn cursor_position() -> Option<(f32, f32)> {
    unsafe {
        let mut p: windows_sys::Win32::Foundation::POINT = std::mem::zeroed();
        if GetCursorPos(&mut p) != 0 {
            Some((p.x as f32, p.y as f32))
        } else {
            None
        }
    }
}

/// Read clipboard text on demand (used by "Download from clipboard"
/// in the tray menu, where the current content matters rather than
/// a change event).
pub fn current_clipboard_text() -> Option<String> {
    unsafe { read_clipboard_text_with_retry() }
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain([0]).collect()
}

#[allow(dead_code)]
fn close_listener() {
    if let Some(&hwnd) = LISTENER_HWND.get() {
        unsafe {
            PostMessageW(hwnd as HWND, 0x0012 /* WM_QUIT */, 0, 0);
        }
    }
}

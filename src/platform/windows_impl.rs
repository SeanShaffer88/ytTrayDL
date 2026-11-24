//! Win32 implementation of the platform glue (see `mod.rs`).
//!
//! One hidden message-only window on a dedicated thread hosts:
//! - the clipboard format listener (`WM_CLIPBOARDUPDATE`),
//! - the "show yourself" named event waiter is a separate tiny
//!   thread (it blocks in WaitForSingleObject),
//! - the Ctrl+Alt+G hotkey (`WM_HOTKEY`).
//!
//! All three report through the same mpsc channel the GUI drains.

use super::{
    Platform, PlatformEvent, PlatformSetup, MANUAL_URL_MESSAGE, OPEN_CONVERTER_MESSAGE,
    OPEN_DOWNLOADS_MESSAGE, OPEN_PLAYER_MESSAGE, OPEN_TAGS_MESSAGE,
};
use std::ffi::OsStr;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
    OpenClipboard, RemoveClipboardFormatListener, COPYDATASTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcessId, GetCurrentThreadId, OpenEventW, OpenProcess,
    QueryFullProcessImageNameW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumThreadWindows, FindWindowExW,
    GetCursorPos, GetMessageW, GetPropW, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    IsWindowVisible, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW,
    SendMessageTimeoutW, SetForegroundWindow, SetPropW, ShowWindow, TranslateMessage, HWND_MESSAGE,
    MB_ICONERROR, MB_OK, MSG, SMTO_ABORTIFHUNG, SW_HIDE, SW_SHOW, WM_CLIPBOARDUPDATE, WM_COPYDATA,
    WM_DESTROY, WM_HOTKEY, WNDCLASSW, WS_OVERLAPPED,
};

const CF_UNICODETEXT: u32 = 13;
const HOTKEY_SHOW: i32 = 0xC11B;
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "ytTrayDL";
const MUTEX_NAME: &str = "Local\\ytTrayDL_SingleInstance_v1";
const SHOW_EVENT_NAME: &str = "Local\\ytTrayDL_ShowYourself_v1";

static SENDER: OnceLock<Sender<PlatformEvent>> = OnceLock::new();
static LISTENER_HWND: OnceLock<isize> = OnceLock::new();
static MAIN_GUI_HWND: AtomicIsize = AtomicIsize::new(0);
static MAIN_GUI_THREAD: AtomicU32 = AtomicU32::new(0);
const WINDOW_ROLE_PROPERTY: &str = "ytTrayDL.PersistentWindowRole.v1";
thread_local! {
    static OWNED_WINDOWS: std::cell::RefCell<std::collections::HashMap<String, isize>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

pub struct Inner {
    mutex: HANDLE,
    event_stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Kept so the handle isn't dropped early; the listener thread
    /// is detached by design and runs for the process lifetime.
    _thread: Option<std::thread::JoinHandle<()>>,
}

// HANDLE is a raw pointer wrapper; the mutex is only ever closed on
// this struct's Drop, from the owning thread's process teardown.
unsafe impl Send for Inner {}

pub fn setup(show_on_launch: bool, has_launch_intent: bool) -> PlatformSetup {
    let (tx, rx) = channel::<PlatformEvent>();
    let _ = SENDER.set(tx.clone());

    // Single instance: try to create the named mutex. If it already
    // exists, signal the running instance to show itself and report
    // `already_running` so the caller exits quietly.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, wide(MUTEX_NAME).as_ptr()) };
    let last_err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    const ERROR_ALREADY_EXISTS: u32 = 183;
    if !mutex.is_null() && last_err == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(mutex);
        }
        if !has_launch_intent {
            signal_show_event();
        }
        return PlatformSetup {
            platform: Platform {
                inner: Inner {
                    mutex: std::ptr::null_mut(),
                    event_stop: None,
                    _thread: None,
                },
            },
            events: rx,
            already_running: true,
        };
    }

    let event_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // "Show yourself" event waiter thread. Drop signals it without polling.
    {
        let tx2 = tx.clone();
        let stop = event_stop.clone();
        std::thread::spawn(move || unsafe {
            let ev = CreateEventW(std::ptr::null(), 0, 0, wide(SHOW_EVENT_NAME).as_ptr());
            if ev.is_null() {
                return;
            }
            loop {
                if stop.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                if WaitForSingleObject(ev, INFINITE) != 0 {
                    break;
                }
                if stop.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                // A hidden viewport may stop scheduling egui frames. Queue the
                // intent before restoring natively, as the tray handler does,
                // so the newly visible window can consume it immediately.
                if tx2.send(PlatformEvent::ShowRequested).is_err() {
                    break;
                }
                show_main_window();
            }
            CloseHandle(ev);
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
                event_stop: Some(event_stop),
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

/// WM_COPYDATA keeps the sender's payload valid for the synchronous call only.
/// Receiver copies before acknowledging; timeout prevents a hung primary blocking launch.
pub fn forward_manual_url(url: &str) -> Result<(), String> {
    let payload =
        super::manual_url_payload(url.as_bytes()).ok_or("expected a bounded HTTP(S) URL")?;
    forward_copydata(MANUAL_URL_MESSAGE, payload.as_bytes())
}

pub fn forward_open_downloads() -> Result<(), String> {
    forward_copydata(OPEN_DOWNLOADS_MESSAGE, b"downloads")
}

pub fn forward_open_converter() -> Result<(), String> {
    forward_copydata(OPEN_CONVERTER_MESSAGE, b"converter")
}

pub fn forward_open_tags() -> Result<(), String> {
    forward_copydata(OPEN_TAGS_MESSAGE, b"tags")
}

pub fn forward_open_player() -> Result<(), String> {
    forward_copydata(OPEN_PLAYER_MESSAGE, b"player")
}

fn forward_copydata(message: usize, payload: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let class = wide("ytTrayDLMessageWindow");
    loop {
        let hwnd = unsafe {
            FindWindowExW(
                HWND_MESSAGE,
                std::ptr::null_mut(),
                class.as_ptr(),
                std::ptr::null(),
            )
        };
        if !hwnd.is_null() && window_executable_matches(hwnd, &exe) {
            let data = COPYDATASTRUCT {
                dwData: message,
                cbData: payload.len() as u32,
                lpData: payload.as_ptr() as *mut std::ffi::c_void,
            };
            let mut acknowledged = 0usize;
            let remaining = deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_millis()
                .min(2000) as u32;
            if remaining == 0 {
                break;
            }
            let sent = unsafe {
                SendMessageTimeoutW(
                    hwnd,
                    WM_COPYDATA,
                    0,
                    &data as *const _ as isize,
                    SMTO_ABORTIFHUNG,
                    remaining,
                    &mut acknowledged,
                )
            };
            return if sent != 0 && acknowledged == 1 {
                Ok(())
            } else {
                Err("the running instance did not accept the request".into())
            };
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Err("the running instance's request receiver was unavailable".into())
}

fn window_executable_matches(hwnd: HWND, expected: &Path) -> bool {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return false;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        );
        CloseHandle(process);
        ok != 0 && Path::new(&std::ffi::OsString::from_wide(&buffer[..length as usize])) == expected
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
        WM_COPYDATA => {
            if lparam == 0 {
                return 0;
            }
            let data = &*(lparam as *const COPYDATASTRUCT);
            if data.cbData == 0
                || data.cbData as usize > super::MAX_MANUAL_URL_BYTES
                || data.lpData.is_null()
            {
                return 0;
            }
            let bytes = std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize);
            let Some(event) = super::copydata_event(data.dwData, bytes) else {
                return 0;
            };
            if SENDER.get().is_some_and(|tx| tx.send(event).is_ok()) {
                show_main_window();
                1
            } else {
                0
            }
        }
        WM_CLIPBOARDUPDATE => {
            static LAST_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let mut last_sequence = LAST_SEQ.load(std::sync::atomic::Ordering::Relaxed);
            let text = super::clipboard_event_text(
                &mut last_sequence,
                GetClipboardSequenceNumber(),
                || read_clipboard_text_with_retry(),
            );
            LAST_SEQ.store(last_sequence, std::sync::atomic::Ordering::Relaxed);
            if let Some(text) = text {
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
                show_main_window();
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
/// event-specific deduplication belongs to WM_CLIPBOARDUPDATE, never manual reads.
unsafe fn read_clipboard_text_with_retry() -> Option<String> {
    for _ in 0..5 {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(15));
            continue;
        }
        let data = GetClipboardData(CF_UNICODETEXT);
        let result = if data.is_null() {
            None
        } else {
            decode_global_text(data)
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
            if let Some(stop) = &self.inner.event_stop {
                stop.store(true, std::sync::atomic::Ordering::Release);
                signal_show_event();
            }
            if let Some(&hwnd) = LISTENER_HWND.get() {
                if hwnd != 0 {
                    PostMessageW(hwnd as HWND, 0x0010 /* WM_CLOSE */, 0, 0);
                }
            }
            if !self.inner.mutex.is_null() {
                CloseHandle(self.inner.mutex);
            }
        }
    }
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let exe = if enabled {
        Some(std::env::current_exe().map_err(|e| e.to_string())?)
    } else {
        None
    };
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
        let result = if let Some(exe) = exe {
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

/// Find this process's main window (title "ytTrayDL") and, if
/// it is hidden, show + foreground it at the OS level. The
/// tray-icon handler calls this directly (0.2.6): restoring
/// the window must not wait for egui to schedule the frame
/// that would process a queued "show" request.
/// Locate this process's main window (title "ytTrayDL").
fn find_main_window() -> Option<HWND> {
    let hwnd = MAIN_GUI_HWND.load(Ordering::Acquire) as HWND;
    valid_cached_window(hwnd, MAIN_GUI_THREAD.load(Ordering::Acquire), 1).then_some(hwnd)
}

/// Called on the GUI thread after the native root exists, even in tray mode.
pub fn remember_main_window() -> bool {
    find_owned_window("ytTrayDL").is_some()
}

fn persistent_role(title: &str) -> usize {
    match title {
        "ytTrayDL" => 1,
        "ytTrayDL — Settings" => 2,
        "ytTrayDL — Downloads" => 3,
        "About ytTrayDL" => 4,
        _ => 0,
    }
}

fn valid_cached_window(hwnd: HWND, thread: u32, role: usize) -> bool {
    if hwnd.is_null() || thread == 0 {
        return false;
    }
    unsafe {
        let mut pid = 0;
        IsWindow(hwnd) != 0
            && GetWindowThreadProcessId(hwnd, &mut pid) == thread
            && pid == GetCurrentProcessId()
            && GetPropW(hwnd, wide(WINDOW_ROLE_PROPERTY).as_ptr()) as usize == role
    }
}

fn find_owned_window(title: &str) -> Option<HWND> {
    let thread = unsafe { GetCurrentThreadId() };
    let role = persistent_role(title);
    if role != 0 {
        // Release cache borrows before any Win32 call, which can re-enter.
        let cached = OWNED_WINDOWS.with(|windows| windows.borrow().get(title).copied());
        if let Some(cached) = cached {
            let hwnd = cached as HWND;
            if valid_cached_window(hwnd, thread, role) {
                return Some(hwnd);
            }
            OWNED_WINDOWS.with(|windows| windows.borrow_mut().remove(title));
        }
    }
    struct Search {
        pid: u32,
        thread: u32,
        found: HWND,
        title: String,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let search = &mut *(lparam as *mut Search);
            let mut pid: u32 = 0;
            let thread = GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != search.pid || thread != search.thread {
                return 1; // keep enumerating
            }
            let mut buf = [0u16; 64];
            let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if len > 0 && String::from_utf16_lossy(&buf[..len as usize]) == search.title {
                search.found = hwnd;
                return 0; // stop
            }
            1
        }
    }
    unsafe {
        let mut search = Search {
            pid: GetCurrentProcessId(),
            thread,
            found: std::ptr::null_mut(),
            title: title.to_owned(),
        };
        let _ = EnumThreadWindows(thread, Some(visit), &mut search as *mut Search as LPARAM);
        if search.found.is_null() {
            None
        } else {
            if role != 0
                && SetPropW(
                    search.found,
                    wide(WINDOW_ROLE_PROPERTY).as_ptr(),
                    role as HANDLE,
                ) != 0
            {
                OWNED_WINDOWS.with(|windows| {
                    windows
                        .borrow_mut()
                        .insert(title.to_owned(), search.found as isize)
                });
                if role == 1 {
                    MAIN_GUI_THREAD.store(thread, Ordering::Release);
                    MAIN_GUI_HWND.store(search.found as isize, Ordering::Release);
                }
            }
            Some(search.found)
        }
    }
}

pub fn show_main_window() -> bool {
    match find_main_window() {
        Some(hwnd) => unsafe {
            let hidden = IsWindowVisible(hwnd) == 0;
            if hidden {
                ShowWindow(hwnd, SW_SHOW);
                SetForegroundWindow(hwnd);
            }
            // Native menus run a nested message loop; an egui repaint request
            // may have been deduplicated before returning from that loop.
            // Invalidating the client area guarantees a new native paint.
            windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 0);
            hidden
        },
        None => false,
    }
}

fn native_geometry(hwnd: HWND) -> Option<crate::window_geometry::WindowGeometry> {
    use windows_sys::Win32::UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::GetWindowRect};
    let mut rect = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return None;
    }
    let value = crate::window_geometry::WindowGeometry {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
        dpi: unsafe { GetDpiForWindow(hwnd) },
    };
    value.valid().then_some(value)
}

fn work_area(hwnd: HWND) -> Option<[i32; 4]> {
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if monitor.is_null() || unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    Some([
        info.rcWork.left,
        info.rcWork.top,
        info.rcWork.right,
        info.rcWork.bottom,
    ])
}

fn set_normal_geometry(
    hwnd: HWND,
    value: crate::window_geometry::WindowGeometry,
    resize: bool,
) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
    };
    let flags =
        SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOZORDER | if resize { 0 } else { SWP_NOSIZE };
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            value.x,
            value.y,
            value.width,
            value.height,
            flags,
        ) != 0
    }
}

pub fn window_geometry(
    kind: crate::window_geometry::WindowKind,
) -> Option<crate::window_geometry::WindowGeometry> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsZoomed};
    let hwnd = find_owned_window(kind.title())?;
    if unsafe { IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 } {
        return None;
    }
    native_geometry(hwnd)
}

/// Called on opening Settings or a cached viewport-monitor change, never per frame.
pub fn window_client_capacity(
    kind: crate::window_geometry::WindowKind,
) -> Option<([f32; 2], [f32; 2])> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
    let hwnd = find_owned_window(kind.title())?;
    let outer = native_geometry(hwnd)?;
    let work = work_area(hwnd)?;
    let mut client: windows_sys::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetClientRect(hwnd, &mut client) } == 0 {
        return None;
    }
    let size = [client.right - client.left, client.bottom - client.top];
    let available = crate::window_geometry::client_capacity(
        [work[2] - work[0], work[3] - work[1]],
        [outer.width, outer.height],
        size,
        outer.dpi,
    )?;
    let scale = outer.dpi as f32 / 96.0;
    Some((available, [size[0] as f32 / scale, size[1] as f32 / scale]))
}

pub fn restore_window_geometry(
    kind: crate::window_geometry::WindowKind,
    value: crate::window_geometry::WindowGeometry,
) -> bool {
    if !value.valid() {
        return false;
    }
    let Some(hwnd) = find_owned_window(kind.title()) else {
        return false;
    };
    let Some(current) = native_geometry(hwnd) else {
        return false;
    };
    // Move first so per-monitor DPI reflects the saved destination; then scale.
    let at_destination = crate::window_geometry::WindowGeometry {
        x: value.x,
        y: value.y,
        ..current
    };
    if !set_normal_geometry(hwnd, at_destination, false) {
        return false;
    }
    let Some(current) = native_geometry(hwnd) else {
        return false;
    };
    let Some(work) = work_area(hwnd) else {
        return false;
    };
    let mut restored = value.at_dpi(current.dpi);
    let inset = ((12 * restored.dpi + 48) / 96) as i32;
    if matches!(
        kind,
        crate::window_geometry::WindowKind::Main | crate::window_geometry::WindowKind::About
    ) {
        // Ignore legacy saved sizes for fixed-size windows. On a small work
        // area, shrink only as necessary to keep the native window reachable.
        restored.width = current.width.min(
            work[2]
                .saturating_sub(work[0])
                .saturating_sub(inset * 2)
                .max(64),
        );
        restored.height = current.height.min(
            work[3]
                .saturating_sub(work[1])
                .saturating_sub(inset * 2)
                .max(64),
        );
    }
    set_normal_geometry(hwnd, restored.clamped(work, inset), true)
}

pub fn constrain_transient_window(window: crate::window_geometry::TransientWindow) {
    let Some(hwnd) = find_owned_window(window.title()) else {
        return;
    };
    let Some(value) = native_geometry(hwnd) else {
        return;
    };
    let Some(work) = work_area(hwnd) else {
        return;
    };
    let clamped = value.clamped(work, ((12 * value.dpi + 48) / 96) as i32);
    if clamped != value {
        set_normal_geometry(hwnd, clamped, false);
    }
}

/// Toggle the main window's visibility (0.2.8): visible →
/// hidden, hidden → shown + foregrounded. Returns the NEW
/// visibility (`Some(true)` = now shown), or `None` when the
/// window doesn't exist. Used by the tray-icon left-click,
/// which reads the window's real OS state every time so the
/// toggle can never diverge from what the user sees.
pub fn toggle_main_window() -> Option<bool> {
    let hwnd = find_main_window()?;
    unsafe {
        if IsWindowVisible(hwnd) != 0 {
            ShowWindow(hwnd, SW_HIDE);
            Some(false)
        } else {
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            Some(true)
        }
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

// ---------------- notification sound + taskbar progress ----------------

use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::UI::Shell::{
    TaskbarList as CLSID_TASKBAR_LIST, TBPF_NOPROGRESS, TBPF_NORMAL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;

/// Play the standard Windows notification sound (Settings →
/// General → Notifications → "Play a sound when a download
/// finishes").
pub fn play_notification_sound() {
    unsafe {
        let _ = MessageBeep(MB_ICONASTERISK);
    }
}

use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Power::SetSuspendState;

/// Free bytes available to the current user on the volume
/// holding `path` (`GetDiskFreeSpaceExW`). The path doesn't
/// need to exist yet — the volume root is derived from it.
pub fn free_disk_bytes(path: &std::path::Path) -> Option<u64> {
    let directory = super::existing_directory(path)?;
    let mut s = directory.to_string_lossy().to_string();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    let w = wide(&s);
    let mut free: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            w.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

/// Suspend the machine (not hibernate, no forced close).
pub fn suspend_system() -> bool {
    unsafe { SetSuspendState(0, 0, 0) != 0 }
}

/// IID of ITaskbarList3 ({EA1AFB91-9E28-4B86-90E9-9E9F8A5EEF75}).
/// windows-sys ships the CLSID and the TBPFLAG constants but not
/// the interface itself, so the few vtbl slots used here are
/// called by index (ITaskbarList3 : ITaskbarList2 : ITaskbarList
/// : IUnknown — Release = 2, HrInit = 3, SetProgressValue = 9,
/// SetProgressState = 10).
const IID_ITASKBAR_LIST3: windows_sys::core::GUID =
    windows_sys::core::GUID::from_u128(0xea1afb91_9e28_4b86_90e9_9e9f8a5eef75);

struct TaskbarObject {
    pointer: *mut std::ffi::c_void,
    owns_com_init: bool,
}

impl Drop for TaskbarObject {
    fn drop(&mut self) {
        unsafe {
            if !self.pointer.is_null() {
                type ReleaseFn = unsafe extern "system" fn(*mut std::ffi::c_void) -> u32;
                let release: ReleaseFn = vtbl_fn(self.pointer, 2);
                release(self.pointer);
            }
            if self.owns_com_init {
                CoUninitialize();
            }
        }
    }
}

thread_local! {
    /// The ITaskbarList3 object pointer (as usize), created
    /// lazily. COM apartment rules: the object must be used
    /// from the thread that created it — every caller runs on
    /// the GUI thread, and the thread-local enforces that.
    static TASKBAR_LIST: std::cell::RefCell<Option<TaskbarObject>> = const { std::cell::RefCell::new(None) };
    static TASKBAR_INITED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn taskbar_list3() -> Option<*mut std::ffi::c_void> {
    let cached = TASKBAR_LIST.with(|c| c.borrow().as_ref().map(|object| object.pointer));
    if cached.is_some() {
        return cached;
    }
    unsafe {
        // Best-effort COM init for this thread; eframe/winit may
        // already have initialized an apartment (an RPC_E_CHANGED_MODE
        // result is fine — an apartment exists either way).
        let init = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        // A changed apartment mode already has COM initialized; other failures
        // cannot safely create an object and must not be balanced with uninit.
        if init < 0 && init != 0x80010106u32 as i32 {
            return None;
        }
        let mut owner = TaskbarObject {
            pointer: std::ptr::null_mut(),
            owns_com_init: init >= 0,
        };
        let mut obj: *mut std::ffi::c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            &CLSID_TASKBAR_LIST,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_ITASKBAR_LIST3,
            &mut obj,
        );
        if hr < 0 || obj.is_null() {
            return None;
        }
        owner.pointer = obj;
        TASKBAR_LIST.with(|c| *c.borrow_mut() = Some(owner));
        Some(obj)
    }
}

/// Read vtbl slot `slot` of a COM object as a function pointer.
unsafe fn vtbl_fn<T: Copy>(obj: *mut std::ffi::c_void, slot: usize) -> T {
    let vtbl = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vtbl.add(slot))
}

/// Set (or clear, with `None`) the main window's taskbar-button
/// progress bar.
pub fn set_taskbar_progress(frac: Option<f64>) {
    let Some(hwnd) = find_main_window() else {
        return;
    };
    let Some(obj) = taskbar_list3() else {
        return;
    };
    unsafe {
        if !TASKBAR_INITED.with(|c| c.get()) {
            type HrInitFn = unsafe extern "system" fn(*mut std::ffi::c_void) -> i32;
            let hr_init: HrInitFn = vtbl_fn(obj, 3);
            let _ = hr_init(obj);
            TASKBAR_INITED.with(|c| c.set(true));
        }
        type SetStateFn = unsafe extern "system" fn(*mut std::ffi::c_void, HWND, i32) -> i32;
        let set_state: SetStateFn = vtbl_fn(obj, 10);
        match frac {
            Some(f) => {
                type SetValueFn =
                    unsafe extern "system" fn(*mut std::ffi::c_void, HWND, u64, u64) -> i32;
                let set_value: SetValueFn = vtbl_fn(obj, 9);
                let _ = set_state(obj, hwnd, TBPF_NORMAL);
                let _ = set_value(obj, hwnd, (f.clamp(0.0, 1.0) * 1000.0) as u64, 1000);
            }
            None => {
                let _ = set_state(obj, hwnd, TBPF_NOPROGRESS);
            }
        }
    }
}

// The allocation remains locked throughout the bounded copy. No clipboard state
// is modified, allowing this acquisition path to be tested with owned memory.
unsafe fn decode_global_text(data: windows_sys::Win32::Foundation::HGLOBAL) -> Option<String> {
    let units = super::clipboard_allocation_units(GlobalSize(data))?;
    let ptr = GlobalLock(data) as *const u16;
    if ptr.is_null() {
        return None;
    }
    let units = std::slice::from_raw_parts(ptr, units);
    let result = super::decode_clipboard_units(units);
    GlobalUnlock(data);
    result
}

/// Owns a kill-on-close job. The root is spawned suspended and resumed only
/// after successful assignment; no child code can escape before containment.
pub(crate) struct ProcessTree(HANDLE);
unsafe impl Send for ProcessTree {}
impl ProcessTree {
    pub(crate) fn spawn(
        command: &mut std::process::Command,
    ) -> std::io::Result<(std::process::Child, Self)> {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::JobObjects::*;
        use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let mut child = command
                .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
                .spawn()?;
            let result = if AssignProcessToJobObject(handle, child.as_raw_handle()) == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                resume_suspended_root(child.id())
            };
            if let Err(error) = result {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            Ok((child, job))
        }
    }
    pub(crate) fn terminate(&self) -> std::io::Result<()> {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        if unsafe { TerminateJobObject(self.0, 1) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            use windows_sys::Win32::System::JobObjects::{
                JobObjectBasicAccountingInformation, QueryInformationJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION =
                    unsafe { std::mem::zeroed() };
                if unsafe {
                    QueryInformationJobObject(
                        self.0,
                        JobObjectBasicAccountingInformation,
                        &mut accounting as *mut _ as *mut _,
                        std::mem::size_of_val(&accounting) as u32,
                        std::ptr::null_mut(),
                    )
                } == 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                if accounting.ActiveProcesses == 0 {
                    return Ok(());
                }
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "owned process tree did not acknowledge termination",
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

unsafe fn resume_suspended_root(pid: u32) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let mut entry: THREADENTRY32 = std::mem::zeroed();
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut found = Thread32First(snapshot, &mut entry);
    let mut result = Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "suspended root thread missing",
    ));
    while found != 0 {
        if entry.th32OwnerProcessID == pid {
            let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
            if thread.is_null() {
                result = Err(std::io::Error::last_os_error());
            } else {
                result = if ResumeThread(thread) == u32::MAX {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                };
                CloseHandle(thread);
            }
            break;
        }
        found = Thread32Next(snapshot, &mut entry);
    }
    CloseHandle(snapshot);
    result
}

/// Poll only our anonymous read pipe. Reading at most the bytes already
/// available prevents synchronous ReadFile from blocking shutdown forever if
/// native tree termination fails. The supervisor always joins these readers.
pub(crate) fn read_owned_pipe<R: std::io::Read + std::os::windows::io::AsRawHandle>(
    pipe: &mut R,
    buffer: &mut [u8],
    stop: &std::sync::atomic::AtomicBool,
) -> std::io::Result<usize> {
    use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_NO_DATA};
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    loop {
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(0);
        }
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                pipe.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32)
                || error.raw_os_error() == Some(ERROR_NO_DATA as i32)
            {
                return Ok(0);
            }
            return Err(error);
        }
        if available > 0 {
            let count = buffer.len().min(available as usize);
            return pipe.read(&mut buffer[..count]);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[cfg(test)]
mod geometry_cache_tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, UnregisterClassW, WM_CLOSE, WM_GETTEXT,
    };
    static SLOW_TEXT_CALLS: AtomicU32 = AtomicU32::new(0);
    unsafe extern "system" fn slow_window(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
    ) -> LRESULT {
        if message == WM_GETTEXT {
            SLOW_TEXT_CALLS.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(150));
        }
        if message == WM_DESTROY {
            PostQuitMessage(0);
            return 0;
        }
        DefWindowProcW(hwnd, message, w, l)
    }
    unsafe fn create(
        class: &[u16],
        title: &str,
        procedure: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
    ) -> HWND {
        let instance = GetModuleHandleW(std::ptr::null());
        let definition = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&definition);
        CreateWindowExW(
            0,
            class.as_ptr(),
            wide(title).as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            320,
            240,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    }
    #[test]
    fn geometry_cache_excludes_other_thread_text_and_invalidates_destroyed_windows() {
        let (tx, rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let foreign = std::thread::spawn(move || unsafe {
            let class = wide(&format!("ytTrayDL-slow-field-{}", GetCurrentThreadId()));
            let window = create(&class, "Owned other-thread sentinel", slow_window);
            tx.send(window as isize).unwrap();
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            UnregisterClassW(class.as_ptr(), GetModuleHandleW(std::ptr::null()));
            let _ = done_tx.send(());
        });
        let other = rx.recv_timeout(Duration::from_secs(2)).unwrap() as HWND;
        assert!(!other.is_null());
        let mut text = [0u16; 64];
        let control = Instant::now();
        unsafe {
            GetWindowTextW(other, text.as_mut_ptr(), text.len() as i32);
        }
        let control_duration = control.elapsed();
        SLOW_TEXT_CALLS.store(0, Ordering::Relaxed);
        let lookup = Instant::now();
        let missing = find_owned_window("Absent owned field window");
        let lookup_duration = lookup.elapsed();
        let crossed = SLOW_TEXT_CALLS.load(Ordering::Relaxed);
        unsafe {
            PostMessageW(other, WM_CLOSE, 0, 0);
        }
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        foreign.join().unwrap();
        assert!(
            control_duration >= Duration::from_millis(140),
            "owned stall negative control must be armed"
        );
        assert!(missing.is_none());
        assert_eq!(crossed, 0, "geometry must not query another thread's text");
        assert!(
            lookup_duration < Duration::from_millis(100),
            "geometry lookup entered the armed 150ms stall"
        );
        unsafe {
            let class = wide(&format!("ytTrayDL-cache-field-{}", GetCurrentThreadId()));
            let first = create(&class, "ytTrayDL", DefWindowProcW);
            assert_eq!(find_owned_window("ytTrayDL"), Some(first));
            assert_eq!(find_main_window(), Some(first));
            DestroyWindow(first);
            assert!(!valid_cached_window(first, GetCurrentThreadId(), 1));
            let second = create(&class, "ytTrayDL", DefWindowProcW);
            assert_eq!(find_owned_window("ytTrayDL"), Some(second));
            assert_eq!(find_main_window(), Some(second));
            DestroyWindow(second);
            UnregisterClassW(class.as_ptr(), GetModuleHandleW(std::ptr::null()));
        }
    }
}

#[cfg(test)]
mod owned_clipboard_tests {
    #[test]
    fn native_owned_allocation_is_bounded() {
        use windows_sys::Win32::System::Memory::*;
        unsafe {
            let data = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, 4);
            assert!(!data.is_null());
            let ptr = GlobalLock(data) as *mut u16;
            assert!(!ptr.is_null());
            *ptr = 65;
            *ptr.add(1) = 0;
            GlobalUnlock(data);
            assert_eq!(super::decode_global_text(data), Some("A".into()));
            let ptr = GlobalLock(data) as *mut u16;
            // Fill the actual rounded allocation; no terminator may be read
            // beyond the allocation even if Windows rounds its size upward.
            for i in 0..GlobalSize(data) / 2 {
                *ptr.add(i) = 65;
            }
            GlobalUnlock(data);
            assert_eq!(super::decode_global_text(data), None);
            windows_sys::Win32::Foundation::GlobalFree(data);
        }
    }
}

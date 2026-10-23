//! Platform glue. The library's logic is platform-independent;
//! everything that touches Win32 lives behind this module, with
//! functional fallbacks on other platforms so the app still builds
//! and can be UI-tested on Linux dev machines.
//!
//! Windows pieces:
//! - Clipboard change notification via `AddClipboardFormatListener`
//!   + `WM_CLIPBOARDUPDATE` on a message-only window (the approach
//!     Microsoft Learn recommends over the legacy clipboard viewer
//!     chain, and over polling).
//! - A single-instance mutex plus a named "show yourself" event:
//!   launching yttraydl.exe while it is already running (e.g. with
//!   the tray icon hidden) signals the running instance to restore
//!   its icon and open Settings.
//! - A global hotkey (Ctrl+Alt+G) registered on the same message
//!   window — the second recovery path for a hidden tray icon.
//! - Start-with-Windows via the per-user `Run` registry key.
//! - Opening folders/files through ShellExecuteW.

#[cfg(windows)]
mod windows_impl;

use std::path::Path;
use std::sync::mpsc::Receiver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformEvent {
    /// Clipboard text changed; carries the new text.
    ClipboardText(String),
    /// The user asked the running instance to show itself
    /// (second launch, `--show`, or the Ctrl+Alt+G hotkey).
    ShowRequested,
}

/// Owns the platform listeners. Dropping it stops the listener
/// thread and releases the single-instance mutex.
pub struct Platform {
    #[cfg(windows)]
    inner: windows_impl::Inner,
}

pub struct PlatformSetup {
    pub platform: Platform,
    pub events: Receiver<PlatformEvent>,
    /// True when another instance was already running and has been
    /// signalled — the caller should exit quietly without starting
    /// a second UI.
    pub already_running: bool,
}

/// Start platform services. `show_on_launch` is true when this
/// process was started with `--show` / `--settings`: if an instance
/// is already running it is told to show itself; if not, this
/// instance starts with Settings open.
pub fn setup(show_on_launch: bool) -> PlatformSetup {
    #[cfg(windows)]
    {
        windows_impl::setup(show_on_launch)
    }
    #[cfg(not(windows))]
    {
        let _ = show_on_launch;
        let (_tx, rx) = std::sync::mpsc::channel();
        // Keep the sender alive inside the (unused) channel pair by
        // leaking it: non-Windows builds are for UI development, and
        // clipboard monitoring there is a documented no-op.
        std::mem::forget(_tx);
        PlatformSetup {
            platform: Platform {},
            events: rx,
            already_running: false,
        }
    }
}

impl Platform {
    /// Enable/disable Start-with-Windows (HKCU Run key on Windows).
    pub fn set_autostart(enabled: bool) -> Result<(), String> {
        #[cfg(windows)]
        {
            windows_impl::set_autostart(enabled)
        }
        #[cfg(not(windows))]
        {
            let _ = enabled;
            Err("autostart is only supported on Windows".into())
        }
    }

    pub fn autostart_enabled() -> bool {
        #[cfg(windows)]
        {
            windows_impl::autostart_enabled()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

/// Open a folder or file with the OS default handler.
pub fn open_path(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_impl::open_path(path)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Open a web page in the default browser.
pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_impl::open_url(url)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Current cursor position in physical screen pixels, when the
/// platform can provide it (for the NearCursor popup position).
pub fn cursor_position() -> Option<(f32, f32)> {
    #[cfg(windows)]
    {
        windows_impl::cursor_position()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Read the current clipboard text on demand (Windows listener
/// path). On non-Windows dev builds this returns `None`.
pub fn current_clipboard_text() -> Option<String> {
    #[cfg(windows)]
    {
        windows_impl::current_clipboard_text()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Show and foreground the main window (titled "ytTrayDL") at
/// the OS level, if it exists and is currently hidden. Used by
/// the tray-icon event handler: restoring the window must not
/// depend on egui running a frame first. Returns true when a
/// hidden window was found and shown.
pub fn show_main_window() -> bool {
    #[cfg(windows)]
    {
        windows_impl::show_main_window()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Toggle the main window's visibility at the OS level
/// (0.2.8). Returns the NEW visibility (`Some(true)` = now
/// shown, `Some(false)` = now hidden), or `None` when the
/// window doesn't exist.
pub fn toggle_main_window() -> Option<bool> {
    #[cfg(windows)]
    {
        windows_impl::toggle_main_window()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Show a fatal startup error the user can actually see. A tray
/// app has no console, so without this a failed start looks like
/// "double-clicking does nothing".
pub fn fatal_error_box(title: &str, message: &str) {
    #[cfg(windows)]
    {
        windows_impl::fatal_error_box(title, message)
    }
    #[cfg(not(windows))]
    {
        eprintln!("{title}: {message}");
    }
}

/// Play the standard Windows notification sound (the
/// "download finished" chime). No-op off Windows.
pub fn play_notification_sound() {
    #[cfg(windows)]
    {
        windows_impl::play_notification_sound()
    }
}

/// Set the main window's taskbar-button progress: `Some(frac)`
/// (0.0–1.0) shows a progress bar at that fraction, `None`
/// removes it. No-op off Windows or when the window doesn't
/// exist (e.g. minimized to the tray — there is no button then).
pub fn set_taskbar_progress(frac: Option<f64>) {
    #[cfg(windows)]
    {
        windows_impl::set_taskbar_progress(frac)
    }
    #[cfg(not(windows))]
    {
        let _ = frac;
    }
}

/// Free bytes available to the current user on the volume
/// holding `path` (the low-disk-space guard). `None` when the
/// volume can't be queried.
pub fn free_disk_bytes(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        windows_impl::free_disk_bytes(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// Put the PC to sleep (the queue-empty action). Returns whether
/// the request was issued. No-op (false) off Windows.
pub fn suspend_system() -> bool {
    #[cfg(windows)]
    {
        windows_impl::suspend_system()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Run a user-configured command line through the system shell
/// (`cmd /C` on Windows, `sh -c` elsewhere), detached. Used by
/// the post-download command; the command is the user's own
/// setting, and placeholders are substituted by the caller.
pub fn run_shell_command(line: &str) {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", line]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", line]);
        c
    };
    let _ = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// True when this process was asked (by a later launch) to show
/// itself before `setup` ran — handled inside setup on Windows;
/// always false elsewhere.
pub fn launched_with_show_flag(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--show" || a == "--settings" || a == "/show")
}

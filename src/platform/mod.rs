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

/// True when this process was asked (by a later launch) to show
/// itself before `setup` ran — handled inside setup on Windows;
/// always false elsewhere.
pub fn launched_with_show_flag(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--show" || a == "--settings" || a == "/show")
}

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
    /// A later invocation supplied a URL explicitly, independent of monitoring.
    ManualUrl(String),
    /// Open the existing Downloads window from a direct launch.
    OpenDownloads,
    /// Open the local video converter from a direct launch.
    OpenConverter,
    /// Open the local lyrics and tags editor.
    OpenTags,
    /// Open the local player queue.
    OpenPlayer,
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
    setup_with_launch_intent(show_on_launch, false)
}

/// Set up the primary instance without sending the generic show event when a
/// secondary instance has a more specific launch intent to forward.
pub fn setup_with_launch_intent(show_on_launch: bool, has_launch_intent: bool) -> PlatformSetup {
    #[cfg(windows)]
    {
        windows_impl::setup(show_on_launch, has_launch_intent)
    }
    #[cfg(not(windows))]
    {
        let _ = (show_on_launch, has_launch_intent);
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

/// Deliver a direct Downloads-window request to the running instance.
pub fn forward_open_downloads() -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_impl::forward_open_downloads()
    }
    #[cfg(not(windows))]
    {
        Err("Launch intent forwarding is only supported on Windows".into())
    }
}

/// Deliver a direct video-converter request to the running instance.
pub fn forward_open_converter() -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_impl::forward_open_converter()
    }
    #[cfg(not(windows))]
    {
        Err("Launch intent forwarding is only supported on Windows".into())
    }
}

/// Deliver a direct lyrics and tags request to the running instance.
pub fn forward_open_tags() -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_impl::forward_open_tags()
    }
    #[cfg(not(windows))]
    {
        Err("Launch intent forwarding is only supported on Windows".into())
    }
}

/// Deliver a direct player request to the running instance.
pub fn forward_open_player() -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_impl::forward_open_player()
    }
    #[cfg(not(windows))]
    {
        Err("Launch intent forwarding is only supported on Windows".into())
    }
}

/// Deliver an explicit CLI URL to the running instance, with a bounded wait.
pub fn forward_manual_url(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_impl::forward_manual_url(url)
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("URL forwarding is only supported on Windows".into())
    }
}

#[cfg(any(windows, test))]
const MANUAL_URL_MESSAGE: usize = 0x59545501;
#[cfg(any(windows, test))]
const OPEN_DOWNLOADS_MESSAGE: usize = 0x59545502;
#[cfg(any(windows, test))]
const OPEN_CONVERTER_MESSAGE: usize = 0x59545503;
#[cfg(any(windows, test))]
const OPEN_TAGS_MESSAGE: usize = 0x59545504;
#[cfg(any(windows, test))]
const OPEN_PLAYER_MESSAGE: usize = 0x59545505;

#[cfg(any(windows, test))]
fn copydata_event(message: usize, bytes: &[u8]) -> Option<PlatformEvent> {
    if bytes.is_empty() || bytes.len() > MAX_MANUAL_URL_BYTES {
        return None;
    }
    match message {
        MANUAL_URL_MESSAGE => manual_url_payload(bytes).map(PlatformEvent::ManualUrl),
        OPEN_DOWNLOADS_MESSAGE if bytes == b"downloads" => Some(PlatformEvent::OpenDownloads),
        OPEN_CONVERTER_MESSAGE if bytes == b"converter" => Some(PlatformEvent::OpenConverter),
        OPEN_TAGS_MESSAGE if bytes == b"tags" => Some(PlatformEvent::OpenTags),
        OPEN_PLAYER_MESSAGE if bytes == b"player" => Some(PlatformEvent::OpenPlayer),
        _ => None,
    }
}

#[cfg(any(windows, test))]
const MAX_MANUAL_URL_BYTES: usize = 32 * 1024;

#[cfg(any(windows, test))]
fn manual_url_payload(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_MANUAL_URL_BYTES {
        return None;
    }
    let url = std::str::from_utf8(bytes).ok()?;
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    if !crate::url_detect::is_http_url(url) {
        return None;
    }
    Some(url.to_owned())
}

#[cfg(any(windows, test))]
fn existing_directory(path: &Path) -> Option<std::path::PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    absolute
        .ancestors()
        .find(|ancestor| ancestor.is_dir())
        .map(Path::to_owned)
}

/// Read a visible, normal owned window. Minimized/maximized/hidden geometry
/// never replaces the user's last normal bounds.
pub fn remember_main_window() -> bool {
    #[cfg(windows)]
    {
        windows_impl::remember_main_window()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Query one persistent window on its owning GUI thread.
pub fn window_geometry(
    kind: crate::window_geometry::WindowKind,
) -> Option<crate::window_geometry::WindowGeometry> {
    #[cfg(windows)]
    {
        windows_impl::window_geometry(kind)
    }
    #[cfg(not(windows))]
    {
        let _ = kind;
        None
    }
}

/// Available and current logical client sizes for an exact owned window.
/// The GUI caches this on open/monitor changes; it is not a frame-time probe.
pub fn window_client_capacity(
    kind: crate::window_geometry::WindowKind,
) -> Option<([f32; 2], [f32; 2])> {
    #[cfg(windows)]
    {
        windows_impl::window_client_capacity(kind)
    }
    #[cfg(not(windows))]
    {
        let _ = kind;
        None
    }
}

/// Restore normal bounds without showing, activating, or raising the window.
pub fn restore_window_geometry(
    kind: crate::window_geometry::WindowKind,
    geometry: crate::window_geometry::WindowGeometry,
) -> bool {
    #[cfg(windows)]
    {
        windows_impl::restore_window_geometry(kind, geometry)
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, geometry);
        false
    }
}

/// Align an auxiliary window to the main window once, clamped to its work area.
pub fn align_tool_window(window: crate::window_geometry::ToolWindow) -> bool {
    #[cfg(windows)]
    {
        windows_impl::align_tool_window(window)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        true
    }
}

/// Translate an exact owned transient window into its monitor work area.
pub fn constrain_transient_window(window: crate::window_geometry::TransientWindow) {
    #[cfg(windows)]
    {
        windows_impl::constrain_transient_window(window);
    }
    #[cfg(not(windows))]
    {
        let _ = window;
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

#[cfg(any(windows, test))]
fn clipboard_event_text(
    last_sequence: &mut u32,
    sequence: u32,
    read: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if sequence != 0 && sequence == *last_sequence {
        return None;
    }
    let text = read()?;
    *last_sequence = sequence;
    Some(text)
}

#[cfg(any(windows, test))]
fn clipboard_allocation_units(bytes: usize) -> Option<usize> {
    if bytes < 2 || !bytes.is_multiple_of(2) {
        None
    } else {
        Some((bytes / 2).min(1_000_000))
    }
}
/// Decode bounded UTF-16 clipboard units, requiring a terminator within the
/// one-million-unit cap. This pure helper performs no native clipboard access.
pub fn decode_clipboard_units(units: &[u16]) -> Option<String> {
    let bounded = &units[..units.len().min(1_000_000)];
    let end = bounded.iter().position(|unit| *unit == 0)?;
    Some(String::from_utf16_lossy(&bounded[..end]))
}

#[cfg(windows)]
pub(crate) use windows_impl::{read_owned_pipe, ProcessTree};

#[cfg(test)]
mod clipboard_tests {
    use super::{clipboard_allocation_units, clipboard_event_text, decode_clipboard_units};
    #[test]
    fn module_intents_reject_cross_module_and_malformed_payloads() {
        use super::{
            copydata_event, PlatformEvent, OPEN_CONVERTER_MESSAGE, OPEN_DOWNLOADS_MESSAGE,
            OPEN_PLAYER_MESSAGE, OPEN_TAGS_MESSAGE,
        };
        let messages = [
            (
                OPEN_DOWNLOADS_MESSAGE,
                b"downloads".as_slice(),
                PlatformEvent::OpenDownloads,
            ),
            (
                OPEN_CONVERTER_MESSAGE,
                b"converter".as_slice(),
                PlatformEvent::OpenConverter,
            ),
            (
                OPEN_TAGS_MESSAGE,
                b"tags".as_slice(),
                PlatformEvent::OpenTags,
            ),
            (
                OPEN_PLAYER_MESSAGE,
                b"player".as_slice(),
                PlatformEvent::OpenPlayer,
            ),
        ];
        for (message, payload, expected) in &messages {
            assert_eq!(copydata_event(*message, payload), Some(expected.clone()));
            assert_eq!(copydata_event(*message, b""), None);
            let mut trailing = payload.to_vec();
            trailing.push(0);
            assert_eq!(copydata_event(*message, &trailing), None);
            assert_eq!(
                copydata_event(*message, &payload.to_ascii_uppercase()),
                None
            );
            assert_eq!(
                copydata_event(*message, &vec![b'x'; super::MAX_MANUAL_URL_BYTES + 1]),
                None
            );
            for (other_message, _, _) in &messages {
                if other_message != message {
                    assert_eq!(copydata_event(*other_message, payload), None);
                }
            }
        }
    }

    #[test]
    fn launch_intents_are_separate_from_urls_and_bounded() {
        use super::{
            copydata_event, PlatformEvent, MANUAL_URL_MESSAGE, OPEN_CONVERTER_MESSAGE,
            OPEN_DOWNLOADS_MESSAGE,
        };
        assert_eq!(
            super::copydata_event(super::OPEN_PLAYER_MESSAGE, b"player"),
            Some(PlatformEvent::OpenPlayer)
        );
        assert_eq!(
            super::copydata_event(super::OPEN_PLAYER_MESSAGE, b"tags"),
            None
        );
        assert_eq!(
            super::copydata_event(super::OPEN_TAGS_MESSAGE, b"tags"),
            Some(PlatformEvent::OpenTags)
        );
        assert_eq!(
            super::copydata_event(super::OPEN_TAGS_MESSAGE, b"converter"),
            None
        );
        assert_eq!(
            copydata_event(OPEN_CONVERTER_MESSAGE, b"converter"),
            Some(PlatformEvent::OpenConverter)
        );
        assert_eq!(copydata_event(OPEN_CONVERTER_MESSAGE, b"downloads"), None);
        assert_eq!(
            copydata_event(OPEN_DOWNLOADS_MESSAGE, b"downloads"),
            Some(PlatformEvent::OpenDownloads)
        );
        assert_eq!(
            copydata_event(OPEN_DOWNLOADS_MESSAGE, b"https://example.com"),
            None
        );
        assert_eq!(copydata_event(MANUAL_URL_MESSAGE, b"downloads"), None);
        assert_eq!(copydata_event(0, b"downloads"), None);
        assert_eq!(
            copydata_event(
                OPEN_DOWNLOADS_MESSAGE,
                &vec![b'x'; super::MAX_MANUAL_URL_BYTES + 1]
            ),
            None
        );
        assert_eq!(
            copydata_event(MANUAL_URL_MESSAGE, b"https://example.com"),
            Some(PlatformEvent::ManualUrl("https://example.com".to_string()))
        );
    }

    #[test]
    fn forwarded_url_requires_bounded_http_payload() {
        for invalid in [
            b"".as_slice(),
            b"httpx://host",
            b"https:///path",
            b"https://host/\0x",
            b"https://host/ two",
            &[0xff],
        ] {
            assert_eq!(super::manual_url_payload(invalid), None);
        }
        assert_eq!(
            super::manual_url_payload(&vec![b'a'; super::MAX_MANUAL_URL_BYTES + 1]),
            None
        );
        for valid in ["https://example.invalid/audit-cli", "http://host/path"] {
            assert_eq!(
                super::manual_url_payload(valid.as_bytes()),
                Some(valid.into())
            );
        }
    }
    #[test]
    fn disk_probe_uses_existing_directory_for_new_or_file_path() {
        let root = std::env::temp_dir();
        let missing = root
            .join(format!("yttraydl-absent-{}", std::process::id()))
            .join("nested");
        assert_eq!(super::existing_directory(&missing), Some(root));
        let executable = std::env::current_exe().unwrap();
        assert_eq!(
            super::existing_directory(&executable),
            executable.parent().map(std::path::Path::to_owned)
        );
    }
    #[test]
    fn failed_event_read_does_not_consume_sequence() {
        let mut last = 0;
        assert_eq!(clipboard_event_text(&mut last, 42, || None), None);
        assert_eq!(last, 0);
        assert_eq!(
            clipboard_event_text(&mut last, 42, || Some("url".into())),
            Some("url".into())
        );
        assert_eq!(
            clipboard_event_text(&mut last, 42, || panic!("duplicate read")),
            None
        );
        assert_eq!(
            clipboard_event_text(&mut last, 43, || Some("next".into())),
            Some("next".into())
        );
    }
    #[test]
    fn clipboard_allocation_must_hold_whole_units() {
        assert_eq!(clipboard_allocation_units(0), None);
        assert_eq!(clipboard_allocation_units(1), None);
        assert_eq!(clipboard_allocation_units(3), None);
        assert_eq!(clipboard_allocation_units(2), Some(1));
        assert_eq!(clipboard_allocation_units(usize::MAX - 1), Some(1_000_000));
    }
    #[test]
    fn bounded_clipboard_requires_terminator() {
        assert_eq!(decode_clipboard_units(&[]), None);
        assert_eq!(decode_clipboard_units(&[65]), None);
        assert_eq!(decode_clipboard_units(&[0]), Some(String::new()));
        assert_eq!(decode_clipboard_units(&[65, 0, 66]), Some("A".into()));
        let mut units = vec![65; 1_000_001];
        units[1_000_000] = 0;
        assert_eq!(decode_clipboard_units(&units), None);
        units[999_999] = 0;
        assert_eq!(decode_clipboard_units(&units).unwrap().len(), 999_999);
    }
}

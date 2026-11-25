//! egui front-end: tray icon + native tray menu, the control panel
//! (the root viewport, hidden until asked for), and the viewports
//! implemented in `gui_windows` (popup, settings, downloads, toasts).
//!
//! Shared dark theme uses FluentGruv's warm surfaces and red accent.
//! Broader light-theme, DPI and accessibility validation remain in progress.

mod converter_window;
mod dialogs;
#[cfg(feature = "field-validation")]
mod field_metrics;
mod gui_windows;
mod player_window;
mod settings_layout;
mod tags_window;
mod theme;
mod widgets;
pub use dialogs::*;
pub use theme::*;
pub use widgets::*;

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};
use yttraydl::dedupe::SeenCache;
use yttraydl::download::{DownloadEvent, DownloadManager, JobState};
use yttraydl::history::{now_unix, History, HistoryEntry};
#[cfg(windows)]
use yttraydl::platform;
use yttraydl::platform::{PlatformEvent, PlatformSetup};
use yttraydl::rules::{decide, Decision};
#[cfg(windows)]
use yttraydl::settings::DefaultAction;
use yttraydl::settings::{CloseAction, Preset, Settings};
use yttraydl::url_detect::{extract_urls, host_of};
use yttraydl::window_geometry::{WindowGeometries, WindowKind};
use yttraydl::ytdlp::{self, VideoInfo, YtDlpBinary};

#[derive(Clone)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub expires: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

pub struct PopupRequest {
    pub url: String,
    pub host: String,
    pub info: Option<VideoInfo>,
    pub info_loading: bool,
    pub already_downloaded: bool,
    pub preset: Preset,
    pub always_auto_domain: bool,
    /// Playlist choice for this download (whole playlist vs
    /// just the linked item); starts from the global setting,
    /// the user can flip it in the popup for playlist links.
    pub playlist_all: bool,
    /// Per-download output folder chosen in the popup;
    /// `None` = the settings download folder.
    pub folder: Option<std::path::PathBuf>,
    pub shown_at: Instant,
    pub last_interaction: Instant,
}

type InfoResult = (String, Option<VideoInfo>, bool);
type FetchResult = Result<std::path::PathBuf, String>;
fn take_tool_terminal(receiver: &mut Option<Receiver<FetchResult>>) -> Option<FetchResult> {
    let result = match receiver.as_ref()?.try_recv() {
        Ok(result) => result,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Err("Tool worker stopped unexpectedly. Please retry.".into())
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => return None,
    };
    *receiver = None;
    Some(result)
}
struct ToolOperation {
    id: u64,
    mailbox: yttraydl::tools::ProgressMailbox,
    progress: yttraydl::tools::ToolProgress,
    cancel: yttraydl::process::CancellationToken,
    cancelling: bool,
}
#[cfg(test)]
mod tool_terminal_tests {
    use super::*;
    #[test]
    fn disconnected_worker_clears_busy_and_allows_retry() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut busy = Some(receiver);
        assert!(take_tool_terminal(&mut busy).is_none());
        assert!(busy.is_some());
        drop(sender);
        assert!(take_tool_terminal(&mut busy)
            .unwrap()
            .unwrap_err()
            .contains("retry"));
        assert!(busy.is_none());
        let (sender, receiver) = std::sync::mpsc::channel();
        busy = Some(receiver);
        sender
            .send(Ok(std::path::PathBuf::from("verified-tool.exe")))
            .unwrap();
        assert_eq!(
            take_tool_terminal(&mut busy).unwrap().unwrap(),
            std::path::PathBuf::from("verified-tool.exe")
        );
        assert!(busy.is_none());
    }
}
type ToolStatusResult = (u64, yttraydl::tools::ToolStatusSnapshot, bool);
/// Watcher poll result: (subscription URL, newest entries) per sub.
type WatchResult = Vec<(String, Vec<yttraydl::ytdlp::PlaylistEntry>)>;
/// Thumbnail fetch result: (source URL, decoded (w, h, RGBA)).
type ThumbResult = (String, Option<(u32, u32, Vec<u8>)>);

/// A final download failure surfaced in the panel with a Retry
/// button (0.4.0). `job_id` refers to the finished-list entry
/// the retry requeues.
#[derive(Clone)]
pub struct FailureNotice {
    pub job_id: u64,
    pub title: String,
    pub error: String,
}

#[cfg(windows)]
pub struct TrayHandles {
    _tray: tray_icon::TrayIcon,
    monitoring: tray_icon::menu::CheckMenuItem,
    auto_mode: tray_icon::menu::CheckMenuItem,
    autostart: tray_icon::menu::CheckMenuItem,
    /// Pause all / Resume all (one item; its label follows the
    /// queue state — see `sync_tray`).
    pause_all: tray_icon::menu::MenuItem,
    id_monitoring: tray_icon::menu::MenuId,
    id_auto: tray_icon::menu::MenuId,
    id_autostart: tray_icon::menu::MenuId,
    id_download_clipboard: tray_icon::menu::MenuId,
    id_downloads: tray_icon::menu::MenuId,
    id_converter: tray_icon::menu::MenuId,
    id_tags: tray_icon::menu::MenuId,
    id_player: tray_icon::menu::MenuId,
    id_open_folder: tray_icon::menu::MenuId,
    id_pause_all: tray_icon::menu::MenuId,
    id_settings: tray_icon::menu::MenuId,
    id_hide_icon: tray_icon::menu::MenuId,
    id_about: tray_icon::menu::MenuId,
    id_exit: tray_icon::menu::MenuId,
}

/// Tray UI events, delivered by the tray-icon / muda event
/// handlers installed in `ensure_tray` (0.2.4).
///
/// Why handlers: the documented integration for winit-based
/// apps (eframe is winit underneath) is
/// `TrayIconEvent::set_event_handler` /
/// `MenuEvent::set_event_handler` forwarding into the app's
/// own queue. Polling the global `receiver()` channels never
/// delivered a single event under eframe on a real machine —
/// field-proven in 0.2.3, whose log shows a clean launch and
/// zero tray clicks while the user clicked Exit on the menu;
/// every "tray doesn't respond" report traces back to this.
/// The handlers also wake the UI directly, so tray clicks act
/// immediately instead of within the 200 ms repaint poll.
#[cfg(windows)]
enum TrayUiEvent {
    Icon(tray_icon::TrayIconEvent),
    Menu(tray_icon::menu::MenuEvent),
    /// The handler toggled the main window's visibility at OS
    /// level (0.2.8); the app adopts the result as its
    /// `show_panel` state so the two never diverge.
    WindowToggled(bool),
}

/// Sender half shared with the static event handlers; the
/// receiver lives on `App`. A replaceable slot, NOT a
/// set-once cell (0.2.6): if the app object is ever recreated,
/// the handlers must feed the LIVE receiver — a set-once cell
/// would strand every send on the first, dropped channel, and
/// `send` errors were being swallowed by `let _ =`.
#[cfg(windows)]
static TRAY_EVENT_TX: std::sync::Mutex<Option<Sender<TrayUiEvent>>> = std::sync::Mutex::new(None);

/// Last tray left-click toggle time (0.2.8): Windows delivers
/// a second Up event for a double-click, which must not
/// toggle the window twice.
#[cfg(windows)]
static LAST_TRAY_TOGGLE: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// Tray-delivery diagnostic counters (0.2.6): handler sends
/// vs app drains, so the log proves which leg is alive.
#[cfg(windows)]
static TRAY_SENDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(windows)]
static TRAY_SEND_ERRS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(windows)]
static TRAY_DRAINED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// The live tray icon + menu items, shared between `App`
// (which creates and syncs them) and the tray event handlers
// (which, since 0.2.7, EXECUTE most tray actions directly —
// see `handle_tray_menu_direct`). tray-icon/muda handles are
// `Rc`-based and thread-affine: creation (in `update`) and
// handler dispatch (from winit's message pump) both happen
// on the main thread, so a thread-local cell is the correct
// sharing mechanism. If a handler ever ran on another
// thread it would see an empty cell and fall back to
// forwarding the event to the app — a safe degradation.
#[cfg(windows)]
thread_local! {
    static TRAY_HANDLES: std::cell::RefCell<Option<TrayHandles>> =
        const { std::cell::RefCell::new(None) };
}

/// Read access to the shared tray handles, when they exist
/// on this thread.
#[cfg(windows)]
fn with_tray_handles<R>(f: impl FnOnce(&TrayHandles) -> R) -> Option<R> {
    TRAY_HANDLES.with(|cell| cell.borrow().as_ref().map(f))
}

/// Mutable access to the shared tray-handles slot (install /
/// remove the handles themselves).
#[cfg(windows)]
fn with_tray_handles_mut<R>(f: impl FnOnce(&mut Option<TrayHandles>) -> R) -> R {
    TRAY_HANDLES.with(|cell| f(&mut cell.borrow_mut()))
}

/// Human name for a tray menu item id (diagnostics).
#[cfg(windows)]
fn tray_item_name(t: &TrayHandles, id: &tray_icon::menu::MenuId) -> &'static str {
    if *id == t.id_monitoring {
        "monitoring toggle"
    } else if *id == t.id_auto {
        "auto-download toggle"
    } else if *id == t.id_autostart {
        "autostart toggle"
    } else if *id == t.id_download_clipboard {
        "download from clipboard"
    } else if *id == t.id_downloads {
        "downloads"
    } else if *id == t.id_converter {
        "video converter"
    } else if *id == t.id_tags {
        "lyrics and tags"
    } else if *id == t.id_player {
        "player"
    } else if *id == t.id_open_folder {
        "open download folder"
    } else if *id == t.id_pause_all {
        "pause all downloads"
    } else if *id == t.id_settings {
        "settings"
    } else if *id == t.id_hide_icon {
        "hide tray icon"
    } else if *id == t.id_about {
        "about"
    } else if *id == t.id_exit {
        "exit"
    } else {
        "unknown"
    }
}

/// Execute a tray-menu action IN THE HANDLER (0.2.7).
///
/// Why: the 0.2.5/0.2.6 field logs prove the handlers fire
/// for every click, while egui passes may never run again
/// once the tray's modal context menu has been shown —
/// eframe 0.33's repaint scheduling (delay dedup +
/// cumulative-pass staleness in its winit runner) is left
/// disarmed by the menu's nested message loop: winit keeps
/// dispatching window messages (handlers keep logging) but
/// no further pass is scheduled, so anything queued for
/// `update()` waits forever. Actions that only need Win32,
/// the registry, or the settings FILE therefore run here,
/// where the click actually arrives. The app picks the
/// file-level changes up via its settings-mtime reload.
/// Returns true when the action was fully handled here;
/// false means "forward it to the app" (actions that need
/// live app state: popup, downloads window, settings
/// window, about).
#[cfg(windows)]
fn handle_tray_menu_direct(event: &tray_icon::menu::MenuEvent) -> bool {
    let name = match with_tray_handles(|t| tray_item_name(t, &event.id)) {
        Some(n) => n,
        None => return false,
    };
    match name {
        "monitoring toggle" => {
            let mut s = Settings::load();
            s.monitoring_enabled = !s.monitoring_enabled;
            let v = s.monitoring_enabled;
            match s.save() {
                Ok(()) => {
                    // No set_checked here (0.5.1): mutating a muda
                    // menu item inside muda's own dispatch is the
                    // RefCell-borrowed panic class proven in the
                    // 0.2.7 field log, and the prime suspect for
                    // the tray autostart untick exiting the
                    // program on v0.5.0. sync_tray (app context)
                    // owns all checkmarks and follows the
                    // settings file this save just wrote.
                    yttraydl::diag_log(&format!(
                        "tray menu click: monitoring toggle -> {v} (handler)"
                    ));
                }
                Err(e) => yttraydl::diag_log(&format!(
                    "tray monitoring toggle: settings save failed: {e}"
                )),
            }
            true
        }
        "auto-download toggle" => {
            let mut s = Settings::load();
            let now_auto = s.default_action != DefaultAction::AutoDownload;
            s.default_action = if now_auto {
                DefaultAction::AutoDownload
            } else {
                DefaultAction::Prompt
            };
            match s.save() {
                Ok(()) => {
                    // No set_checked in-dispatch (0.5.1) — see
                    // the monitoring branch. sync_tray owns it.
                    yttraydl::diag_log(&format!(
                        "tray menu click: auto-download toggle -> {now_auto} (handler)"
                    ));
                }
                Err(e) => yttraydl::diag_log(&format!(
                    "tray auto-download toggle: settings save failed: {e}"
                )),
            }
            true
        }
        "autostart toggle" => {
            // Step-by-step logging (0.5.1): SEAN reported that
            // unticking this on v0.5.0 exited the program. If it
            // ever dies mid-toggle again, the log names the last
            // completed step. NO muda mutation in here (no
            // set_checked in-dispatch — see the monitoring
            // branch); sync_tray owns the checkmark and reads it
            // back from the registry.
            let current = platform::Platform::autostart_enabled();
            let target = !current;
            yttraydl::diag_log(&format!(
                "tray autostart toggle: current={current} -> target={target} (handler)"
            ));
            match platform::Platform::set_autostart(target) {
                Ok(()) => {
                    yttraydl::diag_log(&format!(
                        "tray autostart toggle: registry updated -> {target}"
                    ));
                    let mut s = Settings::load();
                    s.start_with_windows = target;
                    match s.save() {
                        Ok(()) => {
                            yttraydl::diag_log("tray autostart toggle: settings saved");
                        }
                        Err(e) => {
                            yttraydl::diag_log(&format!(
                                "tray autostart toggle: settings save failed: {e}"
                            ));
                        }
                    }
                    yttraydl::diag_log(&format!(
                        "tray menu click: autostart toggle -> {target} (handler)"
                    ));
                }
                Err(e) => {
                    yttraydl::diag_log(&format!("tray autostart toggle failed: {e}"));
                }
            }
            true
        }
        "open download folder" => {
            let folder = Settings::load().download_folder;
            let _ = std::fs::create_dir_all(&folder);
            match platform::open_path(&folder) {
                Ok(()) => {
                    yttraydl::diag_log("tray menu click: open download folder (handler)");
                }
                Err(e) => {
                    yttraydl::diag_log(&format!("tray open folder failed: {e}"));
                }
            }
            true
        }
        "hide tray icon" => {
            // set_visible stays in the handler (judgment call,
            // 0.5.1): it mutates the tray ICON itself, not a menu
            // item, and this exact call has worked in the field
            // since 0.2.7 — the checkmark calls above are the
            // ones that moved out of the dispatch.
            with_tray_handles(|t| {
                t._tray.set_visible(false).ok();
            });
            let mut s = Settings::load();
            s.tray_icon_visible = false;
            let _ = s.save();
            yttraydl::diag_log("tray menu click: hide tray icon (handler)");
            true
        }
        _ => false,
    }
}

/// Current mtime of the settings file, if it exists (0.2.7).
fn settings_file_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(Settings::settings_path())
        .and_then(|m| m.modified())
        .ok()
}

/// Forward one handler-produced event into the app channel,
/// counting the send and logging failures (never silent).
#[cfg(windows)]
fn forward_tray_event(event: TrayUiEvent) {
    use std::sync::atomic::Ordering;
    TRAY_SENDS.fetch_add(1, Ordering::Relaxed);
    let tx = TRAY_EVENT_TX.lock().ok().and_then(|slot| slot.clone());
    match tx {
        Some(tx) => {
            if tx.send(event).is_err() {
                TRAY_SEND_ERRS.fetch_add(1, Ordering::Relaxed);
                yttraydl::diag_log("tray event send FAILED (no live receiver)");
            }
        }
        None => {
            TRAY_SEND_ERRS.fetch_add(1, Ordering::Relaxed);
            yttraydl::diag_log("tray event send FAILED (no channel installed)");
        }
    }
}

pub struct App {
    pub settings: Settings,
    pub settings_dirty: bool,
    platform: PlatformSetup,
    seen: SeenCache,
    pub downloads: DownloadManager,
    background: yttraydl::BackgroundTasks,
    pub history: History,
    bin: Option<YtDlpBinary>,
    pub ffmpeg_ok: bool,
    ytdlp_version: Option<String>,
    version_requested_for: Option<std::path::PathBuf>,
    version_rx: Option<Receiver<String>>,

    tray_failed: bool,
    tray_visible: bool,
    /// Whether the tray's pause-all item currently reads
    /// "Resume all downloads" (kept in sync by `sync_tray`).
    #[cfg_attr(not(windows), allow(dead_code))]
    tray_pause_label_resume: bool,
    /// Receiving end of the handler-fed tray event channel
    /// (0.2.4); created by `ensure_tray` together with the
    /// handlers.
    #[cfg(windows)]
    tray_event_rx: Option<Receiver<TrayUiEvent>>,
    #[cfg(not(windows))]
    #[allow(dead_code)]
    tray_event_rx: Option<()>,

    pub show_panel: bool,
    pub show_settings: bool,
    /// Last autostart checkmark state pushed to the tray menu
    /// by `sync_tray` (0.5.1); `None` = not pushed yet. The tray
    /// handlers no longer touch muda checkmarks inside muda's
    /// dispatch — `sync_tray` owns them (see
    /// `handle_tray_menu_direct`).
    #[cfg_attr(not(windows), allow(dead_code))]
    tray_autostart_checked: Option<bool>,
    /// Last tooltip text + Monitoring/Auto checkmarks pushed by
    /// `sync_tray` (perf): muda setters reach Win32 on every
    /// call and `sync_tray` runs on every update pass (~5/s), so
    /// it pushes only on change, like the pause-all label and
    /// the autostart checkmark already did. `None` = not pushed.
    #[cfg_attr(not(windows), allow(dead_code))]
    tray_tooltip_sent: Option<String>,
    #[cfg_attr(not(windows), allow(dead_code))]
    tray_monitoring_checked: Option<bool>,
    #[cfg_attr(not(windows), allow(dead_code))]
    tray_auto_checked: Option<bool>,
    pub show_downloads: bool,
    pub show_converter: bool,
    pub show_tags: bool,
    pub show_player: bool,
    player: player_window::PlayerState,
    tags: tags_window::TagsState,
    converter: converter_window::ConverterState,
    pub show_about: bool,
    pub confirm_hide_icon: bool,
    /// The "closing the window — what should happen?" dialog.
    pub confirm_close: bool,
    /// The dialog's "remember my choice" checkbox state.
    pub close_remember: bool,
    settings_tab: usize,
    settings_sections: [usize; 4],
    settings_anchors: [usize; 4],
    settings_collection_pages: [usize; 6],
    settings_viewport_capacity: Option<[f32; 2]>,
    settings_viewport_monitor: Option<(egui::Vec2, f32)>,
    settings_viewport_refined: bool,
    pub exit_requested: bool,

    popup_queue: VecDeque<PopupRequest>,
    pub active_popup: Option<PopupRequest>,
    info_tx: Sender<InfoResult>,
    info_rx: Receiver<InfoResult>,
    ytdlp_fetch_rx: Option<Receiver<FetchResult>>,
    ffmpeg_fetch_rx: Option<Receiver<FetchResult>>,
    ytdlp_operation: Option<ToolOperation>,
    ffmpeg_operation: Option<ToolOperation>,
    next_tool_operation: u64,
    tool_status_rx: Option<Receiver<ToolStatusResult>>,
    tool_status_generation: u64,
    tool_status_ready: bool,
    ffmpeg_managed: bool,
    autostart_status: bool,
    was_settings_open: bool,
    app_dir: Option<std::path::PathBuf>,
    repaint: egui::Context,
    main_window_remembered: bool,
    scroll_to_tool_progress: bool,

    pub toasts: Vec<Toast>,
    pub last_ignored: Vec<(String, String)>,
    // Domain-rule editor input buffers (Settings → Domains).
    pub rule_input_ignore: String,
    pub rule_input_auto: String,
    pub rule_input_allow: String,
    pub rule_input_url: String,
    pub rule_input_preset_domain: String,
    pub rule_preset_choice: Preset,
    /// yt-dlp update check (Settings → yt-dlp): the in-flight
    /// request and its result ("latest release tag" lookup).
    ytdlp_check_rx: Option<Receiver<Result<String, String>>>,
    pub ytdlp_latest: Option<Result<String, String>>,
    /// Last taskbar-progress value sent to the platform layer
    /// (-2 = never sent, -1 = cleared, 0..=1000 = fraction).
    taskbar_sent: i32,
    /// The most recent final download failure, shown as a panel
    /// banner with a one-click Retry (0.4.0).
    pub failure_notice: Option<FailureNotice>,
    /// "Save for later" links parked from the popup (0.4.0),
    /// persisted via `saved.rs`.
    pub saved_links: Vec<yttraydl::saved::SavedLink>,
    /// Channel-watcher bookkeeping (seen ids + last poll).
    watch_data: yttraydl::saved::WatchData,
    /// In-flight watcher poll: per-subscription newest entries.
    watch_rx: Option<Receiver<WatchResult>>,
    /// Watcher subscription editor inputs (Settings → Domains).
    pub rule_input_watch_url: String,
    pub rule_watch_preset: Preset,
    /// Set by Settings ("Check watched channels now").
    pub watch_check_now: bool,
    /// In-flight popup thumbnail fetch: (for URL, decoded RGBA).
    thumb_rx: Option<Receiver<ThumbResult>>,
    thumb_attempt: yttraydl::thumbnail::Attempt,
    /// The decoded popup thumbnail, keyed by its source URL.
    pub popup_thumb: Option<(String, egui::TextureHandle)>,
    /// The queue had active work at some point since it last
    /// drained (gates the queue-empty action so it never fires
    /// on a merely idle app).
    had_active_downloads: bool,
    /// The low-disk toast has fired for the current low spell.
    disk_warned: bool,
    pub reset_armed: bool,
    manual_url: String,
    first_frame: bool,
    open_panel_on_start: bool,
    /// Frame counter + last heartbeat time for the tray
    /// diagnostics in `update` (0.2.6).
    #[cfg_attr(not(windows), allow(dead_code))]
    frames: u64,
    #[cfg_attr(not(windows), allow(dead_code))]
    last_heartbeat: Instant,
    /// Settings-file mtime tracking (0.2.7): the tray handlers
    /// apply toggles to the settings file directly, so the app
    /// reloads the file when it changes on disk instead of
    /// overwriting the handler's newer state on its next save.
    settings_mtime: Option<std::time::SystemTime>,
    last_settings_check: Instant,
    window_geometries: WindowGeometries,
    window_geometry_ready: [bool; 4],
    window_geometry_dirty: bool,
    last_geometry_save: Instant,
    /// A creation hint only; queue changes must not resize an open window.
    downloads_initial_size: Option<[f32; 2]>,
    #[cfg(feature = "field-validation")]
    field_metrics: field_metrics::Metrics,
}

/// Reopen an existing module without resetting its state; repeated requests
/// ask the window manager to focus its viewport rather than creating another.
fn request_module(ctx: &egui::Context, visible: &mut bool, viewport: &'static str) {
    if *visible {
        ctx.send_viewport_cmd_to(
            egui::ViewportId::from_hash_of(viewport),
            egui::ViewportCommand::Focus,
        );
    }
    *visible = true;
    ctx.request_repaint();
}

impl App {
    fn restore_new_window_geometry(&mut self, kind: WindowKind) {
        let index = kind.index();
        if self.window_geometry_ready[index] {
            return;
        }
        self.window_geometry_ready[index] = if let Some(saved) = self.window_geometries.get(kind) {
            yttraydl::platform::restore_window_geometry(kind, saved)
        } else if matches!(kind, WindowKind::Main) {
            // Also fit a first-run fixed panel to unusually small work areas.
            yttraydl::platform::window_geometry(kind)
                .is_some_and(|current| yttraydl::platform::restore_window_geometry(kind, current))
        } else {
            yttraydl::platform::window_geometry(kind).is_some()
        };
    }
    /// Restore each persistent window once per opening. Capture only visible
    /// normal bounds, before close/hide intent changes native visibility.
    fn maintain_window_geometry(&mut self, force: bool) {
        for kind in [
            WindowKind::Main,
            WindowKind::Settings,
            WindowKind::Downloads,
            WindowKind::About,
        ] {
            let index = kind.index();
            let open = match kind {
                WindowKind::Main => self.show_panel,
                WindowKind::Settings => self.show_settings,
                WindowKind::Downloads => self.show_downloads,
                WindowKind::About => self.show_about,
            };
            if !open {
                self.window_geometry_ready[index] = false;
                continue;
            }
            if !self.window_geometry_ready[index] {
                self.restore_new_window_geometry(kind);
                continue;
            }
            if let Some(current) = yttraydl::platform::window_geometry(kind) {
                self.window_geometry_dirty |= self.window_geometries.set(kind, current);
            }
        }
        if self.window_geometry_dirty
            && (force || self.last_geometry_save.elapsed() >= Duration::from_millis(500))
        {
            match self.window_geometries.save() {
                Ok(()) => {
                    self.window_geometry_dirty = false;
                    self.last_geometry_save = Instant::now();
                }
                Err(error) => {
                    yttraydl::diag_log(&format!("saving window geometry failed: {error}"));
                    self.last_geometry_save = Instant::now();
                }
            }
        }
    }

    pub fn new(
        cc: &eframe::CreationContext<'_>,
        platform_setup: PlatformSetup,
        show_on_launch: bool,
        manual_url: Option<String>,
    ) -> Self {
        apply_theme(&cc.egui_ctx);
        let main_window_remembered = yttraydl::platform::remember_main_window();
        // 0.2.6 diagnostics: if this line ever appears twice in
        // one launch, the app object was recreated — the prime
        // suspect for tray events being sent to a dead channel.
        yttraydl::diag_log("app created");
        let settings = Settings::load();
        let seen = SeenCache::new(Duration::from_secs(settings.dedupe_window_secs));
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let bin = None;
        let ffmpeg_ok = false;
        let (info_tx, info_rx) = std::sync::mpsc::channel();
        let initial_autostart = settings.start_with_windows;
        let mut app = Self {
            tray_visible: settings.tray_icon_visible,
            settings,
            settings_dirty: false,
            platform: platform_setup,
            seen,
            downloads: DownloadManager::new(),
            background: yttraydl::BackgroundTasks::default(),
            history: History::load(),
            bin,
            ffmpeg_ok,
            ytdlp_version: None,
            version_requested_for: None,
            version_rx: None,
            tray_failed: false,
            tray_pause_label_resume: false,
            tray_event_rx: None,
            show_panel: false,
            show_settings: false,
            tray_autostart_checked: None,
            tray_tooltip_sent: None,
            tray_monitoring_checked: None,
            tray_auto_checked: None,
            show_downloads: false,
            show_converter: false,
            show_tags: false,
            show_player: false,
            player: player_window::PlayerState::default(),
            tags: tags_window::TagsState::default(),
            converter: converter_window::ConverterState::default(),
            show_about: false,
            confirm_hide_icon: false,
            confirm_close: false,
            close_remember: false,
            settings_tab: 0,
            settings_sections: [0; 4],
            settings_anchors: [0; 4],
            settings_collection_pages: [0; 6],
            settings_viewport_capacity: None,
            settings_viewport_monitor: None,
            settings_viewport_refined: false,
            exit_requested: false,
            popup_queue: VecDeque::new(),
            active_popup: None,
            info_tx,
            info_rx,
            ytdlp_fetch_rx: None,
            ffmpeg_fetch_rx: None,
            ytdlp_operation: None,
            ffmpeg_operation: None,
            next_tool_operation: 0,
            tool_status_rx: None,
            tool_status_generation: 0,
            tool_status_ready: false,
            ffmpeg_managed: false,
            autostart_status: initial_autostart,
            was_settings_open: false,
            app_dir: exe_dir,
            repaint: cc.egui_ctx.clone(),
            main_window_remembered,
            scroll_to_tool_progress: false,
            toasts: Vec::new(),
            last_ignored: Vec::new(),
            rule_input_ignore: String::new(),
            rule_input_auto: String::new(),
            rule_input_allow: String::new(),
            rule_input_url: String::new(),
            rule_input_preset_domain: String::new(),
            rule_preset_choice: Preset::default(),
            ytdlp_check_rx: None,
            ytdlp_latest: None,
            taskbar_sent: -2,
            failure_notice: None,
            saved_links: yttraydl::saved::load_saved(),
            watch_data: yttraydl::saved::WatchData::load(),
            watch_rx: None,
            rule_input_watch_url: String::new(),
            rule_watch_preset: Preset::default(),
            watch_check_now: false,
            thumb_rx: None,
            thumb_attempt: yttraydl::thumbnail::Attempt::default(),
            popup_thumb: None,
            had_active_downloads: false,
            disk_warned: false,
            reset_armed: false,
            manual_url: String::new(),
            first_frame: true,
            open_panel_on_start: show_on_launch,
            frames: 0,
            last_heartbeat: Instant::now(),
            settings_mtime: settings_file_mtime(),
            last_settings_check: Instant::now(),
            window_geometries: WindowGeometries::load(),
            window_geometry_ready: [false; 4],
            window_geometry_dirty: false,
            last_geometry_save: Instant::now(),
            downloads_initial_size: None,
            #[cfg(feature = "field-validation")]
            field_metrics: field_metrics::Metrics::start(),
        };
        yttraydl::diag_log("ui start: checking tool status asynchronously");
        app.relocalize_binaries();
        if let Some(url) = manual_url {
            app.handle_detected_url(&url, true);
        }
        app
    }

    // ---------- shared helpers ----------

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            expires: Instant::now() + Duration::from_secs(6),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn exe_dir(&self) -> Option<std::path::PathBuf> {
        self.app_dir.clone()
    }

    pub fn relocalize_binaries(&mut self) {
        self.tool_status_generation = self.tool_status_generation.wrapping_add(1);
        let generation = self.tool_status_generation;
        let settings = self.settings.clone();
        let exe_dir = self.exe_dir();
        let wake = self.repaint.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.tool_status_rx = Some(rx);
        self.version_requested_for = None;
        self.background.spawn(move |_| {
            let snapshot =
                yttraydl::tools::ToolStatusSnapshot::resolve(&settings, exe_dir.as_deref());
            #[cfg(windows)]
            let autostart = platform::Platform::autostart_enabled();
            #[cfg(not(windows))]
            let autostart = false;
            let _ = tx.send((generation, snapshot, autostart));
            wake.request_repaint_of(egui::ViewportId::ROOT);
        });
    }

    fn process_tool_status(&mut self) {
        let Some(receiver) = &self.tool_status_rx else {
            return;
        };
        match receiver.try_recv() {
            Ok((generation, snapshot, autostart)) => {
                self.tool_status_rx = None;
                if generation != self.tool_status_generation {
                    return;
                }
                let first = !self.tool_status_ready;
                let found_after_unknown = self.bin.is_none() && snapshot.ytdlp.is_some();
                self.bin = snapshot.ytdlp;
                self.ffmpeg_ok = snapshot.ffmpeg_available;
                self.ffmpeg_managed = snapshot.managed_ffmpeg_available;
                self.autostart_status = autostart;
                self.tool_status_ready = true;
                if first && self.bin.is_none() {
                    self.show_panel = true;
                }
                if found_after_unknown {
                    if let Some(url) = self
                        .active_popup
                        .as_ref()
                        .filter(|popup| popup.info.is_none())
                        .map(|popup| popup.url.clone())
                    {
                        self.spawn_info_fetch(&url);
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.tool_status_rx = None;
                self.tool_status_ready = true;
                self.toast(
                    "Tool status worker stopped. Reopen Settings to retry.",
                    ToastKind::Error,
                );
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    pub fn bin_path(&self) -> Option<std::path::PathBuf> {
        self.bin.as_ref().map(|b| b.path.clone())
    }

    pub fn start_download(&mut self, url: &str, title: &str, preset: Preset, force: bool) {
        self.start_download_pl(url, title, preset, force, None);
    }

    /// `start_download` with a per-download playlist choice
    /// (from the popup): `Some(true)` = whole playlist,
    /// `Some(false)` = just the linked item, `None` = global.
    pub fn start_download_pl(
        &mut self,
        url: &str,
        title: &str,
        preset: Preset,
        force: bool,
        playlist_all: Option<bool>,
    ) {
        if self.bin.is_none() {
            self.toast(
                "yt-dlp is not installed yet — open ytTrayDL to install it.",
                ToastKind::Error,
            );
            self.show_panel = true;
            return;
        }
        let use_archive = self.settings.skip_already_downloaded && !force;
        self.downloads.enqueue_full(
            url.to_string(),
            title.to_string(),
            preset,
            use_archive,
            0,
            playlist_all,
        );
        self.pump_downloads();
        if self.settings.notify_on_auto_start && title.is_empty() {
            self.toast(
                format!("Downloading from {host}…", host = host_of(url)),
                ToastKind::Info,
            );
        }
    }

    /// `start_download_pl` plus a per-download output folder
    /// (from the popup's folder picker).
    #[allow(clippy::too_many_arguments)]
    pub fn start_download_full(
        &mut self,
        url: &str,
        title: &str,
        preset: Preset,
        force: bool,
        playlist_all: Option<bool>,
        output_dir: Option<std::path::PathBuf>,
    ) {
        if self.bin.is_none() {
            self.toast(
                "yt-dlp is not installed yet — open ytTrayDL to install it.",
                ToastKind::Error,
            );
            self.show_panel = true;
            return;
        }
        let use_archive = self.settings.skip_already_downloaded && !force;
        self.downloads.enqueue_job(
            url.to_string(),
            title.to_string(),
            preset,
            use_archive,
            0,
            playlist_all,
            output_dir,
        );
        self.pump_downloads();
    }

    fn pump_downloads(&mut self) {
        let bin = self.bin_path();
        self.downloads.pump(&self.settings, bin.as_deref());
        let blocked = self.downloads.disk_recheck_after(Instant::now()).is_some();
        if blocked && !self.disk_warned {
            self.toast(
                "Low disk space — affected downloads will resume when space frees up.",
                ToastKind::Error,
            );
        }
        self.disk_warned = blocked;
    }
    /// Pause everything pausable, or — when nothing is active
    /// but jobs are paused — resume everything. Used by the
    /// tray item (Windows); the Downloads window has its own
    /// separate Pause all / Resume all buttons.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn toggle_pause_all(&mut self) {
        if self.downloads.active_count() > 0 {
            self.downloads.pause_all();
            yttraydl::diag_log("pause all downloads");
        } else if self.downloads.has_paused() {
            self.downloads.resume_all();
            self.pump_downloads();
            yttraydl::diag_log("resume all downloads");
        }
    }

    /// Route a clipboard/manual URL through dedupe + the decision
    /// engine. `manual` bypasses rules (explicit user intent) and
    /// always shows the popup.
    pub fn handle_detected_url(&mut self, url: &str, manual: bool) {
        if !manual {
            if !self.seen.mark(url) {
                return;
            }
            match decide(url, &self.settings, false) {
                Decision::Ignore(reason) => {
                    self.last_ignored
                        .insert(0, (url.to_string(), reason.describe()));
                    self.last_ignored.truncate(8);
                }
                Decision::AutoDownload => {
                    let preset = yttraydl::rules::preset_for_url(url, &self.settings)
                        .unwrap_or(self.settings.default_preset);
                    self.start_download(url, "", preset, false);
                }
                Decision::Prompt => self.queue_popup(url),
            }
        } else {
            self.queue_popup(url);
        }
    }

    fn queue_popup(&mut self, url: &str) {
        if self.active_popup.as_ref().is_some_and(|p| p.url == url)
            || self.popup_queue.iter().any(|p| p.url == url)
        {
            return;
        }
        self.popup_queue.push_back(PopupRequest {
            url: url.to_string(),
            host: host_of(url),
            info: None,
            info_loading: false,
            already_downloaded: false,
            preset: yttraydl::rules::preset_for_url(url, &self.settings)
                .unwrap_or(self.settings.default_preset),
            always_auto_domain: false,
            playlist_all: self.settings.allow_playlists,
            folder: None,
            shown_at: Instant::now(),
            last_interaction: Instant::now(),
        });
    }

    fn activate_next_popup(&mut self) {
        if self.active_popup.is_none() {
            if let Some(mut p) = self.popup_queue.pop_front() {
                p.shown_at = Instant::now();
                p.last_interaction = Instant::now();
                p.info_loading = true;
                self.spawn_info_fetch(&p.url);
                self.thumb_attempt = yttraydl::thumbnail::Attempt::default();
                self.thumb_rx = None;
                self.popup_thumb = None;
                self.active_popup = Some(p);
            }
        }
    }

    fn spawn_info_fetch(&self, url: &str) {
        if let Some(bin) = self.bin_path() {
            let tx = self.info_tx.clone();
            let settings = self.settings.clone();
            let url_owned = url.to_string();
            self.background.spawn(move |token| {
                let info = yttraydl::fetch_info_with_cancel(&bin, &settings, &url_owned, &token);
                let already = match &info {
                    Some(i) => {
                        let archive =
                            std::fs::read_to_string(Settings::archive_path()).unwrap_or_default();
                        let extractor = i.extractor_key.clone().unwrap_or_default().to_lowercase();
                        let id = i.id.clone().unwrap_or_default();
                        ytdlp::archive_contains(&archive, &extractor, &id)
                    }
                    None => false,
                };
                let _ = tx.send((url_owned, info, already));
            });
        }
    }

    pub fn start_ytdlp_download(&mut self) {
        self.start_tool_download(yttraydl::tools::ToolFamily::YtDlp);
    }
    pub fn start_ffmpeg_download(&mut self) {
        self.start_tool_download(yttraydl::tools::ToolFamily::FFmpeg);
    }

    fn start_tool_download(&mut self, family: yttraydl::tools::ToolFamily) {
        use yttraydl::tools::{ProgressMailbox, ToolFamily, ToolProgress, ToolStage};
        if match family {
            ToolFamily::YtDlp => self.ytdlp_fetch_rx.is_some(),
            ToolFamily::FFmpeg => self.ffmpeg_fetch_rx.is_some(),
        } {
            return;
        }
        self.next_tool_operation = self.next_tool_operation.wrapping_add(1);
        self.scroll_to_tool_progress = true;
        let id = self.next_tool_operation;
        let mailbox = ProgressMailbox::default();
        mailbox.start(id);
        let cancel = self.background.child_token();
        let operation = ToolOperation {
            id,
            mailbox: mailbox.clone(),
            progress: ToolProgress {
                operation_id: id,
                family,
                stage: ToolStage::Resolve,
                bytes_done: 0,
                total_bytes: None,
                bytes_per_second: None,
                eta_seconds: None,
            },
            cancel: cancel.clone(),
            cancelling: false,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        match family {
            ToolFamily::YtDlp => {
                self.ytdlp_fetch_rx = Some(rx);
                self.ytdlp_operation = Some(operation);
            }
            ToolFamily::FFmpeg => {
                self.ffmpeg_fetch_rx = Some(rx);
                self.ffmpeg_operation = Some(operation);
            }
        }
        let wake = self.repaint.clone();
        self.background.spawn(move |_| {
            let mut progress = |update| {
                mailbox.publish(update);
                wake.request_repaint_of(egui::ViewportId::ROOT);
            };
            let result = match family {
                ToolFamily::YtDlp => {
                    yttraydl::download_managed_ytdlp_with_progress(&cancel, id, &mut progress)
                }
                ToolFamily::FFmpeg => {
                    yttraydl::download_managed_ffmpeg_with_progress(&cancel, id, &mut progress)
                }
            };
            let _ = tx.send(result);
            wake.request_repaint_of(egui::ViewportId::ROOT);
        });
    }

    fn poll_tool_progress(&mut self) {
        for operation in [&mut self.ytdlp_operation, &mut self.ffmpeg_operation]
            .into_iter()
            .flatten()
        {
            if let Some(progress) = operation
                .mailbox
                .take()
                .filter(|progress| progress.operation_id == operation.id)
            {
                operation.progress = progress;
            }
        }
    }

    fn tool_progress_ui(&mut self, ui: &mut egui::Ui) {
        for operation in [&mut self.ytdlp_operation, &mut self.ffmpeg_operation]
            .into_iter()
            .flatten()
        {
            let progress = &operation.progress;
            let name = match progress.family {
                yttraydl::tools::ToolFamily::YtDlp => "yt-dlp",
                yttraydl::tools::ToolFamily::FFmpeg => "ffmpeg",
            };
            egui::Frame::NONE
                .fill(CARD)
                .stroke(egui::Stroke::new(1.0_f32, ACCENT))
                .corner_radius(5)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{name}: {}",
                                if operation.cancelling {
                                    "Cancelling…"
                                } else {
                                    progress.stage.label()
                                }
                            ))
                            .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(!operation.cancelling, egui::Button::new("Cancel"))
                                .clicked()
                            {
                                operation.cancel.cancel();
                                operation.cancelling = true;
                            }
                        });
                    });
                    if progress.stage == yttraydl::tools::ToolStage::Download {
                        if let Some(total) = progress.total_bytes.filter(|total| *total > 0) {
                            ui.add(
                                egui::ProgressBar::new(
                                    (progress.bytes_done as f64 / total as f64).clamp(0.0, 1.0)
                                        as f32,
                                )
                                .desired_width(ui.available_width())
                                .show_percentage(),
                            );
                            ui.label(format!(
                                "{} / {}",
                                ytdlp::format_bytes(progress.bytes_done),
                                ytdlp::format_bytes(total)
                            ));
                        } else {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(format!(
                                    "{} downloaded — total unknown",
                                    ytdlp::format_bytes(progress.bytes_done)
                                ));
                            });
                        }
                        let speed = progress
                            .bytes_per_second
                            .map(|speed| {
                                format!("{}/s", ytdlp::format_bytes(speed.max(0.0) as u64))
                            })
                            .unwrap_or_else(|| "Measuring speed…".into());
                        let eta = progress
                            .eta_seconds
                            .map(|seconds| format!("About {seconds} s remaining"))
                            .unwrap_or_else(|| "ETA unknown".into());
                        ui.label(
                            egui::RichText::new(format!("{speed}  ·  {eta}"))
                                .small()
                                .color(TEXT_DIM),
                        );
                    } else {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                "Preparing verified tools; byte progress resumes during downloads.",
                            );
                        });
                    }
                    if operation.cancelling {
                        ui.label("Waiting for owned download cleanup before retry.");
                    }
                });
            ui.add_space(6.0);
        }
    }

    /// Ask GitHub for the latest yt-dlp release tag (worker
    /// thread; the result lands in `self.ytdlp_latest` via
    /// `process_ytdlp_check`). Only ever runs on demand.
    pub fn start_ytdlp_update_check(&mut self) {
        if self.ytdlp_check_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.ytdlp_check_rx = Some(rx);
        let wake = self.repaint.clone();
        self.background.spawn(move |token| {
            let _ = tx.send(yttraydl::fetch_latest_ytdlp_tag_with_cancel(&token));
            wake.request_repaint_of(egui::ViewportId::ROOT);
        });
    }

    fn process_ytdlp_check(&mut self) {
        let Some(rx) = &self.ytdlp_check_rx else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("Update-check worker stopped. Try again.".into()))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        };
        if let Some(result) = result {
            self.ytdlp_check_rx = None;
            self.ytdlp_latest = Some(result);
        }
    }

    /// Files dropped onto the panel: read any text file
    /// (`.txt`, `.url`, a list of links…) and route every URL
    /// found through the normal dedupe + rules pipeline —
    /// exactly as if the links had been copied one by one.
    fn process_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        let mut found: Vec<String> = Vec::new();
        for file in &dropped {
            let Some(path) = &file.path else { continue };
            let Ok(meta) = std::fs::metadata(path) else {
                continue;
            };
            if !meta.is_file() || meta.len() > 1_048_576 {
                continue;
            }
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            found.extend(extract_urls(&text));
        }
        if found.is_empty() {
            self.toast("No links found in the dropped file(s).", ToastKind::Info);
            return;
        }
        let n = found.len();
        if self.settings.monitoring_enabled {
            for url in &found {
                self.handle_detected_url(url, false);
            }
        } else {
            for url in &found {
                self.queue_popup(url);
            }
        }
        self.toast(
            format!("{n} link(s) dropped — processing them like copied links."),
            ToastKind::Info,
        );
    }

    /// Keep the taskbar button's progress bar in step with the
    /// queue: the value is the mean completion across active
    /// jobs (queued jobs count as 0%). Cleared when nothing is
    /// active. Only talks to the OS when the value changes.
    fn sync_taskbar_progress(&mut self) {
        let active = self.downloads.active_count();
        let target: i32 = if active == 0 {
            -1
        } else {
            let sum: f64 = self
                .downloads
                .running_jobs()
                .map(|j| j.percent as f64 / 100.0)
                .sum();
            ((sum / active as f64) * 1000.0) as i32
        };
        if target != self.taskbar_sent {
            self.taskbar_sent = target;
            if target < 0 {
                yttraydl::platform::set_taskbar_progress(None);
            } else {
                yttraydl::platform::set_taskbar_progress(Some(target as f64 / 1000.0));
            }
        }
    }

    /// Hide the control panel — to the tray, or as part of
    /// `hide_entirely`. Direct since 0.5.1: Settings is its own
    /// window now, so there is no drawer to close first (the
    /// 0.4.1 deferred hide existed only for the drawer and its
    /// window-shrink resize). The Settings window hides with
    /// the panel so nothing floats alone while the app sits in
    /// the tray.
    pub fn begin_panel_hide(&mut self) {
        self.show_settings = false;
        self.show_panel = false;
    }

    /// Hide EVERYTHING — panel and tray icon — while the app keeps
    /// running in the background. Callers are responsible for
    /// having explained recall first (the tray-menu confirm dialog,
    /// or the close-choice dialog which spells it out inline).
    /// Recall: relaunch the exe, Ctrl+Alt+G, or `--show`.
    pub fn hide_entirely(&mut self) {
        self.settings.tray_icon_visible = false;
        self.tray_visible = false;
        #[cfg(windows)]
        with_tray_handles(|t| {
            t._tray.set_visible(false).ok();
        });
        self.settings_dirty = true;
        // begin_panel_hide also closes the Settings window.
        self.begin_panel_hide();
    }

    // ---------- per-frame processing ----------

    fn process_platform_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.platform.events.try_recv() {
            match ev {
                PlatformEvent::ClipboardText(text) => {
                    if self.settings.monitoring_enabled {
                        for url in extract_urls(&text) {
                            self.handle_detected_url(&url, false);
                        }
                    }
                }
                PlatformEvent::ManualUrl(url) => {
                    self.handle_detected_url(&url, true);
                }
                PlatformEvent::OpenDownloads => {
                    request_module(ctx, &mut self.show_downloads, "yttraydl-downloads");
                    self.show_panel = true;
                    ctx.request_repaint();
                }
                PlatformEvent::OpenConverter => {
                    request_module(ctx, &mut self.show_converter, "yttraydl-converter");
                    self.show_panel = true;
                    ctx.request_repaint();
                }
                PlatformEvent::OpenTags => {
                    request_module(ctx, &mut self.show_tags, "yttraydl-tags");
                    self.show_panel = true;
                    ctx.request_repaint();
                }
                PlatformEvent::OpenPlayer => {
                    request_module(ctx, &mut self.show_player, "yttraydl-player");
                    self.show_panel = true;
                    ctx.request_repaint();
                }
                PlatformEvent::ShowRequested => {
                    self.settings.tray_icon_visible = true;
                    self.tray_visible = true;
                    #[cfg(windows)]
                    with_tray_handles(|t| {
                        t._tray.set_visible(true).ok();
                    });
                    self.settings_dirty = true;
                    self.show_settings = true;
                    self.show_panel = true;
                }
            }
        }
    }

    fn process_info_results(&mut self) {
        while let Ok((url, info, already)) = self.info_rx.try_recv() {
            if let Some(p) = &mut self.active_popup {
                if p.url == url {
                    p.info = info.clone();
                    p.info_loading = false;
                    p.already_downloaded = already;
                }
            }
            for p in &mut self.popup_queue {
                if p.url == url {
                    p.info = info.clone();
                    p.info_loading = false;
                    p.already_downloaded = already;
                }
            }
        }
        // Kick off the active popup's thumbnail fetch (once).
        if self.thumb_rx.is_none() {
            if let Some(thumb_url) = self
                .active_popup
                .as_ref()
                .and_then(|p| p.info.as_ref())
                .and_then(|i| i.thumbnail.clone())
            {
                if self.thumb_attempt.begin(&thumb_url) {
                    self.spawn_thumb_fetch(thumb_url);
                }
            }
        }
    }

    /// Fetch + decode a popup thumbnail on a worker thread
    /// (curl for the bytes, the `image` crate for JPEG/PNG).
    fn spawn_thumb_fetch(&mut self, url: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.thumb_rx = Some(rx);
        self.background.spawn(move |token| {
            let decoded = yttraydl::fetch_url_bytes_with_cancel(&url, &token)
                .and_then(|bytes| yttraydl::thumbnail::decode(&bytes));
            let _ = tx.send((url, decoded));
        });
    }

    fn process_thumbs(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.thumb_rx {
            let received = rx.try_recv();
            if matches!(received, Err(std::sync::mpsc::TryRecvError::Disconnected)) {
                self.thumb_rx = None;
            }
            if let Ok((url, decoded)) = received {
                self.thumb_rx = None;
                if let Some((w, h, rgba)) = decoded {
                    // Centre-crop to the popup hero's aspect
                    // (0.6.0): the hero paints the texture
                    // stretched to its exact rect, so the crop
                    // happens here, once, instead of distorting
                    // the image at paint time.
                    const HERO_ASPECT: f32 = 406.0 / 184.0;
                    let (w, h, rgba) = if (w as f32 / h as f32 - HERO_ASPECT).abs() < 0.01 {
                        (w, h, rgba)
                    } else if w as f32 / h as f32 > HERO_ASPECT {
                        let cw = ((h as f32 * HERO_ASPECT) as u32).min(w).max(1);
                        let x0 = (w - cw) / 2;
                        let mut out = Vec::with_capacity((cw * h * 4) as usize);
                        for y in 0..h {
                            let start = (((y * w + x0) * 4) as usize).min(rgba.len());
                            let end = (start + (cw * 4) as usize).min(rgba.len());
                            out.extend_from_slice(&rgba[start..end]);
                        }
                        (cw, h, out)
                    } else {
                        let ch = ((w as f32 / HERO_ASPECT) as u32).min(h).max(1);
                        let y0 = (h - ch) / 2;
                        let start = ((y0 * w * 4) as usize).min(rgba.len());
                        let end = (start + (ch * w * 4) as usize).min(rgba.len());
                        (w, ch, rgba[start..end].to_vec())
                    };
                    let img =
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                    let handle = ctx.load_texture("popup-thumb", img, egui::TextureOptions::LINEAR);
                    self.popup_thumb = Some((url, handle));
                }
            }
        }
    }

    // ---------- channel watcher ----------

    /// Poll every subscription once (worker threads run the
    /// yt-dlp flat probes). No-op while a poll is in flight.
    fn start_watch_check(&mut self) {
        if self.watch_rx.is_some() || self.settings.subscriptions.is_empty() {
            return;
        }
        let Some(bin) = self.bin_path() else {
            return;
        };
        let settings = self.settings.clone();
        let subs = settings.subscriptions.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.watch_rx = Some(rx);
        self.background.spawn(move |token| {
            let results: Vec<(String, Vec<yttraydl::ytdlp::PlaylistEntry>)> = subs
                .iter()
                .map(|sub| {
                    (
                        sub.url.clone(),
                        yttraydl::fetch_playlist_entries_with_cancel(
                            &bin, &settings, &sub.url, 10, &token,
                        ),
                    )
                })
                .collect();
            let _ = tx.send(results);
        });
    }

    fn process_watch(&mut self) {
        // Manual "check now" from Settings.
        if self.watch_check_now {
            self.watch_check_now = false;
            self.start_watch_check();
        }
        // Scheduled poll when the interval has elapsed.
        if self.watch_rx.is_none()
            && self.settings.watch_interval_mins > 0
            && !self.settings.subscriptions.is_empty()
        {
            let due = self.watch_data.last_check_unix == 0
                || yttraydl::history::now_unix()
                    >= self.watch_data.last_check_unix as i64
                        + i64::from(self.settings.watch_interval_mins) * 60;
            if due {
                self.start_watch_check();
            }
        }
        if let Some(rx) = &self.watch_rx {
            let received = rx.try_recv();
            if matches!(received, Err(std::sync::mpsc::TryRecvError::Disconnected)) {
                self.watch_rx = None;
                self.watch_data.last_check_unix = yttraydl::history::now_unix().max(0) as u64;
                yttraydl::diag_log("channel watcher worker disconnected; batch released");
            }
            if let Ok(results) = received {
                self.watch_rx = None;
                self.watch_data.last_check_unix = yttraydl::history::now_unix().max(0) as u64;
                let mut new_count = 0usize;
                for (sub_url, entries) in results {
                    if entries.is_empty() {
                        continue;
                    }
                    let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
                    let Some((preset, fresh)) = self.watch_data.record_subscription_poll(
                        &self.settings.subscriptions,
                        &sub_url,
                        &ids,
                    ) else {
                        continue;
                    };
                    if fresh.is_empty() {
                        continue;
                    }
                    for entry in entries.iter().filter(|e| fresh.contains(&e.id)) {
                        let title = if entry.title.is_empty() {
                            entry.url.clone()
                        } else {
                            entry.title.clone()
                        };
                        self.start_download(&entry.url, &title, preset, false);
                        new_count += 1;
                    }
                }
                let _ = self.watch_data.save();
                if new_count > 0 {
                    self.toast(
                        format!(
                            "Channel watcher: {new_count} new upload{} downloading.",
                            if new_count == 1 { "" } else { "s" }
                        ),
                        ToastKind::Success,
                    );
                }
            }
        }
    }

    // ---------- save for later ----------

    /// Park a link in the saved list (popup "Later" button).
    pub fn save_link_for_later(&mut self, url: &str, title: &str) {
        if self.saved_links.iter().any(|l| l.url == url) {
            self.toast("That link is already in your saved list.", ToastKind::Info);
            return;
        }
        self.saved_links.push(yttraydl::saved::SavedLink {
            url: url.to_string(),
            title: title.to_string(),
            saved_at_unix: yttraydl::history::now_unix(),
        });
        let _ = yttraydl::saved::save_saved(&self.saved_links);
        self.toast("Saved for later.", ToastKind::Success);
    }

    pub fn remove_saved_link(&mut self, index: usize) {
        if index < self.saved_links.len() {
            self.saved_links.remove(index);
            let _ = yttraydl::saved::save_saved(&self.saved_links);
        }
    }

    /// Download a saved link now (and drop it from the list).
    pub fn download_saved_link(&mut self, index: usize) {
        if index >= self.saved_links.len() {
            return;
        }
        let link = self.saved_links.remove(index);
        let _ = yttraydl::saved::save_saved(&self.saved_links);
        let preset = yttraydl::rules::preset_for_url(&link.url, &self.settings)
            .unwrap_or(self.settings.default_preset);
        let title = if link.title.is_empty() {
            link.url.clone()
        } else {
            link.title.clone()
        };
        self.start_download(&link.url, &title, preset, false);
    }

    /// Retry the failure shown in the panel banner.
    pub fn retry_failure_notice(&mut self) {
        if let Some(notice) = self.failure_notice.take() {
            if self.downloads.requeue_finished(notice.job_id).is_some() {
                self.pump_downloads();
            }
        }
    }

    fn process_download_events(&mut self) {
        let events = self.downloads.poll_events();
        let mut finished_any = false;
        for ev in events {
            if let DownloadEvent::Finished(job) = ev {
                finished_any = true;
                if job.state == JobState::Paused {
                    // Not a finish: no history entry, no toast.
                    // The pump below refills the freed slot.
                    continue;
                }
                if let Some(next) = job.automatic_retry_attempt(self.settings.auto_retry_attempts) {
                    // Auto-retry with the original preset; the
                    // failure is not recorded as final yet.
                    self.downloads.enqueue_job(
                        job.url.clone(),
                        job.title.clone(),
                        job.preset,
                        job.use_archive,
                        next,
                        job.playlist_all,
                        job.output_dir.clone(),
                    );
                    self.toast(
                        format!(
                            "Download failed — retrying (attempt {} of {}): {}",
                            next + 1,
                            self.settings.auto_retry_attempts + 1,
                            job.title
                        ),
                        ToastKind::Info,
                    );
                    continue;
                }
                let Some(entry) = HistoryEntry::from_finished_job(&job, now_unix()) else {
                    continue;
                };
                let error = entry.error.clone();
                self.history.push(entry);
                let _ = self.history.save();
                match job.state {
                    JobState::Done => {
                        if job.already_downloaded {
                            self.seen.forget(&job.url);
                        }
                        if self.settings.sound_on_finish {
                            yttraydl::platform::play_notification_sound();
                        }
                        if self.settings.notify_on_complete {
                            self.toast(
                                format!("Download finished: {}", job.title),
                                ToastKind::Success,
                            );
                        }
                        if self.settings.open_folder_on_finish {
                            let folder = job
                                .final_file
                                .as_ref()
                                .and_then(|f| f.parent().map(|p| p.to_path_buf()))
                                .unwrap_or_else(|| self.settings.download_folder.clone());
                            let _ = yttraydl::platform::open_path(&folder);
                        }
                        // Post-download command (user-configured,
                        // placeholders substituted here).
                        let cmd = self.settings.post_download_command.trim();
                        if !cmd.is_empty() {
                            if let Some(file) = &job.final_file {
                                let dir = file
                                    .parent()
                                    .map(|p| p.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                match yttraydl::hooks::HookSpec::parse(cmd) {
                                    Ok(hook) => {
                                        let hook = hook.expand(
                                            &file.to_string_lossy(),
                                            &dir,
                                            &job.url,
                                            &job.title,
                                        );
                                        if let Err(error) =
                                            std::process::Command::new(&hook.executable)
                                                .args(&hook.args)
                                                .spawn()
                                        {
                                            self.toast(
                                                format!("Completion hook failed: {error}"),
                                                ToastKind::Error,
                                            );
                                        }
                                    }
                                    Err(error) => self.toast(error, ToastKind::Error),
                                }
                            }
                        }
                    }
                    JobState::Failed => {
                        self.seen.forget(&job.url);
                        // Panel banner with one-click retry (the
                        // job is in the finished list under this id).
                        self.failure_notice = Some(FailureNotice {
                            job_id: job.id,
                            title: if job.title.is_empty() {
                                job.url.clone()
                            } else {
                                job.title.clone()
                            },
                            error: error
                                .clone()
                                .unwrap_or_else(|| "yt-dlp reported an error".to_string()),
                        });
                        if self.settings.notify_on_error {
                            self.toast(
                                format!(
                                    "Download failed: {}",
                                    error.unwrap_or_else(|| job.title.clone())
                                ),
                                ToastKind::Error,
                            );
                        }
                    }
                    JobState::Cancelled => {
                        self.seen.forget(&job.url);
                    }
                    _ => {}
                }
            }
        }
        if finished_any {
            self.pump_downloads();
        }
        // Queue-empty action: fires once when a queue that had
        // active work drains completely (nothing queued, running
        // or paused).
        if self.downloads.active_count() > 0 {
            self.had_active_downloads = true;
        } else if self.had_active_downloads && !self.downloads.has_paused() {
            self.had_active_downloads = false;
            match self.settings.queue_empty_action {
                yttraydl::settings::QueueEmptyAction::Nothing => {}
                yttraydl::settings::QueueEmptyAction::Sound => {
                    yttraydl::platform::play_notification_sound();
                }
                yttraydl::settings::QueueEmptyAction::Sleep => {
                    yttraydl::diag_log("queue empty — sleeping the PC (user setting)");
                    yttraydl::platform::suspend_system();
                }
                yttraydl::settings::QueueEmptyAction::ExitApp => {
                    yttraydl::diag_log("queue empty — exiting (user setting)");
                    self.exit_requested = true;
                }
            }
        }
    }

    fn process_ytdlp_fetch(&mut self) {
        self.process_tool_fetch(yttraydl::tools::ToolFamily::YtDlp);
    }
    fn process_ffmpeg_fetch(&mut self) {
        self.process_tool_fetch(yttraydl::tools::ToolFamily::FFmpeg);
    }
    fn process_tool_fetch(&mut self, family: yttraydl::tools::ToolFamily) {
        use yttraydl::tools::ToolFamily;
        let receiver = match family {
            ToolFamily::YtDlp => &mut self.ytdlp_fetch_rx,
            ToolFamily::FFmpeg => &mut self.ffmpeg_fetch_rx,
        };
        let Some(result) = take_tool_terminal(receiver) else {
            return;
        };
        let cancelled = match family {
            ToolFamily::YtDlp => {
                self.ytdlp_fetch_rx = None;
                self.ytdlp_operation
                    .take()
                    .is_some_and(|operation| operation.cancelling)
            }
            ToolFamily::FFmpeg => {
                self.ffmpeg_fetch_rx = None;
                self.ffmpeg_operation
                    .take()
                    .is_some_and(|operation| operation.cancelling)
            }
        };
        let name = match family {
            ToolFamily::YtDlp => "yt-dlp",
            ToolFamily::FFmpeg => "ffmpeg",
        };
        // Even an error after activation/reporting may have committed verified
        // tools. Reconcile actual status independently from the terminal label.
        self.relocalize_binaries();
        match result {
            Ok(_) => {
                self.toast(format!("{name} is ready."), ToastKind::Success);
            }
            Err(error) => self.toast(
                format!(
                    "{name} {}: {error}",
                    if cancelled {
                        "operation cancelled"
                    } else {
                        "download failed"
                    }
                ),
                ToastKind::Error,
            ),
        }
    }

    fn ensure_version(&mut self) {
        if let Some(bin) = self.bin_path() {
            if self.version_requested_for.as_ref() != Some(&bin) {
                self.version_requested_for = Some(bin.clone());
                self.ytdlp_version = None;
                let (tx, rx) = std::sync::mpsc::channel::<String>();
                let wake = self.repaint.clone();
                self.background.spawn(move |token| {
                    let version = yttraydl::fetch_version(&bin, &token)
                        .unwrap_or_else(|| "Version unavailable".into());
                    let _ = tx.send(version);
                    wake.request_repaint_of(egui::ViewportId::ROOT);
                });
                // Drain lazily via a leaked receiver pattern: store
                // in a thread-local-ish slot by polling in update
                // is overkill; use a static-free approach — attach
                // to self via a oneshot stored in a field is not
                // available, so block-free read next frames:
                self.version_rx = Some(rx);
            }
        }
        if let Some(rx) = &self.version_rx {
            match rx.try_recv() {
                Ok(version) => {
                    self.ytdlp_version = Some(if version.is_empty() {
                        "Version unavailable".into()
                    } else {
                        version
                    });
                    self.version_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.ytdlp_version = Some("Version unavailable".into());
                    self.version_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    #[cfg(windows)]
    fn ensure_tray(&mut self, ctx: &egui::Context) {
        let tray_exists = with_tray_handles(|_| ()).is_some();
        if tray_exists || self.tray_failed {
            return;
        }
        // 0.2.4: install the tray event handlers BEFORE the
        // icon exists, so no click can race ahead of delivery
        // setup. Each handler logs at the point of production
        // (so a future "clicks do nothing" report localizes
        // from the log alone), forwards into the app channel,
        // and pokes the UI awake.
        if self.tray_event_rx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<TrayUiEvent>();
            self.tray_event_rx = Some(rx);
            // Replace (never set-once) the shared sender: the
            // handlers must always feed the live receiver.
            let replaced = {
                let mut slot = TRAY_EVENT_TX.lock().unwrap();
                let had = slot.is_some();
                *slot = Some(tx);
                had
            };
            let wake = ctx.clone();
            tray_icon::TrayIconEvent::set_event_handler(Some(
                move |event: tray_icon::TrayIconEvent| {
                    let is_left_up = matches!(
                        event,
                        tray_icon::TrayIconEvent::Click {
                            button: tray_icon::MouseButton::Left,
                            button_state: tray_icon::MouseButtonState::Up,
                            ..
                        }
                    );
                    if matches!(event, tray_icon::TrayIconEvent::Click { .. }) {
                        yttraydl::diag_log(&format!("tray icon event (handler): {event:?}"));
                    }
                    if is_left_up {
                        // Toggle the window (0.2.8): a single
                        // left-click shows the window when it's
                        // hidden and hides it when it's shown —
                        // at OS level, in the handler, reading
                        // the window's real visibility so the
                        // toggle can never diverge from what the
                        // user sees. The app is told the result
                        // (WindowToggled) to adopt as its state.
                        // A double-click delivers a second Up
                        // within milliseconds — debounce it, or
                        // the window would toggle twice and look
                        // like nothing happened (seen in the
                        // 0.2.7 field log).
                        let now = Instant::now();
                        let debounced = {
                            let mut last = LAST_TRAY_TOGGLE.lock().unwrap();
                            if last
                                .is_some_and(|t| now.duration_since(t) < Duration::from_millis(350))
                            {
                                true
                            } else {
                                *last = Some(now);
                                false
                            }
                        };
                        if debounced {
                            yttraydl::diag_log("tray left-click: toggle debounced (double-click)");
                        } else {
                            match platform::toggle_main_window() {
                                Some(visible) => {
                                    yttraydl::diag_log(&format!(
                                        "tray left-click: toggle -> visible={visible}"
                                    ));
                                    forward_tray_event(TrayUiEvent::WindowToggled(visible));
                                }
                                None => {
                                    yttraydl::diag_log(
                                        "tray left-click: toggle -> window not found; forwarding",
                                    );
                                    forward_tray_event(TrayUiEvent::Icon(event));
                                }
                            }
                        }
                    } else {
                        forward_tray_event(TrayUiEvent::Icon(event));
                    }
                    wake.request_repaint();
                },
            ));
            let wake = ctx.clone();
            tray_icon::menu::MenuEvent::set_event_handler(Some(
                move |event: tray_icon::menu::MenuEvent| {
                    yttraydl::diag_log(&format!("tray menu event (handler): id={:?}", event.id));
                    // 0.2.7: most actions execute HERE, in the
                    // handler — they cannot wait for an egui
                    // pass that may never be scheduled again
                    // (see handle_tray_menu_direct).
                    if handle_tray_menu_direct(&event) {
                        return;
                    }
                    forward_tray_event(TrayUiEvent::Menu(event));
                    // Give the forwarded action a visible home
                    // right now; the app applies it on its
                    // next pass.
                    platform::show_main_window();
                    wake.request_repaint();
                },
            ));
            yttraydl::diag_log(&format!(
                "tray event handlers installed (channel replaced: {replaced})"
            ));
        }
        use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
        let menu = Menu::new();
        let header = MenuItem::new(
            format!("ytTrayDL v{}", env!("CARGO_PKG_VERSION")),
            false,
            None,
        );
        let monitoring = CheckMenuItem::new(
            "Monitoring clipboard",
            true,
            self.settings.monitoring_enabled,
            None,
        );
        let auto_mode = CheckMenuItem::new(
            "Auto-download (use my defaults, no popup)",
            true,
            self.settings.default_action == DefaultAction::AutoDownload,
            None,
        );
        let download_clipboard = MenuItem::new("Download from clipboard", true, None);
        let downloads = MenuItem::new("Downloads\u{2026}", true, None);
        let converter = MenuItem::new("Video converter…", true, None);
        let tags_item = MenuItem::new("Lyrics & tags…", true, None);
        let player_item = MenuItem::new("Player…", true, None);
        let open_folder = MenuItem::new("Open download folder", true, None);
        let pause_all = MenuItem::new("Pause all downloads", true, None);
        let settings_item = MenuItem::new("Settings\u{2026}", true, None);
        let autostart = CheckMenuItem::new("Start with Windows", true, self.autostart_status, None);
        let hide_icon = MenuItem::new("Hide tray icon\u{2026}", true, None);
        let about = MenuItem::new("About ytTrayDL", true, None);
        let exit = MenuItem::new("Exit ytTrayDL", true, None);

        let ids = (
            monitoring.id().clone(),
            auto_mode.id().clone(),
            autostart.id().clone(),
            download_clipboard.id().clone(),
            downloads.id().clone(),
            open_folder.id().clone(),
            pause_all.id().clone(),
            settings_item.id().clone(),
            hide_icon.id().clone(),
            about.id().clone(),
            exit.id().clone(),
            converter.id().clone(),
            tags_item.id().clone(),
            player_item.id().clone(),
        );
        let _ = menu.append_items(&[
            &header,
            &PredefinedMenuItem::separator(),
            &monitoring,
            &auto_mode,
            &PredefinedMenuItem::separator(),
            &download_clipboard,
            &downloads,
            &converter,
            &tags_item,
            &player_item,
            &open_folder,
            &pause_all,
            &PredefinedMenuItem::separator(),
            &settings_item,
            &autostart,
            &hide_icon,
            &PredefinedMenuItem::separator(),
            &about,
            &exit,
        ]);

        let icon = match tray_icon::Icon::from_rgba(app_icon_data().rgba, 64, 64) {
            Ok(i) => i,
            Err(e) => {
                yttraydl::diag_log(&format!("tray icon creation failed: {e}"));
                self.tray_failed = true;
                // Never strand the user with an invisible app:
                // if the tray can't exist, the panel must.
                self.show_panel = true;
                return;
            }
        };
        let tray = match tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            // SEAN (0.2.3): single left-click opens the window;
            // the menu belongs to the right-click. Without this,
            // tray-icon pops the menu on left-click too, and the
            // click ALSO toggled the window — a confusing mess.
            .with_menu_on_left_click(false)
            .with_tooltip("ytTrayDL \u{2014} monitoring clipboard")
            .with_icon(icon)
            .build()
        {
            Ok(t) => t,
            Err(e) => {
                yttraydl::diag_log(&format!("tray creation failed: {e}"));
                self.tray_failed = true;
                self.show_panel = true;
                return;
            }
        };
        yttraydl::diag_log("tray created");
        if !self.tray_visible {
            tray.set_visible(false).ok();
        }
        with_tray_handles_mut(|slot| {
            *slot = Some(TrayHandles {
                _tray: tray,
                monitoring,
                auto_mode,
                autostart,
                pause_all,
                id_monitoring: ids.0,
                id_auto: ids.1,
                id_autostart: ids.2,
                id_download_clipboard: ids.3,
                id_downloads: ids.4,
                id_converter: ids.11,
                id_tags: ids.12,
                id_player: ids.13,
                id_open_folder: ids.5,
                id_pause_all: ids.6,
                id_settings: ids.7,
                id_hide_icon: ids.8,
                id_about: ids.9,
                id_exit: ids.10,
            });
        });
    }

    #[cfg(windows)]
    fn process_tray_events(&mut self, ctx: &egui::Context) {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        // Gather from every delivery route (0.2.4): the
        // handler-fed app channel is primary; the global
        // receivers are drained as a fallback — tray-icon
        // documents that they receive nothing once handlers
        // are installed, so no event can arrive twice.
        let mut icon_events: Vec<TrayIconEvent> = Vec::new();
        let mut menu_events: Vec<tray_icon::menu::MenuEvent> = Vec::new();
        if let Some(rx) = &self.tray_event_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    TrayUiEvent::Icon(e) => icon_events.push(e),
                    TrayUiEvent::Menu(e) => menu_events.push(e),
                    TrayUiEvent::WindowToggled(visible) => {
                        // Adopt the handler's OS-level toggle
                        // (0.2.8) so app state and the real
                        // window can never diverge.
                        TRAY_DRAINED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        self.show_panel = visible;
                        if visible {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        }
                        yttraydl::diag_log(&format!("tray toggle adopted: panel={visible}"));
                    }
                }
            }
        }
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            icon_events.push(ev);
        }
        while let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            menu_events.push(ev);
        }
        TRAY_DRAINED.fetch_add(
            (icon_events.len() + menu_events.len()) as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        for ev in icon_events {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                // Single click OPENS the window (SEAN, 0.2.3) —
                // it used to toggle, which read as randomness
                // combined with the left-click menu popup.
                yttraydl::diag_log("tray icon left-click: opening window");
                self.show_panel = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        for ev in menu_events {
            // Forwarded here: the actions that need live app
            // state, plus Exit — which the handler routes
            // through this graceful path with a forced
            // fallback if no pass ever runs (0.2.8; the
            // toggles, open folder and hide icon execute fully
            // in the handler — see handle_tray_menu_direct).
            // This line is the consumption marker.
            let (
                item_name,
                id_download_clipboard,
                id_downloads,
                id_converter,
                id_tags,
                id_player,
                id_settings,
                id_about,
                id_exit,
                id_pause_all,
            ) = match with_tray_handles(|t| {
                (
                    tray_item_name(t, &ev.id),
                    t.id_download_clipboard.clone(),
                    t.id_downloads.clone(),
                    t.id_converter.clone(),
                    t.id_tags.clone(),
                    t.id_player.clone(),
                    t.id_settings.clone(),
                    t.id_about.clone(),
                    t.id_exit.clone(),
                    t.id_pause_all.clone(),
                )
            }) {
                Some(fields) => fields,
                None => continue,
            };
            yttraydl::diag_log(&format!("tray menu click: {item_name}"));
            if ev.id == id_download_clipboard {
                match platform::current_clipboard_text() {
                    Some(text) => {
                        let urls = extract_urls(&text);
                        if urls.is_empty() {
                            self.toast("No link found on the clipboard.", ToastKind::Info);
                        } else {
                            for u in &urls {
                                self.handle_detected_url(u, true);
                            }
                        }
                    }
                    None => self.toast("Clipboard is empty or not text.", ToastKind::Info),
                }
            } else if ev.id == id_downloads {
                request_module(ctx, &mut self.show_downloads, "yttraydl-downloads");
                self.show_panel = true;
            } else if ev.id == id_converter {
                request_module(ctx, &mut self.show_converter, "yttraydl-converter");
                self.show_panel = true;
            } else if ev.id == id_tags {
                request_module(ctx, &mut self.show_tags, "yttraydl-tags");
                self.show_panel = true;
            } else if ev.id == id_player {
                request_module(ctx, &mut self.show_player, "yttraydl-player");
                self.show_panel = true;
            } else if ev.id == id_settings {
                // Settings is its own window (0.5.1) — opening
                // it from the tray also shows the panel, as
                // before.
                self.show_settings = true;
                self.show_panel = true;
            } else if ev.id == id_about {
                self.show_about = true;
                self.show_panel = true;
            } else if ev.id == id_pause_all {
                self.toggle_pause_all();
            } else if ev.id == id_exit {
                // The app loop joins owned workers before exiting.
                yttraydl::diag_log("exit requested via tray menu");
                self.exit_requested = true;
            }
        }
    }

    #[cfg(windows)]
    fn sync_tray(&mut self) {
        // Pause-all label follows the queue state: it offers
        // Resume only when nothing is active and jobs are paused.
        let want_resume = self.downloads.active_count() == 0 && self.downloads.has_paused();
        if want_resume != self.tray_pause_label_resume {
            self.tray_pause_label_resume = want_resume;
            with_tray_handles(|t| {
                t.pause_all.set_text(if want_resume {
                    "Resume all downloads"
                } else {
                    "Pause all downloads"
                });
            });
        }
        // Tooltip + Monitoring/Auto checkmarks (perf): push
        // only on change — the muda setters issue Win32 calls
        // every time, and this runs on every update pass.
        let summary = self.downloads.summary();
        if self.tray_tooltip_sent.as_deref() != Some(summary.as_str()) {
            self.tray_tooltip_sent = Some(summary);
            let tip = self.tray_tooltip_sent.clone();
            with_tray_handles(|t| {
                t._tray.set_tooltip(tip.as_deref()).ok();
            });
        }
        let monitoring = self.settings.monitoring_enabled;
        if self.tray_monitoring_checked != Some(monitoring) {
            self.tray_monitoring_checked = Some(monitoring);
            with_tray_handles(|t| t.monitoring.set_checked(monitoring));
        }
        let auto = self.settings.default_action == DefaultAction::AutoDownload;
        if self.tray_auto_checked != Some(auto) {
            self.tray_auto_checked = Some(auto);
            with_tray_handles(|t| t.auto_mode.set_checked(auto));
        }
        // Native registry state is supplied by the worker snapshot.
        let autostart_now = self.autostart_status;
        if self.tray_autostart_checked != Some(autostart_now) {
            self.tray_autostart_checked = Some(autostart_now);
            with_tray_handles(|t| t.autostart.set_checked(autostart_now));
        }
    }
}

#[cfg(not(windows))]
impl App {
    fn ensure_tray(&mut self, _ctx: &egui::Context) {
        // No tray on non-Windows dev builds.
        self.tray_failed = true;
    }
    fn process_tray_events(&mut self, _ctx: &egui::Context) {}
    fn sync_tray(&mut self) {}
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.main_window_remembered {
            self.main_window_remembered = yttraydl::platform::remember_main_window();
        }
        #[cfg(feature = "field-validation")]
        let field_update_start = Instant::now();
        #[cfg(feature = "field-validation")]
        let field_geometry_start = Instant::now();
        self.maintain_window_geometry(false);
        #[cfg(feature = "field-validation")]
        self.field_metrics
            .record("geometry-before", field_geometry_start.elapsed());
        #[cfg(feature = "field-validation")]
        let field_events_start = Instant::now();
        self.process_tool_status();
        if self.show_settings && !self.was_settings_open {
            self.relocalize_binaries();
        }
        self.was_settings_open = self.show_settings;
        self.ensure_tray(ctx);
        self.process_platform_events(ctx);
        self.process_info_results();
        self.process_download_events();
        if let Some(delay) = self.downloads.disk_recheck_after(Instant::now()) {
            if delay.is_zero() {
                self.pump_downloads();
            }
            if let Some(delay) = self.downloads.disk_recheck_after(Instant::now()) {
                ctx.request_repaint_after(delay);
            }
        }
        self.poll_tool_progress();
        self.process_ytdlp_fetch();
        self.process_ffmpeg_fetch();
        self.process_ytdlp_check();
        self.process_thumbs(ctx);
        self.process_watch();
        self.process_dropped_files(ctx);
        self.sync_taskbar_progress();
        self.process_tray_events(ctx);

        // Settings-file reload (0.2.7): the tray handlers apply
        // toggles directly to the settings file (they cannot
        // wait for this loop). When the file changes on disk,
        // adopt it wholesale — the alternative is this app's
        // next save silently reverting the user's tray clicks.
        if self.last_settings_check.elapsed() >= Duration::from_secs(1) {
            self.last_settings_check = Instant::now();
            let disk_mtime = settings_file_mtime();
            if disk_mtime.is_some() && disk_mtime != self.settings_mtime {
                self.settings_mtime = disk_mtime;
                self.settings = Settings::load();
                self.relocalize_binaries();
                self.tray_visible = self.settings.tray_icon_visible;
                yttraydl::diag_log("settings reloaded (changed outside the UI)");
            }
        }
        self.ensure_version();
        self.activate_next_popup();
        self.sync_tray();
        #[cfg(feature = "field-validation")]
        self.field_metrics
            .record("events-and-state", field_events_start.elapsed());
        #[cfg(feature = "field-validation")]
        let field_render_start = Instant::now();

        if self.first_frame {
            self.first_frame = false;
            if self.open_panel_on_start {
                self.show_panel = true;
            }
        }

        // (0.5.1: no drawer, no deferred hide — Settings is
        // its own window, rendered by settings_ui below.)

        // Root viewport = the control panel.
        if self.show_panel {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            // Margin-free frame (0.6.0): the panel paints its
            // own layout — the gradient header band and the
            // recessed footer bar run full-bleed to the window
            // edges, and `panel_ui` insets the content itself.
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(PANEL_BG))
                .show(ctx, |ui| {
                    self.panel_ui(ui);
                });
            self.restore_new_window_geometry(WindowKind::Main);
            if ctx.input(|i| i.viewport().close_requested()) && !self.exit_requested {
                // Closing the main window is a choice, not a
                // reflex: the user's saved CloseAction decides
                // (Ask = show the choice dialog). See
                // `close_choice_ui` in gui_windows.
                //
                // Field fix (0.2.2): an unanswered close request
                // closes the window anyway — egui only holds it
                // open when the app sends `CancelClose` in
                // response. The 0.2.0/0.2.1 code reacted (opened
                // the dialog, hid the panel…) but never vetoed,
                // so the X button simply exited the app. Every
                // non-exit action now vetoes first, then acts.
                // (Exit is the one action that wants the close —
                // and it ends the process itself, deterministically.)
                let veto = matches!(
                    self.settings.close_action,
                    CloseAction::Ask | CloseAction::Tray | CloseAction::Taskbar | CloseAction::Hide
                );
                if veto {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
                match self.settings.close_action {
                    CloseAction::Ask => self.confirm_close = true,
                    CloseAction::Exit => self.exit_requested = true,
                    CloseAction::Tray => self.begin_panel_hide(),
                    CloseAction::Taskbar => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true))
                    }
                    CloseAction::Hide => self.hide_entirely(),
                }
            }
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            // A hidden root still needs a paint so viewports update.
            egui::CentralPanel::default().show(ctx, |_ui| {});
        }

        self.popup_ui(ctx);
        self.settings_ui(ctx);
        self.downloads_ui(ctx);
        self.converter_ui(ctx);
        self.tags_ui(ctx);
        self.player_ui(ctx);
        self.toasts_ui(ctx);
        self.about_ui(ctx);
        self.hide_icon_confirm_ui(ctx);
        self.close_choice_ui(ctx);
        #[cfg(feature = "field-validation")]
        self.field_metrics
            .record("render", field_render_start.elapsed());
        #[cfg(feature = "field-validation")]
        let field_geometry_start = Instant::now();
        // Capture existing windows once at frame start, before hide/close
        // intents. A newly-created viewport is restored on its next frame.
        #[cfg(feature = "field-validation")]
        self.field_metrics
            .record("geometry-after", field_geometry_start.elapsed());
        #[cfg(feature = "field-validation")]
        let field_persist_start = Instant::now();

        if self.settings_dirty {
            self.settings_dirty = false;
            self.seen
                .set_window(Duration::from_secs(self.settings.dedupe_window_secs));
            if let Err(e) = self.settings.save() {
                self.toast(format!("Saving settings failed: {e}"), ToastKind::Error);
            } else {
                // Our own save must not look like an external
                // change to the reload check above.
                self.settings_mtime = settings_file_mtime();
            }
        }

        if self.exit_requested {
            #[cfg(feature = "field-validation")]
            {
                self.field_metrics
                    .record("update", field_update_start.elapsed());
                self.field_metrics.finish();
            }
            self.maintain_window_geometry(true);
            // Exit only after owned workers acknowledge process-tree cleanup.
            let active = self.downloads.active_count();
            self.downloads.shutdown();
            self.background.shutdown();
            let _ = self.settings.save();
            let _ = self.history.save();
            yttraydl::diag_log(&format!("exiting (active downloads cancelled: {active})"));
            std::process::exit(0);
        }

        // Tray-delivery heartbeat (0.2.6): every ~5 s, record
        // whether update() is running at all plus the handler
        // send/drain counters — a "tray does nothing" report
        // then localizes from the log alone: no hb lines while
        // hidden = update is stalled; sends > drained with hb
        // running = the channel leg is broken.
        #[cfg(windows)]
        {
            self.frames += 1;
            if self.last_heartbeat.elapsed() >= Duration::from_secs(5) {
                self.last_heartbeat = Instant::now();
                use std::sync::atomic::Ordering;
                yttraydl::diag_log(&format!(
                    "hb: frames={} sends={} send_errs={} drained={} pending={} panel={}",
                    self.frames,
                    TRAY_SENDS.load(Ordering::Relaxed),
                    TRAY_SEND_ERRS.load(Ordering::Relaxed),
                    TRAY_DRAINED.load(Ordering::Relaxed),
                    TRAY_SENDS
                        .load(Ordering::Relaxed)
                        .saturating_sub(TRAY_DRAINED.load(Ordering::Relaxed)),
                    self.show_panel
                ));
            }
        }

        ctx.request_repaint_after(Duration::from_millis(200));
        #[cfg(feature = "field-validation")]
        {
            self.field_metrics
                .record("persistence", field_persist_start.elapsed());
            self.field_metrics
                .record("update", field_update_start.elapsed());
        }
    }
}

#[cfg(test)]
mod module_request_tests {
    use super::*;

    #[test]
    fn repeated_requests_focus_only_the_requested_existing_viewport() {
        for name in [
            "yttraydl-downloads",
            "yttraydl-converter",
            "yttraydl-tags",
            "yttraydl-player",
        ] {
            let ctx = egui::Context::default();
            ctx.set_embed_viewports(false);
            let id = egui::ViewportId::from_hash_of(name);
            let mut visible = false;
            let first = ctx.run(egui::RawInput::default(), |ctx| {
                request_module(ctx, &mut visible, name);
                ctx.show_viewport_deferred(id, egui::ViewportBuilder::default(), |_, _| {});
            });
            assert!(visible);
            assert!(!first.viewport_output.get(&id).is_some_and(|o| o
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Focus))));
            let second = ctx.run(egui::RawInput::default(), |ctx| {
                request_module(ctx, &mut visible, name);
                ctx.show_viewport_deferred(id, egui::ViewportBuilder::default(), |_, _| {});
            });
            assert!(visible);
            assert!(second.viewport_output.get(&id).is_some_and(|o| o
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Focus))));
        }
    }
}

//! egui front-end: tray icon + native tray menu, the control panel
//! (the root viewport, hidden until asked for), and the viewports
//! implemented in `gui_windows` (popup, settings, downloads, toasts).
//!
//! Family look: the black/red Command Deck identity shared with
//! InstallScope — near-black panels, hairline dividers, one red
//! accent. Every interactive control carries a tooltip (hover text),
//! and no essential information is tooltip-only.

mod gui_windows;

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};
use yttraydl::dedupe::SeenCache;
use yttraydl::download::{DownloadEvent, DownloadManager, JobState};
use yttraydl::history::{now_unix, History, HistoryEntry, HistoryStatus};
#[cfg(windows)]
use yttraydl::platform;
use yttraydl::platform::{PlatformEvent, PlatformSetup};
use yttraydl::rules::{decide, Decision};
#[cfg(windows)]
use yttraydl::settings::DefaultAction;
use yttraydl::settings::{CloseAction, Preset, Settings};
use yttraydl::url_detect::{extract_urls, host_of};
use yttraydl::ytdlp::{self, VideoInfo, YtDlpBinary};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0xE5, 0x48, 0x4D);
pub const ACCENT_DARK: egui::Color32 = egui::Color32::from_rgb(0x6B, 0x07, 0x00);
pub const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(0x10, 0x10, 0x10);
pub const HAIRLINE: egui::Color32 = egui::Color32::from_rgb(0x24, 0x24, 0x24);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xF5, 0xF5, 0xF5);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0xA8, 0xA8, 0xA8);
pub const TEXT_FAINT: egui::Color32 = egui::Color32::from_rgb(0x6E, 0x6E, 0x6E);
pub const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(0x4C, 0xC3, 0x6B);
/// Width of the slide-out settings drawer (0.2.1).
pub const SETTINGS_DRAWER_W: f32 = 530.0;

/// Runtime-generated window/tray icon: white down-arrow on a black
/// rounded square with a red outline (matches assets/icon.ico).
pub fn app_icon_data() -> egui::IconData {
    let size = 64usize;
    let mut rgba = vec![0u8; size * size * 4];
    let put = |rgba: &mut Vec<u8>, x: usize, y: usize, c: [u8; 4]| {
        if x < size && y < size {
            let i = (y * size + x) * 4;
            rgba[i..i + 4].copy_from_slice(&c);
        }
    };
    for y in 0..size {
        for x in 0..size {
            let border = x < 3 || y < 3 || x >= size - 3 || y >= size - 3;
            let in_arrow_shaft = (28..36).contains(&x) && (14..38).contains(&y);
            let in_arrow_head = (38..50).contains(&y) && {
                let half = (y - 38) as i32;
                (32 - (12 - half.min(12))) as usize <= x && x <= (32 + (12 - half.min(12))) as usize
            };
            let in_tray_line = (52..55).contains(&y) && (16..48).contains(&x);
            let px = if border {
                [0xE5, 0x48, 0x4D, 0xFF]
            } else if in_arrow_shaft || in_arrow_head {
                [0xF5, 0xF5, 0xF5, 0xFF]
            } else if in_tray_line {
                [0xE5, 0x48, 0x4D, 0xFF]
            } else {
                [0x10, 0x10, 0x10, 0xFF]
            };
            put(&mut rgba, x, y, px);
        }
    }
    egui::IconData {
        rgba,
        width: size as u32,
        height: size as u32,
    }
}

pub fn pick_file(filter_name: &str, ext: &str) -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        rfd::FileDialog::new()
            .add_filter(filter_name, &[ext])
            .pick_file()
    }
    #[cfg(not(windows))]
    {
        let _ = (filter_name, ext);
        None
    }
}

pub fn pick_save_json(default_name: &str) -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        rfd::FileDialog::new()
            .set_file_name(default_name)
            .add_filter("JSON settings", &["json"])
            .save_file()
    }
    #[cfg(not(windows))]
    {
        let _ = default_name;
        None
    }
}

pub fn pick_folder(initial: &std::path::Path) -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        rfd::FileDialog::new().set_directory(initial).pick_folder()
    }
    #[cfg(not(windows))]
    {
        let _ = initial;
        None
    }
}

pub fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL_BG;
    visuals.window_fill = PANEL_BG;
    visuals.extreme_bg_color = egui::Color32::BLACK;
    visuals.faint_bg_color = egui::Color32::from_rgb(0x1A, 0x1A, 0x1A);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, HAIRLINE);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(0x1C, 0x1C, 0x1C);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x28, 0x28, 0x28);
    visuals.widgets.active.bg_fill = ACCENT_DARK;
    visuals.selection.bg_fill = ACCENT_DARK;
    visuals.hyperlink_color = ACCENT;
    ctx.set_visuals(visuals);
}

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
/// drawer, about).
#[cfg(windows)]
fn handle_tray_menu_direct(event: &tray_icon::menu::MenuEvent) -> bool {
    let name = match with_tray_handles(|t| tray_item_name(t, &event.id)) {
        Some(n) => n,
        None => return false,
    };
    match name {
        "exit" => {
            yttraydl::diag_log("exit requested via tray menu (handler)");
            // Route the exit through the app's graceful
            // shutdown AND arm a forced fallback (0.2.8). Do
            // NOT drop the tray handles here: this handler runs
            // inside muda's menu dispatch, which holds a
            // RefCell borrow on its menu state — dropping muda
            // objects there panicked with "RefCell already
            // borrowed" in the field (0.2.7 log) and aborted
            // the process mid-exit. If the app's loop is
            // healthy it exits gracefully (cancelling
            // downloads, saving state) long before the
            // fallback fires; if it is wedged, the fallback
            // still guarantees the click works.
            forward_tray_event(TrayUiEvent::Menu(tray_icon::menu::MenuEvent {
                id: event.id.clone(),
            }));
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(750));
                yttraydl::diag_log("exiting (tray menu, forced fallback)");
                std::process::exit(0);
            });
            true
        }
        "monitoring toggle" => {
            let mut s = Settings::load();
            s.monitoring_enabled = !s.monitoring_enabled;
            let v = s.monitoring_enabled;
            match s.save() {
                Ok(()) => {
                    with_tray_handles(|t| t.monitoring.set_checked(v));
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
                    with_tray_handles(|t| t.auto_mode.set_checked(now_auto));
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
            let target = !platform::Platform::autostart_enabled();
            match platform::Platform::set_autostart(target) {
                Ok(()) => {
                    with_tray_handles(|t| t.autostart.set_checked(target));
                    let mut s = Settings::load();
                    s.start_with_windows = target;
                    let _ = s.save();
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
    /// Slide-out settings drawer animation, 0 = closed, 1 = open.
    settings_anim: f32,
    /// Hide the panel as soon as the settings drawer has fully
    /// closed — set by `begin_panel_hide`, consumed in `update`.
    /// Field fix (0.4.1): the panel must never hide in the same
    /// frame the drawer state is reset (see `begin_panel_hide`).
    pending_panel_hide: bool,
    /// Main-window size while the drawer is closed — the base the
    /// open animation grows from (tracks user resizes).
    closed_size: egui::Vec2,
    pub show_downloads: bool,
    pub show_about: bool,
    pub confirm_hide_icon: bool,
    /// The "closing the window — what should happen?" dialog.
    pub confirm_close: bool,
    /// The dialog's "remember my choice" checkbox state.
    pub close_remember: bool,
    settings_tab: usize,
    pub exit_requested: bool,

    popup_queue: VecDeque<PopupRequest>,
    pub active_popup: Option<PopupRequest>,
    info_tx: Sender<InfoResult>,
    info_rx: Receiver<InfoResult>,
    ytdlp_fetch_rx: Option<Receiver<FetchResult>>,
    ffmpeg_fetch_rx: Option<Receiver<FetchResult>>,

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
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        platform_setup: PlatformSetup,
        show_on_launch: bool,
        manual_url: Option<String>,
    ) -> Self {
        apply_theme(&cc.egui_ctx);
        // 0.2.6 diagnostics: if this line ever appears twice in
        // one launch, the app object was recreated — the prime
        // suspect for tray events being sent to a dead channel.
        yttraydl::diag_log("app created");
        let settings = Settings::load();
        let seen = SeenCache::new(Duration::from_secs(settings.dedupe_window_secs));
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let bin = ytdlp::locate_ytdlp(&settings, exe_dir.as_deref());
        let ffmpeg_ok = ytdlp::ffmpeg_available(&settings, exe_dir.as_deref());
        let (info_tx, info_rx) = std::sync::mpsc::channel();
        let mut app = Self {
            tray_visible: settings.tray_icon_visible,
            settings,
            settings_dirty: false,
            platform: platform_setup,
            seen,
            downloads: DownloadManager::new(),
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
            settings_anim: 0.0,
            pending_panel_hide: false,
            closed_size: egui::vec2(400.0, 540.0),
            show_downloads: false,
            show_about: false,
            confirm_hide_icon: false,
            confirm_close: false,
            close_remember: false,
            settings_tab: 0,
            exit_requested: false,
            popup_queue: VecDeque::new(),
            active_popup: None,
            info_tx,
            info_rx,
            ytdlp_fetch_rx: None,
            ffmpeg_fetch_rx: None,
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
        };
        yttraydl::diag_log(&format!(
            "ui start: yt-dlp {} (source: {}), ffmpeg {}",
            match &app.bin {
                Some(b) => b.path.display().to_string(),
                None => "not found".to_string(),
            },
            app.bin
                .as_ref()
                .map(|b| format!("{:?}", b.source))
                .unwrap_or_else(|| "-".to_string()),
            if app.ffmpeg_ok { "found" } else { "missing" },
        ));
        if app.bin.is_none() {
            // First-run honesty: without yt-dlp nothing can download.
            // Open the panel so the one-click install is in front of
            // the user instead of failing silently later.
            app.open_panel_on_start = true;
        }
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
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    pub fn relocalize_binaries(&mut self) {
        self.bin = ytdlp::locate_ytdlp(&self.settings, self.exe_dir().as_deref());
        self.ffmpeg_ok = ytdlp::ffmpeg_available(&self.settings, self.exe_dir().as_deref());
        self.version_requested_for = None;
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
        // Low-disk guard: while the download drive is under the
        // user's threshold, don't START queued jobs (running
        // ones finish). One toast per low spell.
        if self.settings.low_disk_threshold_mb > 0 && self.downloads.active_count() > 0 {
            let threshold = self
                .settings
                .low_disk_threshold_mb
                .saturating_mul(1024 * 1024);
            // The folder may not exist yet; query the nearest
            // existing ancestor.
            let mut probe = self.settings.download_folder.clone();
            while !probe.exists() {
                if !probe.pop() {
                    break;
                }
            }
            if let Some(free) = yttraydl::platform::free_disk_bytes(&probe) {
                if free < threshold {
                    if !self.disk_warned {
                        self.disk_warned = true;
                        self.toast(
                            format!(
                                "Low disk space ({} free) — new downloads are paused until space frees up.",
                                yttraydl::ytdlp::format_bytes(free)
                            ),
                            ToastKind::Error,
                        );
                    }
                    return;
                }
            }
        }
        self.disk_warned = false;
        let bin = self.bin_path();
        self.downloads.pump(&self.settings, bin.as_deref());
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
                self.active_popup = Some(p);
            }
        }
    }

    fn spawn_info_fetch(&self, url: &str) {
        if let Some(bin) = self.bin_path() {
            let tx = self.info_tx.clone();
            let settings = self.settings.clone();
            let url_owned = url.to_string();
            std::thread::spawn(move || {
                let info = yttraydl::fetch_info(&bin, &settings, &url_owned);
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
        if self.ytdlp_fetch_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.ytdlp_fetch_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(yttraydl::download_managed_ytdlp());
        });
        self.toast("Downloading yt-dlp from GitHub…", ToastKind::Info);
    }

    pub fn start_ffmpeg_download(&mut self) {
        if self.ffmpeg_fetch_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.ffmpeg_fetch_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(yttraydl::download_managed_ffmpeg());
        });
        self.toast("Downloading ffmpeg (about 110 MB)…", ToastKind::Info);
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
        std::thread::spawn(move || {
            let _ = tx.send(yttraydl::fetch_latest_ytdlp_tag());
        });
    }

    fn process_ytdlp_check(&mut self) {
        if let Some(rx) = &self.ytdlp_check_rx {
            if let Ok(result) = rx.try_recv() {
                self.ytdlp_check_rx = None;
                self.ytdlp_latest = Some(result);
            }
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
    /// `hide_entirely` — closing the settings drawer FIRST.
    ///
    /// Field report (SEAN, v0.4.0): closing the window with the
    /// Settings drawer open, then restoring it, brought back a
    /// BLANK window. The old code reset the drawer state and hid
    /// the window in the same frame, so the window went away at
    /// its drawer-open width and the shrink-back resize never
    /// ran. Now the drawer slides shut and the window shrinks to
    /// its panel-only width like a normal Settings close, and
    /// only once it is fully closed does the panel hide (the
    /// deferred half lives in `update`). With the animation
    /// setting off, the close snaps and the hide follows a frame
    /// later — same guarantee.
    pub fn begin_panel_hide(&mut self) {
        if self.show_settings || self.settings_anim > 0.0 {
            self.show_settings = false;
            self.pending_panel_hide = true;
        } else {
            self.show_panel = false;
        }
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
        // The panel half defers until the settings drawer has
        // closed — see `begin_panel_hide` (0.4.1).
        self.begin_panel_hide();
    }

    // ---------- per-frame processing ----------

    fn process_platform_events(&mut self) {
        while let Ok(ev) = self.platform.events.try_recv() {
            match ev {
                PlatformEvent::ClipboardText(text) => {
                    if self.settings.monitoring_enabled {
                        for url in extract_urls(&text) {
                            self.handle_detected_url(&url, false);
                        }
                    }
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
                if self
                    .popup_thumb
                    .as_ref()
                    .map(|(u, _)| u != &thumb_url)
                    .unwrap_or(true)
                {
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
        std::thread::spawn(move || {
            let decoded = yttraydl::fetch_url_bytes(&url).and_then(|bytes| {
                let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
                let (w, h) = img.dimensions();
                Some((w, h, img.into_raw()))
            });
            let _ = tx.send((url, decoded));
        });
    }

    fn process_thumbs(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.thumb_rx {
            if let Ok((url, decoded)) = rx.try_recv() {
                self.thumb_rx = None;
                if let Some((w, h, rgba)) = decoded {
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
        std::thread::spawn(move || {
            let results: Vec<(String, Vec<yttraydl::ytdlp::PlaylistEntry>)> = subs
                .iter()
                .map(|sub| {
                    (
                        sub.url.clone(),
                        yttraydl::fetch_playlist_entries(&bin, &settings, &sub.url, 10),
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
            if let Ok(results) = rx.try_recv() {
                self.watch_rx = None;
                self.watch_data.last_check_unix = yttraydl::history::now_unix().max(0) as u64;
                let mut new_count = 0usize;
                for (sub_url, entries) in results {
                    if entries.is_empty() {
                        continue;
                    }
                    let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
                    let fresh = self.watch_data.record_poll(&sub_url, &ids);
                    if fresh.is_empty() {
                        continue;
                    }
                    let preset = self
                        .settings
                        .subscriptions
                        .iter()
                        .find(|s| s.url == sub_url)
                        .map(|s| s.preset)
                        .unwrap_or(self.settings.default_preset);
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
                if job.state == JobState::Failed && job.attempts < self.settings.auto_retry_attempts
                {
                    // Auto-retry with the original preset; the
                    // failure is not recorded as final yet.
                    let next = job.attempts + 1;
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
                let (status, error) = match job.state {
                    JobState::Done => (HistoryStatus::Completed, None),
                    JobState::Cancelled => (HistoryStatus::Cancelled, None),
                    _ => (
                        HistoryStatus::Failed,
                        job.error.clone().or_else(|| {
                            job.tail_lines
                                .iter()
                                .rev()
                                .find(|l| l.contains("ERROR"))
                                .cloned()
                        }),
                    ),
                };
                self.history.push(HistoryEntry {
                    finished_at_unix: now_unix(),
                    url: job.url.clone(),
                    title: if job.title.is_empty() || job.title == job.url {
                        host_of(&job.url)
                    } else {
                        job.title.clone()
                    },
                    status,
                    file: job.final_file.clone(),
                    error: error.clone(),
                    preset_label: job.preset.label().to_string(),
                });
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
                                let line = cmd
                                    .replace("{file}", &file.to_string_lossy())
                                    .replace("{dir}", &dir)
                                    .replace("{url}", &job.url)
                                    .replace("{title}", &job.title);
                                yttraydl::platform::run_shell_command(&line);
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
        if let Some(rx) = &self.ytdlp_fetch_rx {
            if let Ok(result) = rx.try_recv() {
                self.ytdlp_fetch_rx = None;
                match result {
                    Ok(path) => {
                        self.relocalize_binaries();
                        self.toast(
                            format!("yt-dlp installed at {}", path.display()),
                            ToastKind::Success,
                        );
                        self.pump_downloads();
                    }
                    Err(e) => {
                        self.toast(format!("yt-dlp download failed: {e}"), ToastKind::Error);
                    }
                }
            }
        }
    }

    fn process_ffmpeg_fetch(&mut self) {
        if let Some(rx) = &self.ffmpeg_fetch_rx {
            if let Ok(result) = rx.try_recv() {
                self.ffmpeg_fetch_rx = None;
                match result {
                    Ok(path) => {
                        self.relocalize_binaries();
                        self.toast(
                            format!(
                                "ffmpeg installed at {} — merging and MP3 conversion are available",
                                path.display()
                            ),
                            ToastKind::Success,
                        );
                    }
                    Err(e) => {
                        self.toast(format!("ffmpeg download failed: {e}"), ToastKind::Error);
                    }
                }
            }
        }
    }

    fn ensure_version(&mut self) {
        if let Some(bin) = self.bin_path() {
            if self.version_requested_for.as_ref() != Some(&bin) {
                self.version_requested_for = Some(bin.clone());
                self.ytdlp_version = None;
                let (tx, rx) = std::sync::mpsc::channel::<String>();
                std::thread::spawn(move || {
                    if let Ok(out) = std::process::Command::new(&bin).arg("--version").output() {
                        let _ = tx.send(String::from_utf8_lossy(&out.stdout).trim().to_string());
                    }
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
            if let Ok(v) = rx.try_recv() {
                if !v.is_empty() {
                    self.ytdlp_version = Some(v);
                }
                self.version_rx = None;
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
        let open_folder = MenuItem::new("Open download folder", true, None);
        let pause_all = MenuItem::new("Pause all downloads", true, None);
        let settings_item = MenuItem::new("Settings\u{2026}", true, None);
        let autostart = CheckMenuItem::new(
            "Start with Windows",
            true,
            platform::Platform::autostart_enabled(),
            None,
        );
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
        );
        let _ = menu.append_items(&[
            &header,
            &PredefinedMenuItem::separator(),
            &monitoring,
            &auto_mode,
            &PredefinedMenuItem::separator(),
            &download_clipboard,
            &downloads,
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
                id_settings,
                id_about,
                id_exit,
                id_pause_all,
            ) = match with_tray_handles(|t| {
                (
                    tray_item_name(t, &ev.id),
                    t.id_download_clipboard.clone(),
                    t.id_downloads.clone(),
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
                self.show_downloads = true;
                self.show_panel = true;
            } else if ev.id == id_settings {
                // Settings is a drawer in the main window now —
                // opening it means showing the window too.
                self.show_settings = true;
                self.show_panel = true;
            } else if ev.id == id_about {
                self.show_about = true;
                self.show_panel = true;
            } else if ev.id == id_pause_all {
                self.toggle_pause_all();
            } else if ev.id == id_exit {
                // Graceful half of the handler's exit route
                // (0.2.8): the full shutdown below cancels
                // downloads and saves state; the handler's
                // forced fallback only fires if this never runs.
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
        with_tray_handles(|t| {
            t._tray.set_tooltip(Some(self.downloads.summary())).ok();
            t.monitoring.set_checked(self.settings.monitoring_enabled);
            t.auto_mode
                .set_checked(self.settings.default_action == DefaultAction::AutoDownload);
        });
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
        self.ensure_tray(ctx);
        self.process_platform_events();
        self.process_info_results();
        self.process_download_events();
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
                self.tray_visible = self.settings.tray_icon_visible;
                yttraydl::diag_log("settings reloaded (changed outside the UI)");
            }
        }
        self.ensure_version();
        self.activate_next_popup();
        self.sync_tray();

        if self.first_frame {
            self.first_frame = false;
            if self.open_panel_on_start {
                self.show_panel = true;
            }
        }

        // Reopening Settings cancels a deferred panel hide (the
        // user changed their mind mid-close).
        if self.show_settings {
            self.pending_panel_hide = false;
        }

        // Settings drawer slide (0.2.1): animate 0<->1 over
        // ~220 ms; while it moves, the window itself grows/shrinks
        // by the drawer width so the panel never gets squeezed.
        // Once settled, sizing is the user's again.
        {
            let target = if self.show_settings { 1.0 } else { 0.0 };
            if (self.settings_anim - target).abs() > 0.0005 && self.show_panel {
                if !self.settings.animate_settings_drawer {
                    // Animation off (user setting): snap straight
                    // to the end state; the resize below runs once
                    // with the final width.
                    self.settings_anim = target;
                } else {
                    let dt = ctx.input(|i| i.stable_dt).clamp(0.001, 0.1);
                    let step = dt / 0.22;
                    self.settings_anim = if target > self.settings_anim {
                        (self.settings_anim + step).min(1.0)
                    } else {
                        (self.settings_anim - step).max(0.0)
                    };
                }
                let e = 1.0 - (1.0 - self.settings_anim).powi(3); // ease-out cubic
                let w = self.closed_size.x + SETTINGS_DRAWER_W * e;
                // SEAN (0.2.5): opening Settings expands the
                // window sideways ONLY — the height never
                // changes. (Before, it also grew toward
                // 580 px tall.) The drawer's content scrolls,
                // so any window height works.
                let h = ctx
                    .input(|i| i.viewport().inner_rect)
                    .map(|r| r.height())
                    .unwrap_or(self.closed_size.y);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
                ctx.request_repaint();
            } else if self.settings_anim == 0.0 && self.show_panel {
                // Fully closed: follow the user's window size as
                // the base the next open-animation grows from.
                if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
                    self.closed_size = r.size();
                }
            }
        }

        // Deferred panel hide (0.4.1): the drawer has fully
        // closed and the block above has shrunk the window back
        // to its panel-only width — the panel may hide now.
        if self.pending_panel_hide && !self.show_settings && self.settings_anim == 0.0 {
            self.pending_panel_hide = false;
            self.show_panel = false;
        }

        // Root viewport = the control panel (+ settings drawer).
        if self.show_panel {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            egui::CentralPanel::default().show(ctx, |ui| {
                if self.settings_anim > 0.001 {
                    let e = 1.0 - (1.0 - self.settings_anim).powi(3);
                    let resp = egui::SidePanel::right("settings-drawer")
                        .exact_width(SETTINGS_DRAWER_W * e)
                        .resizable(false)
                        .frame(
                            egui::Frame::NONE
                                .fill(egui::Color32::from_rgb(0x0B, 0x0B, 0x0B))
                                .inner_margin(egui::Margin::same(10)),
                        )
                        .show_inside(ui, |ui| {
                            self.settings_drawer_ui(ui);
                        });
                    // Hairline on the drawer's left edge.
                    let r = resp.response.rect;
                    ui.painter()
                        .vline(r.left(), r.y_range(), egui::Stroke::new(1.0_f32, HAIRLINE));
                }
                self.panel_ui(ui);
            });
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
        self.downloads_ui(ctx);
        self.toasts_ui(ctx);
        self.about_ui(ctx);
        self.hide_icon_confirm_ui(ctx);
        self.close_choice_ui(ctx);

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
            // Deterministic exit. The graceful path (sending
            // ViewportCommand::Close and waiting for the window
            // stack to unwind) failed in the field — the tray
            // Exit click appeared to do nothing. So: cancel any
            // running downloads (workers kill their yt-dlp child
            // within ~120 ms; give them a beat), persist state,
            // log, and end the process. No handshake left to stall.
            let active = self.downloads.active_count();
            self.downloads.cancel_all();
            if active > 0 {
                std::thread::sleep(Duration::from_millis(300));
            }
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
    }
}

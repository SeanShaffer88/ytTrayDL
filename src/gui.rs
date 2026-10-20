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
use yttraydl::settings::{Preset, Settings};
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
    pub shown_at: Instant,
    pub last_interaction: Instant,
}

type InfoResult = (String, Option<VideoInfo>, bool);
type FetchResult = Result<std::path::PathBuf, String>;

#[cfg(windows)]
pub struct TrayHandles {
    _tray: tray_icon::TrayIcon,
    monitoring: tray_icon::menu::CheckMenuItem,
    auto_mode: tray_icon::menu::CheckMenuItem,
    autostart: tray_icon::menu::CheckMenuItem,
    id_monitoring: tray_icon::menu::MenuId,
    id_auto: tray_icon::menu::MenuId,
    id_autostart: tray_icon::menu::MenuId,
    id_download_clipboard: tray_icon::menu::MenuId,
    id_downloads: tray_icon::menu::MenuId,
    id_open_folder: tray_icon::menu::MenuId,
    id_settings: tray_icon::menu::MenuId,
    id_hide_icon: tray_icon::menu::MenuId,
    id_about: tray_icon::menu::MenuId,
    id_exit: tray_icon::menu::MenuId,
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

    #[cfg(windows)]
    tray: Option<TrayHandles>,
    #[cfg(not(windows))]
    #[allow(dead_code)]
    tray: Option<()>,
    tray_failed: bool,
    tray_visible: bool,

    pub show_panel: bool,
    pub show_settings: bool,
    pub show_downloads: bool,
    pub show_about: bool,
    pub confirm_hide_icon: bool,
    settings_tab: usize,
    pub exit_requested: bool,

    popup_queue: VecDeque<PopupRequest>,
    pub active_popup: Option<PopupRequest>,
    info_tx: Sender<InfoResult>,
    info_rx: Receiver<InfoResult>,
    ytdlp_fetch_rx: Option<Receiver<FetchResult>>,

    pub toasts: Vec<Toast>,
    pub last_ignored: Vec<(String, String)>,
    // Domain-rule editor input buffers (Settings → Domains).
    pub rule_input_ignore: String,
    pub rule_input_auto: String,
    pub rule_input_allow: String,
    pub rule_input_url: String,
    pub reset_armed: bool,
    manual_url: String,
    first_frame: bool,
    open_panel_on_start: bool,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        platform_setup: PlatformSetup,
        show_on_launch: bool,
        manual_url: Option<String>,
    ) -> Self {
        apply_theme(&cc.egui_ctx);
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
            tray: None,
            tray_failed: false,
            show_panel: false,
            show_settings: false,
            show_downloads: false,
            show_about: false,
            confirm_hide_icon: false,
            settings_tab: 0,
            exit_requested: false,
            popup_queue: VecDeque::new(),
            active_popup: None,
            info_tx,
            info_rx,
            ytdlp_fetch_rx: None,
            toasts: Vec::new(),
            last_ignored: Vec::new(),
            rule_input_ignore: String::new(),
            rule_input_auto: String::new(),
            rule_input_allow: String::new(),
            rule_input_url: String::new(),
            reset_armed: false,
            manual_url: String::new(),
            first_frame: true,
            open_panel_on_start: show_on_launch,
        };
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
        if self.bin.is_none() {
            self.toast(
                "yt-dlp is not installed yet — open ytTrayDL to install it.",
                ToastKind::Error,
            );
            self.show_panel = true;
            return;
        }
        let use_archive = self.settings.skip_already_downloaded && !force;
        self.downloads
            .enqueue(url.to_string(), title.to_string(), preset, use_archive);
        self.pump_downloads();
        if self.settings.notify_on_auto_start && title.is_empty() {
            self.toast(
                format!("Downloading from {host}…", host = host_of(url)),
                ToastKind::Info,
            );
        }
    }

    fn pump_downloads(&mut self) {
        let bin = self.bin_path();
        self.downloads.pump(&self.settings, bin.as_deref());
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
                    self.start_download(url, "", self.settings.default_preset, false);
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
            preset: self.settings.default_preset,
            always_auto_domain: false,
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
                    if let Some(t) = &self.tray {
                        t._tray.set_visible(true).ok();
                    }
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
    }

    fn process_download_events(&mut self) {
        let events = self.downloads.poll_events();
        let mut finished_any = false;
        for ev in events {
            if let DownloadEvent::Finished(job) = ev {
                finished_any = true;
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
                        if self.settings.notify_on_complete {
                            self.toast(
                                format!("Download finished: {}", job.title),
                                ToastKind::Success,
                            );
                        }
                    }
                    JobState::Failed => {
                        self.seen.forget(&job.url);
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
    fn ensure_tray(&mut self, _ctx: &egui::Context) {
        if self.tray.is_some() || self.tray_failed {
            return;
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
            Err(_) => {
                self.tray_failed = true;
                return;
            }
        };
        let tray = match tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("ytTrayDL \u{2014} monitoring clipboard")
            .with_icon(icon)
            .build()
        {
            Ok(t) => t,
            Err(_) => {
                // No tray available (e.g. Linux dev build without an
                // app-indicator host). The panel still works; tray
                // features are Windows-verified functionality.
                self.tray_failed = true;
                return;
            }
        };
        if !self.tray_visible {
            tray.set_visible(false).ok();
        }
        self.tray = Some(TrayHandles {
            _tray: tray,
            monitoring,
            auto_mode,
            autostart,
            id_monitoring: ids.0,
            id_auto: ids.1,
            id_autostart: ids.2,
            id_download_clipboard: ids.3,
            id_downloads: ids.4,
            id_open_folder: ids.5,
            id_settings: ids.6,
            id_hide_icon: ids.7,
            id_about: ids.8,
            id_exit: ids.9,
        });
    }

    #[cfg(windows)]
    fn process_tray_events(&mut self) {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                self.show_panel = !self.show_panel;
            }
        }
        while let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            let ids = match &self.tray {
                Some(t) => (
                    t.id_monitoring.clone(),
                    t.id_auto.clone(),
                    t.id_autostart.clone(),
                    t.id_download_clipboard.clone(),
                    t.id_downloads.clone(),
                    t.id_open_folder.clone(),
                    t.id_settings.clone(),
                    t.id_hide_icon.clone(),
                    t.id_about.clone(),
                    t.id_exit.clone(),
                ),
                None => break,
            };
            if ev.id == ids.0 {
                self.settings.monitoring_enabled = !self.settings.monitoring_enabled;
                let v = self.settings.monitoring_enabled;
                if let Some(t) = &self.tray {
                    t.monitoring.set_checked(v);
                }
                self.settings_dirty = true;
            } else if ev.id == ids.1 {
                let now_auto = self.settings.default_action != DefaultAction::AutoDownload;
                self.settings.default_action = if now_auto {
                    DefaultAction::AutoDownload
                } else {
                    DefaultAction::Prompt
                };
                if let Some(t) = &self.tray {
                    t.auto_mode.set_checked(now_auto);
                }
                self.settings_dirty = true;
            } else if ev.id == ids.2 {
                let target = !platform::Platform::autostart_enabled();
                match platform::Platform::set_autostart(target) {
                    Ok(()) => {
                        if let Some(t) = &self.tray {
                            t.autostart.set_checked(target);
                        }
                        self.settings.start_with_windows = target;
                        self.settings_dirty = true;
                    }
                    Err(e) => self.toast(format!("Autostart change failed: {e}"), ToastKind::Error),
                }
            } else if ev.id == ids.3 {
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
            } else if ev.id == ids.4 {
                self.show_downloads = true;
            } else if ev.id == ids.5 {
                let folder = self.settings.download_folder.clone();
                let _ = std::fs::create_dir_all(&folder);
                if let Err(e) = platform::open_path(&folder) {
                    self.toast(format!("Could not open folder: {e}"), ToastKind::Error);
                }
            } else if ev.id == ids.6 {
                self.show_settings = true;
            } else if ev.id == ids.7 {
                self.confirm_hide_icon = true;
            } else if ev.id == ids.8 {
                self.show_about = true;
            } else if ev.id == ids.9 {
                self.exit_requested = true;
            }
        }
    }

    #[cfg(windows)]
    fn sync_tray(&mut self) {
        if let Some(t) = &self.tray {
            t._tray.set_tooltip(Some(self.downloads.summary())).ok();
            t.monitoring.set_checked(self.settings.monitoring_enabled);
            t.auto_mode
                .set_checked(self.settings.default_action == DefaultAction::AutoDownload);
        }
    }
}

#[cfg(not(windows))]
impl App {
    fn ensure_tray(&mut self, _ctx: &egui::Context) {
        // No tray on non-Windows dev builds.
        self.tray_failed = true;
    }
    fn process_tray_events(&mut self) {}
    fn sync_tray(&mut self) {}
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ensure_tray(ctx);
        self.process_platform_events();
        self.process_info_results();
        self.process_download_events();
        self.process_ytdlp_fetch();
        self.process_tray_events();
        self.ensure_version();
        self.activate_next_popup();
        self.sync_tray();

        if self.first_frame {
            self.first_frame = false;
            if self.open_panel_on_start {
                self.show_panel = true;
            }
        }

        // Root viewport = the control panel.
        if self.show_panel {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            egui::CentralPanel::default().show(ctx, |ui| {
                self.panel_ui(ui);
            });
            if ctx.input(|i| i.viewport().close_requested()) {
                self.show_panel = false;
            }
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            // A hidden root still needs a paint so viewports update.
            egui::CentralPanel::default().show(ctx, |_ui| {});
        }

        self.popup_ui(ctx);
        self.settings_ui(ctx);
        self.downloads_ui(ctx);
        self.toasts_ui(ctx);
        self.about_ui(ctx);
        self.hide_icon_confirm_ui(ctx);

        if self.settings_dirty {
            self.settings_dirty = false;
            self.seen
                .set_window(Duration::from_secs(self.settings.dedupe_window_secs));
            if let Err(e) = self.settings.save() {
                self.toast(format!("Saving settings failed: {e}"), ToastKind::Error);
            }
        }

        if self.exit_requested {
            let _ = self.settings.save();
            let _ = self.history.save();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

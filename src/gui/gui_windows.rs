//! The viewports: control panel (root), download popup, settings,
//! downloads, toasts, about, and the hide-icon confirmation.
//!
//! Viewport closures can't borrow `self` while `update` also holds
//! it, so each window extracts the state it needs (popup via
//! `Option::take`, settings/downloads via `mem::take`), renders,
//! and returns an outcome struct the caller applies afterwards.

use super::*;
use yttraydl::platform;
use yttraydl::settings::{Container, DefaultAction, DetectionScope, PopupPosition, Preset};
use yttraydl::ytdlp::YtDlpSource;

fn status_color(status: HistoryStatus) -> egui::Color32 {
    match status {
        HistoryStatus::Completed => OK_GREEN,
        HistoryStatus::Failed => ACCENT,
        HistoryStatus::Cancelled => TEXT_FAINT,
    }
}

fn source_label(source: &YtDlpSource) -> &'static str {
    match source {
        YtDlpSource::UserConfigured => "path you set in Settings",
        YtDlpSource::BesideExe => "next to ytTrayDL",
        YtDlpSource::Managed => "ytTrayDL-managed copy",
        YtDlpSource::Path => "found on PATH",
    }
}

fn monitor_size(ctx: &egui::Context) -> egui::Vec2 {
    ctx.input(|i| i.viewport().monitor_size)
        .unwrap_or(egui::vec2(1920.0, 1080.0))
}

fn bottom_right(ctx: &egui::Context, size: egui::Vec2, lift: f32) -> egui::Pos2 {
    let mon = monitor_size(ctx);
    egui::pos2(
        (mon.x - size.x - 12.0).max(0.0),
        (mon.y - size.y - 12.0 - lift).max(0.0),
    )
}

impl App {
    // ---------------- Control panel (root viewport) ----------------

    pub fn panel_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("ytTrayDL").color(TEXT));
            ui.label(
                egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).color(TEXT_FAINT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("✕ Close")
                    .on_hover_text("Close this panel. ytTrayDL keeps running in the tray.")
                    .clicked()
                {
                    self.show_panel = false;
                }
            });
        });
        ui.separator();

        // Status line.
        let (dot, word) = if self.settings.monitoring_enabled {
            (OK_GREEN, "Monitoring clipboard")
        } else {
            (ACCENT, "Monitoring paused")
        };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("●").color(dot));
            ui.label(word);
            ui.label(egui::RichText::new("·").color(TEXT_FAINT));
            ui.label(match self.settings.default_action {
                DefaultAction::Prompt => "asks before downloading",
                DefaultAction::AutoDownload => "auto-downloads with your defaults",
            })
            .on_hover_text("Change this in Settings → General, or from the tray menu.");
        });

        // yt-dlp status / first-run install.
        ui.add_space(6.0);
        egui::Frame::group(ui.style())
            .fill(egui::Color32::from_rgb(0x18, 0x18, 0x18))
            .show(ui, |ui| match &self.bin {
                Some(bin) => {
                    let version = self
                        .ytdlp_version
                        .clone()
                        .unwrap_or_else(|| "…".into());
                    ui.label(format!(
                        "yt-dlp {version} — {}",
                        source_label(&bin.source)
                    ))
                    .on_hover_text(bin.path.display().to_string());
                    if self.ffmpeg_ok {
                        ui.label(egui::RichText::new("ffmpeg found — merging and MP3 conversion available.").color(TEXT_DIM));
                    } else {
                        ui.label(egui::RichText::new("ffmpeg not found — best-quality streams that need merging and MP3 conversion will fail. Put ffmpeg.exe next to ytTrayDL or on PATH.").color(ACCENT));
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new("yt-dlp is not installed yet — ytTrayDL can't download without it.")
                            .color(ACCENT)
                            .strong(),
                    );
                    ui.label("One click installs the official standalone yt-dlp.exe from GitHub into ytTrayDL's own folder. Nothing else on your PC is changed.");
                    ui.horizontal(|ui| {
                        let busy = self.ytdlp_fetch_rx.is_some();
                        if ui
                            .add_enabled(!busy, egui::Button::new(if busy { "Downloading…" } else { "⬇ Install yt-dlp" }))
                            .on_hover_text("Download yt-dlp.exe from github.com/yt-dlp/yt-dlp releases into %APPDATA%\\ytTrayDL\\bin")
                            .clicked()
                        {
                            self.start_ytdlp_download();
                        }
                        if ui
                            .button("Choose yt-dlp.exe…")
                            .on_hover_text("Use a yt-dlp.exe you already have")
                            .clicked()
                        {
                            if let Some(f) = pick_file("yt-dlp", "exe") {
                                self.settings.ytdlp_path = f;
                                self.settings_dirty = true;
                                self.relocalize_binaries();
                            }
                        }
                        if ui
                            .button("Releases page")
                            .on_hover_text("Open the yt-dlp releases page in your browser")
                            .clicked()
                        {
                            let _ = platform::open_url("https://github.com/yt-dlp/yt-dlp/releases");
                        }
                    });
                },
            });

        // Quick controls.
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut monitoring = self.settings.monitoring_enabled;
            if ui
                .checkbox(&mut monitoring, "Monitoring")
                .on_hover_text("Watch the clipboard for supported links")
                .changed()
            {
                self.settings.monitoring_enabled = monitoring;
                self.settings_dirty = true;
            }
            let mut auto = self.settings.default_action == DefaultAction::AutoDownload;
            if ui
                .checkbox(&mut auto, "Auto-download")
                .on_hover_text("Skip the popup and download with your default preset")
                .changed()
            {
                self.settings.default_action = if auto {
                    DefaultAction::AutoDownload
                } else {
                    DefaultAction::Prompt
                };
                self.settings_dirty = true;
            }
        });
        ui.horizontal(|ui| {
            if ui
                .button("⚙ Settings…")
                .on_hover_text("All ytTrayDL settings")
                .clicked()
            {
                self.show_settings = true;
            }
            let active = self.downloads.active_count();
            let label = if active > 0 {
                format!("⬇ Downloads ({active} active)…")
            } else {
                "⬇ Downloads…".to_string()
            };
            if ui
                .button(label)
                .on_hover_text("Active downloads and history")
                .clicked()
            {
                self.show_downloads = true;
            }
            if ui
                .button("📂 Download folder")
                .on_hover_text("Open the download folder in Explorer")
                .clicked()
            {
                let folder = self.settings.download_folder.clone();
                let _ = std::fs::create_dir_all(&folder);
                if let Err(e) = platform::open_path(&folder) {
                    self.toast(format!("Could not open folder: {e}"), ToastKind::Error);
                }
            }
        });

        // Manual URL entry.
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Download a link directly").color(TEXT_DIM));
        ui.horizontal(|ui| {
            let resp = ui
                .add(
                    egui::TextEdit::singleline(&mut self.manual_url)
                        .hint_text("Paste a video URL…")
                        .desired_width(240.0),
                )
                .on_hover_text("Type or paste a URL, then press Enter or Download");
            let go = ui
                .button("Download…")
                .on_hover_text("Open the download popup for this link")
                .clicked();
            if (go || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                && !self.manual_url.trim().is_empty()
            {
                let url = self.manual_url.trim().to_string();
                self.manual_url.clear();
                self.handle_detected_url(&url, true);
            }
        });

        // Active downloads (compact).
        let active: Vec<(u64, String, f32, Option<String>, JobState)> = self
            .downloads
            .running_jobs()
            .chain(self.downloads.queued_jobs())
            .map(|j| {
                (
                    j.id,
                    j.title.clone(),
                    j.percent,
                    j.eta_text.clone(),
                    j.state,
                )
            })
            .collect();
        if !active.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Downloading").strong());
            let mut cancel: Option<u64> = None;
            for (id, title, pct, eta, state) in &active {
                ui.horizontal(|ui| {
                    ui.label(truncate(title, 26)).on_hover_text(title.clone());
                    if *state == JobState::Queued {
                        ui.label(egui::RichText::new("queued").color(TEXT_FAINT));
                    } else {
                        ui.add(egui::ProgressBar::new(pct / 100.0).desired_width(90.0));
                        ui.label(format!(
                            "{pct:.0}%{}",
                            eta.as_ref()
                                .map(|e| format!(" · ETA {e}"))
                                .unwrap_or_default()
                        ));
                    }
                    if ui
                        .small_button("✕")
                        .on_hover_text("Cancel this download (the partial file is kept for resume)")
                        .clicked()
                    {
                        cancel = Some(*id);
                    }
                });
            }
            if let Some(id) = cancel {
                self.downloads.cancel(id);
            }
        }

        // Recent history.
        if !self.history.entries.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Recent").strong());
            let recent: Vec<(String, HistoryStatus, Option<std::path::PathBuf>)> = self
                .history
                .recent(5)
                .iter()
                .map(|e| (e.title.clone(), e.status, e.file.clone()))
                .collect();
            let mut open_file: Option<std::path::PathBuf> = None;
            for (title, status, file) in &recent {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("●").color(status_color(*status)));
                    ui.label(truncate(title, 34)).on_hover_text(title.clone());
                    if let Some(f) = file {
                        if ui
                            .small_button("📂")
                            .on_hover_text("Show in Explorer")
                            .clicked()
                        {
                            open_file = Some(f.clone());
                        }
                    }
                });
            }
            if let Some(f) = open_file {
                if let Some(parent) = f.parent() {
                    let _ = platform::open_path(parent);
                }
            }
        }

        // Recently ignored (transparency: the user can see why a
        // copied link produced no popup).
        if !self.last_ignored.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Ignored recently").color(TEXT_FAINT));
            for (url, reason) in self.last_ignored.iter().take(3) {
                ui.label(
                    egui::RichText::new(format!("• {} — {reason}", truncate(url, 44)))
                        .color(TEXT_FAINT)
                        .small(),
                )
                .on_hover_text(url.clone());
            }
        }

        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .button("Hide tray icon…")
                .on_hover_text("Run without a tray icon. Relaunch ytTrayDL or press Ctrl+Alt+G to get it back.")
                .clicked()
            {
                self.confirm_hide_icon = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("Exit ytTrayDL")
                    .on_hover_text("Quit completely (downloads in progress are interrupted; partial files can resume)")
                    .clicked()
                {
                    self.exit_requested = true;
                }
            });
        });
    }

    // ---------------- Download popup ----------------

    pub fn popup_ui(&mut self, ctx: &egui::Context) {
        #[derive(PartialEq)]
        enum Action {
            None,
            Dismiss,
            Download,
            IgnoreUrl,
            IgnoreDomain,
        }
        let Some(mut popup) = self.active_popup.take() else {
            return;
        };
        let size = egui::vec2(400.0, 268.0);
        let pos = match self.settings.popup_position {
            PopupPosition::BottomRight => bottom_right(ctx, size, 0.0),
            PopupPosition::TopRight => {
                let mon = monitor_size(ctx);
                egui::pos2((mon.x - size.x - 12.0).max(0.0), 12.0)
            }
            PopupPosition::NearCursor => match platform::cursor_position() {
                Some((x, y)) => {
                    let mon = monitor_size(ctx);
                    egui::pos2(
                        (x + 14.0).min((mon.x - size.x - 4.0).max(0.0)),
                        (y + 14.0).min((mon.y - size.y - 4.0).max(0.0)),
                    )
                }
                None => bottom_right(ctx, size, 0.0),
            },
        };
        let mut action = Action::None;
        let mut interacted = false;
        let playlists_allowed = self.settings.allow_playlists;
        let bin_missing = self.bin.is_none();
        let custom_format = self.settings.custom_format.clone();
        let timeout_secs = self.settings.popup_timeout_secs;
        let id = egui::ViewportId::from_hash_of(("yttraydl-popup", &popup.url));
        ctx.show_viewport_immediate(
            id,
            egui::ViewportBuilder::default()
                .with_title("ytTrayDL — download?")
                .with_inner_size(size)
                .with_min_inner_size(size)
                .with_max_inner_size(size)
                .with_position(pos)
                .with_always_on_top()
                .with_decorations(true),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    action = Action::Dismiss;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    if ui.ui_contains_pointer() {
                        interacted = true;
                    }
                    // Header: title block.
                    match &popup.info {
                        Some(info) => {
                            ui.label(
                                egui::RichText::new(truncate(&info.display_title(), 78))
                                    .strong()
                                    .color(TEXT),
                            )
                            .on_hover_text(info.display_title());
                            let mut meta: Vec<String> = Vec::new();
                            if let Some(u) = info.display_uploader() {
                                meta.push(u);
                            }
                            if let Some(d) = info.duration {
                                let s = yttraydl::ytdlp::format_duration(d);
                                if !s.is_empty() {
                                    meta.push(s);
                                }
                            }
                            meta.push(popup.host.clone());
                            if info.is_playlist() {
                                meta.push(if playlists_allowed {
                                    "playlist".into()
                                } else {
                                    "playlist — first item only".into()
                                });
                            }
                            ui.label(egui::RichText::new(meta.join(" · ")).color(TEXT_DIM).small());
                        }
                        None => {
                            ui.label(egui::RichText::new(popup.host.clone()).strong().color(TEXT));
                            ui.label(
                                egui::RichText::new(if popup.info_loading {
                                    "Fetching video info…"
                                } else if bin_missing {
                                    "yt-dlp is not installed — open ytTrayDL to install it."
                                } else {
                                    "Couldn't fetch video info. You can still try downloading."
                                })
                                .color(TEXT_DIM)
                                .small(),
                            );
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(truncate(&popup.url, 46))
                                .color(TEXT_FAINT)
                                .small()
                                .monospace(),
                        )
                        .on_hover_text(popup.url.clone());
                        if ui.small_button("⧉").on_hover_text("Copy the link").clicked() {
                            ui.ctx().copy_text(popup.url.clone());
                            interacted = true;
                        }
                    });

                    if popup.already_downloaded {
                        ui.label(
                            egui::RichText::new("You've downloaded this before. Download will skip it — use “Download anyway” to force a fresh copy.")
                                .color(ACCENT)
                                .small(),
                        );
                    }

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Quality:");
                        let before = popup.preset;
                        egui::ComboBox::from_id_salt(("popup-preset", &popup.url))
                            .selected_text(popup.preset.label())
                            .show_ui(ui, |ui| {
                                for p in Preset::ALL {
                                    ui.selectable_value(&mut popup.preset, p, p.label());
                                }
                            });
                        if popup.preset != before {
                            interacted = true;
                        }
                    });
                    if popup.preset == Preset::Custom {
                        ui.label(egui::RichText::new(format!("Custom format: {custom_format}")).color(TEXT_FAINT).small());
                    }
                    let mut always = popup.always_auto_domain;
                    if ui
                        .checkbox(&mut always, format!("Always auto-download from {}", registrable(&popup.host)))
                        .on_hover_text("Adds this domain to Settings → Domains → Auto-download")
                        .changed()
                    {
                        popup.always_auto_domain = always;
                        interacted = true;
                    }

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let download_label = if popup.already_downloaded {
                            "⬇ Download anyway"
                        } else {
                            "⬇ Download"
                        };
                        if ui
                            .add(egui::Button::new(egui::RichText::new(download_label).color(TEXT)).fill(ACCENT_DARK))
                            .on_hover_text("Download with the selected quality into your download folder")
                            .clicked()
                        {
                            action = Action::Download;
                            interacted = true;
                        }
                        if ui.button("Not now").on_hover_text("Dismiss. Copy the link again later to be asked again after the dedupe window.").clicked() {
                            action = Action::Dismiss;
                        }
                        ui.menu_button("Ignore ▾", |ui| {
                            if ui.button("Never ask for this link").on_hover_text("Adds this exact URL to your ignore list").clicked() {
                                action = Action::IgnoreUrl;
                                ui.close();
                            }
                            if ui.button(format!("Ignore {} entirely", registrable(&popup.host))).on_hover_text("Never react to links from this domain").clicked() {
                                action = Action::IgnoreDomain;
                                ui.close();
                            }
                        });
                    });

                    let timeout = timeout_secs;
                    if timeout > 0 {
                        let left = timeout.saturating_sub(popup.last_interaction.elapsed().as_secs());
                        ui.label(
                            egui::RichText::new(format!("Closes automatically in {left}s — nothing downloads unless you click Download."))
                                .color(TEXT_FAINT)
                                .small(),
                        );
                    }
                });
            },
        );

        if interacted {
            // Hovering/interacting counts as "I'm deciding": the
            // auto-dismiss timer restarts, the download never does.
            popup.last_interaction = Instant::now();
        }
        // Auto-dismiss (never auto-downloads).
        let timeout = self.settings.popup_timeout_secs;
        if action == Action::None
            && timeout > 0
            && popup.last_interaction.elapsed().as_secs() >= timeout
        {
            action = Action::Dismiss;
        }

        match action {
            Action::None => {
                self.active_popup = Some(popup);
            }
            Action::Dismiss => {}
            Action::Download => {
                if popup.always_auto_domain {
                    let d = registrable(&popup.host).to_string();
                    if !self.settings.auto_domains.contains(&d) {
                        self.settings.auto_domains.push(d);
                        self.settings.auto_domains.sort();
                        self.settings_dirty = true;
                    }
                }
                let title = popup
                    .info
                    .as_ref()
                    .map(|i| i.display_title())
                    .unwrap_or_else(|| popup.url.clone());
                self.start_download(&popup.url, &title, popup.preset, popup.already_downloaded);
            }
            Action::IgnoreUrl => {
                self.settings.ignored_urls.push(popup.url.clone());
                self.settings_dirty = true;
                self.toast("Link added to your ignore list.", ToastKind::Info);
            }
            Action::IgnoreDomain => {
                let d = registrable(&popup.host).to_string();
                if !self.settings.ignored_domains.contains(&d) {
                    self.settings.ignored_domains.push(d);
                    self.settings.ignored_domains.sort();
                    self.settings_dirty = true;
                }
                self.toast(
                    format!("{} will be ignored from now on.", registrable(&popup.host)),
                    ToastKind::Info,
                );
            }
        }
    }
}

/// A short display form of a host: strips a leading "www."/"m." so
/// rules read naturally ("youtube.com" not "www.youtube.com").
pub fn registrable(host: &str) -> &str {
    host.strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .unwrap_or(host)
}

pub fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn local_time_hhmm(unix: i64) -> String {
    // No datetime crate: derive HH:MM from the local offset is not
    // available in std, so show relative-friendly UTC time of day.
    // (History ordering matters more than wall-clock precision;
    // full timestamps are in history.json.)
    let secs = unix.rem_euclid(86_400);
    format!("{:02}:{:02} UTC", secs / 3600, (secs % 3600) / 60)
}

// ---------------- Settings ----------------

#[derive(Default)]
pub struct SettingsOutcome {
    pub changed: bool,
    pub relocalize: bool,
    pub install_ytdlp: bool,
    pub set_autostart: Option<bool>,
    pub confirm_hide_icon: bool,
    pub clear_history: bool,
    pub reset_all: bool,
    pub open_config_dir: bool,
    pub pick_ytdlp: bool,
    pub pick_ffmpeg: bool,
}

impl App {
    pub fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut settings = std::mem::take(&mut self.settings);
        let mut rule_ignore = std::mem::take(&mut self.rule_input_ignore);
        let mut rule_auto = std::mem::take(&mut self.rule_input_auto);
        let mut rule_allow = std::mem::take(&mut self.rule_input_allow);
        let mut rule_url = std::mem::take(&mut self.rule_input_url);
        let mut tab = self.settings_tab;
        let mut reset_armed = self.reset_armed;
        let mut open = true;
        let mut outcome = SettingsOutcome::default();
        let bin_info: Option<(String, String, String)> = self.bin.as_ref().map(|b| {
            (
                b.path.display().to_string(),
                source_label(&b.source).to_string(),
                self.ytdlp_version.clone().unwrap_or_else(|| "…".into()),
            )
        });
        let ffmpeg_ok = self.ffmpeg_ok;
        let fetching = self.ytdlp_fetch_rx.is_some();
        let autostart_now = platform::Platform::autostart_enabled();

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-settings"),
            egui::ViewportBuilder::default()
                .with_title("ytTrayDL — Settings")
                .with_inner_size([640.0, 680.0])
                .with_min_inner_size([520.0, 480.0]),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        for (i, name) in ["General", "Downloads", "Domains & rules", "yt-dlp"]
                            .iter()
                            .enumerate()
                        {
                            if ui
                                .selectable_label(tab == i, *name)
                                .on_hover_text("Switch settings section")
                                .clicked()
                            {
                                tab = i;
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new("Settings save automatically")
                                    .color(TEXT_FAINT)
                                    .small(),
                            );
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| match tab {
                        0 => settings_general(ui, &mut settings, &mut outcome, autostart_now),
                        1 => settings_downloads(ui, &mut settings, &mut outcome),
                        2 => settings_domains(
                            ui,
                            &mut settings,
                            &mut outcome,
                            &mut rule_ignore,
                            &mut rule_auto,
                            &mut rule_allow,
                            &mut rule_url,
                        ),
                        _ => settings_ytdlp(
                            ui,
                            &mut settings,
                            &mut outcome,
                            &bin_info,
                            ffmpeg_ok,
                            fetching,
                            &mut reset_armed,
                        ),
                    });
                });
            },
        );

        self.settings = settings;
        self.rule_input_ignore = rule_ignore;
        self.rule_input_auto = rule_auto;
        self.rule_input_allow = rule_allow;
        self.rule_input_url = rule_url;
        self.settings_tab = tab;
        self.reset_armed = reset_armed;
        self.show_settings = open;

        if outcome.changed {
            self.settings_dirty = true;
        }
        if outcome.relocalize {
            self.relocalize_binaries();
        }
        if outcome.install_ytdlp {
            self.start_ytdlp_download();
        }
        if let Some(target) = outcome.set_autostart {
            match platform::Platform::set_autostart(target) {
                Ok(()) => {
                    self.settings.start_with_windows = target;
                    self.settings_dirty = true;
                }
                Err(e) => {
                    self.toast(format!("Autostart change failed: {e}"), ToastKind::Error);
                }
            }
        }
        if outcome.confirm_hide_icon {
            self.confirm_hide_icon = true;
        }
        if outcome.clear_history {
            self.history.entries.clear();
            let _ = self.history.save();
            self.toast("History cleared.", ToastKind::Info);
        }
        if outcome.reset_all {
            self.settings = Settings::default();
            self.settings_dirty = true;
            self.relocalize_binaries();
            self.toast("All settings reset to defaults.", ToastKind::Info);
        }
        if outcome.open_config_dir {
            let dir = yttraydl::settings::config_dir();
            let _ = std::fs::create_dir_all(&dir);
            if let Err(e) = platform::open_path(&dir) {
                self.toast(
                    format!("Could not open config folder: {e}"),
                    ToastKind::Error,
                );
            }
        }
        if outcome.pick_ytdlp {
            if let Some(f) = pick_file("yt-dlp", "exe") {
                self.settings.ytdlp_path = f;
                self.settings_dirty = true;
                self.relocalize_binaries();
            }
        }
        if outcome.pick_ffmpeg {
            if let Some(f) = pick_file("ffmpeg", "exe") {
                self.settings.ffmpeg_path = f;
                self.settings_dirty = true;
                self.relocalize_binaries();
            }
        }
    }
}

fn settings_general(
    ui: &mut egui::Ui,
    s: &mut Settings,
    out: &mut SettingsOutcome,
    autostart_now: bool,
) {
    ui.label(egui::RichText::new("When a supported link is copied").strong());
    if ui.checkbox(&mut s.monitoring_enabled, "Monitor the clipboard").on_hover_text("When off, ytTrayDL never reacts to copied links. You can still download from the panel.").changed() {
        out.changed = true;
    }
    ui.horizontal(|ui| {
        ui.label("Default action:");
        if ui
            .radio_value(
                &mut s.default_action,
                DefaultAction::Prompt,
                "Ask me (popup)",
            )
            .on_hover_text("Show the small download popup and let you pick quality each time")
            .changed()
        {
            out.changed = true;
        }
        if ui
            .radio_value(
                &mut s.default_action,
                DefaultAction::AutoDownload,
                "Auto-download",
            )
            .on_hover_text(
                "Download immediately with your default quality — no popup, no interruption",
            )
            .changed()
        {
            out.changed = true;
        }
    });
    ui.label(egui::RichText::new("Per-domain rules (Domains & rules tab) can override this: a domain can always auto-download, or always be ignored.").color(TEXT_DIM).small());

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Which links count as supported").strong());
    let mut scope = s.detection_scope;
    egui::ComboBox::from_id_salt("scope")
        .selected_text(match scope {
            DetectionScope::KnownSitesOnly => "Known video/audio sites only",
            DetectionScope::AllUrls => "Any web link",
            DetectionScope::AllowListOnly => "Only my allow list",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut scope, DetectionScope::KnownSitesOnly, "Known video/audio sites only")
                .on_hover_text("YouTube, Vimeo, TikTok, Twitch, X, Instagram and ~60 more built-in domains, plus your allow list");
            ui.selectable_value(&mut scope, DetectionScope::AllUrls, "Any web link")
                .on_hover_text("yt-dlp supports 1,000+ sites including long-tail ones — but ordinary links will also pop up");
            ui.selectable_value(&mut scope, DetectionScope::AllowListOnly, "Only my allow list")
                .on_hover_text("React only to domains on the Domains & rules tab allow list");
        });
    if scope != s.detection_scope {
        s.detection_scope = scope;
        out.changed = true;
    }

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Tray & startup").strong());
    let mut visible = s.tray_icon_visible;
    if ui.checkbox(&mut visible, "Show tray icon").on_hover_text("Uncheck to run invisibly. You can always get the icon back by launching ytTrayDL again or pressing Ctrl+Alt+G.").changed() {
        if !visible {
            out.confirm_hide_icon = true; // hiding needs the recovery explanation first
        } else {
            s.tray_icon_visible = true;
            out.changed = true;
        }
    }
    let mut autostart = autostart_now;
    if ui
        .checkbox(&mut autostart, "Start with Windows")
        .on_hover_text(
            "Adds ytTrayDL to your per-user startup (HKCU Run key). No admin rights needed.",
        )
        .changed()
    {
        out.set_autostart = Some(autostart);
    }
    ui.horizontal(|ui| {
        ui.label("Popup position:");
        let mut pos = s.popup_position;
        egui::ComboBox::from_id_salt("popup-pos")
            .selected_text(match pos {
                PopupPosition::BottomRight => "Bottom-right",
                PopupPosition::TopRight => "Top-right",
                PopupPosition::NearCursor => "Near the mouse cursor",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut pos, PopupPosition::BottomRight, "Bottom-right");
                ui.selectable_value(&mut pos, PopupPosition::TopRight, "Top-right");
                ui.selectable_value(&mut pos, PopupPosition::NearCursor, "Near the mouse cursor");
            });
        if pos != s.popup_position {
            s.popup_position = pos;
            out.changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Popup auto-close:");
        let mut t = s.popup_timeout_secs;
        if ui.add(egui::Slider::new(&mut t, 0..=300).suffix("s")).on_hover_text("The popup closes without downloading after this long. 0 = stay until you decide. Hovering restarts the timer.").changed() {
            s.popup_timeout_secs = t;
            out.changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Ignore re-copies for:");
        let mut d = s.dedupe_window_secs;
        if ui.add(egui::Slider::new(&mut d, 0..=3600).suffix("s")).on_hover_text("Copying the same link again within this window won't re-trigger ytTrayDL. 0 = always react.").changed() {
            s.dedupe_window_secs = d;
            out.changed = true;
        }
    });

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Notifications (small toasts, bottom-right)").strong());
    for (flag, label, tip) in [
        (
            &mut s.notify_on_complete,
            "When a download finishes",
            "A brief toast naming the finished file",
        ),
        (
            &mut s.notify_on_error,
            "When a download fails",
            "Shows the error so silent downloads never fail silently",
        ),
        (
            &mut s.notify_on_auto_start,
            "When an auto-download starts",
            "Confirms ytTrayDL reacted; off by default to stay unobtrusive",
        ),
    ] {
        if ui.checkbox(flag, label).on_hover_text(tip).changed() {
            out.changed = true;
        }
    }
}

fn settings_downloads(ui: &mut egui::Ui, s: &mut Settings, out: &mut SettingsOutcome) {
    ui.label(egui::RichText::new("Where downloads go").strong());
    ui.horizontal(|ui| {
        let mut folder = s.download_folder.display().to_string();
        if ui
            .add(egui::TextEdit::singleline(&mut folder).desired_width(360.0))
            .on_hover_text("Downloads are saved here")
            .changed()
        {
            s.download_folder = std::path::PathBuf::from(folder);
            out.changed = true;
        }
        if ui
            .button("Browse…")
            .on_hover_text("Pick the download folder")
            .clicked()
        {
            if let Some(f) = pick_folder(&s.download_folder) {
                s.download_folder = f;
                out.changed = true;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("File name template:");
        if ui.add(egui::TextEdit::singleline(&mut s.filename_template).desired_width(300.0)).on_hover_text("yt-dlp output template, e.g. %(title)s [%(id)s].%(ext)s — see yt-dlp's output template documentation").changed() {
            out.changed = true;
        }
    });

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Default quality").strong());
    ui.horizontal(|ui| {
        ui.label("Preset:");
        let mut preset = s.default_preset;
        egui::ComboBox::from_id_salt("default-preset")
            .selected_text(preset.label())
            .show_ui(ui, |ui| {
                for p in Preset::ALL {
                    ui.selectable_value(&mut preset, p, p.label());
                }
            });
        if preset != s.default_preset {
            s.default_preset = preset;
            out.changed = true;
        }
    });
    if s.default_preset == Preset::Custom {
        ui.horizontal(|ui| {
            ui.label("Custom -f:");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut s.custom_format)
                        .desired_width(300.0)
                        .font(egui::TextStyle::Monospace),
                )
                .on_hover_text("A yt-dlp format selector, passed as the -f argument")
                .changed()
            {
                out.changed = true;
            }
        });
    }
    ui.horizontal(|ui| {
        ui.label("Container:");
        let mut c = s.container;
        egui::ComboBox::from_id_salt("container")
            .selected_text(c.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut c, Container::Mp4, "MP4");
                ui.selectable_value(&mut c, Container::Mkv, "MKV");
                ui.selectable_value(&mut c, Container::Original, "Original (no remux)");
            });
        if c != s.container {
            s.container = c;
            out.changed = true;
        }
    });
    if ui
        .checkbox(&mut s.embed_thumbnail, "Embed thumbnail")
        .on_hover_text(
            "Embed the cover art / thumbnail into the file when the container supports it",
        )
        .changed()
    {
        out.changed = true;
    }
    if ui
        .checkbox(&mut s.embed_metadata, "Embed metadata (title, uploader, …)")
        .on_hover_text("Writes title/artist metadata into the file")
        .changed()
    {
        out.changed = true;
    }
    if ui
        .checkbox(
            &mut s.write_subtitles,
            "Download subtitles and embed them in video",
        )
        .on_hover_text("Includes auto-generated subtitles when no manual ones exist")
        .changed()
    {
        out.changed = true;
    }
    if s.write_subtitles {
        ui.horizontal(|ui| {
            ui.label("Subtitle languages:");
            if ui
                .add(egui::TextEdit::singleline(&mut s.subtitle_languages).desired_width(160.0))
                .on_hover_text("yt-dlp --sub-langs pattern, e.g. en.* for all English variants")
                .changed()
            {
                out.changed = true;
            }
        });
    }
    if ui.checkbox(&mut s.allow_playlists, "Download whole playlists").on_hover_text("Off (default): a playlist link downloads just the linked/first item. On: the entire playlist, item by item.").changed() {
        out.changed = true;
    }
    if ui.checkbox(&mut s.skip_already_downloaded, "Skip videos already downloaded before").on_hover_text("Uses a yt-dlp download archive stored with your ytTrayDL settings, so re-copying an old link doesn't fetch it twice").changed() {
        out.changed = true;
    }
    ui.horizontal(|ui| {
        ui.label("Simultaneous downloads:");
        let mut n = s.max_concurrent_downloads;
        if ui
            .add(egui::Slider::new(&mut n, 1..=4))
            .on_hover_text(
                "More parallelism isn't always faster — sites may throttle. 2 is a good default.",
            )
            .changed()
        {
            s.max_concurrent_downloads = n;
            out.changed = true;
        }
    });

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Advanced").strong());
    ui.horizontal(|ui| {
        ui.label("Extra yt-dlp arguments:");
        if ui.add(egui::TextEdit::singleline(&mut s.extra_args).desired_width(320.0).font(egui::TextStyle::Monospace)).on_hover_text("Passed to yt-dlp after ytTrayDL's own arguments, so yours win on conflicts. Example: --limit-rate 10M --retries 5").changed() {
            out.changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Cookies from browser:");
        if ui.add(egui::TextEdit::singleline(&mut s.cookies_from_browser).desired_width(140.0).hint_text("chrome / firefox / edge…")).on_hover_text("For members-only or age-restricted videos you're signed in to. yt-dlp reads the browser's cookies at download time; ytTrayDL never sees or stores them. Leave empty for none.").changed() {
            out.changed = true;
        }
    });
}

fn domain_list_editor(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    explanation: &str,
    list: &mut Vec<String>,
    input: &mut String,
    out: &mut SettingsOutcome,
) {
    ui.label(egui::RichText::new(title).strong());
    ui.label(egui::RichText::new(explanation).color(TEXT_DIM).small());
    let mut remove: Option<String> = None;
    for d in list.iter() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(d.clone()).monospace());
            if ui
                .small_button("✕")
                .on_hover_text("Remove this rule")
                .clicked()
            {
                remove = Some(d.clone());
            }
        });
    }
    if let Some(d) = remove {
        list.retain(|x| x != &d);
        out.changed = true;
    }
    ui.horizontal(|ui| {
        let resp = ui
            .add(egui::TextEdit::singleline(input).hint_text("example.com").desired_width(220.0))
            .on_hover_text("Type a domain — subdomains are included automatically (youtube.com also covers music.youtube.com)");
        let add = ui.button("Add").on_hover_text("Add this domain rule").clicked();
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (add || enter) && !input.trim().is_empty() {
            let d = yttraydl::settings::normalize_domain(input);
            if !d.is_empty() && !list.contains(&d) {
                list.push(d);
                list.sort();
                out.changed = true;
            }
            input.clear();
        }
        let _ = id;
    });
    ui.add_space(6.0);
}

fn settings_domains(
    ui: &mut egui::Ui,
    s: &mut Settings,
    out: &mut SettingsOutcome,
    rule_ignore: &mut String,
    rule_auto: &mut String,
    rule_allow: &mut String,
    rule_url: &mut String,
) {
    ui.label(egui::RichText::new("Rules are checked in this order: ignored links → ignored domains → already-downloaded → scope → auto-download domains → your default action.").color(TEXT_DIM).small());
    ui.add_space(6.0);
    domain_list_editor(
        ui,
        "ignore",
        "🚫 Ignored domains — never react",
        "Links from these domains never pop up and never download.",
        &mut s.ignored_domains,
        rule_ignore,
        out,
    );
    domain_list_editor(
        ui,
        "auto",
        "⚡ Auto-download domains — skip the popup",
        "Links from these domains download immediately with your defaults, even in Ask mode.",
        &mut s.auto_domains,
        rule_auto,
        out,
    );
    domain_list_editor(ui, "allow", "✓ Allowed domains — count as supported", "Extra domains treated as supported sites (also the only sites that count under 'Only my allow list' scope).", &mut s.allowed_domains, rule_allow, out);

    ui.label(egui::RichText::new("🚫 Ignored links").strong());
    ui.label(
        egui::RichText::new("Specific URLs you chose 'Never ask for this link' on.")
            .color(TEXT_DIM)
            .small(),
    );
    let mut remove_url: Option<String> = None;
    for u in s.ignored_urls.iter() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(truncate(u, 52)).monospace().small())
                .on_hover_text(u.clone());
            if ui
                .small_button("✕")
                .on_hover_text("Stop ignoring this link")
                .clicked()
            {
                remove_url = Some(u.clone());
            }
        });
    }
    if let Some(u) = remove_url {
        s.ignored_urls.retain(|x| x != &u);
        out.changed = true;
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(rule_url)
                .hint_text("https://…")
                .desired_width(300.0),
        );
        if ui
            .button("Add")
            .on_hover_text("Ignore this exact link")
            .clicked()
            && !rule_url.trim().is_empty()
        {
            let u = rule_url.trim().to_string();
            if !s.ignored_urls.contains(&u) {
                s.ignored_urls.push(u);
                out.changed = true;
            }
            rule_url.clear();
        }
    });
}

fn settings_ytdlp(
    ui: &mut egui::Ui,
    s: &mut Settings,
    out: &mut SettingsOutcome,
    bin_info: &Option<(String, String, String)>,
    ffmpeg_ok: bool,
    fetching: bool,
    reset_armed: &mut bool,
) {
    ui.label(egui::RichText::new("yt-dlp").strong());
    match bin_info {
        Some((path, source, version)) => {
            ui.label(format!("Version {version} — {source}"));
            ui.label(
                egui::RichText::new(path.clone())
                    .monospace()
                    .small()
                    .color(TEXT_DIM),
            );
        }
        None => {
            ui.label(
                egui::RichText::new("Not found. Downloads can't run until yt-dlp is installed.")
                    .color(ACCENT),
            );
        }
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(!fetching, egui::Button::new(if fetching { "Downloading…" } else if bin_info.is_some() { "Update yt-dlp (managed copy)" } else { "⬇ Install yt-dlp" }))
            .on_hover_text("Downloads the latest official yt-dlp.exe from GitHub into %APPDATA%\\ytTrayDL\\bin. If you use your own copy elsewhere, update that one yourself.")
            .clicked()
        {
            out.install_ytdlp = true;
        }
        if ui.button("Choose yt-dlp.exe…").on_hover_text("Point ytTrayDL at a yt-dlp.exe you manage yourself").clicked() {
            out.pick_ytdlp = true;
        }
        if ui.button("Releases page").on_hover_text("Open yt-dlp releases on GitHub").clicked() {
            let _ = platform::open_url("https://github.com/yt-dlp/yt-dlp/releases");
        }
    });
    if !s.ytdlp_path.as_os_str().is_empty() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Your path: {}", s.ytdlp_path.display()))
                    .small()
                    .color(TEXT_DIM),
            );
            if ui
                .small_button("Clear")
                .on_hover_text("Go back to automatic locating")
                .clicked()
            {
                s.ytdlp_path.clear();
                out.changed = true;
                out.relocalize = true;
            }
        });
    }

    ui.add_space(8.0);
    ui.label(egui::RichText::new("ffmpeg").strong());
    ui.label(if ffmpeg_ok {
        "Found — merging and MP3 conversion are available."
    } else {
        "Not found — downloads that need merging (most best-quality) and MP3 conversion will fail. Install ffmpeg and put it on PATH, next to ytTrayDL, or choose it here."
    });
    ui.horizontal(|ui| {
        if ui
            .button("Choose ffmpeg.exe…")
            .on_hover_text("Point ytTrayDL at ffmpeg.exe")
            .clicked()
        {
            out.pick_ffmpeg = true;
        }
        if !s.ffmpeg_path.as_os_str().is_empty()
            && ui.small_button("Clear custom ffmpeg path").clicked()
        {
            s.ffmpeg_path.clear();
            out.changed = true;
            out.relocalize = true;
        }
    });

    ui.add_space(8.0);
    ui.label(egui::RichText::new("ytTrayDL data").strong());
    ui.label(egui::RichText::new("Settings, history and the download archive live in %APPDATA%\\ytTrayDL. Nothing is sent anywhere — ytTrayDL has no telemetry.").color(TEXT_DIM).small());
    ui.horizontal(|ui| {
        if ui
            .button("Open config folder")
            .on_hover_text("Open %APPDATA%\\ytTrayDL in Explorer")
            .clicked()
        {
            out.open_config_dir = true;
        }
        if ui
            .button("Clear download history")
            .on_hover_text(
                "Removes the history list only — downloaded files and the yt-dlp archive stay",
            )
            .clicked()
        {
            out.clear_history = true;
        }
    });

    ui.add_space(10.0);
    ui.label(egui::RichText::new("Danger zone").color(ACCENT).strong());
    if *reset_armed {
        ui.label("Reset every setting to defaults? Domain rules, folder and quality choices will be lost. Downloaded files are not touched.");
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new("Yes, reset all settings").fill(ACCENT_DARK))
                .clicked()
            {
                out.reset_all = true;
                *reset_armed = false;
            }
            if ui.button("Cancel").clicked() {
                *reset_armed = false;
            }
        });
    } else if ui
        .button("Reset all settings…")
        .on_hover_text("Restore factory defaults (asks for confirmation)")
        .clicked()
    {
        *reset_armed = true;
    }
}

// ---------------- Downloads window ----------------

/// (id, title, url, percent, speed, eta, state, error)
type ActiveRow = (
    u64,
    String,
    String,
    f32,
    Option<String>,
    Option<String>,
    JobState,
    Option<String>,
);
/// (title, url, state, file, error, preset)
type FinishedRow = (
    String,
    String,
    JobState,
    Option<std::path::PathBuf>,
    Option<String>,
    Preset,
);

impl App {
    pub fn downloads_ui(&mut self, ctx: &egui::Context) {
        if !self.show_downloads {
            return;
        }
        let mut open = true;
        // Snapshot everything the closure needs.
        let active: Vec<ActiveRow> = self
            .downloads
            .running_jobs()
            .chain(self.downloads.queued_jobs())
            .map(|j| {
                (
                    j.id,
                    if j.title.is_empty() {
                        j.url.clone()
                    } else {
                        j.title.clone()
                    },
                    j.url.clone(),
                    j.percent,
                    j.speed_text.clone(),
                    j.eta_text.clone(),
                    j.state,
                    j.error.clone(),
                )
            })
            .collect();
        let finished: Vec<FinishedRow> = self
            .downloads
            .finished_jobs()
            .iter()
            .map(|j| {
                (
                    if j.title.is_empty() {
                        j.url.clone()
                    } else {
                        j.title.clone()
                    },
                    j.url.clone(),
                    j.state,
                    j.final_file.clone(),
                    j.error.clone(),
                    j.preset,
                )
            })
            .collect();
        let history_entries: Vec<HistoryEntry> = self.history.entries.clone();
        let bin_missing = self.bin.is_none();
        let fetching = self.ytdlp_fetch_rx.is_some();

        let mut cancel: Option<u64> = None;
        let mut retry: Option<(String, Preset)> = None;
        let mut open_target: Option<std::path::PathBuf> = None;
        let mut install = false;
        let mut clear_history = false;

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-downloads"),
            egui::ViewportBuilder::default()
                .with_title("ytTrayDL — Downloads")
                .with_inner_size([580.0, 560.0])
                .with_min_inner_size([460.0, 360.0]),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    if bin_missing {
                        ui.label(egui::RichText::new("yt-dlp isn't installed — downloads will wait in the queue.").color(ACCENT));
                        if ui.add_enabled(!fetching, egui::Button::new("⬇ Install yt-dlp")).clicked() {
                            install = true;
                        }
                        ui.separator();
                    }
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.label(egui::RichText::new("Active").strong());
                        if active.is_empty() {
                            ui.label(egui::RichText::new("Nothing downloading. Copy a supported link, or paste one in the panel.").color(TEXT_DIM));
                        }
                        for (id, title, url, pct, speed, eta, state, _err) in &active {
                            ui.horizontal(|ui| {
                                ui.label(truncate(title, 40)).on_hover_text(format!("{title}\n{url}"));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕ Cancel").on_hover_text("Cancel (partial file kept for resume)").clicked() {
                                        cancel = Some(*id);
                                    }
                                });
                            });
                            match state {
                                JobState::Queued => {
                                    ui.label(egui::RichText::new("Queued").color(TEXT_FAINT).small());
                                }
                                JobState::PostProcessing => {
                                    ui.label(egui::RichText::new("Processing — merging / converting…").color(TEXT_DIM).small());
                                    ui.add(egui::ProgressBar::new(1.0).desired_width(200.0));
                                }
                                _ => {
                                    ui.add(egui::ProgressBar::new(pct / 100.0).desired_width(260.0));
                                    let mut bits = vec![format!("{pct:.1}%")];
                                    if let Some(sp) = speed {
                                        bits.push(sp.to_string());
                                    }
                                    if let Some(e) = eta {
                                        bits.push(format!("ETA {e}"));
                                    }
                                    ui.label(egui::RichText::new(bits.join(" · ")).small().color(TEXT_DIM));
                                }
                            }
                            ui.add_space(4.0);
                        }

                        if !finished.is_empty() {
                            ui.separator();
                            ui.label(egui::RichText::new("Finished this session").strong());
                            for (title, url, state, file, err, preset) in &finished {
                                job_row(ui, &(title.clone(), url.clone(), *state, file.clone(), err.clone(), *preset), &mut open_target, &mut retry);
                            }
                        }

                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("History").strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Clear history").on_hover_text("Clear the stored history (files stay)").clicked() {
                                    clear_history = true;
                                }
                            });
                        });
                        if history_entries.is_empty() {
                            ui.label(egui::RichText::new("No downloads yet.").color(TEXT_DIM));
                        }
                        for e in history_entries.iter().take(40) {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("●").color(status_color(e.status)));
                                ui.label(truncate(&e.title, 44)).on_hover_text(format!("{}\n{}\n{}", e.title, e.url, e.error.clone().unwrap_or_default()));
                                ui.label(egui::RichText::new(format!("{} · {}", local_time_hhmm(e.finished_at_unix), e.preset_label)).color(TEXT_FAINT).small());
                                if let Some(f) = &e.file {
                                    if ui.small_button("📂").on_hover_text("Show in Explorer").clicked() {
                                        open_target = f.parent().map(|p| p.to_path_buf());
                                    }
                                }
                                if e.status == HistoryStatus::Failed
                                    && ui.small_button("↻ Retry").on_hover_text("Download this link again with your current defaults").clicked()
                                {
                                    retry = Some((e.url.clone(), Preset::Best));
                                }
                            });
                        }
                    });
                });
            },
        );

        self.show_downloads = open;
        if install {
            self.start_ytdlp_download();
        }
        if let Some(id) = cancel {
            self.downloads.cancel(id);
        }
        if let Some((url, _preset)) = retry {
            let preset = self.settings.default_preset;
            self.start_download(&url, "", preset, false);
        }
        if let Some(p) = open_target {
            let _ = platform::open_path(&p);
        }
        if clear_history {
            self.history.entries.clear();
            let _ = self.history.save();
        }
    }

    // ---------------- Toasts ----------------

    pub fn toasts_ui(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|t| t.expires > now);
        if self.toasts.is_empty() {
            return;
        }
        let toasts = self.toasts.clone();
        let lift = if self.active_popup.is_some() {
            284.0
        } else {
            0.0
        };
        let size = egui::vec2(360.0, (toasts.len() as f32) * 46.0 + 12.0);
        let pos = bottom_right(ctx, size, lift);
        let mut dismiss: Option<usize> = None;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-toasts"),
            egui::ViewportBuilder::default()
                .with_inner_size(size)
                .with_position(pos)
                .with_decorations(false)
                .with_always_on_top()
                .with_taskbar(false),
            |ctx, _class| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for (i, t) in toasts.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let color = match t.kind {
                                ToastKind::Success => OK_GREEN,
                                ToastKind::Error => ACCENT,
                                ToastKind::Info => TEXT_DIM,
                            };
                            ui.label(egui::RichText::new("●").color(color));
                            ui.label(truncate(&t.text, 56))
                                .on_hover_text(t.text.clone());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                                        dismiss = Some(i);
                                    }
                                },
                            );
                        });
                    }
                });
            },
        );
        if let Some(i) = dismiss {
            if i < self.toasts.len() {
                self.toasts.remove(i);
            }
        }
    }

    // ---------------- About ----------------

    pub fn about_ui(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut open = true;
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-about"),
            egui::ViewportBuilder::default()
                .with_title("About ytTrayDL")
                .with_inner_size([440.0, 330.0])
                .with_min_inner_size([440.0, 330.0])
                .with_max_inner_size([440.0, 330.0]),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.heading("ytTrayDL");
                    ui.label(format!("Version {} · MIT License · by Sean Shaffer", env!("CARGO_PKG_VERSION")));
                    ui.add_space(6.0);
                    ui.label("ytTrayDL watches your clipboard for links yt-dlp can download. Copy a link and it either asks (with a small popup) or quietly downloads with your defaults — your choice, per domain if you like.");
                    ui.add_space(4.0);
                    ui.label("All downloading is done by yt-dlp; ytTrayDL is a friendly shell around it. No telemetry, no accounts, settings stay on this PC.");
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Tray icon hidden? Launch ytTrayDL again, or press Ctrl+Alt+G, and it comes back with Settings open.").color(TEXT_DIM));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("yt-dlp on GitHub").on_hover_text("The download engine ytTrayDL drives").clicked() {
                            let _ = platform::open_url("https://github.com/yt-dlp/yt-dlp");
                        }
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
            },
        );
        self.show_about = open && !close;
    }

    // ---------------- Hide-icon confirmation ----------------

    pub fn hide_icon_confirm_ui(&mut self, ctx: &egui::Context) {
        if !self.confirm_hide_icon {
            return;
        }
        let mut open = true;
        let mut hide = false;
        let mut cancel = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-hide-confirm"),
            egui::ViewportBuilder::default()
                .with_title("Hide the tray icon?")
                .with_inner_size([460.0, 270.0])
                .with_min_inner_size([460.0, 270.0])
                .with_max_inner_size([460.0, 270.0])
                .with_always_on_top(),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label(egui::RichText::new("ytTrayDL will keep running and monitoring — there just won't be an icon.").strong());
                    ui.add_space(6.0);
                    ui.label("To get the icon and settings back, any of these works:");
                    ui.label("• Launch ytTrayDL again (Start menu / the .exe) — the running copy restores its icon and opens Settings.");
                    ui.label("• Press Ctrl+Alt+G anywhere.");
                    ui.label("• Run:  yttraydl.exe --show");
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Downloads and clipboard monitoring are not affected.").color(TEXT_DIM));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new("Hide the icon").fill(ACCENT_DARK)).on_hover_text("Hide the tray icon now").clicked() {
                            hide = true;
                        }
                        if ui.button("Keep the icon").on_hover_text("Cancel — keep the tray icon visible").clicked() {
                            cancel = true;
                        }
                    });
                });
            },
        );
        if hide {
            self.settings.tray_icon_visible = false;
            self.tray_visible = false;
            #[cfg(windows)]
            if let Some(t) = &self.tray {
                t._tray.set_visible(false).ok();
            }
            self.settings_dirty = true;
            self.confirm_hide_icon = false;
        } else if cancel || !open {
            self.confirm_hide_icon = false;
        }
    }
}

fn job_row(
    ui: &mut egui::Ui,
    row: &FinishedRow,
    open_target: &mut Option<std::path::PathBuf>,
    retry: &mut Option<(String, Preset)>,
) {
    let (title, url, state, file, err, preset) = row;
    let state = *state;
    ui.horizontal(|ui| {
        let (ch, color) = match state {
            JobState::Done => ("✓", OK_GREEN),
            JobState::Failed => ("✗", ACCENT),
            JobState::Cancelled => ("—", TEXT_FAINT),
            _ => ("…", TEXT_DIM),
        };
        ui.label(egui::RichText::new(ch).color(color));
        ui.label(truncate(title, 40)).on_hover_text(format!(
            "{title}\n{url}{}",
            err.as_ref().map(|e| format!("\n{e}")).unwrap_or_default()
        ));
        if let Some(f) = file {
            if ui
                .small_button("📂")
                .on_hover_text("Show in Explorer")
                .clicked()
            {
                *open_target = f.parent().map(|p| p.to_path_buf());
            }
        }
        if state == JobState::Failed
            && ui
                .small_button("↻ Retry")
                .on_hover_text("Try this download again")
                .clicked()
        {
            *retry = Some((url.to_string(), *preset));
        }
    });
}

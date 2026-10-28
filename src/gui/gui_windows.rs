//! The viewports: control panel (root), download popup, settings,
//! downloads, toasts, about, and the hide-icon confirmation.
//!
//! Viewport closures can't borrow `self` while `update` also holds
//! it, so each window extracts the state it needs (popup via
//! `Option::take`, settings/downloads via `mem::take`), renders,
//! and returns an outcome struct the caller applies afterwards.

use super::*;
use yttraydl::platform;
use yttraydl::settings::{
    Container, DefaultAction, DetectionScope, PopupPosition, Preset, QueueEmptyAction,
};
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
        // Hard clip at the panel's own content rect (0.5.1):
        // rows are width-budgeted to fit (see the status pill,
        // manual-link, Recent and footer rows), and this
        // guarantees any residual overflow is cut at the panel
        // edge instead of painting past the window — the
        // v0.5.0 field captures showed content running off the
        // right edge at launch and after resizes.
        ui.set_clip_rect(ui.max_rect());
        let full = ui.max_rect();

        // Gradient header band (0.6.0): full-bleed to the
        // window edges (the root panel frame is margin-free —
        // see `update`), near-black at the top shading into a
        // deep-red tint, carrying the logo tile, the title and
        // the status pill. Fixed height: tile row 42 + pill
        // row 26 + spacing, inside 10px vertical insets.
        const BAND_H: f32 = 96.0;
        let band_rect =
            egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, full.min.y + BAND_H));
        paint_vertical_gradient(ui.painter(), band_rect, 0.0, BAND_TOP, BAND_BOTTOM);
        ui.painter().line_segment(
            [band_rect.left_bottom(), band_rect.right_bottom()],
            egui::Stroke::new(1.0_f32, BAND_EDGE),
        );
        let mut band = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(band_rect.shrink2(egui::vec2(12.0, 10.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        band.horizontal(|ui| {
            // Logo tile: raised dark square, red-tinted
            // outline, top light edge, the download glyph
            // drawn (white arrow, red tray line).
            let (tile, _) = ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::hover());
            paint_soft_glow(ui.painter(), tile, 11.0, egui::Color32::BLACK);
            ui.painter()
                .rect_filled(tile, 11.0, egui::Color32::from_rgb(0x19, 0x19, 0x1B));
            ui.painter().rect_stroke(
                tile,
                egui::CornerRadius::same(11),
                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0x4A, 0x30, 0x32)),
                egui::epaint::StrokeKind::Inside,
            );
            paint_top_highlight(ui.painter(), tile, 11.0);
            paint_icon(
                ui.painter(),
                IconKind::Download,
                tile.center(),
                1.05,
                egui::Color32::WHITE,
                ACCENT,
            );
            ui.label(
                egui::RichText::new("ytTrayDL")
                    .strong()
                    .size(20.0)
                    .color(TEXT),
            );
            ui.label(
                egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).color(TEXT_FAINT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("✕ Close")
                    .on_hover_text("Close this panel. ytTrayDL keeps running in the tray.")
                    .clicked()
                {
                    self.begin_panel_hide();
                }
            });
        });
        band.add_space(8.0);
        // Status pill (0.5.0), sitting in the band (0.6.0):
        // dot + state + mode, in a rounded pill instead of a
        // line of loose labels.
        let (dot, word) = if self.settings.monitoring_enabled {
            (OK_GREEN, "Monitoring clipboard")
        } else {
            (ACCENT, "Monitoring paused")
        };
        band.horizontal(|ui| {
            status_pill(
                ui,
                dot,
                word,
                match self.settings.default_action {
                    DefaultAction::Prompt => "asks before downloading",
                    DefaultAction::AutoDownload => "auto-downloads with your defaults",
                },
                "Change this in Settings → General, or from the tray menu.",
            );
        });
        ui.allocate_rect(band_rect, egui::Sense::hover());

        // Content below the band, inset 12px on both sides —
        // the margins the old window_margin style used to give
        // every row (the 0.5.1 width budgets are computed from
        // `available_width` exactly as before).
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(full.min.x + 12.0, band_rect.max.y + 10.0),
            egui::pos2(full.max.x - 12.0, full.max.y),
        );
        let mut content = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        self.panel_body(&mut content, full);
    }

    fn panel_body(&mut self, ui: &mut egui::Ui, full: egui::Rect) {
        // Failure banner (0.4.0): the latest final failure, with
        // one-click retry — failures used to be easy to miss.
        if let Some(notice) = self.failure_notice.clone() {
            ui.add_space(8.0);
            egui::Frame::NONE
                .fill(DANGER_BG)
                .stroke(egui::Stroke::new(1.0_f32, ACCENT_DARK))
                .corner_radius(RADIUS)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "✖ Download failed: {}",
                            truncate(&notice.title, 44)
                        ))
                        .color(ACCENT)
                        .strong(),
                    )
                    .on_hover_text(notice.title.clone());
                    ui.label(
                        egui::RichText::new(truncate(&notice.error, 90))
                            .color(TEXT_DIM)
                            .small(),
                    )
                    .on_hover_text(notice.error.clone());
                    ui.horizontal(|ui| {
                        if ui
                            .add(primary_button("↻ Retry"))
                            .on_hover_text("Download it again with the same quality and folder")
                            .clicked()
                        {
                            self.retry_failure_notice();
                        }
                        if ui.button("Dismiss").clicked() {
                            self.failure_notice = None;
                        }
                    });
                });
        }

        // yt-dlp / ffmpeg status — the ENGINE well (0.6.0):
        // a recessed surface under a micro-label, per the
        // premium mockup. The status rows are dot + strong
        // item + faint right-aligned note; the install flows
        // (ffmpeg missing / yt-dlp missing) keep their 0.5.0
        // content inside the same well.
        ui.add_space(10.0);
        ui.label(section_heading("Engine"));
        ui.add_space(4.0);
        premium_well(ui, |ui| match &self.bin {
            Some(bin) => {
                let version = self.ytdlp_version.clone().unwrap_or_else(|| "…".into());
                ui.horizontal(|ui| {
                    status_dot(ui, OK_GREEN);
                    ui.label(
                        egui::RichText::new(format!("yt-dlp {version}"))
                            .color(TEXT)
                            .size(13.5),
                    )
                    .on_hover_text(bin.path.display().to_string());
                    let remaining = ui.available_width();
                    if remaining > 40.0 {
                        ui.allocate_ui_with_layout(
                            egui::vec2(remaining, 20.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(source_label(&bin.source))
                                            .color(TEXT_FAINT)
                                            .size(11.5),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                    }
                });
                if self.ffmpeg_ok {
                    ui.horizontal(|ui| {
                        status_dot(ui, OK_GREEN);
                        ui.label(egui::RichText::new("ffmpeg found").color(TEXT).size(13.5));
                        let remaining = ui.available_width();
                        if remaining > 40.0 {
                            ui.allocate_ui_with_layout(
                                egui::vec2(remaining, 20.0),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(
                                                "merging and MP3 conversion available",
                                            )
                                            .color(TEXT_FAINT)
                                            .size(11.5),
                                        )
                                        .truncate(),
                                    );
                                },
                            );
                        }
                    });
                } else {
                    ui.label(egui::RichText::new("ffmpeg not found — best-quality streams that need merging and MP3 conversion will fail.").color(ACCENT));
                    ui.horizontal(|ui| {
                            let busy = self.ffmpeg_fetch_rx.is_some();
                            if ui
                                .add_enabled(!busy, primary_button(if busy { "Downloading…" } else { "⬇ Install ffmpeg" }))
                                .on_hover_text("Download the GyanD essentials build (about 110 MB) from gyan.dev into %APPDATA%\\ytTrayDL\\bin — the same folder as the managed yt-dlp")
                                .clicked()
                            {
                                self.start_ffmpeg_download();
                            }
                            ui.label(
                                egui::RichText::new("or put ffmpeg.exe next to ytTrayDL or on PATH")
                                    .small()
                                    .color(TEXT_DIM),
                            );
                        });
                }
            }
            None => {
                ui.label(
                    egui::RichText::new(
                        "yt-dlp is not installed yet — ytTrayDL can't download without it.",
                    )
                    .color(ACCENT)
                    .strong(),
                );
                ui.label("One click installs the official standalone yt-dlp.exe from GitHub into ytTrayDL's own folder. Nothing else on your PC is changed.");
                ui.horizontal(|ui| {
                        let busy = self.ytdlp_fetch_rx.is_some();
                        if ui
                            .add_enabled(!busy, primary_button(if busy { "Downloading…" } else { "⬇ Install yt-dlp" }))
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
            }
        });

        // Quick controls — painter-drawn switches (0.6.0,
        // replacing the checkboxes, per the premium mockup).
        ui.horizontal(|ui| {
            let mut monitoring = self.settings.monitoring_enabled;
            if switch(ui, &mut monitoring)
                .on_hover_text("Watch the clipboard for supported links")
                .changed()
            {
                self.settings.monitoring_enabled = monitoring;
                self.settings_dirty = true;
            }
            ui.label("Monitoring")
                .on_hover_text("Watch the clipboard for supported links");
            ui.add_space(24.0);
            let mut auto = self.settings.default_action == DefaultAction::AutoDownload;
            if switch(ui, &mut auto)
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
            ui.label("Auto-download")
                .on_hover_text("Skip the popup and download with your default preset");
        });
        ui.horizontal(|ui| {
            // Explicit equal shares (0.5.1): at the panel's
            // 340px minimum width the three natural-width
            // buttons sum wider than the row, and egui paints
            // the excess past the right edge instead of
            // shrinking them (v0.5.0 field captures). Each
            // button now gets a fixed third of the row and
            // truncates its label inside that share. 0.6.0:
            // drawn line icons instead of the ⚙/⬇/📂 glyphs.
            let share = (ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0;
            let btn_size = egui::vec2(share, 34.0);
            if icon_text_button(ui, IconKind::Gear, "Settings…", btn_size)
                .on_hover_text("Show/hide the Settings window")
                .clicked()
            {
                // A toggle (SEAN, 0.2.2): the button opens the
                // Settings window and closes it again (0.5.1:
                // its own window, not a slide-out drawer).
                self.show_settings = !self.show_settings;
            }
            let active = self.downloads.active_count();
            let label = if active > 0 {
                format!("Downloads ({active} active)…")
            } else {
                "Downloads…".to_string()
            };
            if icon_text_button(ui, IconKind::Download, &label, btn_size)
                .on_hover_text("Active downloads and history")
                .clicked()
            {
                self.show_downloads = true;
            }
            if icon_text_button(ui, IconKind::Folder, "Download folder", btn_size)
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
        ui.add_space(12.0);
        ui.label(section_heading("Download a link"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // Reserve the button FIRST (0.5.1). The 0.5.0 budget
            // (available − 112) ran the row past the panel's
            // right edge in the field: egui's TextEdit outer
            // width is its desired_width PLUS its frame margin
            // (2×8px \u2014 vendored egui 0.33.3 TextEdit
            // builder), which the budget never subtracted, and
            // the button's min_size was treated as its exact
            // width. Now the button has a fixed size and the
            // field gets what remains after button + spacing +
            // the field's own margin.
            let btn_w = 108.0;
            let field_w =
                (ui.available_width() - btn_w - ui.spacing().item_spacing.x - 16.0).max(80.0);
            let resp = ui
                .add(
                    egui::TextEdit::singleline(&mut self.manual_url)
                        .hint_text("Paste a video URL…")
                        .desired_width(field_w),
                )
                .on_hover_text("Type or paste a URL, then press Enter or Download");
            // Accent focus ring + deep-red outer glow (0.6.0).
            paint_focus_ring_for_field(ui, &resp);
            let go = ui
                .add_sized(egui::vec2(btn_w, 30.0), primary_button("Download…"))
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
            ui.label(section_heading("Downloading"));
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
            ui.label(section_heading("Recent"));
            type RecentRow = (
                String,
                HistoryStatus,
                Option<std::path::PathBuf>,
                String,
                String,
                i64,
            );
            let recent: Vec<RecentRow> = self
                .history
                .recent(5)
                .iter()
                .map(|e| {
                    (
                        e.title.clone(),
                        e.status,
                        e.file.clone(),
                        e.url.clone(),
                        e.preset_label.clone(),
                        e.finished_at_unix,
                    )
                })
                .collect();
            let mut open_file: Option<std::path::PathBuf> = None;
            for (title, status, file, url, preset_label, finished_at) in &recent {
                // Each entry is a small raised card (0.6.0):
                // gradient thumbnail tile with a play triangle,
                // two-line text (title + host · quality · time —
                // history stores no uploader or duration, so the
                // mockup's meta line is built from what history
                // does record), the painted status dot, and the
                // folder button. Widths are budgeted exactly as
                // in 0.5.1: tile, text, dot and button all have
                // fixed shares and the title truncates inside
                // its share, so the row fits at 340px too.
                let shown = egui::Frame::NONE
                    .fill(CARD)
                    .stroke(egui::Stroke::new(1.0_f32, HAIRLINE))
                    .corner_radius(RADIUS)
                    .inner_margin(egui::Margin::symmetric(10, 7))
                    .outer_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: 0,
                        bottom: 6,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (tile, _) = ui
                                .allocate_exact_size(egui::vec2(56.0, 32.0), egui::Sense::hover());
                            paint_vertical_gradient(
                                ui.painter(),
                                tile,
                                7.0,
                                egui::Color32::from_rgb(0x3A, 0x22, 0x26),
                                egui::Color32::from_rgb(0x1C, 0x14, 0x18),
                            );
                            let c = tile.center();
                            ui.painter().add(egui::Shape::convex_polygon(
                                vec![
                                    egui::pos2(c.x - 5.0, c.y - 7.0),
                                    egui::pos2(c.x - 5.0, c.y + 7.0),
                                    egui::pos2(c.x + 7.0, c.y),
                                ],
                                egui::Color32::WHITE,
                                egui::Stroke::NONE,
                            ));
                            let spacing = ui.spacing().item_spacing.x;
                            let btn_w = if file.is_some() { 30.0 } else { 0.0 };
                            let text_w = (ui.available_width()
                                - 10.0
                                - spacing
                                - btn_w
                                - if file.is_some() { spacing } else { 0.0 })
                            .max(40.0);
                            let meta = format!(
                                "{} · {} · {}",
                                host_of(url),
                                preset_label,
                                local_time_hhmm(*finished_at)
                            );
                            ui.allocate_ui_with_layout(
                                egui::vec2(text_w, 34.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(title.clone())
                                                .color(TEXT)
                                                .size(13.0),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(title.clone());
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(meta).color(TEXT_FAINT).size(11.5),
                                        )
                                        .truncate(),
                                    );
                                },
                            );
                            // Painted dot, not the "●" character
                            // (0.5.1 — U+25CF is missing from
                            // egui's default font and rendered
                            // as a hollow square in the field).
                            status_dot(ui, status_color(*status));
                            if let Some(f) = file {
                                if icon_text_button(
                                    ui,
                                    IconKind::Folder,
                                    "",
                                    egui::vec2(btn_w, 24.0),
                                )
                                .on_hover_text("Show in Explorer")
                                .clicked()
                                {
                                    open_file = Some(f.clone());
                                }
                            }
                        });
                    });
                paint_top_highlight(ui.painter(), shown.response.rect, RADIUS as f32);
            }
            if let Some(f) = open_file {
                if let Some(parent) = f.parent() {
                    let _ = platform::open_path(parent);
                }
            }
        }

        // Saved for later (0.4.0): links parked from the popup.
        if !self.saved_links.is_empty() {
            ui.add_space(8.0);
            ui.label(section_heading("Saved for later"));
            let mut act: Option<(usize, bool)> = None; // (index, download?)
            for (i, link) in self.saved_links.iter().enumerate() {
                ui.horizontal(|ui| {
                    let label = if link.title.is_empty() || link.title == link.url {
                        truncate(&link.url, 40)
                    } else {
                        truncate(&link.title, 40)
                    };
                    ui.label(egui::RichText::new(label).color(TEXT))
                        .on_hover_text(link.url.clone());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("✕")
                            .on_hover_text("Remove from the saved list")
                            .clicked()
                        {
                            act = Some((i, false));
                        }
                        if ui
                            .small_button("⬇ Download")
                            .on_hover_text(
                                "Download now with your default quality (or its domain pin)",
                            )
                            .clicked()
                        {
                            act = Some((i, true));
                        }
                    });
                });
            }
            if let Some((i, download)) = act {
                if download {
                    self.download_saved_link(i);
                } else {
                    self.remove_saved_link(i);
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

        // Footer (0.5.0), a recessed bar since 0.6.0: pinned
        // to the bottom of the window, painted full-bleed to
        // the window edges (WELL fill, dark top line) so a
        // short panel no longer reads as content floating over
        // an empty black void. When the content is taller than
        // the window the spare height is negative and the
        // footer just follows it. The buttons keep their
        // 0.5.1 fixed sizes inside the bar.
        let spare = ui.available_height() - 46.0;
        if spare > 8.0 {
            ui.add_space(spare);
        } else {
            ui.add_space(8.0);
        }
        let bar_rect =
            egui::Rect::from_min_max(egui::pos2(full.min.x, ui.cursor().top()), full.max);
        // painter_at: the content child's own clip is inset,
        // and the bar must reach the window edges.
        let bar_painter = ui.painter_at(bar_rect);
        bar_painter.rect_filled(bar_rect, 0.0, WELL);
        bar_painter.line_segment(
            [bar_rect.left_top(), bar_rect.right_top()],
            egui::Stroke::new(1.0_f32, STROKE_DARK),
        );
        ui.horizontal(|ui| {
            // Explicit sizes (0.5.1): Exit used to be placed by
            // a nested right-to-left layout and landed on the
            // panel's extreme right edge, clipped at every
            // width (field captures). Both buttons now have
            // fixed sizes, labels truncate inside them, and the
            // gap between them is computed from the row's width.
            let hide_size = egui::vec2(134.0, 30.0);
            let exit_size = egui::vec2(124.0, 30.0);
            let gap = (ui.available_width()
                - hide_size.x
                - exit_size.x
                - 2.0 * ui.spacing().item_spacing.x)
                .max(0.0);
            if ui
                .add_sized(
                    hide_size,
                    egui::Button::new("Hide tray icon…").truncate(),
                )
                .on_hover_text("Run without a tray icon. Relaunch ytTrayDL or press Ctrl+Alt+G to get it back.")
                .clicked()
            {
                self.confirm_hide_icon = true;
            }
            ui.add_space(gap);
            if ui
                .add_sized(
                    exit_size,
                    egui::Button::new("Exit ytTrayDL").truncate(),
                )
                .on_hover_text("Quit completely (downloads in progress are interrupted; partial files can resume)")
                .clicked()
            {
                self.exit_requested = true;
            }
        });
    }

    // ---------------- Download popup ----------------

    pub fn popup_ui(&mut self, ctx: &egui::Context) {
        #[derive(PartialEq)]
        enum Action {
            None,
            Dismiss,
            Download,
            SaveLater,
            IgnoreUrl,
            IgnoreDomain,
        }
        let Some(mut popup) = self.active_popup.take() else {
            return;
        };
        let is_playlist = popup.info.as_ref().is_some_and(|i| i.is_playlist());
        // Popup size (0.6.0 hero layout): fixed 430 wide; the
        // height is built up from the sections actually shown
        // — hero, title zone, channel row, URL row, format
        // chips (wrapped; the row count is estimated from the
        // chip label widths at the chip font size), checkbox,
        // save-to row, actions, and the optional rows
        // (already-downloaded note, custom-format line,
        // playlist chips, countdown). Playlist links grow the
        // popup once the probe says "this is a playlist".
        let size = {
            let font = egui::FontId::new(12.5, egui::FontFamily::Proportional);
            let mut chip_rows = 1usize;
            let mut x = 0.0f32;
            for p in Preset::ALL {
                let w = ctx.fonts_mut(|f| {
                    f.layout_no_wrap(preset_chip_label(p).to_string(), font.clone(), TEXT)
                        .size()
                        .x
                }) + 28.0;
                if x > 0.0 && x + w > 406.0 {
                    chip_rows += 1;
                    x = 0.0;
                }
                x += w + 8.0;
            }
            let chips_h = chip_rows as f32 * 28.0 + (chip_rows - 1) as f32 * 8.0;
            let mut h = 12.0
                + 184.0 // hero
                + 10.0
                + 46.0 // title zone (two lines)
                + 6.0
                + 24.0 // channel row
                + 6.0
                + 18.0 // URL row
                + 6.0
                + 8.0
                + chips_h
                + 8.0
                + 22.0 // always-auto checkbox
                + 24.0 // save-to row
                + 6.0
                + 36.0 // actions
                + 12.0;
            if popup.info.is_none() {
                // No channel row; the fallback title block is
                // shorter than the two-line title zone.
                h -= 46.0 + 6.0 + 24.0 - 38.0;
            }
            if popup.already_downloaded {
                h += 38.0;
            }
            if popup.preset == Preset::Custom {
                h += 18.0;
            }
            if is_playlist {
                h += 34.0;
            }
            if self.settings.popup_timeout_secs > 0 {
                h += 24.0;
            }
            egui::vec2(430.0, h)
        };
        let pos = match self.settings.popup_position {
            PopupPosition::BottomRight => bottom_right(ctx, size, 0.0),
            PopupPosition::TopRight => {
                let mon = monitor_size(ctx);
                egui::pos2((mon.x - size.x - 12.0).max(0.0), 12.0)
            }
            PopupPosition::TopLeft => egui::pos2(12.0, 12.0),
            PopupPosition::BottomLeft => {
                let mon = monitor_size(ctx);
                egui::pos2(12.0, (mon.y - size.y - 12.0).max(0.0))
            }
            PopupPosition::Center => {
                let mon = monitor_size(ctx);
                egui::pos2(
                    ((mon.x - size.x) / 2.0).max(0.0),
                    ((mon.y - size.y) / 2.0).max(0.0),
                )
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
                    // Hero header (0.6.0): a full-width
                    // thumbnail with rounded corners and a soft
                    // shadow, a source chip, a duration chip and
                    // a painted play badge — or, while info /
                    // the thumbnail is still loading, the
                    // mockup's deep-red gradient placeholder with
                    // the same furniture. Below it: the two-line
                    // title and the channel row.
                    let thumb = match (&popup.info, &self.popup_thumb) {
                        (Some(info), Some((url, handle))) if info.thumbnail.as_ref() == Some(url) => {
                            Some(handle.clone())
                        }
                        _ => None,
                    };
                    let hero_w = ui.available_width();
                    let (hero, _) =
                        ui.allocate_exact_size(egui::vec2(hero_w, 184.0), egui::Sense::hover());
                    paint_soft_glow(ui.painter(), hero, 12.0, egui::Color32::BLACK);
                    match &thumb {
                        Some(handle) => {
                            // The texture was centre-cropped to
                            // this aspect when it was decoded
                            // (process_thumbs), so an exact fit
                            // does not distort it.
                            ui.put(
                                hero,
                                egui::Image::new((handle.id(), hero.size()))
                                    .corner_radius(12)
                                    .fit_to_exact_size(hero.size()),
                            );
                        }
                        None => {
                            paint_vertical_gradient(
                                ui.painter(),
                                hero,
                                12.0,
                                egui::Color32::from_rgb(0x34, 0x1D, 0x21),
                                egui::Color32::from_rgb(0x12, 0x0E, 0x12),
                            );
                        }
                    }
                    {
                        let painter = ui.painter();
                        // Light top edge over the hero.
                        painter.line_segment(
                            [
                                egui::pos2(hero.left() + 12.0, hero.top() + 0.5),
                                egui::pos2(hero.right() - 12.0, hero.top() + 0.5),
                            ],
                            egui::Stroke::new(
                                1.0_f32,
                                with_alpha(egui::Color32::from_rgb(0x6E, 0x54, 0x56), 150),
                            ),
                        );
                        // Source chip (top-left): dark pill,
                        // accent dot, host.
                        let host_text = registrable(&popup.host).to_string();
                        let host_galley = painter.layout_no_wrap(
                            host_text.clone(),
                            egui::FontId::new(11.0, egui::FontFamily::Proportional),
                            TEXT,
                        );
                        let chip_rect = egui::Rect::from_min_size(
                            hero.min + egui::vec2(12.0, 12.0),
                            egui::vec2(host_galley.size().x + 34.0, 24.0),
                        );
                        painter.rect_filled(
                            chip_rect,
                            7.0,
                            egui::Color32::from_rgba_unmultiplied(10, 10, 11, 235),
                        );
                        painter.rect_stroke(
                            chip_rect,
                            egui::CornerRadius::same(7),
                            egui::Stroke::new(
                                1.0_f32,
                                egui::Color32::from_rgb(0x46, 0x3C, 0x3C),
                            ),
                            egui::epaint::StrokeKind::Inside,
                        );
                        painter.circle_filled(
                            chip_rect.min + egui::vec2(13.0, 12.0),
                            3.5,
                            ACCENT,
                        );
                        painter.galley(
                            chip_rect.min + egui::vec2(22.0, 12.0) - host_galley.size() / 2.0,
                            host_galley,
                            TEXT,
                        );
                        // Duration chip (bottom-right).
                        if let Some(d) = popup.info.as_ref().and_then(|i| i.duration) {
                            let dur = yttraydl::ytdlp::format_duration(d);
                            if !dur.is_empty() {
                                let galley = painter.layout_no_wrap(
                                    dur,
                                    egui::FontId::new(11.0, egui::FontFamily::Proportional),
                                    egui::Color32::WHITE,
                                );
                                let dur_rect = egui::Rect::from_min_size(
                                    hero.max - egui::vec2(galley.size().x + 26.0, 34.0),
                                    egui::vec2(galley.size().x + 16.0, 24.0),
                                );
                                painter.rect_filled(
                                    dur_rect,
                                    6.0,
                                    egui::Color32::from_rgba_unmultiplied(8, 8, 9, 235),
                                );
                                painter.galley(
                                    dur_rect.center() - galley.size() / 2.0,
                                    galley,
                                    egui::Color32::WHITE,
                                );
                            }
                        }
                        // Play badge: dark ringed circle with a
                        // white triangle, soft shadow beneath.
                        let c = hero.center();
                        painter.circle_filled(
                            c + egui::vec2(0.0, 3.0),
                            27.0,
                            egui::Color32::from_black_alpha(110),
                        );
                        painter.circle_filled(
                            c,
                            27.0,
                            egui::Color32::from_rgba_unmultiplied(14, 14, 16, 235),
                        );
                        painter.circle_stroke(
                            c,
                            27.0,
                            egui::Stroke::new(
                                1.0_f32,
                                egui::Color32::from_rgb(0x78, 0x78, 0x7A),
                            ),
                        );
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                egui::pos2(c.x - 6.0, c.y - 14.0),
                                egui::pos2(c.x - 6.0, c.y + 14.0),
                                egui::pos2(c.x + 16.0, c.y),
                            ],
                            egui::Color32::WHITE,
                            egui::Stroke::NONE,
                        ));
                    }
                    ui.add_space(10.0);
                    match &popup.info {
                        Some(info) => {
                            // Two-line title (0.6.0): wrapped,
                            // clipped to a fixed two-line zone;
                            // the full title is on hover.
                            let (zone, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 46.0),
                                egui::Sense::hover(),
                            );
                            let mut title_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(zone)
                                    .layout(egui::Layout::top_down(egui::Align::Min)),
                            );
                            title_ui.set_clip_rect(zone);
                            title_ui
                                .add(
                                    egui::Label::new(
                                        egui::RichText::new(truncate(&info.display_title(), 78))
                                            .strong()
                                            .size(16.0)
                                            .color(TEXT),
                                    )
                                    .wrap(),
                                )
                                .on_hover_text(info.display_title());
                            // Channel row: gradient-initial
                            // avatar + uploader, then the faint
                            // meta line (resolution · size ·
                            // duration · host · playlist note) —
                            // the same facts the old header meta
                            // line carried.
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                let mut meta: Vec<String> = Vec::new();
                                if let Some(res) = info.resolution_text() {
                                    meta.push(res);
                                }
                                if let Some(bytes) = info.size_bytes() {
                                    meta.push(format!(
                                        "{}{}",
                                        if info.size_is_approx() { "~" } else { "" },
                                        yttraydl::ytdlp::format_bytes(bytes)
                                    ));
                                }
                                if let Some(d) = info.duration {
                                    let s = yttraydl::ytdlp::format_duration(d);
                                    if !s.is_empty() {
                                        meta.push(s);
                                    }
                                }
                                meta.push(popup.host.clone());
                                if info.is_playlist() {
                                    meta.push(if popup.playlist_all {
                                        "whole playlist".into()
                                    } else {
                                        "playlist — first item only".into()
                                    });
                                }
                                let uploader = info.display_uploader();
                                if let Some(u) = &uploader {
                                    let (av, _) = ui.allocate_exact_size(
                                        egui::vec2(24.0, 24.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(av.center(), 12.0, CHIP_GRAD_TOP);
                                    let initials: String = u
                                        .split_whitespace()
                                        .take(2)
                                        .filter_map(|w| w.chars().next())
                                        .collect::<String>()
                                        .to_uppercase();
                                    if !initials.is_empty() {
                                        ui.painter().text(
                                            av.center(),
                                            egui::Align2::CENTER_CENTER,
                                            initials,
                                            egui::FontId::new(9.5, egui::FontFamily::Proportional),
                                            egui::Color32::WHITE,
                                        );
                                    }
                                    ui.label(egui::RichText::new(u.clone()).color(TEXT).size(12.5))
                                        .on_hover_text(u.clone());
                                }
                                if !meta.is_empty() {
                                    // After a channel name the meta
                                    // reads as a continuation ("· …");
                                    // without one it stands alone.
                                    let text = if uploader.is_some() {
                                        format!("· {}", meta.join(" · "))
                                    } else {
                                        meta.join(" · ")
                                    };
                                    ui.label(
                                        egui::RichText::new(text).color(TEXT_FAINT).size(12.0),
                                    );
                                }
                            });
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
                    ui.add_space(6.0);
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

                    // Format chips (0.6.0): the preset control
                    // restyled from a combo box to the mockup's
                    // chip row — same Preset values, same
                    // change-marks-interaction behaviour; every
                    // preset is a chip, wrapping to more rows,
                    // and the full preset label is on each
                    // chip's hover.
                    ui.add_space(8.0);
                    {
                        let before = popup.preset;
                        ui.horizontal_wrapped(|ui| {
                            for p in Preset::ALL {
                                if chip(ui, preset_chip_label(p), popup.preset == p)
                                    .on_hover_text(p.label())
                                    .clicked()
                                {
                                    popup.preset = p;
                                }
                            }
                        });
                        if popup.preset != before {
                            interacted = true;
                        }
                    }
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

                    if is_playlist {
                        ui.horizontal(|ui| {
                            ui.label("Playlist:");
                            if chip(ui, "Just this video", !popup.playlist_all)
                                .on_hover_text("Download only the linked/first item")
                                .clicked()
                            {
                                popup.playlist_all = false;
                                interacted = true;
                            }
                            if chip(ui, "Whole playlist", popup.playlist_all)
                                .on_hover_text("Download every item in the playlist, one by one")
                                .clicked()
                            {
                                popup.playlist_all = true;
                                interacted = true;
                            }
                        });
                    }

                    ui.horizontal(|ui| {
                        ui.add_sized(egui::vec2(56.0, 24.0), egui::Label::new("Save to:"));
                        match &popup.folder {
                            Some(f) => {
                                ui.label(
                                    egui::RichText::new(truncate(
                                        &f.file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_else(|| f.display().to_string()),
                                        22,
                                    ))
                                    .color(TEXT),
                                )
                                .on_hover_text(f.display().to_string());
                                if ui
                                    .small_button("Reset")
                                    .on_hover_text("Use the default download folder")
                                    .clicked()
                                {
                                    popup.folder = None;
                                    interacted = true;
                                }
                            }
                            None => {
                                ui.label(egui::RichText::new("default folder").color(TEXT_DIM));
                            }
                        }
                        if ui
                            .small_button("Change…")
                            .on_hover_text("Save THIS download to a different folder")
                            .clicked()
                        {
                            let start = popup
                                .folder
                                .clone()
                                .unwrap_or_else(|| self.settings.download_folder.clone());
                            if let Some(f) = pick_folder(&start) {
                                popup.folder = Some(f);
                            }
                            interacted = true;
                        }
                    });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        // Mockup order (0.6.0): Download, Later,
                        // Not now, Ignore.
                        let download_label = if popup.already_downloaded {
                            "⬇ Download anyway"
                        } else {
                            "⬇ Download"
                        };
                        if ui
                            .add(primary_button(download_label).min_size(egui::vec2(150.0, 36.0)))
                            .on_hover_text("Download with the selected quality into your download folder")
                            .clicked()
                        {
                            action = Action::Download;
                            interacted = true;
                        }
                        if ui.add(egui::Button::new("Later").min_size(egui::vec2(84.0, 36.0))).on_hover_text("Park this link in the panel's Saved list and download it whenever you're ready").clicked() {
                            action = Action::SaveLater;
                        }
                        if ui.add(egui::Button::new("Not now").min_size(egui::vec2(84.0, 36.0))).on_hover_text("Dismiss. Copy the link again later to be asked again after the dedupe window.").clicked() {
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
                        ui.add_space(6.0);
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
                let playlist_choice = popup
                    .info
                    .as_ref()
                    .is_some_and(|i| i.is_playlist())
                    .then_some(popup.playlist_all);
                self.start_download_full(
                    &popup.url,
                    &title,
                    popup.preset,
                    popup.already_downloaded,
                    playlist_choice,
                    popup.folder.clone(),
                );
            }
            Action::SaveLater => {
                let title = popup
                    .info
                    .as_ref()
                    .map(|i| i.display_title())
                    .unwrap_or_else(|| popup.url.clone());
                self.save_link_for_later(&popup.url, &title);
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

/// Short chip labels for the popup's format chips (0.6.0):
/// the full preset labels ("Up to 2160p (4K)", "Audio only —
/// original format") are combo-box text; chips need the
/// mockup's compact forms. The full label rides on each
/// chip's tooltip, and the preset VALUES are untouched.
pub fn preset_chip_label(p: Preset) -> &'static str {
    match p {
        Preset::Best => "Best",
        Preset::P2160 => "2160p 4K",
        Preset::P1440 => "1440p",
        Preset::P1080 => "1080p",
        Preset::P720 => "720p",
        Preset::P480 => "480p",
        Preset::P360 => "360p",
        Preset::AudioMp3 => "MP3",
        Preset::AudioM4a => "M4A",
        Preset::AudioOpus => "Opus",
        Preset::AudioFlac => "FLAC",
        Preset::AudioWav => "WAV",
        Preset::AudioOriginal => "Original audio",
        Preset::SubtitlesOnly => "Subtitles only",
        Preset::Custom => "Custom…",
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
    pub install_ffmpeg: bool,
    pub set_autostart: Option<bool>,
    pub confirm_hide_icon: bool,
    pub clear_history: bool,
    pub reset_all: bool,
    pub open_config_dir: bool,
    pub pick_ytdlp: bool,
    pub pick_ffmpeg: bool,
    pub check_ytdlp_update: bool,
    pub export_settings: bool,
    pub import_settings: bool,
    pub check_watch_now: bool,
}

impl App {
    /// Settings — its own window since 0.5.1. It was a
    /// separate window until 0.2.1, then a slide-out drawer in
    /// the main window (0.2.1–0.5.0); the v0.5.0 field
    /// captures showed the drawer squeezing the panel and
    /// panel content painting under it, so Settings is a
    /// standalone window again, built like Downloads/About.
    /// The tab content functions and the 0.5.0 styling (pill
    /// tabs, cards, aligned sliders) are unchanged — only
    /// the container is new. Closing the window (its X or the
    /// header button) just hides it; the app keeps running.
    pub fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut settings = std::mem::take(&mut self.settings);
        let mut rule_ignore = std::mem::take(&mut self.rule_input_ignore);
        let mut rule_auto = std::mem::take(&mut self.rule_input_auto);
        let mut rule_allow = std::mem::take(&mut self.rule_input_allow);
        let mut rule_url = std::mem::take(&mut self.rule_input_url);
        let mut rule_preset_domain = std::mem::take(&mut self.rule_input_preset_domain);
        let mut rule_preset_choice = self.rule_preset_choice;
        let mut rule_watch_url = std::mem::take(&mut self.rule_input_watch_url);
        let mut rule_watch_preset = self.rule_watch_preset;
        let mut tab = self.settings_tab;
        let mut reset_armed = self.reset_armed;
        let mut open = true;
        let mut close = false;
        let mut outcome = SettingsOutcome::default();
        let bin_info: Option<(String, String, String)> = self.bin.as_ref().map(|b| {
            (
                b.path.display().to_string(),
                source_label(&b.source).to_string(),
                self.ytdlp_version.clone().unwrap_or_else(|| "…".into()),
            )
        });
        let ffmpeg_ok = self.ffmpeg_ok;
        let ffmpeg_managed = Settings::managed_bin_dir().join("ffmpeg.exe").is_file();
        let fetching = self.ytdlp_fetch_rx.is_some();
        let fetching_ffmpeg = self.ffmpeg_fetch_rx.is_some();
        let autostart_now = platform::Platform::autostart_enabled();

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-settings"),
            egui::ViewportBuilder::default()
                .with_title("ytTrayDL — Settings")
                .with_inner_size(SETTINGS_WINDOW_SIZE)
                .with_min_inner_size([480.0, 420.0]),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Settings")
                                .strong()
                                .size(20.0)
                                .color(TEXT),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("✕").on_hover_text("Close settings").clicked() {
                                close = true;
                            }
                            ui.label(
                                egui::RichText::new("Settings save automatically")
                                    .color(TEXT_FAINT)
                                    .small(),
                            );
                        });
                    });
                    ui.add_space(6.0);
                    // Segmented tab bar (0.6.0): a recessed
                    // track; the selected segment is filled
                    // with the red gradient. (0.5.0 used pill
                    // buttons; the save note moved up beside
                    // the title, as in the mockup.)
                    settings_tab_bar(ui, &mut tab);
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
                            &mut rule_preset_domain,
                            &mut rule_preset_choice,
                            &mut rule_watch_url,
                            &mut rule_watch_preset,
                        ),
                        _ => settings_ytdlp(
                            ui,
                            &mut settings,
                            &mut outcome,
                            &bin_info,
                            ToolsUi {
                                ffmpeg_ok,
                                ffmpeg_managed,
                                ffmpeg_fetching: fetching_ffmpeg,
                                update_checking: self.ytdlp_check_rx.is_some(),
                                latest: self.ytdlp_latest.clone(),
                                current_version: self.ytdlp_version.clone(),
                            },
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
        self.rule_input_preset_domain = rule_preset_domain;
        self.rule_preset_choice = rule_preset_choice;
        self.rule_input_watch_url = rule_watch_url;
        self.rule_watch_preset = rule_watch_preset;
        self.settings_tab = tab;
        self.reset_armed = reset_armed;
        if close || !open {
            self.show_settings = false;
        }

        if outcome.changed {
            self.settings_dirty = true;
        }
        if outcome.relocalize {
            self.relocalize_binaries();
        }
        if outcome.install_ytdlp {
            self.start_ytdlp_download();
        }
        if outcome.install_ffmpeg {
            self.start_ffmpeg_download();
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
        if outcome.check_ytdlp_update {
            self.start_ytdlp_update_check();
        }
        if outcome.check_watch_now {
            self.watch_check_now = true;
        }
        if outcome.export_settings {
            if let Some(path) = pick_save_json("yttraydl-settings.json") {
                match serde_json::to_string_pretty(&self.settings) {
                    Ok(text) => match std::fs::write(&path, text) {
                        Ok(()) => {
                            self.toast("Settings exported.", ToastKind::Success);
                        }
                        Err(e) => {
                            self.toast(
                                format!("Could not write the settings file: {e}"),
                                ToastKind::Error,
                            );
                        }
                    },
                    Err(e) => {
                        self.toast(format!("Export failed: {e}"), ToastKind::Error);
                    }
                }
            }
        }
        if outcome.import_settings {
            if let Some(path) = pick_file("JSON settings", "json") {
                match std::fs::read_to_string(&path) {
                    Ok(text) => match Settings::from_json(&text) {
                        Ok(new_settings) => {
                            self.settings = new_settings;
                            self.settings_dirty = true;
                            let _ =
                                platform::Platform::set_autostart(self.settings.start_with_windows);
                            self.relocalize_binaries();
                            self.toast(
                                "Settings imported — they apply now and are saved.",
                                ToastKind::Success,
                            );
                        }
                        Err(e) => {
                            self.toast(format!("Import failed: {e}"), ToastKind::Error);
                        }
                    },
                    Err(e) => {
                        self.toast(format!("Could not read that file: {e}"), ToastKind::Error);
                    }
                }
            }
        }
    }
}

/// Fixed-width label for a slider row (0.5.0): every slider
/// label in Settings gets the same 190px column, so the
/// sliders (and their value boxes, e.g. "30 s") line up in
/// columns instead of starting wherever the label ended.
fn slider_label(ui: &mut egui::Ui, text: &str) {
    ui.add_sized(egui::vec2(190.0, 24.0), egui::Label::new(text));
}

/// The value chip that replaces a slider's readout (0.6.0):
/// a small raised pill (CARD2 fill, hairline stroke) holding
/// the formatted value — "30 s", "300 s", "0 KB/s".
fn value_chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::NONE
        .fill(CARD2)
        .stroke(egui::Stroke::new(1.0_f32, HAIRLINE))
        .corner_radius(7)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(TEXT).size(12.5));
        });
}

/// Width a value chip will occupy for this text (measured at
/// the chip's font size, plus its padding and stroke) — the
/// slider row budgets the slider from what remains.
fn value_chip_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.fonts_mut(|f| {
        f.layout_no_wrap(
            text.to_string(),
            egui::FontId::new(12.5, egui::FontFamily::Proportional),
            TEXT,
        )
        .size()
        .x
    }) + 22.0
}

/// A slider with the mockup's finish (0.6.0): thin track and
/// accent trail from the theme, egui's editable value box
/// replaced by a value chip, and the thumb painted white
/// with a soft shadow. egui 0.33 has no thumb-colour API —
/// its handle uses the widget visuals — so the white thumb
/// is painted over egui's own at exactly the position the
/// vendored slider code computes (handle radius =
/// rect height / 2.5; travel range = the rect shrunk by that
/// radius on both ends). Dragging, keyboard control and the
/// changed() signal are egui's own, unchanged.
fn slider_with_chip<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    chip_text: &str,
    slider_width: f32,
    tip: &str,
) -> egui::Response {
    let (min, max) = (range.start().to_f64(), range.end().to_f64());
    ui.style_mut().spacing.slider_width = slider_width.max(40.0);
    let resp = ui
        .add(egui::Slider::new(value, range).show_value(false))
        .on_hover_text(tip);
    let frac = if max > min {
        ((value.to_f64() - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let rect = resp.rect;
    let r = rect.height() / 2.5;
    let cx = rect.left() + r + frac as f32 * (rect.width() - 2.0 * r);
    let center = egui::pos2(cx, rect.center().y);
    let painter = ui.painter();
    painter.circle_filled(
        center + egui::vec2(0.0, 1.0),
        r + 1.0,
        egui::Color32::from_black_alpha(70),
    );
    painter.circle_filled(center, r, egui::Color32::WHITE);
    value_chip(ui, chip_text);
    resp
}

/// A full slider row (0.6.0): fixed 190px label column (the
/// 0.5.0 alignment), the slider, and its value chip. Returns
/// the slider's response.
fn slider_row<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    chip_text: &str,
    tip: &str,
) -> egui::Response {
    let mut resp: Option<egui::Response> = None;
    ui.horizontal(|ui| {
        slider_label(ui, label);
        let slider_w = ui.available_width()
            - value_chip_width(ui, chip_text)
            - 2.0 * ui.spacing().item_spacing.x;
        resp = Some(slider_with_chip(ui, value, range, chip_text, slider_w, tip));
    });
    resp.expect("slider row always produces a response")
}

fn settings_general(
    ui: &mut egui::Ui,
    s: &mut Settings,
    out: &mut SettingsOutcome,
    autostart_now: bool,
) {
    section_card(ui, "When a supported link is copied", |ui| {
        // Switch (0.6.0) instead of a checkbox, per the mockup.
        ui.horizontal(|ui| {
            if switch(ui, &mut s.monitoring_enabled)
                .on_hover_text("When off, ytTrayDL never reacts to copied links. You can still download from the panel.")
                .changed()
            {
                out.changed = true;
            }
            ui.label("Monitor the clipboard")
                .on_hover_text("When off, ytTrayDL never reacts to copied links. You can still download from the panel.");
        });
        ui.horizontal(|ui| {
            ui.label("Default action:");
            // Segmented control (0.6.0) instead of the radio
            // pair, per the mockup; both options keep their
            // radio tooltips.
            let current = match s.default_action {
                DefaultAction::Prompt => 0,
                DefaultAction::AutoDownload => 1,
            };
            if let Some(picked) = segmented_choice(
                ui,
                &["Ask me (popup)", "Auto-download"],
                &[
                    "Show the small download popup and let you pick quality each time",
                    "Download immediately with your default quality — no popup, no interruption",
                ],
                current,
                300.0,
            ) {
                s.default_action = if picked == 1 {
                    DefaultAction::AutoDownload
                } else {
                    DefaultAction::Prompt
                };
                out.changed = true;
            }
        });
        ui.label(egui::RichText::new("Per-domain rules (Domains & rules tab) can override this: a domain can always auto-download, or always be ignored.").color(TEXT_DIM).small());
    });

    section_card(ui, "Which links count as supported", |ui| {
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
    });

    section_card(ui, "Tray & startup", |ui| {
        // Switches (0.6.0) instead of checkboxes, per the
        // mockup; the hide-icon switch keeps the checkbox's
        // exact semantics (off asks for confirmation first and
        // writes nothing until confirmed).
        ui.horizontal(|ui| {
            let mut visible = s.tray_icon_visible;
            if switch(ui, &mut visible)
                .on_hover_text("Uncheck to run invisibly. You can always get the icon back by launching ytTrayDL again or pressing Ctrl+Alt+G.")
                .changed()
            {
                if !visible {
                    out.confirm_hide_icon = true; // hiding needs the recovery explanation first
                } else {
                    s.tray_icon_visible = true;
                    out.changed = true;
                }
            }
            ui.label("Show tray icon")
                .on_hover_text("Uncheck to run invisibly. You can always get the icon back by launching ytTrayDL again or pressing Ctrl+Alt+G.");
            ui.add_space(24.0);
            let mut autostart = autostart_now;
            if switch(ui, &mut autostart)
                .on_hover_text(
                    "Adds ytTrayDL to your per-user startup (HKCU Run key). No admin rights needed.",
                )
                .changed()
            {
                out.set_autostart = Some(autostart);
            }
            ui.label("Start with Windows").on_hover_text(
                "Adds ytTrayDL to your per-user startup (HKCU Run key). No admin rights needed.",
            );
        });
        ui.horizontal(|ui| {
        ui.label("When I close the main window:")
            .on_hover_text("What the window's X button does. 'Ask every time' shows a choice dialog with a remember option.");
        let mut action = s.close_action;
        egui::ComboBox::from_id_salt("close-action")
            .selected_text(action.label())
            .show_ui(ui, |ui| {
                for a in CloseAction::ALL {
                    ui.selectable_value(&mut action, a, a.label());
                }
            });
        if action != s.close_action {
            s.close_action = action;
            out.changed = true;
        }
    });
        ui.horizontal(|ui| {
        ui.label("When the download queue empties:")
            .on_hover_text("Runs once when a queue that had active downloads finishes completely — never just because the app is idle.");
        let mut qe = s.queue_empty_action;
        egui::ComboBox::from_id_salt("queue-empty")
            .selected_text(qe.label())
            .show_ui(ui, |ui| {
                for a in QueueEmptyAction::ALL {
                    ui.selectable_value(&mut qe, a, a.label());
                }
            });
        if qe != s.queue_empty_action {
            s.queue_empty_action = qe;
            out.changed = true;
        }
    });
        ui.horizontal(|ui| {
            ui.label("Popup position:");
            let mut pos = s.popup_position;
            egui::ComboBox::from_id_salt("popup-pos")
                .selected_text(match pos {
                    PopupPosition::BottomRight => "Bottom-right",
                    PopupPosition::TopRight => "Top-right",
                    PopupPosition::TopLeft => "Top-left",
                    PopupPosition::BottomLeft => "Bottom-left",
                    PopupPosition::Center => "Center of the screen",
                    PopupPosition::NearCursor => "Near the mouse cursor",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut pos, PopupPosition::BottomRight, "Bottom-right");
                    ui.selectable_value(&mut pos, PopupPosition::TopRight, "Top-right");
                    ui.selectable_value(&mut pos, PopupPosition::TopLeft, "Top-left");
                    ui.selectable_value(&mut pos, PopupPosition::BottomLeft, "Bottom-left");
                    ui.selectable_value(&mut pos, PopupPosition::Center, "Center of the screen");
                    ui.selectable_value(
                        &mut pos,
                        PopupPosition::NearCursor,
                        "Near the mouse cursor",
                    );
                });
            if pos != s.popup_position {
                s.popup_position = pos;
                out.changed = true;
            }
        });
        {
            let mut t = s.popup_timeout_secs;
            let chip_text = format!("{t} s");
            if slider_row(
                ui,
                "Popup auto-close:",
                &mut t,
                0..=300,
                &chip_text,
                "The popup closes without downloading after this long. 0 = stay until you decide. Hovering restarts the timer.",
            )
            .changed()
            {
                s.popup_timeout_secs = t;
                out.changed = true;
            }
        }
        {
            let mut d = s.dedupe_window_secs;
            let chip_text = format!("{d} s");
            if slider_row(
                ui,
                "Ignore re-copies for:",
                &mut d,
                0..=3600,
                &chip_text,
                "Copying the same link again within this window won't re-trigger ytTrayDL. 0 = always react.",
            )
            .changed()
            {
                s.dedupe_window_secs = d;
                out.changed = true;
            }
        }
    });

    premium_card(ui, |ui| {
        egui::CollapsingHeader::new("Notifications (small toasts, bottom-right)")
        .default_open(false)
        .show(ui, |ui| {
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
            if ui
                .checkbox(
                    &mut s.open_folder_on_finish,
                    "Open the download folder when a download finishes",
                )
                .on_hover_text("Opens the finished file's folder in Explorer. Off by default — it can be intrusive if several downloads finish together.")
                .changed()
            {
                out.changed = true;
            }
            if ui
                .checkbox(
                    &mut s.sound_on_finish,
                    "Play a sound when a download finishes",
                )
                .on_hover_text("The standard Windows notification sound. Off by default.")
                .changed()
            {
                out.changed = true;
            }
        });
    });
}

fn settings_downloads(ui: &mut egui::Ui, s: &mut Settings, out: &mut SettingsOutcome) {
    section_card(ui, "Where downloads go", |ui| {
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
    });

    section_card(ui, "Default quality", |ui| {
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
        if ui.checkbox(&mut s.allow_playlists, "Download whole playlists").on_hover_text("Off (default): a playlist link downloads just the linked/first item. On: the entire playlist, item by item. Either way, the popup lets you choose per download.").changed() {
        out.changed = true;
    }
        if ui.checkbox(&mut s.sponsorblock_remove, "Remove sponsored segments (SponsorBlock)").on_hover_text("yt-dlp cuts SponsorBlock-marked segments — sponsors, self-promos, interaction reminders — out of the downloaded file. Needs ffmpeg.").changed() {
        out.changed = true;
    }
        if ui.checkbox(&mut s.split_chapters, "Split chapters into separate files").on_hover_text("A chaptered video (streams, albums, podcasts) downloads as one file per chapter instead of one long file. Needs ffmpeg.").changed() {
        out.changed = true;
    }
        ui.label(egui::RichText::new("Tip: the “Subtitles only” quality preset downloads just the subtitle files (as .srt) — no video or audio. It uses the subtitle languages above.").color(TEXT_DIM).small());
        if ui.checkbox(&mut s.skip_already_downloaded, "Skip videos already downloaded before").on_hover_text("Uses a yt-dlp download archive stored with your ytTrayDL settings, so re-copying an old link doesn't fetch it twice").changed() {
        out.changed = true;
    }
        {
            let mut n = s.max_concurrent_downloads;
            let chip_text = format!("{n}");
            if slider_row(
                ui,
                "Simultaneous downloads:",
                &mut n,
                1..=4,
                &chip_text,
                "More parallelism isn't always faster — sites may throttle. 2 is a good default.",
            )
            .changed()
            {
                s.max_concurrent_downloads = n;
                out.changed = true;
            }
        }
        {
            let mut r = s.rate_limit_kbps;
            let chip_text = format!("{r} KB/s");
            if slider_row(
                ui,
                "Speed limit:",
                &mut r,
                0..=102_400,
                &chip_text,
                "Cap the download speed (passed to yt-dlp as --limit-rate). 0 = no limit.",
            )
            .changed()
            {
                s.rate_limit_kbps = r;
                out.changed = true;
            }
        }
        {
            let mut a = s.auto_retry_attempts;
            let chip_text = format!("{a} ×");
            if slider_row(
                ui,
                "Auto-retry failures:",
                &mut a,
                0..=5,
                &chip_text,
                "Retry a failed download this many times with its original quality before giving up. 0 = off. A manual cancel is never retried.",
            )
            .changed()
            {
                s.auto_retry_attempts = a;
                out.changed = true;
            }
        }
    });

    section_card(ui, "Disk space & automation", |ui| {
        {
            let mut mb = s.low_disk_threshold_mb;
            let chip_text = format!("{mb} MB free");
            if slider_row(
                ui,
                "Pause new downloads below:",
                &mut mb,
                0..=10_240,
                &chip_text,
                "While the download drive has less free space than this, queued downloads wait (running ones finish). 0 = guard off.",
            )
            .changed()
            {
                s.low_disk_threshold_mb = mb;
                out.changed = true;
            }
        }
        ui.horizontal(|ui| {
        ui.label("After each download, run:");
        if ui
            .add(egui::TextEdit::singleline(&mut s.post_download_command).desired_width(300.0).font(egui::TextStyle::Monospace))
            .on_hover_text("A command line run through the shell when a download succeeds. Placeholders: {file} {dir} {url} {title}. Leave empty for none. Example: explorer.exe \"{dir}\"")
            .changed()
        {
            out.changed = true;
        }
    });
    });

    premium_card(ui, |ui| {
        egui::CollapsingHeader::new("Advanced")
        .default_open(false)
        .show(ui, |ui| {
    ui.horizontal(|ui| {
        ui.label("Extra yt-dlp arguments:");
        if ui.add(egui::TextEdit::singleline(&mut s.extra_args).desired_width(320.0).font(egui::TextStyle::Monospace)).on_hover_text("Passed to yt-dlp after ytTrayDL's own arguments, so yours win on conflicts. Example: --limit-rate 10M --retries 5").changed() {
            out.changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Cookies from browser:");
        const COOKIE_BROWSERS: [(&str, &str); 8] = [
            ("", "Off"),
            ("chrome", "Chrome"),
            ("edge", "Edge"),
            ("firefox", "Firefox"),
            ("brave", "Brave"),
            ("opera", "Opera"),
            ("vivaldi", "Vivaldi"),
            ("chromium", "Chromium"),
        ];
        let current = s.cookies_from_browser.trim().to_lowercase();
        let known = COOKIE_BROWSERS.iter().any(|(v, _)| *v == current);
        let selected_label = if !known {
            "Custom…"
        } else {
            COOKIE_BROWSERS
                .iter()
                .find(|(v, _)| *v == current)
                .map(|(_, l)| *l)
                .unwrap_or("Off")
        };
        let mut picked: Option<String> = None;
        egui::ComboBox::from_id_salt("cookies-browser")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for (v, l) in COOKIE_BROWSERS {
                    if ui
                        .selectable_label(current == v, l)
                        .on_hover_text("For members-only or age-restricted videos you're signed in to. yt-dlp reads the browser's cookies at download time; ytTrayDL never sees or stores them.")
                        .clicked()
                    {
                        picked = Some(v.to_string());
                    }
                }
            });
        if let Some(v) = picked {
            s.cookies_from_browser = v;
            out.changed = true;
        }
        if !known {
            // A value outside the preset list (typed in an older
            // version or by hand): keep it editable as raw text.
            if ui.add(egui::TextEdit::singleline(&mut s.cookies_from_browser).desired_width(120.0)).on_hover_text("Custom --cookies-from-browser value").changed() {
                out.changed = true;
            }
        }
    });
        });
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
    premium_card(ui, |ui| {
        ui.label(section_heading(title));
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
    });
}

#[allow(clippy::too_many_arguments)]
fn settings_domains(
    ui: &mut egui::Ui,
    s: &mut Settings,
    out: &mut SettingsOutcome,
    rule_ignore: &mut String,
    rule_auto: &mut String,
    rule_allow: &mut String,
    rule_url: &mut String,
    rule_preset_domain: &mut String,
    rule_preset_choice: &mut Preset,
    rule_watch_url: &mut String,
    rule_watch_preset: &mut Preset,
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

    // Per-domain quality pins.
    section_card(ui, "🎚 Quality by domain", |ui| {
        ui.label(
        egui::RichText::new("Links from these domains start with the pinned quality instead of your global default — in the popup (you can still change it there) and in auto-downloads.")
            .color(TEXT_DIM)
            .small(),
    );
        let mut remove_preset: Option<String> = None;
        for dp in s.domain_presets.iter() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(dp.domain.clone()).monospace());
                ui.label(egui::RichText::new(format!("→ {}", dp.preset.label())).color(TEXT_DIM));
                if ui
                    .small_button("✕")
                    .on_hover_text("Remove this quality pin")
                    .clicked()
                {
                    remove_preset = Some(dp.domain.clone());
                }
            });
        }
        if let Some(d) = remove_preset {
            s.domain_presets.retain(|dp| dp.domain != d);
            out.changed = true;
        }
        ui.horizontal(|ui| {
            let resp = ui
                .add(
                    egui::TextEdit::singleline(rule_preset_domain)
                        .hint_text("example.com")
                        .desired_width(160.0),
                )
                .on_hover_text("Domain to pin — subdomains included, like the other rules");
            egui::ComboBox::from_id_salt("domain-preset-choice")
                .selected_text(rule_preset_choice.label())
                .show_ui(ui, |ui| {
                    for p in Preset::ALL {
                        ui.selectable_value(rule_preset_choice, p, p.label());
                    }
                });
            let add = ui
                .button("Add")
                .on_hover_text("Pin this quality for the domain")
                .clicked();
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (add || enter) && !rule_preset_domain.trim().is_empty() {
                let d = yttraydl::settings::normalize_domain(rule_preset_domain);
                if !d.is_empty() {
                    s.domain_presets.retain(|dp| dp.domain != d);
                    s.domain_presets.push(yttraydl::settings::DomainPreset {
                        domain: d,
                        preset: *rule_preset_choice,
                    });
                    s.domain_presets.sort_by(|a, b| a.domain.cmp(&b.domain));
                    out.changed = true;
                }
                rule_preset_domain.clear();
            }
        });
    });

    // Channel watcher.
    section_card(ui, "📺 Channel watcher", |ui| {
        ui.label(
        egui::RichText::new("ytTrayDL quietly checks these channels/playlists on an interval and auto-downloads new uploads with the chosen quality. The first check only takes stock — it never dumps a channel's back-catalogue. Checks happen only while ytTrayDL is running.")
            .color(TEXT_DIM)
            .small(),
    );
        let mut remove_sub: Option<String> = None;
        for sub in s.subscriptions.iter() {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(truncate(&sub.url, 44))
                        .monospace()
                        .small(),
                )
                .on_hover_text(sub.url.clone());
                ui.label(egui::RichText::new(format!("→ {}", sub.preset.label())).color(TEXT_DIM));
                if ui
                    .small_button("✕")
                    .on_hover_text("Stop watching this channel/playlist")
                    .clicked()
                {
                    remove_sub = Some(sub.url.clone());
                }
            });
        }
        if let Some(u) = remove_sub {
            s.subscriptions.retain(|x| x.url != u);
            out.changed = true;
        }
        ui.horizontal(|ui| {
            let resp = ui
                .add(
                    egui::TextEdit::singleline(rule_watch_url)
                        .hint_text("https://www.youtube.com/@channel")
                        .desired_width(220.0),
                )
                .on_hover_text("A channel page or a playlist URL to watch");
            egui::ComboBox::from_id_salt("watch-preset-choice")
                .selected_text(rule_watch_preset.label())
                .show_ui(ui, |ui| {
                    for p in Preset::ALL {
                        ui.selectable_value(rule_watch_preset, p, p.label());
                    }
                });
            let add = ui
                .button("Watch")
                .on_hover_text("Start watching this channel/playlist")
                .clicked();
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (add || enter) && !rule_watch_url.trim().is_empty() {
                let u = rule_watch_url.trim().to_string();
                if !s.subscriptions.iter().any(|x| x.url == u) {
                    s.subscriptions.push(yttraydl::settings::Subscription {
                        url: u,
                        preset: *rule_watch_preset,
                    });
                    out.changed = true;
                }
                rule_watch_url.clear();
            }
        });
        ui.horizontal(|ui| {
            ui.add_sized(egui::vec2(110.0, 24.0), egui::Label::new("Check every:"));
            let mut mins = s.watch_interval_mins;
            let chip_text = format!("{mins} min");
            let slider_w = ui.available_width()
                - value_chip_width(ui, &chip_text)
                - 86.0
                - 2.0 * ui.spacing().item_spacing.x;
            if slider_with_chip(
                ui,
                &mut mins,
                0..=720,
                &chip_text,
                slider_w,
                "How often watched channels are polled. 0 = watcher off.",
            )
            .changed()
            {
                s.watch_interval_mins = mins;
                out.changed = true;
            }
            if ui
                .button("Check now")
                .on_hover_text("Poll all watched channels right away")
                .clicked()
            {
                out.check_watch_now = true;
            }
        });
    });

    section_card(ui, "🚫 Ignored links", |ui| {
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
    });
}

/// Tool state for the yt-dlp settings tab (bundled so the
/// tab function stays inside clippy's argument limit).
pub struct ToolsUi {
    pub ffmpeg_ok: bool,
    pub ffmpeg_managed: bool,
    pub ffmpeg_fetching: bool,
    /// An update check is in flight.
    pub update_checking: bool,
    /// Result of the last update check: the latest release tag.
    pub latest: Option<Result<String, String>>,
    /// Version of the yt-dlp copy currently in use, if known.
    pub current_version: Option<String>,
}

fn settings_ytdlp(
    ui: &mut egui::Ui,
    s: &mut Settings,
    out: &mut SettingsOutcome,
    bin_info: &Option<(String, String, String)>,
    tools: ToolsUi,
    fetching: bool,
    reset_armed: &mut bool,
) {
    section_card(ui, "yt-dlp", |ui| {
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
                    egui::RichText::new(
                        "Not found. Downloads can't run until yt-dlp is installed.",
                    )
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
        ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !tools.update_checking,
                egui::Button::new(if tools.update_checking {
                    "Checking…"
                } else {
                    "Check for yt-dlp updates"
                }),
            )
            .on_hover_text("Asks GitHub for the newest yt-dlp release and compares it with the copy ytTrayDL is using. Runs only when you click — ytTrayDL never phones home on its own.")
            .clicked()
        {
            out.check_ytdlp_update = true;
        }
        match &tools.latest {
            Some(Ok(tag)) => {
                let current = tools.current_version.as_deref().unwrap_or("");
                if current == tag.as_str() {
                    ui.label(
                        egui::RichText::new("You're on the latest version.")
                            .color(TEXT_DIM)
                            .small(),
                    );
                } else if current.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("Latest release: {tag}"))
                            .color(TEXT_DIM)
                            .small(),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(format!("Newer yt-dlp available: {tag}"))
                            .color(ACCENT)
                            .small(),
                    );
                }
            }
            Some(Err(e)) => {
                ui.label(
                    egui::RichText::new(format!("Update check failed: {e}"))
                        .color(ACCENT)
                        .small(),
                );
            }
            None => {}
        }
    });
    });

    section_card(ui, "ffmpeg", |ui| {
        ui.label(if tools.ffmpeg_ok {
        "Found — merging and MP3 conversion are available."
    } else {
        "Not found — downloads that need merging (most best-quality) and MP3 conversion will fail. Install it here, put it on PATH, next to ytTrayDL, or choose it below."
    });
        ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !tools.ffmpeg_fetching,
                egui::Button::new(if tools.ffmpeg_fetching {
                    "Downloading…"
                } else if tools.ffmpeg_managed {
                    "Update ffmpeg (managed copy)"
                } else {
                    "⬇ Install ffmpeg"
                }),
            )
            .on_hover_text("Downloads the GyanD essentials build (about 110 MB) from gyan.dev into %APPDATA%\\ytTrayDL\\bin and unpacks ffmpeg.exe + ffprobe.exe there")
            .clicked()
        {
            out.install_ffmpeg = true;
        }
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
    });

    section_card(ui, "ytTrayDL data", |ui| {
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
        ui.horizontal(|ui| {
        if ui
            .button("Export settings…")
            .on_hover_text("Save all settings — including domain rules and quality pins — to a JSON file you can keep or move to another PC")
            .clicked()
        {
            out.export_settings = true;
        }
        if ui
            .button("Import settings…")
            .on_hover_text("Replace all current settings with the contents of an exported settings file")
            .clicked()
        {
            out.import_settings = true;
        }
    });
    });

    premium_card(ui, |ui| {
        ui.label(
            egui::RichText::new("Danger zone")
                .color(ACCENT)
                .strong()
                .size(15.0),
        );
        if *reset_armed {
            ui.label("Reset every setting to defaults? Domain rules, folder and quality choices will be lost. Downloaded files are not touched.");
            ui.horizontal(|ui| {
                if ui.add(primary_button("Yes, reset all settings")).clicked() {
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
    });
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
/// (id, title, url, state, file, error, preset)
type FinishedRow = (
    u64,
    String,
    String,
    JobState,
    Option<std::path::PathBuf>,
    Option<String>,
    Preset,
);

/// (id, title, url, percent)
type PausedRow = (u64, String, String, f32);

impl App {
    pub fn downloads_ui(&mut self, ctx: &egui::Context) {
        if !self.show_downloads {
            return;
        }
        let mut open = true;
        // Snapshot everything the closure needs.
        let row_of = |j: &yttraydl::download::Job| -> ActiveRow {
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
        };
        let running: Vec<ActiveRow> = self.downloads.running_jobs().map(row_of).collect();
        let queued: Vec<ActiveRow> = self.downloads.queued_jobs().map(row_of).collect();
        let paused: Vec<PausedRow> = self
            .downloads
            .paused_jobs()
            .iter()
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
                )
            })
            .collect();
        let finished: Vec<FinishedRow> = self
            .downloads
            .finished_jobs()
            .iter()
            .map(|j| {
                (
                    j.id,
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
        let any_active = !running.is_empty() || !queued.is_empty();
        let history_entries: Vec<HistoryEntry> = self.history.entries.clone();
        let bin_missing = self.bin.is_none();
        let fetching = self.ytdlp_fetch_rx.is_some();

        let mut cancel: Option<u64> = None;
        let mut pause_job: Option<u64> = None;
        let mut resume_job: Option<u64> = None;
        let mut move_job: Option<(u64, bool)> = None;
        let mut pause_all_flag = false;
        let mut resume_all_flag = false;
        let mut cancel_all_flag = false;
        let mut retry_finished: Option<u64> = None;
        let mut remove_finished: Option<u64> = None;
        let mut clear_finished_flag = false;
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
                        premium_card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(section_heading("Active"));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add_enabled(any_active || !paused.is_empty(), egui::Button::new("✕ Cancel all").small()).on_hover_text("Cancel everything queued, paused and running (partial files are kept)").clicked() {
                                    cancel_all_flag = true;
                                }
                                if ui.add_enabled(!paused.is_empty(), egui::Button::new("▶ Resume all").small()).on_hover_text("Resume every paused download").clicked() {
                                    resume_all_flag = true;
                                }
                                if ui.add_enabled(any_active, egui::Button::new("⏸ Pause all").small()).on_hover_text("Pause every queued and running download (partial files are kept)").clicked() {
                                    pause_all_flag = true;
                                }
                            });
                        });
                        if !any_active && paused.is_empty() {
                            ui.label(egui::RichText::new("Nothing downloading. Copy a supported link, or paste one in the panel.").color(TEXT_DIM));
                        }
                        for (id, title, url, pct, speed, eta, state, _err) in &running {
                            ui.horizontal(|ui| {
                                ui.label(truncate(title, 40)).on_hover_text(format!("{title}\n{url}"));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕ Cancel").on_hover_text("Cancel (partial file kept for resume)").clicked() {
                                        cancel = Some(*id);
                                    }
                                    if ui.small_button("⏸ Pause").on_hover_text("Pause (partial file kept; resume continues it)").clicked() {
                                        pause_job = Some(*id);
                                    }
                                });
                            });
                            match state {
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
                        let queued_len = queued.len();
                        for (qi, (id, title, url, _pct, _speed, _eta, _state, _err)) in queued.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(truncate(title, 40)).on_hover_text(format!("{title}\n{url}"));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕ Cancel").on_hover_text("Cancel this queued download").clicked() {
                                        cancel = Some(*id);
                                    }
                                    if ui.small_button("⏸ Pause").on_hover_text("Pause (stays out of the queue until resumed)").clicked() {
                                        pause_job = Some(*id);
                                    }
                                    if ui.add_enabled(qi + 1 < queued_len, egui::Button::new("↓").small()).on_hover_text("Move down in the queue").clicked() {
                                        move_job = Some((*id, false));
                                    }
                                    if ui.add_enabled(qi > 0, egui::Button::new("↑").small()).on_hover_text("Move up in the queue").clicked() {
                                        move_job = Some((*id, true));
                                    }
                                });
                            });
                            ui.label(egui::RichText::new(format!("Queued — #{} in queue", qi + 1)).color(TEXT_FAINT).small());
                            ui.add_space(4.0);
                        }
                        });

                        if !paused.is_empty() {
                            premium_card(ui, |ui| {
                            ui.label(section_heading("Paused"));
                            for (id, title, url, pct) in &paused {
                                ui.horizontal(|ui| {
                                    ui.label(truncate(title, 40)).on_hover_text(format!("{title}\n{url}"));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.small_button("✕ Cancel").on_hover_text("Cancel for good (partial file kept on disk)").clicked() {
                                            cancel = Some(*id);
                                        }
                                        if ui.small_button("▶ Resume").on_hover_text("Resume (continues the partial file)").clicked() {
                                            resume_job = Some(*id);
                                        }
                                    });
                                });
                                let txt = if *pct > 0.0 {
                                    format!("Paused at {pct:.0}%")
                                } else {
                                    "Paused".to_string()
                                };
                                ui.label(egui::RichText::new(txt).color(TEXT_FAINT).small());
                                ui.add_space(4.0);
                            }
                            });
                        }

                        if !finished.is_empty() {
                            premium_card(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(section_heading("Finished this session"));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("Clear finished").on_hover_text("Remove all finished entries from this list (files stay)").clicked() {
                                        clear_finished_flag = true;
                                    }
                                });
                            });
                            for row in &finished {
                                job_row(ui, row, &mut open_target, &mut retry_finished, &mut remove_finished);
                                ui.add_space(2.0);
                            }
                            });
                        }

                        premium_card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(section_heading("History"));
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
                                status_dot(ui, status_color(e.status));
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
                });
            },
        );

        self.show_downloads = open;
        if install {
            self.start_ytdlp_download();
        }
        if pause_all_flag {
            self.downloads.pause_all();
        }
        if resume_all_flag {
            self.downloads.resume_all();
            self.pump_downloads();
        }
        if cancel_all_flag {
            self.downloads.cancel_all();
        }
        if let Some((id, up)) = move_job {
            self.downloads.move_queued(id, up);
        }
        if let Some(id) = pause_job {
            self.downloads.pause(id);
        }
        if let Some(id) = resume_job {
            if self.downloads.resume(id) {
                self.pump_downloads();
            }
        }
        if let Some(id) = cancel {
            self.downloads.cancel(id);
        }
        if let Some(id) = retry_finished {
            if self.downloads.requeue_finished(id).is_some() {
                self.pump_downloads();
            }
        }
        if let Some(id) = remove_finished {
            self.downloads.remove_finished(id);
        }
        if clear_finished_flag {
            self.downloads.clear_finished();
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
        let size = egui::vec2(360.0, (toasts.len() as f32) * 52.0 + 12.0);
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
                // Each toast is its own rounded card (0.5.0)
                // with a coloured accent bar for its kind; the
                // viewport panel itself stays transparent so
                // the cards read as floating notifications.
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        for (i, t) in toasts.iter().enumerate() {
                            let color = match t.kind {
                                ToastKind::Success => OK_GREEN,
                                ToastKind::Error => ACCENT,
                                ToastKind::Info => TEXT_DIM,
                            };
                            let toast_shown = egui::Frame::NONE
                                .fill(CARD)
                                .stroke(egui::Stroke::new(1.0_f32, HAIRLINE))
                                .corner_radius(RADIUS)
                                .shadow(egui::epaint::Shadow {
                                    offset: [0, 4],
                                    blur: 12,
                                    spread: 0,
                                    color: egui::Color32::from_black_alpha(80),
                                })
                                .inner_margin(egui::Margin::symmetric(10, 8))
                                .outer_margin(egui::Margin {
                                    left: 0,
                                    right: 0,
                                    top: 0,
                                    bottom: 6,
                                })
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        // Accent bar.
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::vec2(4.0, 22.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().rect_filled(rect, 2.0, color);
                                        ui.label(truncate(&t.text, 52))
                                            .on_hover_text(t.text.clone());
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .small_button("✕")
                                                    .on_hover_text("Dismiss")
                                                    .clicked()
                                                {
                                                    dismiss = Some(i);
                                                }
                                            },
                                        );
                                    });
                                });
                            // Card top highlight (0.6.0).
                            paint_top_highlight(
                                ui.painter(),
                                toast_shown.response.rect,
                                RADIUS as f32,
                            );
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
                    ui.heading(egui::RichText::new("ytTrayDL").color(TEXT));
                    ui.label(egui::RichText::new(format!("Version {} · MIT License · by Sean Shaffer", env!("CARGO_PKG_VERSION"))).color(TEXT_DIM));
                    ui.add_space(8.0);
                    premium_card(ui, |ui| {
                        ui.label("ytTrayDL watches your clipboard for links yt-dlp can download. Copy a link and it either asks (with a small popup) or quietly downloads with your defaults — your choice, per domain if you like.");
                        ui.add_space(4.0);
                        ui.label("All downloading is done by yt-dlp; ytTrayDL is a friendly shell around it. No telemetry, no accounts, settings stay on this PC.");
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Tray icon hidden? Launch ytTrayDL again, or press Ctrl+Alt+G, and it comes back with Settings open.").color(TEXT_DIM));
                    });
                    ui.horizontal(|ui| {
                        if ui.button("yt-dlp on GitHub").on_hover_text("The download engine ytTrayDL drives").clicked() {
                            let _ = platform::open_url("https://github.com/yt-dlp/yt-dlp");
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(primary_button("Close")).on_hover_text("Close this window").clicked() {
                                close = true;
                            }
                        });
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
                    ui.label(egui::RichText::new("ytTrayDL will keep running and monitoring — there just won't be an icon.").strong().size(16.0).color(TEXT));
                    ui.add_space(8.0);
                    premium_card(ui, |ui| {
                        ui.label(egui::RichText::new("To get the icon and settings back, any of these works:").strong());
                        ui.add_space(4.0);
                        ui.label("• Launch ytTrayDL again (Start menu / the .exe) — the running copy restores its icon and opens Settings.");
                        ui.label("• Press Ctrl+Alt+G anywhere.");
                        ui.label("• Run:  yttraydl.exe --show");
                    });
                    ui.label(egui::RichText::new("Downloads and clipboard monitoring are not affected.").color(TEXT_DIM));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary_button("Hide the icon")).on_hover_text("Hide the tray icon now").clicked() {
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
            with_tray_handles(|t| {
                t._tray.set_visible(false).ok();
            });
            self.settings_dirty = true;
            self.confirm_hide_icon = false;
        } else if cancel || !open {
            self.confirm_hide_icon = false;
        }
    }

    // ---------------- Close-window choice ----------------

    /// Shown when the main window is closed and the saved
    /// CloseAction is "Ask". Offers all four outcomes plus Cancel,
    /// with a "remember my choice" checkbox; the choice can be
    /// changed later in Settings → General.
    pub fn close_choice_ui(&mut self, ctx: &egui::Context) {
        if !self.confirm_close {
            return;
        }
        let mut open = true;
        let mut remember = self.close_remember;
        let mut chosen: Option<CloseAction> = None;
        let mut cancel = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-close-choice"),
            egui::ViewportBuilder::default()
                .with_title("Close ytTrayDL?")
                .with_inner_size([470.0, 330.0])
                .with_min_inner_size([470.0, 330.0])
                .with_max_inner_size([470.0, 330.0])
                .with_always_on_top(),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "ytTrayDL keeps watching your clipboard unless you exit.",
                        )
                        .strong()
                        .size(16.0)
                        .color(TEXT),
                    );
                    ui.add_space(8.0);
                    if ui
                        .add(primary_button("Exit ytTrayDL").min_size(egui::vec2(300.0, 28.0)))
                        .on_hover_text("Quit completely — clipboard monitoring stops")
                        .clicked()
                    {
                        chosen = Some(CloseAction::Exit);
                    }
                    ui.label(
                        egui::RichText::new("Stops monitoring until you launch it again.")
                            .small()
                            .color(TEXT_DIM),
                    );
                    ui.add_space(4.0);
                    if ui
                        .add(egui::Button::new("Minimize to tray").min_size(egui::vec2(300.0, 28.0)))
                        .on_hover_text("Close the window; the tray icon stays for one-click reopening")
                        .clicked()
                    {
                        chosen = Some(CloseAction::Tray);
                    }
                    ui.label(
                        egui::RichText::new("Keeps running; reopen from the tray icon near the clock.")
                            .small()
                            .color(TEXT_DIM),
                    );
                    ui.add_space(4.0);
                    if ui
                        .add(egui::Button::new("Minimize to taskbar").min_size(egui::vec2(300.0, 28.0)))
                        .on_hover_text("Keep the window, minimized to the taskbar like a normal app")
                        .clicked()
                    {
                        chosen = Some(CloseAction::Taskbar);
                    }
                    ui.label(
                        egui::RichText::new("Keeps running; click its taskbar button to bring it back.")
                            .small()
                            .color(TEXT_DIM),
                    );
                    ui.add_space(4.0);
                    if ui
                        .add(egui::Button::new("Hide entirely — keep running in the background").min_size(egui::vec2(300.0, 28.0)))
                        .on_hover_text("No window, no tray icon. Ctrl+Alt+G or relaunching brings ytTrayDL back")
                        .clicked()
                    {
                        chosen = Some(CloseAction::Hide);
                    }
                    ui.label(
                        egui::RichText::new("No window and no tray icon. Bring it back any time with Ctrl+Alt+G, by launching ytTrayDL again, or with --show. Monitoring and downloads keep working.")
                            .small()
                            .color(TEXT_DIM),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut remember, "Remember my choice")
                            .on_hover_text("Apply this choice every time the window closes. Change it later in Settings → General.");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Cancel").on_hover_text("Keep the window open").clicked() {
                                cancel = true;
                            }
                        });
                    });
                });
            },
        );
        self.close_remember = remember;
        if cancel || !open {
            self.confirm_close = false;
            return;
        }
        if let Some(action) = chosen {
            self.confirm_close = false;
            if remember {
                self.settings.close_action = action;
                self.settings_dirty = true;
            }
            match action {
                CloseAction::Exit => self.exit_requested = true,
                CloseAction::Tray => self.begin_panel_hide(),
                CloseAction::Taskbar => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true))
                }
                CloseAction::Hide => self.hide_entirely(),
                CloseAction::Ask => {}
            }
        }
    }
}

fn job_row(
    ui: &mut egui::Ui,
    row: &FinishedRow,
    open_target: &mut Option<std::path::PathBuf>,
    retry_finished: &mut Option<u64>,
    remove_finished: &mut Option<u64>,
) {
    let (id, title, url, state, file, err, _preset) = row;
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
        if matches!(state, JobState::Failed | JobState::Cancelled)
            && ui
                .small_button("↻ Retry")
                .on_hover_text("Download again with the same quality as the original attempt")
                .clicked()
        {
            *retry_finished = Some(*id);
        }
        if ui
            .small_button("✕")
            .on_hover_text("Remove from this list (the file stays)")
            .clicked()
        {
            *remove_finished = Some(*id);
        }
    });
}

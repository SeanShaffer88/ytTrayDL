//! Bounded Settings pages. No scroll containers or hidden off-page controls.
use super::super::settings_layout;
use super::*;

pub(super) const SECTIONS: [&[&str]; 4] = [
    &["Monitoring", "Tray & Startup", "Popup", "Notifications"],
    &[
        "Files",
        "Quality",
        "Metadata",
        "Subtitles",
        "Processing",
        "Limits",
        "Automation",
        "Advanced",
    ],
    &[
        "Ignored Domains",
        "Auto-download Domains",
        "Allowed Domains",
        "Quality by Domain",
        "Channel Watcher",
        "Ignored Links",
        "Watch Schedule",
    ],
    &["yt-dlp", "ffmpeg", "Data", "Reset"],
];

pub(super) fn rows(tab: usize, section: usize, s: &Settings) -> Vec<&'static str> {
    match (tab, section) {
        (0, 0) => vec!["monitor", "action", "scope", "dedupe"],
        (0, 1) => vec!["tray", "startup", "close", "queue"],
        (0, 2) => vec!["popup-position", "popup-timeout"],
        (0, _) => vec![
            "notify-complete",
            "notify-error",
            "notify-start",
            "open-folder",
            "sound",
        ],
        (1, 0) => vec!["folder", "template"],
        (1, 1) => {
            let mut r = vec!["preset"];
            if s.default_preset == Preset::Custom {
                r.push("format");
            }
            r.push("container");
            r.push("audio-quality");
            r
        }
        (1, 2) => vec!["thumbnail", "metadata"],
        (1, 3) => vec!["subtitles", "languages"],
        (1, 4) => vec!["playlists", "sponsorblock", "chapters", "archive"],
        (1, 5) => vec!["concurrency", "rate", "retries"],
        (1, 6) => vec!["disk", "hook"],
        (1, _) => {
            let mut result = vec!["extra", "cookies"];
            if ![
                "", "chrome", "edge", "firefox", "brave", "opera", "vivaldi", "chromium",
            ]
            .contains(&s.cookies_from_browser.trim().to_lowercase().as_str())
            {
                result.push("cookie-custom");
            }
            result
        }
        (2, 6) => vec!["watch-interval", "watch-now"],
        (2, _) => vec![],
        (3, 0) => vec![
            "ytdlp-status",
            "ytdlp-install",
            "ytdlp-path",
            "ytdlp-clear",
            "ytdlp-update",
        ],
        (3, 1) => vec![
            "ffmpeg-status",
            "ffmpeg-install",
            "ffmpeg-path",
            "ffmpeg-clear",
        ],
        (3, 2) => vec!["config", "history", "export", "import"],
        _ => vec!["reset-info", "reset"],
    }
}

pub(super) fn row_height(row: &str) -> f32 {
    match row {
        "monitor" | "tray" | "startup" | "notify-complete" | "notify-error" | "notify-start"
        | "open-folder" | "sound" | "thumbnail" | "metadata" | "subtitles" | "playlists"
        | "sponsorblock" | "chapters" | "archive" | "ytdlp-install" | "ffmpeg-install"
        | "config" | "history" | "export" | "import" => 32.0,
        "reset" | "reset-info" | "ytdlp-clear" | "ffmpeg-clear" => 32.0,
        "ytdlp-path" | "ffmpeg-path" => 56.0,
        "ytdlp-status" | "ytdlp-update" | "cookies" => 76.0,
        "action" => 84.0,
        "scope" | "container" => 112.0,
        _ => 56.0,
    }
}

fn button_width(ui: &egui::Ui, label: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            label.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            TEXT,
        )
        .size()
        .x
        + 2.0 * ui.spacing().button_padding.x
        + 4.0
}
fn remaining_after_button(ui: &egui::Ui, label: &str) -> f32 {
    (ui.available_width() - button_width(ui, label) - 2.0 * ui.spacing().item_spacing.x).max(1.0)
}

fn navigation_button(ui: &mut egui::Ui, enabled: bool, label: &str, name: &str) -> egui::Response {
    let response = ui
        .push_id(name, |ui| ui.add_enabled(enabled, egui::Button::new(label)))
        .inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
    response.on_hover_text(name)
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(text).truncate())
        .on_hover_text(text);
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, tip: &str) -> bool {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()))
        .on_hover_text(tip)
        .changed()
}
fn choice<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    if !label.is_empty() {
        ui.label(label);
    }
    let mut changed = false;
    if options.len() <= 3 {
        // Short groups stay visible without an extra click.
        for &(option, text) in options {
            changed |= ui.radio_value(value, option, text).changed();
        }
    } else {
        // Large groups would exceed the compact window's bounded page. Keep
        // the current value visible and put the same radio controls in a
        // scrollable popup rather than hiding options behind cyclic arrows.
        let selected = options
            .iter()
            .find(|(option, _)| option == value)
            .map_or("Select…", |(_, text)| *text);
        egui::ComboBox::from_id_salt("choice")
            .selected_text(selected)
            .width((ui.available_width() - 16.0).max(1.0))
            .show_ui(ui, |ui| {
                for &(option, text) in options {
                    changed |= ui.radio_value(value, option, text).changed();
                }
            });
    }
    changed
}
fn number<N>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    suffix: &str,
    tip: &str,
) -> bool
where
    N: egui::emath::Numeric,
{
    ui.label(label);
    ui.add(egui::Slider::new(value, range).suffix(suffix))
        .on_hover_text(tip)
        .changed()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_row(
    ui: &mut egui::Ui,
    row: &str,
    s: &mut Settings,
    out: &mut SettingsOutcome,
    autostart: bool,
    bin: &Option<(String, String, String)>,
    tools: &ToolsUi,
    fetching: bool,
    reset: &mut bool,
) {
    let mut changed = false;
    macro_rules! check {
        ($name:ident,$label:expr,$tip:expr) => {
            changed = ui
                .checkbox(&mut s.$name, $label)
                .on_hover_text($tip)
                .changed()
        };
    }
    match row {
        "monitor" => check!(
            monitoring_enabled,
            "Monitor clipboard",
            "When off, copied links are ignored; manual downloads remain available."
        ),
        "action" => {
            changed = choice(
                ui,
                "Default action",
                &mut s.default_action,
                &[
                    (DefaultAction::Prompt, "Ask me (popup)"),
                    (DefaultAction::AutoDownload, "Auto-download"),
                ],
            )
        }
        "scope" => {
            changed = choice(
                ui,
                "Supported links",
                &mut s.detection_scope,
                &[
                    (
                        DetectionScope::KnownSitesOnly,
                        "Known video/audio sites + allow list",
                    ),
                    (DetectionScope::AllUrls, "Any web link"),
                    (DetectionScope::AllowListOnly, "Only my allow list"),
                ],
            )
        }
        "dedupe" => {
            changed = number(
                ui,
                "Ignore re-copies for",
                &mut s.dedupe_window_secs,
                0..=3600,
                " s",
                "0 = always react",
            )
        }
        "tray" => {
            let mut visible = s.tray_icon_visible;
            if ui
                .checkbox(&mut visible, "Show tray icon")
                .on_hover_text("Ctrl+Alt+G or launch again to recover the panel")
                .changed()
            {
                if visible {
                    s.tray_icon_visible = true;
                    changed = true;
                } else {
                    out.confirm_hide_icon = true;
                }
            }
        }
        "startup" => {
            let mut enabled = autostart;
            if ui.checkbox(&mut enabled, "Start with Windows").changed() {
                out.set_autostart = Some(enabled);
            }
        }
        "close" => {
            changed = choice(
                ui,
                "When I close the main window",
                &mut s.close_action,
                &CloseAction::ALL.map(|v| (v, v.label())),
            )
        }
        "queue" => {
            changed = choice(
                ui,
                "When the queue finishes",
                &mut s.queue_empty_action,
                &QueueEmptyAction::ALL.map(|v| (v, v.label())),
            )
        }
        "popup-position" => {
            changed = choice(
                ui,
                "Popup position",
                &mut s.popup_position,
                &[
                    (PopupPosition::BottomRight, "Bottom-right"),
                    (PopupPosition::TopRight, "Top-right"),
                    (PopupPosition::TopLeft, "Top-left"),
                    (PopupPosition::BottomLeft, "Bottom-left"),
                    (PopupPosition::Center, "Center of screen"),
                    (PopupPosition::NearCursor, "Near the mouse cursor"),
                ],
            )
        }
        "popup-timeout" => {
            changed = number(
                ui,
                "Popup auto-close",
                &mut s.popup_timeout_secs,
                0..=300,
                " s",
                "0 = wait until you decide. Hovering restarts the timer.",
            )
        }
        "notify-complete" => check!(
            notify_on_complete,
            "Completion toast",
            "Brief toast naming the finished file"
        ),
        "notify-error" => check!(notify_on_error, "Failure toast", "Show the download error"),
        "notify-start" => check!(
            notify_on_auto_start,
            "Auto-download toast",
            "Confirm a copied link started downloading"
        ),
        "open-folder" => check!(
            open_folder_on_finish,
            "Open finished folder",
            "Opens Explorer for each finished download"
        ),
        "sound" => check!(
            sound_on_finish,
            "Completion sound",
            "Standard Windows notification sound"
        ),
        "folder" => {
            ui.label("Download folder");
            ui.horizontal(|ui| {
                let mut text = s.download_folder.display().to_string();
                let width = remaining_after_button(ui, "Browse…");
                if ui
                    .add(egui::TextEdit::singleline(&mut text).desired_width(width))
                    .changed()
                {
                    s.download_folder = text.into();
                    changed = true;
                }
                if ui.button("Browse…").clicked() {
                    if let Some(path) = pick_folder(&s.download_folder) {
                        s.download_folder = path;
                        changed = true;
                    }
                }
            });
        }
        "template" => {
            changed = field(
                ui,
                "File name template",
                &mut s.filename_template,
                "yt-dlp output template: %(title)s [%(id)s].%(ext)s",
            )
        }
        "preset" => {
            changed = choice(
                ui,
                "Default quality",
                &mut s.default_preset,
                &Preset::ALL.map(|v| (v, v.label())),
            )
        }
        "format" => {
            changed = field(
                ui,
                "Custom format (-f)",
                &mut s.custom_format,
                "A yt-dlp format selector",
            )
        }
        "container" => {
            changed = choice(
                ui,
                "Container",
                &mut s.container,
                &[
                    (Container::Mp4, "MP4"),
                    (Container::Mkv, "MKV"),
                    (Container::Original, "Original (no remux)"),
                ],
            )
        }
        "audio-quality" => {
            changed = choice(
                ui,
                "Audio quality (MP3, M4A, Opus)",
                &mut s.audio_quality,
                &AudioQuality::ALL.map(|v| (v, v.label())),
            );
        }
        "thumbnail" => check!(
            embed_thumbnail,
            "Embed thumbnail",
            "Container must support cover art"
        ),
        "metadata" => check!(
            embed_metadata,
            "Embed metadata",
            "Writes supported metadata into the file"
        ),
        "subtitles" => check!(
            write_subtitles,
            "Download subtitles",
            "Includes automatic subtitles when needed"
        ),
        "languages" => {
            changed = field(
                ui,
                "Subtitle languages",
                &mut s.subtitle_languages,
                "yt-dlp --sub-langs pattern, e.g. en.*. Also used by Subtitles only preset.",
            )
        }
        "playlists" => check!(
            allow_playlists,
            "Whole playlists",
            "Otherwise download only the linked/first item; popup can override"
        ),
        "sponsorblock" => check!(
            sponsorblock_remove,
            "Remove sponsors",
            "SponsorBlock; needs ffmpeg"
        ),
        "chapters" => check!(split_chapters, "Separate chapter files", "Needs ffmpeg"),
        "archive" => check!(
            skip_already_downloaded,
            "Skip downloaded videos",
            "Use the app-owned yt-dlp download archive"
        ),
        "concurrency" => {
            changed = number(
                ui,
                "Simultaneous downloads",
                &mut s.max_concurrent_downloads,
                1..=4,
                "",
                "2 is a good default",
            )
        }
        "rate" => {
            changed = number(
                ui,
                "Speed limit",
                &mut s.rate_limit_kbps,
                0..=102400,
                " KB/s",
                "0 = no limit",
            )
        }
        "retries" => {
            changed = number(
                ui,
                "Auto-retry failures",
                &mut s.auto_retry_attempts,
                0..=5,
                " ×",
                "Manual cancellation is never retried",
            )
        }
        "disk" => {
            changed = number(
                ui,
                "Pause new downloads below",
                &mut s.low_disk_threshold_mb,
                0..=10240,
                " MB free",
                "0 = guard off. Running downloads finish.",
            )
        }
        "hook" => {
            changed = field(
                ui,
                "After each download, run",
                &mut s.post_download_command,
                "Executable + arguments; no shell syntax. Placeholders: {file} {dir} {url} {title}",
            )
        }
        "extra" => {
            changed = field(
                ui,
                "Extra yt-dlp arguments",
                &mut s.extra_args,
                "Passed after app arguments. Example: --limit-rate 10M --retries 5",
            )
        }
        "cookies" => {
            const OPTIONS: [(&str, &str); 8] = [
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
            let mut index = OPTIONS
                .iter()
                .position(|(value, _)| *value == current)
                .unwrap_or(8);
            let choices = [
                (0, "Off"),
                (1, "Chrome"),
                (2, "Edge"),
                (3, "Firefox"),
                (4, "Brave"),
                (5, "Opera"),
                (6, "Vivaldi"),
                (7, "Chromium"),
                (8, "Custom"),
            ];
            if choice(ui, "Cookies from browser", &mut index, &choices) {
                s.cookies_from_browser = if index < 8 {
                    OPTIONS[index].0.into()
                } else {
                    "custom".into()
                };
                changed = true;
            }
        }
        "cookie-custom" => {
            changed = field(
                ui,
                "Custom browser cookies",
                &mut s.cookies_from_browser,
                "Custom yt-dlp --cookies-from-browser value",
            )
        }
        "ytdlp-status" => {
            if let Some((path, source, version)) = bin {
                note(ui, &format!("Version {version} — {source}"));
                tool_path_row(ui, path);
            } else {
                note(ui, "yt-dlp not found. Install it to enable downloads.");
            }
        }
        "ytdlp-install" => {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !fetching,
                        egui::Button::new(if fetching {
                            "Busy…"
                        } else if bin.is_some() {
                            "Update"
                        } else {
                            "Install"
                        }),
                    )
                    .clicked()
                {
                    out.install_ytdlp = true;
                }
                if ui.button("Choose…").clicked() {
                    out.pick_ytdlp = true;
                }
                if ui.button("Web").clicked() {
                    let _ = platform::open_url("https://github.com/yt-dlp/yt-dlp/releases");
                }
            });
        }
        "ytdlp-path" => {
            note(ui, "Custom yt-dlp path (empty = automatic)");
            let full = s.ytdlp_path.display().to_string();
            tool_path_row(ui, &full);
        }
        "ytdlp-clear" => {
            let full = s.ytdlp_path.display().to_string();
            if !full.is_empty() && ui.small_button("Clear custom path").clicked() {
                s.ytdlp_path.clear();
                changed = true;
                out.relocalize = true;
            }
        }
        "ytdlp-update" => {
            if ui
                .add_enabled(
                    !tools.update_checking,
                    egui::Button::new(if tools.update_checking {
                        "Checking…"
                    } else {
                        "Check for updates"
                    }),
                )
                .clicked()
            {
                out.check_ytdlp_update = true;
            }
            if let Some(result) = &tools.latest {
                let text = match result {
                    Ok(tag) if tools.current_version.as_deref() == Some(tag) => {
                        "You're on the latest version.".into()
                    }
                    Ok(tag) => format!("Latest release: {tag}"),
                    Err(e) => format!("Update check failed: {e}"),
                };
                note(ui, &text);
            }
        }
        "ffmpeg-status" => note(
            ui,
            if tools.ffmpeg_ok {
                "ffmpeg found — merging and MP3 conversion available."
            } else {
                "ffmpeg missing — merging and MP3 conversion need it."
            },
        ),
        "ffmpeg-install" => {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !tools.ffmpeg_fetching,
                        egui::Button::new(if tools.ffmpeg_fetching {
                            "Downloading…"
                        } else if tools.ffmpeg_managed {
                            "Update"
                        } else {
                            "Install"
                        }),
                    )
                    .clicked()
                {
                    out.install_ffmpeg = true;
                }
                if ui.button("Choose…").clicked() {
                    out.pick_ffmpeg = true;
                }
            });
        }
        "ffmpeg-path" => {
            note(ui, "Custom ffmpeg path (empty = automatic)");
            let full = s.ffmpeg_path.display().to_string();
            tool_path_row(ui, &full);
        }
        "ffmpeg-clear" => {
            let full = s.ffmpeg_path.display().to_string();
            if !full.is_empty() && ui.small_button("Clear custom path").clicked() {
                s.ffmpeg_path.clear();
                changed = true;
                out.relocalize = true;
            }
        }
        "watch-interval" => {
            changed = number(
                ui,
                "Check watched channels every",
                &mut s.watch_interval_mins,
                0..=720,
                " min",
                "0 = watcher off; first check takes stock without downloading old uploads",
            )
        }
        "watch-now" => {
            if ui.button("Check watched channels now").clicked() {
                out.check_watch_now = true;
            }
        }
        "config" => {
            if ui
                .button("Open config folder")
                .on_hover_text("Settings, history and archive live here; no telemetry")
                .clicked()
            {
                out.open_config_dir = true;
            }
        }
        "history" => {
            if ui
                .button("Clear download history")
                .on_hover_text("Downloaded files and the download archive remain")
                .clicked()
            {
                out.clear_history = true;
            }
        }
        "export" => {
            if ui.button("Export settings…").clicked() {
                out.export_settings = true;
            }
        }
        "import" => {
            if ui.button("Import settings…").clicked() {
                out.import_settings = true;
            }
        }
        "reset-info" => note(
            ui,
            if *reset {
                "Reset every setting? Downloaded files remain."
            } else {
                "Reset rules, paths and preferences to defaults."
            },
        ),
        "reset" => {
            if *reset {
                ui.horizontal(|ui| {
                    if ui.button("Reset now").clicked() {
                        out.reset_all = true;
                        *reset = false;
                    }
                    if ui.button("Cancel").clicked() {
                        *reset = false;
                    }
                });
            } else if ui.button("Reset all settings…").clicked() {
                *reset = true;
            }
        }
        _ => unreachable!("unknown settings row"),
    }
    out.changed |= changed;
}

pub(super) fn collection_capacity(ui: &egui::Ui, section: usize) -> usize {
    let scale = (ui.text_style_height(&egui::TextStyle::Body) / 14.0).max(1.0);
    let editor_height = if (section == 3 || section == 4) && ui.available_height() >= 180.0 * scale
    {
        92.0
    } else {
        40.0
    };
    ((ui.available_height() - (editor_height + 34.0) * scale) / (30.0 * scale))
        .floor()
        .max(1.0) as usize
}

#[allow(clippy::too_many_arguments)]
pub(super) fn collection(
    ui: &mut egui::Ui,
    section: usize,
    page: &mut usize,
    s: &mut Settings,
    out: &mut SettingsOutcome,
    inputs: &mut [&mut String; 6],
    preset: &mut Preset,
    watch_preset: &mut Preset,
) {
    let count = match section {
        0 => s.ignored_domains.len(),
        1 => s.auto_domains.len(),
        2 => s.allowed_domains.len(),
        3 => s.domain_presets.len(),
        4 => s.subscriptions.len(),
        _ => s.ignored_urls.len(),
    };
    let capacity = collection_capacity(ui, section);
    let range = settings_layout::collection_range(page, count, capacity);
    let scale = (ui.text_style_height(&egui::TextStyle::Body) / 14.0).max(1.0);
    let constrained = (section == 3 || section == 4) && ui.available_height() < 180.0 * scale;
    let edit_key = ui.id().with("collection-editor-mode");
    let editing = constrained
        && ui
            .ctx()
            .data(|data| data.get_temp::<bool>(edit_key))
            .unwrap_or(false);
    if !editing {
        let mut remove = None;
        for index in range.clone() {
            let (key, display) = match section {
                0 => {
                    let v = &s.ignored_domains[index];
                    (v.clone(), v.clone())
                }
                1 => {
                    let v = &s.auto_domains[index];
                    (v.clone(), v.clone())
                }
                2 => {
                    let v = &s.allowed_domains[index];
                    (v.clone(), v.clone())
                }
                3 => {
                    let v = &s.domain_presets[index];
                    (
                        v.domain.clone(),
                        format!("{} → {}", v.domain, v.preset.label()),
                    )
                }
                4 => {
                    let v = &s.subscriptions[index];
                    (v.url.clone(), format!("{} → {}", v.url, v.preset.label()))
                }
                _ => {
                    let v = &s.ignored_urls[index];
                    (v.clone(), v.clone())
                }
            };
            ui.push_id(("collection-item", section, &key), |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [remaining_after_button(ui, "Remove"), 24.0],
                        egui::Label::new(&display).truncate(),
                    )
                    .on_hover_text(&display);
                    if ui.small_button("Remove").clicked() {
                        remove = Some(key.clone());
                    }
                });
            });
        }
        if count == 0 {
            ui.label("No entries yet.");
        }
        ui.horizontal(|ui| {
            if navigation_button(ui, *page > 0, "◀", "Previous Entries").clicked() {
                *page -= 1;
            }
            ui.label(format!(
                "{}–{} of {count}",
                if count == 0 { 0 } else { range.start + 1 },
                range.end
            ));
            if navigation_button(ui, range.end < count, "▶", "Next Entries").clicked() {
                *page += 1;
            }
        });
        if let Some(key) = remove {
            match section {
                0 => s.ignored_domains.retain(|v| v != &key),
                1 => s.auto_domains.retain(|v| v != &key),
                2 => s.allowed_domains.retain(|v| v != &key),
                3 => s.domain_presets.retain(|v| v.domain != key),
                4 => s.subscriptions.retain(|v| v.url != key),
                _ => s.ignored_urls.retain(|v| v != &key),
            }
            out.changed = true;
            let remaining = match section {
                0 => s.ignored_domains.len(),
                1 => s.auto_domains.len(),
                2 => s.allowed_domains.len(),
                3 => s.domain_presets.len(),
                4 => s.subscriptions.len(),
                _ => s.ignored_urls.len(),
            };
            settings_layout::collection_range(page, remaining, capacity);
        }
        if constrained {
            if ui.button("Add entry…").clicked() {
                ui.ctx().data_mut(|data| data.insert_temp(edit_key, true));
            }
            return;
        }
    }
    if section == 3 {
        choice(
            ui,
            if constrained { "" } else { "Pinned quality" },
            preset,
            &Preset::ALL.map(|v| (v, v.label())),
        );
    }
    if section == 4 {
        choice(
            ui,
            if constrained {
                ""
            } else {
                "Watched channel quality"
            },
            watch_preset,
            &Preset::ALL.map(|v| (v, v.label())),
        );
    }
    let mut submitted = false;
    let input = &mut inputs[section];
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(*input)
                .id(egui::Id::new(("yttraydl-settings-rule-input", section)))
                .hint_text(if section >= 4 {
                    "https://…"
                } else {
                    "example.com"
                })
                .desired_width(remaining_after_button(
                    ui,
                    if section == 4 { "Watch" } else { "Add" },
                )),
        );
        let add = ui
            .button(if section == 4 { "Watch" } else { "Add" })
            .clicked()
            || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if add && !input.trim().is_empty() {
            let key = if section < 4 {
                yttraydl::settings::normalize_domain(input)
            } else {
                input.trim().to_string()
            };
            if !key.is_empty() {
                let index = match section {
                    0..=2 => {
                        let list = match section {
                            0 => &mut s.ignored_domains,
                            1 => &mut s.auto_domains,
                            _ => &mut s.allowed_domains,
                        };
                        if !list.contains(&key) {
                            list.push(key.clone());
                            list.sort();
                            out.changed = true;
                        }
                        list.iter().position(|v| v == &key).unwrap()
                    }
                    3 => {
                        s.domain_presets.retain(|v| v.domain != key);
                        s.domain_presets.push(yttraydl::settings::DomainPreset {
                            domain: key.clone(),
                            preset: *preset,
                        });
                        s.domain_presets.sort_by(|a, b| a.domain.cmp(&b.domain));
                        out.changed = true;
                        s.domain_presets
                            .iter()
                            .position(|v| v.domain == key)
                            .unwrap()
                    }
                    4 => {
                        if !s.subscriptions.iter().any(|v| v.url == key) {
                            s.subscriptions.push(yttraydl::settings::Subscription {
                                url: key.clone(),
                                preset: *watch_preset,
                            });
                            out.changed = true;
                        }
                        s.subscriptions.iter().position(|v| v.url == key).unwrap()
                    }
                    _ => {
                        if !s.ignored_urls.contains(&key) {
                            s.ignored_urls.push(key.clone());
                            out.changed = true;
                        }
                        s.ignored_urls.iter().position(|v| v == &key).unwrap()
                    }
                };
                *page = index / capacity;
            }
            submitted = true;
            input.clear();
        }
    });
    if constrained && (submitted || ui.button("Back to entries").clicked()) {
        ui.ctx().data_mut(|data| data.insert_temp(edit_key, false));
    }
}

pub(super) fn flat_rows(tab: usize, s: &Settings) -> Vec<(usize, &'static str)> {
    SECTIONS[tab]
        .iter()
        .enumerate()
        .flat_map(|(section, _)| {
            rows(tab, section, s)
                .into_iter()
                .map(move |row| (section, row))
        })
        .collect()
}

fn compact_progress(ui: &mut egui::Ui, operation: &mut ToolOperation) {
    ui.push_id(
        ("settings-operation", operation.progress.operation_id),
        |ui| {
            let p = &operation.progress;
            let name = match p.family {
                yttraydl::tools::ToolFamily::YtDlp => "yt-dlp",
                yttraydl::tools::ToolFamily::FFmpeg => "ffmpeg",
            };
            ui.horizontal(|ui| {
                let text = format!(
                    "{name}: {}",
                    if operation.cancelling {
                        "Cancelling…"
                    } else {
                        p.stage.label()
                    }
                );
                ui.add_sized(
                    [
                        remaining_after_button(ui, "Cancel"),
                        ui.spacing().interact_size.y,
                    ],
                    egui::Label::new(&text).truncate(),
                )
                .on_hover_text(&text);
                if ui
                    .add_enabled(!operation.cancelling, egui::Button::new("Cancel"))
                    .clicked()
                {
                    operation.cancel.cancel();
                    operation.cancelling = true;
                }
            });
            if p.stage == yttraydl::tools::ToolStage::Download {
                let pct = p
                    .total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| format!("{:.0}%", 100.0 * p.bytes_done as f64 / total as f64))
                    .unwrap_or_else(|| "?%".into());
                let text = format!(
                    "{pct} {}{} {} {}",
                    ytdlp::format_bytes(p.bytes_done).replace(' ', ""),
                    p.total_bytes
                        .map(|v| format!("/{}", ytdlp::format_bytes(v).replace(' ', "")))
                        .unwrap_or_else(|| "/?".into()),
                    p.bytes_per_second
                        .map(|v| format!(
                            "{}/s",
                            ytdlp::format_bytes(v.max(0.0) as u64).replace(' ', "")
                        ))
                        .unwrap_or_else(|| "?/s".into()),
                    p.eta_seconds
                        .map(|v| format!("~{v}s"))
                        .unwrap_or_else(|| "ETA ?".into())
                );
                ui.label(text);
            }
        },
    );
}

/// Actual Settings scaffold, shared by the native viewport and headless bounds tests.
#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    ui: &mut egui::Ui,
    tab: &mut usize,
    sections: &mut [usize; 4],
    anchors: &mut [usize; 4],
    collection_pages: &mut [usize; 6],
    s: &mut Settings,
    out: &mut SettingsOutcome,
    autostart: bool,
    bin: &Option<(String, String, String)>,
    tools: &ToolsUi,
    fetching: bool,
    reset: &mut bool,
    inputs: &mut [&mut String; 6],
    preset: &mut Preset,
    watch_preset: &mut Preset,
    operations: [Option<&mut ToolOperation>; 2],
) -> bool {
    let mut close = false;
    let compact = ui.available_height() < 500.0;
    if !compact {
        ui.push_id("settings-header", |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Settings").strong())
                    .on_hover_text("Settings save automatically");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = ui.button("Close").clicked();
                });
            });
        });
    }
    ui.push_id("settings-tabs", |ui| {
        let share = (ui.available_width() - 3.0 * ui.spacing().item_spacing.x) / 4.0;
        ui.horizontal(|ui| {
            for (index, label) in ["General", "Downloads", "Rules", "Tools"]
                .iter()
                .enumerate()
            {
                if ui
                    .add_sized(
                        [share, ui.spacing().interact_size.y],
                        egui::Button::new(*label).selected(*tab == index).truncate(),
                    )
                    .on_hover_text(*label)
                    .clicked()
                {
                    *tab = index;
                }
            }
        });
    });
    for operation in operations.into_iter().flatten() {
        compact_progress(ui, operation);
    }
    let labels = SECTIONS[*tab];
    sections[*tab] = sections[*tab].min(labels.len() - 1);
    let old_section = sections[*tab];
    if !compact || *tab == 2 {
        ui.push_id("settings-section-navigation", |ui| {
            ui.horizontal(|ui| {
                if navigation_button(ui, sections[*tab] > 0, "◀", "Previous Section")
                    .on_hover_text("Previous section")
                    .clicked()
                {
                    sections[*tab] -= 1;
                }
                ui.add_sized(
                    [remaining_after_button(ui, "▶"), 24.0],
                    egui::Label::new(format!(
                        "{} ({}/{})",
                        labels[sections[*tab]],
                        sections[*tab] + 1,
                        labels.len()
                    ))
                    .truncate(),
                );
                if navigation_button(ui, sections[*tab] + 1 < labels.len(), "▶", "Next Section")
                    .on_hover_text("Next section")
                    .clicked()
                {
                    sections[*tab] += 1;
                }
            });
        });
    }
    let all = if *tab == 2 {
        rows(*tab, sections[*tab], s)
            .into_iter()
            .map(|row| (sections[*tab], row))
            .collect()
    } else {
        flat_rows(*tab, s)
    };
    if old_section != sections[*tab] {
        anchors[*tab] = all
            .iter()
            .position(|(section, _)| *section == sections[*tab])
            .unwrap_or(0);
    }
    let scale = (ui.text_style_height(&egui::TextStyle::Body) / 14.0).max(1.0);
    let footer_height = ui.spacing().interact_size.y.max(26.0) + 6.0;
    let body_height = (ui.available_height() - footer_height).max(1.0);
    let focus_cache = ui.id().with(("settings-focus-rows", *tab));
    if old_section == sections[*tab] {
        if let Some(focused) = ui
            .memory(|memory| memory.focused())
            .and_then(|id| ui.ctx().read_response(id))
        {
            if let Some(previous) = ui
                .ctx()
                .data(|data| data.get_temp::<Vec<((usize, &'static str), egui::Rect)>>(focus_cache))
            {
                if let Some((key, _)) = previous
                    .iter()
                    .find(|(_, rect)| rect.contains(focused.rect.center()))
                {
                    if let Some(index) = all.iter().position(|candidate| candidate == key) {
                        anchors[*tab] = index;
                    }
                }
            }
        }
    }
    let mut rendered_rows: Vec<((usize, &'static str), egui::Rect)> = Vec::new();
    let heights: Vec<_> = all
        .iter()
        .map(|(_, row)| {
            row_height(row) * scale + 6.0 + ui.text_style_height(&egui::TextStyle::Body) + 4.0
        })
        .collect();
    let plan = settings_layout::pages(&heights, body_height);
    anchors[*tab] = anchors[*tab].min(all.len().saturating_sub(1));
    let page = plan
        .iter()
        .position(|range| range.contains(&anchors[*tab]))
        .unwrap_or(0);
    // Keep the focused/first-visible atomic row as the anchor as capacity changes.
    if let Some((section, _)) = all.get(anchors[*tab]) {
        sections[*tab] = *section;
    }
    ui.push_id("settings-body", |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), body_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                if *tab == 2 && sections[*tab] < 6 {
                    ui.scope_builder(
                        egui::UiBuilder::new().id(("yttraydl-settings-collection", sections[*tab])),
                        |ui| {
                            collection(
                                ui,
                                sections[*tab],
                                &mut collection_pages[sections[*tab]],
                                s,
                                out,
                                inputs,
                                preset,
                                watch_preset,
                            )
                        },
                    );
                } else {
                    for index in plan[page].clone() {
                        let (section, row) = all[index];
                        if index == plan[page].start || all[index - 1].0 != section {
                            ui.label(
                                egui::RichText::new(labels[section])
                                    .strong()
                                    .color(TEXT_DIM),
                            );
                        }
                        let rendered = ui.scope_builder(
                            egui::UiBuilder::new().id((
                                "yttraydl-settings-control",
                                *tab,
                                section,
                                row,
                            )),
                            |ui| {
                                render_row(ui, row, s, out, autostart, bin, tools, fetching, reset)
                            },
                        );
                        rendered_rows.push(((section, row), rendered.response.rect));
                        ui.add_space(6.0);
                    }
                }
            },
        );
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(focus_cache, rendered_rows));
    ui.push_id("settings-page-navigation", |ui| {
        ui.horizontal(|ui| {
            if navigation_button(ui, page > 0, "◀", "Previous Page").clicked() {
                anchors[*tab] = plan[page - 1].start;
            }
            ui.label(format!("Page {} of {}", page + 1, plan.len()));
            if navigation_button(ui, page + 1 < plan.len(), "▶", "Next Page").clicked() {
                anchors[*tab] = plan[page + 1].start;
            }
        });
    });
    close
}

#[cfg(test)]
#[path = "settings_pages_tests.rs"]
mod tests;

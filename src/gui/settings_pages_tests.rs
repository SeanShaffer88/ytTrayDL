//! Actual egui Settings controls: bounds are measured after widget layout.
use super::*;

fn context(scale: f32) -> egui::Context {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    ctx.style_mut(|style| {
        for font in style.text_styles.values_mut() {
            font.size *= scale;
        }
    });
    ctx
}

fn settings_fixture() -> Settings {
    Settings {
        monitoring_enabled: false,
        tray_icon_visible: false,
        start_with_windows: true,
        default_action: DefaultAction::AutoDownload,
        detection_scope: DetectionScope::AllUrls,
        close_action: CloseAction::ALL[1],
        animate_settings_drawer: false,
        dedupe_window_secs: 211,
        skip_already_downloaded: true,
        download_folder: "C:/fixture/destination with spaces".into(),
        container: Container::Mkv,
        default_preset: Preset::Custom,
        embed_thumbnail: true,
        embed_metadata: true,
        write_subtitles: true,
        subtitle_languages: "en.*,de.*,ja.*".into(),
        allow_playlists: true,
        max_concurrent_downloads: 3,
        rate_limit_kbps: 2345,
        auto_retry_attempts: 4,
        sponsorblock_remove: true,
        split_chapters: true,
        queue_empty_action: QueueEmptyAction::ALL[1],
        low_disk_threshold_mb: 5678,
        post_download_command: "C:/fixture/post-download.exe {file}".into(),
        watch_interval_mins: 31,
        notify_on_complete: false,
        sound_on_finish: true,
        open_folder_on_finish: true,
        notify_on_error: false,
        notify_on_auto_start: true,
        popup_position: PopupPosition::NearCursor,
        popup_timeout_secs: 211,
        custom_format: "bestvideo[height<=2160]+bestaudio/best".repeat(8),
        filename_template: "%(title)s [%(id)s].%(ext)s".repeat(8),
        extra_args: "--match-filter duration>30 --write-description".repeat(8),
        cookies_from_browser: "firefox:very-long-private-profile-name".repeat(8),
        ytdlp_path: std::path::PathBuf::from("C:/fixture/long tool directory/".repeat(12)),
        ffmpeg_path: std::path::PathBuf::from("C:/fixture/long media directory/".repeat(12)),
        ..Settings::default()
    }
}

fn tools_fixture() -> ToolsUi {
    ToolsUi {
        ffmpeg_ok: false,
        ffmpeg_managed: true,
        ffmpeg_fetching: false,
        update_checking: false,
        latest: Some(Err(
            "A deliberately long actionable update error. ".repeat(12)
        )),
        current_version: Some("2026.11.22".into()),
    }
}

fn frame(ctx: &egui::Context, size: egui::Vec2, mut render: impl FnMut(&mut egui::Ui)) {
    // First pass initializes fonts; second pass has the same settled metrics as
    // the native app. No platform calls, process launch, or persistence occurs.
    for _ in 0..2 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| render(ui));
            },
        );
    }
}

#[test]
fn every_settings_row_renders_within_its_allocated_height_and_width() {
    let mut failures = Vec::new();
    for width in [600.0, 480.0, 400.0] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let ctx = context(scale);
            for (tab, sections) in SECTIONS.iter().enumerate() {
                for section in 0..sections.len() {
                    let mut settings = settings_fixture();
                    for row in rows(tab, section, &settings) {
                        let mut actual = egui::Rect::NOTHING;
                        let mut available = egui::Rect::NOTHING;
                        let mut reserved = 0.0;
                        frame(&ctx, egui::vec2(width, 400.0), |ui| {
                            available = ui.available_rect_before_wrap();
                            reserved = row_height(row)
                                * (ui.text_style_height(&egui::TextStyle::Body) / 14.0).max(1.0);
                            actual = ui
                                .scope(|ui| {
                                    render_row(
                                        ui,
                                        row,
                                        &mut settings,
                                        &mut SettingsOutcome::default(),
                                        false,
                                        &None,
                                        &tools_fixture(),
                                        false,
                                        &mut false,
                                    );
                                })
                                .response
                                .rect;
                        });
                        if actual.right() > available.right() + 0.5
                            || actual.height() > reserved + 0.5
                        {
                            failures.push(format!("{width}px text={scale} tab={tab} section={section} row={row}: actual={actual:?}, reserved height={}, right={}", row_height(row), available.right()));
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} overflowing actual rows:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn operation(id: u64) -> ToolOperation {
    ToolOperation {
        id,
        mailbox: yttraydl::tools::ProgressMailbox::default(),
        progress: yttraydl::tools::ToolProgress {
            operation_id: id,
            family: if id == 1 {
                yttraydl::tools::ToolFamily::YtDlp
            } else {
                yttraydl::tools::ToolFamily::FFmpeg
            },
            stage: yttraydl::tools::ToolStage::Download,
            bytes_done: 12_345_678,
            total_bytes: Some(120_000_000),
            bytes_per_second: Some(1_200_000.0),
            eta_seconds: Some(90),
        },
        cancel: yttraydl::process::CancellationToken::default(),
        cancelling: false,
    }
}

#[test]
fn required_settings_matrix_fits_with_separately_reported_extra_stress() {
    let mut failures = Vec::new();
    let mut extra_stress_failures = Vec::new();
    let mut required_cases = 0usize;
    let mut extra_stress_cases = 0usize;
    for size in [
        egui::vec2(600.0, 640.0),
        egui::vec2(480.0, 420.0),
        egui::vec2(400.0, 350.0),
    ] {
        for (scale, dpi) in [
            (1.0, 1.0),
            (1.25, 1.0),
            (1.5, 1.0),
            (2.0, 1.0),
            (1.0, 1.25),
            (1.0, 1.5),
            (1.0, 2.0),
        ] {
            let ctx = context(scale);
            ctx.set_pixels_per_point(dpi);
            // Approved release matrix distinguishes enlarged application text
            // from OS DPI. Extra combinations remain executed and reported.
            let mandatory = size.y >= 600.0 || (size.x >= 480.0 && scale <= 1.5) || scale <= 1.0;
            for progress_count in 0..=2 {
                for initial_tab in 0..4 {
                    let seed = settings_fixture();
                    let anchors_to_visit = if initial_tab == 2 {
                        7
                    } else {
                        flat_rows(initial_tab, &seed).len()
                    };
                    for requested_anchor in 0..anchors_to_visit {
                        if mandatory {
                            required_cases += 1;
                        } else {
                            extra_stress_cases += 1;
                        }
                        let mut settings = seed.clone();
                        let serialized_before = serde_json::to_value(&settings).unwrap();
                        let mut tab = initial_tab;
                        let mut sections = [0; 4];
                        let mut anchors = [0; 4];
                        if tab == 2 {
                            sections[tab] = requested_anchor;
                        } else {
                            anchors[tab] = requested_anchor;
                        }
                        let mut collection_pages = [0; 6];
                        let mut drafts: [String; 6] =
                            std::array::from_fn(|_| "long-unsubmitted-draft.example".repeat(8));
                        let drafts_before = drafts.clone();
                        let mut first = operation(1);
                        let mut second = operation(2);
                        let mut bounds = egui::Rect::NOTHING;
                        let mut available = egui::Rect::NOTHING;
                        frame(&ctx, size, |ui| {
                            available = ui.available_rect_before_wrap();
                            let [a, b, c, d, e, f] = &mut drafts;
                            bounds = ui
                                .scope(|ui| {
                                    render(
                                        ui,
                                        &mut tab,
                                        &mut sections,
                                        &mut anchors,
                                        &mut collection_pages,
                                        &mut settings,
                                        &mut SettingsOutcome::default(),
                                        false,
                                        &None,
                                        &tools_fixture(),
                                        false,
                                        &mut false,
                                        &mut [a, b, c, d, e, f],
                                        &mut Preset::Custom,
                                        &mut Preset::Custom,
                                        [
                                            if progress_count > 0 {
                                                Some(&mut first)
                                            } else {
                                                None
                                            },
                                            if progress_count > 1 {
                                                Some(&mut second)
                                            } else {
                                                None
                                            },
                                        ],
                                    );
                                })
                                .response
                                .rect;
                        });
                        assert_eq!(serde_json::to_value(&settings).unwrap(), serialized_before,
                            "Rendering changed settings: tab={initial_tab}, anchor={requested_anchor}");
                        assert_eq!(
                            drafts, drafts_before,
                            "Rendering changed unsubmitted drafts"
                        );
                        if !available.expand(0.5).contains_rect(bounds) {
                            let failure = format!("{size:?} text={scale} ppp={dpi} progress={progress_count} tab={initial_tab} anchor={requested_anchor}: {bounds:?} outside {available:?}");
                            if mandatory {
                                failures.push(failure);
                            } else {
                                extra_stress_failures.push(failure);
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "SETTINGS_EXTRA_STRESS {}",
        serde_json::to_string(&extra_stress_failures).unwrap()
    );
    eprintln!(
        "SETTINGS_MATRIX_SUMMARY {}",
        serde_json::json!({
            "requiredCases": required_cases, "requiredFailures": failures.len(),
            "extraStressCases": extra_stress_cases, "extraStressFailures": extra_stress_failures.len(),
            "osPixelsPerPoint": [1.0,1.25,1.5,2.0], "textScales": [1.0,1.25,1.5,2.0],
            "scope": "actual headless egui rendering; extra stress failures are observations, not supported release-matrix passes"
        })
    );
    assert!(
        failures.is_empty(),
        "{} overflowing actual Settings pages:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn populate_collection(settings: &mut Settings, section: usize, count: usize) {
    let names: Vec<_> = (0..count)
        .map(|i| {
            format!(
                "entry-{i:03}.{}.example",
                "long-domain-component-".repeat(8)
            )
        })
        .collect();
    match section {
        0 => settings.ignored_domains = names,
        1 => settings.auto_domains = names,
        2 => settings.allowed_domains = names,
        3 => {
            settings.domain_presets = names
                .into_iter()
                .map(|domain| yttraydl::settings::DomainPreset {
                    domain,
                    preset: Preset::Custom,
                })
                .collect()
        }
        4 => {
            settings.subscriptions = names
                .into_iter()
                .map(|name| yttraydl::settings::Subscription {
                    url: format!("https://{name}/channel"),
                    preset: Preset::Custom,
                })
                .collect()
        }
        _ => {
            settings.ignored_urls = names
                .into_iter()
                .map(|name| format!("https://{name}/video"))
                .collect()
        }
    }
}

#[test]
fn six_actual_collection_editors_fit_and_visit_every_item_at_capacity_boundaries() {
    for size in [
        egui::vec2(576.0, 380.0),
        egui::vec2(456.0, 210.0),
        egui::vec2(376.0, 200.0),
    ] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let ctx = context(scale);
            for section in 0..6 {
                let mut capacity = 0;
                frame(&ctx, size, |ui| capacity = collection_capacity(ui, section));
                for count in [0, 1, capacity, capacity + 1, 100] {
                    let mut seen = Vec::new();
                    for requested_page in 0..count.max(1).div_ceil(capacity) {
                        let mut settings = settings_fixture();
                        populate_collection(&mut settings, section, count);
                        let mut page = requested_page;
                        let mut drafts: [String; 6] =
                            std::array::from_fn(|_| "unsubmitted-long-draft".repeat(8));
                        let mut actual = egui::Rect::NOTHING;
                        let mut available = egui::Rect::NOTHING;
                        frame(&ctx, size, |ui| {
                            available = ui.available_rect_before_wrap();
                            let [a, b, c, d, e, f] = &mut drafts;
                            actual = ui
                                .scope(|ui| {
                                    collection(
                                        ui,
                                        section,
                                        &mut page,
                                        &mut settings,
                                        &mut SettingsOutcome::default(),
                                        &mut [a, b, c, d, e, f],
                                        &mut Preset::Custom,
                                        &mut Preset::Custom,
                                    )
                                })
                                .response
                                .rect;
                        });
                        let mandatory =
                            size.y >= 300.0 || (size.x >= 450.0 && scale <= 1.5) || scale <= 1.0;
                        if mandatory {
                            assert!(available.expand(0.5).contains_rect(actual), "collection={section} count={count} capacity={capacity} page={requested_page} text={scale} actual={actual:?} available={available:?}");
                        } else if !available.expand(0.5).contains_rect(actual)
                            && requested_page == 0
                        {
                            eprintln!("COLLECTION_EXTRA_STRESS section={section} count={count} text={scale} actual={actual:?} available={available:?}");
                        }
                        seen.extend(settings_layout::collection_range(
                            &mut page, count, capacity,
                        ));
                    }
                    assert_eq!(seen, (0..count).collect::<Vec<_>>());
                }
            }
        }
    }
}

#[test]
fn conditional_quality_controls_remain_present_and_pagination_is_lossless() {
    let mut settings = settings_fixture();
    let custom = rows(1, 1, &settings);
    assert!(custom.contains(&"format"));
    settings.default_preset = Preset::ALL
        .iter()
        .copied()
        .find(|p| *p != Preset::Custom)
        .unwrap();
    assert!(!rows(1, 1, &settings).contains(&"format"));
    for (tab, sections) in SECTIONS.iter().enumerate() {
        for section in 0..sections.len() {
            let controls = rows(tab, section, &settings);
            let heights = controls
                .iter()
                .map(|row| row_height(row))
                .collect::<Vec<_>>();
            for budget in [100.0, 160.0, 260.0, 500.0] {
                let pages = settings_layout::pages(&heights, budget);
                assert_eq!(
                    pages.iter().flat_map(|p| p.clone()).collect::<Vec<_>>(),
                    (0..controls.len()).collect::<Vec<_>>()
                );
                assert!(pages
                    .iter()
                    .all(|p| heights[p.clone()].iter().sum::<f32>() <= budget));
            }
        }
    }
}

struct CollectionHarness {
    ctx: egui::Context,
    settings: Settings,
    section: usize,
    page: usize,
    capacity: usize,
    drafts: [String; 6],
    buttons: Vec<(String, egui::Rect)>,
    size: egui::Vec2,
    content_bounds: egui::Rect,
}

impl CollectionHarness {
    fn new(section: usize) -> Self {
        Self::with_layout(section, egui::vec2(600.0, 420.0), 1.0)
    }

    fn with_layout(section: usize, size: egui::Vec2, scale: f32) -> Self {
        let ctx = context(scale);
        ctx.enable_accesskit();
        let mut fixture = Self {
            ctx,
            settings: settings_fixture(),
            section,
            page: 0,
            capacity: 0,
            drafts: std::array::from_fn(|index| format!("preserved-draft-{index}")),
            buttons: Vec::new(),
            size,
            content_bounds: egui::Rect::NOTHING,
        };
        fixture.draw(Vec::new());
        populate_collection(&mut fixture.settings, section, fixture.capacity + 1);
        fixture.draw(Vec::new());
        fixture
    }

    fn draw(&mut self, events: Vec<egui::Event>) {
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    self.capacity = collection_capacity(ui, self.section);
                    let [a, b, c, d, e, f] = &mut self.drafts;
                    self.content_bounds = ui
                        .scope(|ui| {
                            collection(
                                ui,
                                self.section,
                                &mut self.page,
                                &mut self.settings,
                                &mut SettingsOutcome::default(),
                                &mut [a, b, c, d, e, f],
                                &mut Preset::Custom,
                                &mut Preset::Custom,
                            )
                        })
                        .response
                        .rect;
                });
            },
        );
        self.buttons = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, node)| {
                Some((node.label()?.to_string(), {
                    let r = node.bounds()?;
                    egui::Rect::from_min_max(
                        egui::pos2(r.x0 as f32, r.y0 as f32),
                        egui::pos2(r.x1 as f32, r.y1 as f32),
                    )
                }))
            })
            .collect();
    }

    fn click(&mut self, label: &str) {
        let target = self
            .buttons
            .iter()
            .find(|(name, _)| name == label)
            .unwrap_or_else(|| panic!("Missing actual button {label}: {:?}", self.buttons));
        let pos = target.1.center();
        self.draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        self.draw(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        self.draw(Vec::new());
    }
}

#[test]
fn constrained_collection_editor_pages_are_reachable_and_preserve_drafts() {
    for section in [3, 4] {
        for (size, scale) in [
            (egui::vec2(464.0, 180.0), 1.5),
            (egui::vec2(384.0, 145.0), 1.0),
        ] {
            let mut fixture = CollectionHarness::with_layout(section, size, scale);
            let before = fixture.drafts.clone();
            fixture.click("Add entry…");
            assert!(fixture
                .buttons
                .iter()
                .any(|(name, _)| name == "Back to entries"));
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                    .contains_rect(fixture.content_bounds),
                "Constrained editor section={section} scale={scale}: {:?} outside {size:?}",
                fixture.content_bounds
            );
            fixture.click("Back to entries");
            assert_eq!(fixture.drafts, before);
            fixture.click("Add entry…");
            fixture.drafts[section] = if section == 4 {
                "https://zzzz-final.example/channel"
            } else {
                "zzzz-final.example"
            }
            .into();
            fixture.draw(Vec::new());
            fixture.click(if section == 4 { "Watch" } else { "Add" });
            assert!(fixture.drafts[section].is_empty());
            assert!(fixture.buttons.iter().any(|(name, _)| name == "Add entry…"));
            for (index, draft) in fixture.drafts.iter().enumerate() {
                if index != section {
                    assert_eq!(draft, &before[index]);
                }
            }
        }
    }
}

#[test]
fn actual_collection_buttons_page_delete_last_and_add_without_losing_other_drafts() {
    for section in 0..6 {
        let mut fixture = CollectionHarness::new(section);
        let before = fixture.drafts.clone();
        let settings_before = serde_json::to_value(&fixture.settings).unwrap();
        fixture.click("Next Entries");
        assert_eq!(fixture.page, 1, "Next collection={section}");
        fixture.click("Previous Entries");
        assert_eq!(fixture.page, 0, "Previous collection={section}");
        assert_eq!(
            serde_json::to_value(&fixture.settings).unwrap(),
            settings_before
        );
        assert_eq!(fixture.drafts, before);
        fixture.click("Next Entries");
        fixture.click("Remove");
        assert_eq!(
            fixture.page, 0,
            "Removing the only item on the last page must clamp collection={section}"
        );
        fixture.drafts[section] = if section >= 4 {
            "https://zzzz-final.example/channel"
        } else {
            "zzzz-final.example"
        }
        .into();
        fixture.draw(Vec::new());
        fixture.click(if section == 4 { "Watch" } else { "Add" });
        assert_eq!(
            fixture.page, 1,
            "Add must select its actual final page collection={section}"
        );
        assert!(fixture.drafts[section].is_empty());
        for (index, draft) in fixture.drafts.iter().enumerate() {
            if index != section {
                assert_eq!(draft, &before[index]);
            }
        }
    }
}

#[test]
fn focused_editable_row_stays_visible_when_resize_and_progress_repack_the_page() {
    let ctx = context(1.0);
    ctx.enable_accesskit();
    let mut settings = settings_fixture();
    settings.subtitle_languages = "focused-language-marker".into();
    settings.default_preset = Preset::Best;
    let show_custom = std::cell::Cell::new(false);
    let mut tab = 1;
    let mut sections = [0; 4];
    let initial_anchor = flat_rows(1, &settings)
        .iter()
        .position(|(_, row)| *row == "languages")
        .unwrap();
    let mut anchors = [0, initial_anchor, 0, 0];
    let mut collection_pages = [0; 6];
    let mut drafts: [String; 6] = Default::default();
    let mut first = operation(1);
    let mut second = operation(2);
    let mut draw = |size, progress_count, events| {
        settings.default_preset = if show_custom.get() {
            Preset::Custom
        } else {
            Preset::Best
        };
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let [a, b, c, d, e, f] = &mut drafts;
                    render(
                        ui,
                        &mut tab,
                        &mut sections,
                        &mut anchors,
                        &mut collection_pages,
                        &mut settings,
                        &mut SettingsOutcome::default(),
                        false,
                        &None,
                        &tools_fixture(),
                        false,
                        &mut false,
                        &mut [a, b, c, d, e, f],
                        &mut Preset::Custom,
                        &mut Preset::Custom,
                        [
                            if progress_count > 0 {
                                Some(&mut first)
                            } else {
                                None
                            },
                            if progress_count > 1 {
                                Some(&mut second)
                            } else {
                                None
                            },
                        ],
                    );
                });
            },
        );
        let edit = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .find_map(|(_, node)| {
                if node.value() != Some("focused-language-marker") {
                    return None;
                }
                let r = node.bounds()?;
                Some(egui::Rect::from_min_max(
                    egui::pos2(r.x0 as f32, r.y0 as f32),
                    egui::pos2(r.x1 as f32, r.y1 as f32),
                ))
            });
        (edit, ctx.memory(|memory| memory.focused()))
    };
    draw(egui::vec2(600.0, 640.0), 0, Vec::new());
    let pos = draw(egui::vec2(600.0, 640.0), 0, Vec::new())
        .0
        .expect("Initial actual editable row")
        .center();
    draw(
        egui::vec2(600.0, 640.0),
        0,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let focused = draw(
        egui::vec2(600.0, 640.0),
        0,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    )
    .1
    .expect("Actual pointer click focused the editable widget");
    show_custom.set(true);
    let (edit, focus) = draw(egui::vec2(600.0, 640.0), 0, Vec::new());
    assert_eq!(
        focus,
        Some(focused),
        "Inserting the custom-format row shifted focus"
    );
    assert!(
        edit.is_some(),
        "Focused row disappeared after conditional insertion"
    );
    for progress_count in [2, 0] {
        let (edit, focus) = draw(egui::vec2(480.0, 420.0), progress_count, Vec::new());
        assert_eq!(focus, Some(focused), "Repacking lost keyboard focus");
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 420.0))
                .contains_rect(edit.expect("Focused row must remain on the visible page"))
        );
    }
}

#[test]
fn actual_second_progress_cancel_stays_local_and_disables_repeat_cancel() {
    let ctx = context(1.0);
    ctx.enable_accesskit();
    let parent = yttraydl::process::CancellationToken::default();
    let mut first = operation(1);
    let mut second = operation(2);
    first.cancel = parent.child();
    second.cancel = parent.child();
    let mut draw = |events| {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(480.0, 420.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    compact_progress(ui, &mut first);
                    compact_progress(ui, &mut second);
                });
            },
        );
        let mut buttons: Vec<_> = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, node)| {
                if node.label() != Some("Cancel") {
                    return None;
                }
                let r = node.bounds()?;
                Some((
                    egui::Rect::from_min_max(
                        egui::pos2(r.x0 as f32, r.y0 as f32),
                        egui::pos2(r.x1 as f32, r.y1 as f32),
                    ),
                    node.is_disabled(),
                ))
            })
            .collect();
        buttons.sort_by(|a, b| a.0.top().total_cmp(&b.0.top()));
        (
            buttons,
            first.cancel.is_cancelled(),
            first.cancelling,
            second.cancel.is_cancelled(),
            second.cancelling,
        )
    };
    draw(Vec::new());
    let (buttons, ..) = draw(Vec::new());
    assert_eq!(buttons.len(), 2);
    assert!(!buttons[0].1 && !buttons[1].1);
    let pos = buttons[1].0.center();
    for _ in 0..2 {
        draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (_, first_cancelled, first_cancelling, second_cancelled, second_cancelling) =
            draw(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        assert!(!first_cancelled && !first_cancelling);
        assert!(second_cancelled && second_cancelling);
        let (buttons, ..) = draw(Vec::new());
        assert!(!buttons[0].1 && buttons[1].1);
        assert!(!parent.is_cancelled());
    }
}

#[test]
fn short_radio_choices_change_only_on_selection() {
    let ctx = context(1.0);
    ctx.enable_accesskit();
    let mut short = 0;
    let mut long = 0;
    let options = [(0, "Alpha"), (1, "Beta"), (2, "Gamma"), (3, "Delta")];
    let mut draw = |events| {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 500.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.push_id("short", |ui| {
                        choice(ui, "Short", &mut short, &[(0, "First"), (1, "Second")]);
                    });
                    ui.push_id("long", |ui| {
                        choice(ui, "Long", &mut long, &options);
                    });
                });
            },
        );
        let nodes: Vec<_> = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, node)| {
                let name = node.label()?.to_string();
                let r = node.bounds()?;
                Some((
                    name,
                    egui::Rect::from_min_max(
                        egui::pos2(r.x0 as f32, r.y0 as f32),
                        egui::pos2(r.x1 as f32, r.y1 as f32),
                    ),
                ))
            })
            .collect();
        (nodes, short, long)
    };
    draw(Vec::new());
    let (nodes, a, b) = draw(Vec::new());
    assert_eq!((a, b), (0, 0));
    let pos = nodes
        .iter()
        .find(|(name, _)| name == "Second")
        .unwrap()
        .1
        .center();
    draw(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    let (_, a, b) = draw(vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!((a, b), (1, 0));
}

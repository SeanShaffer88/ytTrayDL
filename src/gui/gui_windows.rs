//! The viewports: control panel (root), download popup, settings,
//! downloads, toasts, about, and the hide-icon confirmation.
//!
//! Viewport closures can't borrow `self` while `update` also holds
//! it, so each window extracts the state it needs (popup via
//! `Option::take`, settings/downloads via `mem::take`), renders,
//! and returns an outcome struct the caller applies afterwards.

use super::*;
#[path = "settings_pages.rs"]
mod settings_pages;
use yttraydl::platform;
use yttraydl::settings::{
    AudioQuality, Container, DefaultAction, DetectionScope, PopupPosition, Preset, QueueEmptyAction,
};
use yttraydl::window_geometry::{TransientWindow, WindowKind};
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

fn confirmation_position(ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
    ctx.input(|input| input.viewport().outer_rect)
        .map(|parent| parent.center() - size / 2.0)
        .unwrap_or_else(|| bottom_right(ctx, size, 0.0))
}

fn panel_regions(full: egui::Rect, header_bottom: f32) -> (egui::Rect, egui::Rect) {
    let footer_top = (full.bottom() - 46.0).max(full.top());
    let footer = egui::Rect::from_min_max(egui::pos2(full.left(), footer_top), full.max);
    let body_top = (header_bottom + 10.0).min(footer_top);
    let body_bottom = (footer_top - 8.0).max(body_top);
    let body = egui::Rect::from_min_max(
        egui::pos2(full.left() + 12.0, body_top),
        egui::pos2(full.right() - 12.0, body_bottom),
    );
    (body, footer)
}

fn flexible_title_row<R>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    title: &str,
    tooltip: &str,
    action_labels: &[&str],
    leading: impl FnOnce(&mut egui::Ui),
    actions: impl FnOnce(&mut egui::Ui) -> R,
) -> (egui::Response, R) {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            leading(ui);
            // Button::small uses TextStyle::Button and the normal horizontal
            // frame padding; only vertical padding differs (egui 0.33.3).
            // Reserve measured controls, then create them in their original order
            // so keyboard traversal and visual action ordering remain unchanged.
            let padding = if ui.visuals().button_frame {
                2.0 * ui.spacing().button_padding.x
            } else {
                0.0
            };
            let controls: f32 = action_labels
                .iter()
                .map(|label| {
                    egui::WidgetText::from(*label)
                        .into_galley(
                            ui,
                            Some(egui::TextWrapMode::Extend),
                            f32::INFINITY,
                            egui::TextStyle::Button,
                        )
                        .size()
                        .x
                        .ceil()
                        + padding
                })
                .sum();
            let reserved = controls + action_labels.len() as f32 * ui.spacing().item_spacing.x;
            let title_width = (ui.available_width() - reserved).max(0.0);
            let title = ui
                .allocate_ui_with_layout(
                    egui::vec2(title_width, ui.text_style_height(&egui::TextStyle::Body)),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        // add_sized centers short labels within their reserved
                        // cell. Keep the full cell but left-align actual text.
                        ui.set_min_width(title_width);
                        ui.add(egui::Label::new(title).truncate().halign(egui::Align::Min));
                    },
                )
                .response
                .on_hover_text(tooltip);
            (title, actions(ui))
        })
        .inner
    })
    .inner
}

fn grid_action(response: egui::Response, rectangles: &mut Vec<egui::Rect>) -> egui::Response {
    rectangles.push(response.rect);
    response
}

fn paint_grid_columns(ui: &egui::Ui, title: egui::Rect, actions: &[egui::Rect]) {
    let mut cells = vec![title];
    cells.extend_from_slice(actions);
    cells.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let row = cells
        .iter()
        .copied()
        .fold(egui::Rect::NOTHING, |row, cell| row.union(cell));
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(row));
    for pair in cells.windows(2) {
        if pair[1].left() > pair[0].right() {
            let x = (pair[0].right() + pair[1].left()) / 2.0;
            painter.line_segment(
                [egui::pos2(x, row.top()), egui::pos2(x, row.bottom())],
                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0x46, 0x46, 0x4e)),
            );
        }
    }
}

fn paint_entry_boundary(ui: &egui::Ui, previous_bottom: Option<f32>) {
    if let Some(bottom) = previous_bottom {
        let top = ui.cursor().top();
        if top > bottom {
            let y = (bottom + top) / 2.0;
            ui.painter().line_segment(
                [
                    egui::pos2(ui.max_rect().left(), y),
                    egui::pos2(ui.max_rect().right(), y),
                ],
                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0x46, 0x46, 0x4e)),
            );
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::{flexible_title_row, panel_regions};
    #[test]
    fn panel_regions_keep_overflow_separate_from_reachable_footer() {
        for size in [
            egui::vec2(340.0, 420.0),
            egui::vec2(400.0, 420.0),
            egui::vec2(560.0, 650.0),
        ] {
            let full = egui::Rect::from_min_size(egui::pos2(40.0, 80.0), size);
            let (body, footer) = panel_regions(full, full.top() + 96.0);
            assert!(full.contains_rect(body));
            assert!(full.contains_rect(footer));
            assert!(body.bottom() < footer.top());
            assert!(body.height() > 0.0 && footer.height() >= 30.0);
            assert_eq!(footer.bottom(), full.bottom());
            assert!(body.width() < full.width());
            // An arbitrarily long body belongs to a bounded scroll viewport,
            // never the footer's hit region.
            assert!(body.height() < 2000.0);
            assert!(!body.intersects(footer));
        }
    }

    #[test]
    fn title_cells_expand_without_overlapping_real_buttons_and_tab_order() {
        let ctx = egui::Context::default();
        super::super::apply_theme(&ctx);
        let render = |width: f32, events: Vec<egui::Event>, focus_first: bool| {
            let mut bounds = None;
            let _=ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,200.0))),events,..Default::default()},|ctx|{
                egui::CentralPanel::default().show(ctx,|ui|{
                    let row=ui.max_rect();
                    let (title,buttons)=flexible_title_row(ui,"test-row","Long café 雪 video title / https://example.invalid/verylongunbrokenoriginaltitle", "full name and URL", &["📂","↻ Retry","✕"], |_|{}, |ui|{
                        let first=ui.small_button("📂");
                        if focus_first {first.request_focus();}
                        [first,ui.small_button("↻ Retry"),ui.small_button("✕")]
                    });
                    bounds=Some((row,title,buttons));
                });
            });
            bounds.unwrap()
        };
        let (small_area, small, buttons) = render(340.0, vec![], true);
        for button in &buttons {
            assert!(small_area.contains_rect(button.rect));
            assert!(!small.rect.intersects(button.rect));
        }
        let tab = egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, _, next) = render(340.0, vec![tab], false);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(next[1].id));
        let point = next[1].rect.center();
        let (_, _, clicked) = render(
            340.0,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            false,
        );
        assert!(clicked[1].clicked() && !clicked[0].clicked() && !clicked[2].clicked());
        let (_, wide, _) = render(800.0, vec![], false);
        assert!(wide.rect.width() > small.rect.width() + 300.0);
    }

    #[test]
    fn right_anchored_actions_keep_creation_order_and_fit_minimum_width() {
        let ctx = egui::Context::default();
        super::super::apply_theme(&ctx);
        let render = |events: Vec<egui::Event>, focus: bool| {
            let mut result = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(460.0, 200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let area = ui.max_rect();
                        let (title, buttons) = flexible_title_row(
                            ui,
                            "queued-row",
                            "雪 café https://example.invalid/unbrokenvideoidentifierverylong",
                            "full tooltip",
                            &["✕ Cancel", "⏸ Pause", "↓", "↑"],
                            |_| {},
                            |ui| {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let first = ui.small_button("✕ Cancel");
                                        if focus {
                                            first.request_focus();
                                        }
                                        [
                                            first,
                                            ui.small_button("⏸ Pause"),
                                            ui.add_enabled(false, egui::Button::new("↓").small()),
                                            ui.add_enabled(false, egui::Button::new("↑").small()),
                                        ]
                                    },
                                )
                                .inner
                            },
                        );
                        result = Some((area, title, buttons));
                    });
                },
            );
            result.unwrap()
        };
        let (area, title, buttons) = render(vec![], true);
        for button in &buttons {
            assert!(area.contains_rect(button.rect));
            assert!(!title.rect.intersects(button.rect));
        }
        assert!(buttons[0].rect.left() > buttons[1].rect.left());
        let (_, _, next) = render(
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            false,
        );
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(next[1].id));
        assert!(!next[2].enabled() && !next[3].enabled());
    }

    #[test]
    fn short_and_long_titles_paint_at_the_same_left_edge() {
        let ctx = egui::Context::default();
        super::super::apply_theme(&ctx);
        let short = "Part 04 — café";
        let long="Part 04 — café 雪 original video title with a very long unbroken https://example.invalid/videoidentifierandmorecharacters";
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 200.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for (index, text) in [short, long].iter().enumerate() {
                        let _ = flexible_title_row(
                            ui,
                            index,
                            text,
                            text,
                            &["✕", "⬇ Download"],
                            |_| {},
                            |ui| {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let _ = ui.small_button("✕");
                                        let _ = ui.small_button("⬇ Download");
                                    },
                                );
                            },
                        );
                    }
                });
            },
        );
        fn find(shape: &egui::epaint::Shape, text: &str) -> Option<f32> {
            match shape {
                egui::epaint::Shape::Text(shape) if shape.galley.job.text == text => {
                    Some(shape.pos.x + shape.galley.rows.first()?.pos.x)
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find(shape, text))
                }
                _ => None,
            }
        }
        let x = |text| {
            output
                .shapes
                .iter()
                .find_map(|shape| find(&shape.shape, text))
                .expect("painted title galley")
        };
        assert!(
            (x(short) - x(long)).abs() <= 0.5,
            "short text x{} differs from long text x{}",
            x(short),
            x(long)
        );
    }

    #[test]
    fn grid_paint_uses_real_gaps_and_complete_metadata_without_moving_layout() {
        let ctx = egui::Context::default();
        super::super::apply_theme(&ctx);
        let mut primary = Vec::new();
        let mut cells = Vec::new();
        let mut metadata_bottoms = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 240.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_clip_rect(egui::Rect::from_min_max(
                        ui.max_rect().min,
                        egui::pos2(ui.max_rect().right(), 120.0),
                    ));
                    let mut previous = None;
                    for index in 0..2 {
                        let cursor = ui.cursor();
                        super::paint_entry_boundary(ui, previous);
                        assert_eq!(ui.cursor(), cursor);
                        let mut actions = Vec::new();
                        let (title, ()) = flexible_title_row(
                            ui,
                            index,
                            "Part 01 café original long title",
                            "tooltip",
                            &["📂", "↻ Retry"],
                            |_| {},
                            |ui| {
                                let layout = if index == 0 {
                                    egui::Layout::left_to_right(egui::Align::Center)
                                } else {
                                    egui::Layout::right_to_left(egui::Align::Center)
                                };
                                ui.with_layout(layout, |ui| {
                                    let _ = super::grid_action(ui.small_button("📂"), &mut actions);
                                    let _ = super::grid_action(
                                        ui.small_button("↻ Retry"),
                                        &mut actions,
                                    );
                                });
                            },
                        );
                        let cursor = ui.cursor();
                        super::paint_grid_columns(ui, title.rect, &actions);
                        assert_eq!(ui.cursor(), cursor);
                        primary.push(title.rect.union(actions[0]).union(actions[1]));
                        cells.push(title.rect);
                        cells.extend_from_slice(&actions);
                        let metadata =
                            ui.label("Time and quality metadata span the complete entry");
                        previous = Some(metadata.rect.bottom());
                        metadata_bottoms.push(metadata.rect.bottom());
                    }
                });
            },
        );
        let lines = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::LineSegment { points, stroke }
                    if stroke.color == egui::Color32::from_rgb(0x46, 0x46, 0x4e) =>
                {
                    Some((points, shape.clip_rect))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 5);
        let horizontal = lines
            .iter()
            .filter(|(points, _)| points[0].y == points[1].y)
            .collect::<Vec<_>>();
        assert_eq!(horizontal.len(), 1);
        assert!(
            horizontal[0].0[0].y > metadata_bottoms[0] && horizontal[0].0[0].y < primary[1].top()
        );
        for (points, clip) in lines {
            assert!(clip.bottom() <= 120.0);
            if points[0].x == points[1].x {
                let center = egui::pos2(points[0].x, (points[0].y + points[1].y) / 2.0);
                assert!(
                    !cells.iter().any(|cell| cell.contains(center)),
                    "divider crosses an actual title/action cell"
                );
            }
        }
    }
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
                    .button("Close")
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

        // The footer has its own hit region. Long or populated body content
        // scrolls within the remaining height instead of pushing it offscreen.
        let (content_rect, footer_rect) = panel_regions(full, band_rect.bottom());
        let mut content = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        content.set_clip_rect(content_rect.intersect(ui.clip_rect()));
        let mut body = egui::ScrollArea::vertical()
            .id_salt("control-panel-body")
            .auto_shrink([false, false])
            .max_height(content_rect.height());
        if self.scroll_to_tool_progress {
            body = body.vertical_scroll_offset(0.0);
            self.scroll_to_tool_progress = false;
        }
        body.show(&mut content, |ui| self.panel_body(ui));
        let mut footer = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(footer_rect.shrink2(egui::vec2(12.0, 8.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        self.panel_footer(&mut footer, footer_rect);
    }

    fn panel_body(&mut self, ui: &mut egui::Ui) {
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

        self.tool_progress_ui(ui);

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
        premium_well(ui, |ui| {
            if !self.tool_status_ready {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking installed tools…");
                });
                return;
            }
            match &self.bin {
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
                        ui.add(egui::Label::new(egui::RichText::new("ffmpeg not found — best-quality streams that need merging and MP3 conversion will fail.").color(ACCENT)).wrap());
                        ui.horizontal(|ui| {
                            let busy = self.ffmpeg_fetch_rx.is_some();
                            if ui
                                .add_enabled(!busy, primary_button(if busy { "Downloading…" } else { "Install ffmpeg" }))
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
                    ui.horizontal_wrapped(|ui| {
                        let busy = self.ytdlp_fetch_rx.is_some();
                        if ui
                            .add_enabled(!busy, primary_button(if busy { "Downloading…" } else { "Install yt-dlp" }))
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

        if ui.button("Video converter…").on_hover_text("Open the local video converter").clicked() {
            self.show_converter = true;
        }

        // Active downloads (compact).
        let active: Vec<(u64, String, String, f32, Option<String>, JobState)> = self
            .downloads
            .running_jobs()
            .chain(self.downloads.queued_jobs())
            .map(|j| {
                (
                    j.id,
                    j.title.clone(),
                    j.url.clone(),
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
            let mut previous_bottom = None;
            for (id, title, url, pct, eta, state) in &active {
                paint_entry_boundary(ui, previous_bottom);
                let mut action_rects = Vec::new();
                let (title_cell, ()) = flexible_title_row(
                    ui,
                    ("main-job", id),
                    if title.is_empty() { url } else { title },
                    &format!("{title}\n{url}"),
                    &["✕"],
                    |_| {},
                    |ui| {
                        if grid_action(ui.small_button("✕"), &mut action_rects)
                            .on_hover_text(
                                "Cancel this download (the partial file is kept for resume)",
                            )
                            .clicked()
                        {
                            cancel = Some(*id);
                        }
                    },
                );
                paint_grid_columns(ui, title_cell.rect, &action_rects);
                ui.horizontal(|ui| {
                    if *state == JobState::Queued {
                        ui.label(egui::RichText::new("queued").color(TEXT_FAINT));
                    } else {
                        ui.add(egui::ProgressBar::new(pct / 100.0).desired_width(90.0));
                        let metadata = format!(
                            "{pct:.0}%{}",
                            eta.as_ref()
                                .map(|e| format!(" · ETA {e}"))
                                .unwrap_or_default()
                        );
                        ui.add(egui::Label::new(&metadata).truncate())
                            .on_hover_text(metadata);
                    }
                });
                previous_bottom = Some(ui.min_rect().bottom());
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
            let mut previous_bottom = None;
            for (i, link) in self.saved_links.iter().enumerate() {
                let label = if link.title.is_empty() || link.title == link.url {
                    &link.url
                } else {
                    &link.title
                };
                paint_entry_boundary(ui, previous_bottom);
                let mut action_rects = Vec::new();
                let (title_cell, ()) = flexible_title_row(
                    ui,
                    ("saved", &link.url),
                    label,
                    &format!("{label}\n{}", link.url),
                    &["✕", "⬇ Download"],
                    |_| {},
                    |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if grid_action(ui.small_button("✕"), &mut action_rects)
                                .on_hover_text("Remove from the saved list")
                                .clicked()
                            {
                                act = Some((i, false));
                            }
                            if grid_action(ui.small_button("⬇ Download"), &mut action_rects)
                                .on_hover_text(
                                    "Download now with your default quality (or its domain pin)",
                                )
                                .clicked()
                            {
                                act = Some((i, true));
                            }
                        });
                    },
                );
                paint_grid_columns(ui, title_cell.rect, &action_rects);
                previous_bottom = Some(ui.min_rect().bottom());
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
    }

    fn panel_footer(&mut self, ui: &mut egui::Ui, bar_rect: egui::Rect) {
        // The existing full-bleed recessed bar and button sizes are retained.
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

        platform::constrain_transient_window(TransientWindow::DownloadPrompt);

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
            self.settings_viewport_capacity = None;
            self.settings_viewport_monitor = None;
            self.settings_viewport_refined = false;
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
        let mut sections = self.settings_sections;
        let mut anchors = self.settings_anchors;
        let mut collection_pages = self.settings_collection_pages;
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
        let ffmpeg_managed = self.ffmpeg_managed;
        let fetching = self.ytdlp_fetch_rx.is_some();
        let fetching_ffmpeg = self.ffmpeg_fetch_rx.is_some();
        let autostart_now = self.autostart_status;

        if self.settings_viewport_capacity.is_none() {
            self.settings_viewport_capacity =
                platform::window_client_capacity(WindowKind::Settings)
                    .or_else(|| platform::window_client_capacity(WindowKind::Main))
                    .map(|(capacity, _)| capacity);
        }
        let capacity = self.settings_viewport_capacity.unwrap_or_else(|| {
            // Cached egui points are already logical. This is only a fallback
            // before an owned native window can supply its exact work area.
            let size = monitor_size(ctx) - egui::vec2(48.0, 96.0);
            [size.x.max(1.0), size.y.max(1.0)]
        });
        let initial = [
            SETTINGS_WINDOW_SIZE[0].min(capacity[0]),
            SETTINGS_WINDOW_SIZE[1].min(capacity[1]),
        ];
        let minimum = [480.0_f32.min(capacity[0]), 420.0_f32.min(capacity[1])];
        let mut monitor_signature = None;

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-settings"),
            egui::ViewportBuilder::default()
                .with_title("ytTrayDL — Settings")
                .with_inner_size(initial)
                .with_min_inner_size(minimum)
                .with_max_inner_size(capacity),
            |ctx, _class| {
                monitor_signature = ctx.input(|input| {
                    input.viewport().monitor_size.map(|size| {
                        (
                            size,
                            input.viewport().native_pixels_per_point.unwrap_or(1.0),
                        )
                    })
                });

                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    let tools = ToolsUi {
                        ffmpeg_ok,
                        ffmpeg_managed,
                        ffmpeg_fetching: fetching_ffmpeg,
                        update_checking: self.ytdlp_check_rx.is_some(),
                        latest: self.ytdlp_latest.clone(),
                        current_version: self.ytdlp_version.clone(),
                    };
                    let mut inputs = [
                        &mut rule_ignore,
                        &mut rule_auto,
                        &mut rule_allow,
                        &mut rule_preset_domain,
                        &mut rule_watch_url,
                        &mut rule_url,
                    ];
                    close = settings_pages::render(
                        ui,
                        &mut tab,
                        &mut sections,
                        &mut anchors,
                        &mut collection_pages,
                        &mut settings,
                        &mut outcome,
                        autostart_now,
                        &bin_info,
                        &tools,
                        fetching,
                        &mut reset_armed,
                        &mut inputs,
                        &mut rule_preset_choice,
                        &mut rule_watch_preset,
                        [
                            self.ytdlp_operation.as_mut(),
                            self.ffmpeg_operation.as_mut(),
                        ],
                    );
                });
            },
        );

        self.restore_new_window_geometry(WindowKind::Settings);
        if !self.settings_viewport_refined || monitor_signature != self.settings_viewport_monitor {
            if let Some((available, current)) =
                platform::window_client_capacity(WindowKind::Settings)
            {
                self.settings_viewport_capacity = Some(available);
                self.settings_viewport_monitor = monitor_signature;
                self.settings_viewport_refined = true;
                let viewport = egui::ViewportId::from_hash_of("yttraydl-settings");
                ctx.send_viewport_cmd_to(
                    viewport,
                    egui::ViewportCommand::MinInnerSize(egui::vec2(
                        480.0_f32.min(available[0]),
                        420.0_f32.min(available[1]),
                    )),
                );
                ctx.send_viewport_cmd_to(
                    viewport,
                    egui::ViewportCommand::MaxInnerSize(available.into()),
                );
                if current[0] > available[0] || current[1] > available[1] {
                    ctx.send_viewport_cmd_to(
                        viewport,
                        egui::ViewportCommand::InnerSize(egui::vec2(
                            current[0].min(available[0]),
                            current[1].min(available[1]),
                        )),
                    );
                }
            }
        }

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
        self.settings_sections = sections;
        self.settings_anchors = anchors;
        self.settings_collection_pages = collection_pages;
        self.reset_armed = reset_armed;
        if close || !open {
            self.show_settings = false;
            self.settings_viewport_capacity = None;
            self.settings_viewport_monitor = None;
            self.settings_viewport_refined = false;
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
                    self.autostart_status = target;
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

struct ToolsUi {
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

fn tool_path_row(ui: &mut egui::Ui, path: &str) {
    ui.horizontal(|ui| {
        let button = ui
            .painter()
            .layout_no_wrap(
                "Copy".to_owned(),
                egui::TextStyle::Button.resolve(ui.style()),
                TEXT,
            )
            .size()
            .x
            + 2.0 * ui.spacing().button_padding.x
            + 8.0;
        let width = (ui.available_width() - button - ui.spacing().item_spacing.x).max(1.0);
        ui.add_sized(
            [width, 20.0],
            egui::Label::new(
                egui::RichText::new(path)
                    .monospace()
                    .small()
                    .color(TEXT_DIM),
            )
            .truncate(),
        )
        .on_hover_text(path);
        if ui
            .small_button("Copy")
            .on_hover_text("Copy full tool path")
            .clicked()
        {
            ui.ctx().copy_text(path.to_owned());
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
            self.downloads_initial_size = None;
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
        let empty = running.is_empty()
            && queued.is_empty()
            && paused.is_empty()
            && finished.is_empty()
            && history_entries.is_empty();
        let initial_size = *self.downloads_initial_size.get_or_insert(if empty {
            [800.0, 360.0]
        } else {
            [800.0, 560.0]
        });
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
                .with_inner_size(initial_size)
                .with_min_inner_size([460.0, 360.0]),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    if bin_missing {
                        ui.label(egui::RichText::new("yt-dlp isn't installed — downloads will wait in the queue.").color(ACCENT));
                        if ui.add_enabled(!fetching, egui::Button::new("Install yt-dlp")).clicked() {
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
                        let mut previous_active_bottom=None;
                        for (id, title, url, pct, speed, eta, state, _err) in &running {
                            paint_entry_boundary(ui,previous_active_bottom);
                            let mut action_rects=Vec::new();
                            let (title_cell,())=flexible_title_row(ui,("running",id),title,&format!("{title}\n{url}"),&["✕ Cancel","⏸ Pause"],|_|{},|ui|{
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if grid_action(ui.small_button("✕ Cancel"),&mut action_rects).on_hover_text("Cancel (partial file kept for resume)").clicked() {
                                        cancel = Some(*id);
                                    }
                                    if grid_action(ui.small_button("⏸ Pause"),&mut action_rects).on_hover_text("Pause (partial file kept; resume continues it)").clicked() {
                                        pause_job = Some(*id);
                                    }
                                });
                            });
                            paint_grid_columns(ui,title_cell.rect,&action_rects);
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
                                    let metadata=bits.join(" · ");
                                    ui.add(egui::Label::new(egui::RichText::new(&metadata).small().color(TEXT_DIM)).truncate()).on_hover_text(metadata);
                                }
                            }
                            ui.add_space(4.0);
                            previous_active_bottom=Some(ui.min_rect().bottom());
                        }
                        let queued_len = queued.len();
                        for (qi, (id, title, url, _pct, _speed, _eta, _state, _err)) in queued.iter().enumerate() {
                            paint_entry_boundary(ui,previous_active_bottom);
                            let mut action_rects=Vec::new();
                            let (title_cell,())=flexible_title_row(ui,("queued",id),title,&format!("{title}\n{url}"),&["✕ Cancel","⏸ Pause","↓","↑"],|_|{},|ui|{
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if grid_action(ui.small_button("✕ Cancel"),&mut action_rects).on_hover_text("Cancel this queued download").clicked() {
                                        cancel = Some(*id);
                                    }
                                    if grid_action(ui.small_button("⏸ Pause"),&mut action_rects).on_hover_text("Pause (stays out of the queue until resumed)").clicked() {
                                        pause_job = Some(*id);
                                    }
                                    if grid_action(ui.add_enabled(qi + 1 < queued_len, egui::Button::new("↓").small()),&mut action_rects).on_hover_text("Move down in the queue").clicked() {
                                        move_job = Some((*id, false));
                                    }
                                    if grid_action(ui.add_enabled(qi > 0, egui::Button::new("↑").small()),&mut action_rects).on_hover_text("Move up in the queue").clicked() {
                                        move_job = Some((*id, true));
                                    }
                                });
                            });
                            paint_grid_columns(ui,title_cell.rect,&action_rects);
                            ui.label(egui::RichText::new(format!("Queued — #{} in queue", qi + 1)).color(TEXT_FAINT).small());
                            ui.add_space(4.0);
                            previous_active_bottom=Some(ui.min_rect().bottom());
                        }
                        });

                        if !paused.is_empty() {
                            premium_card(ui, |ui| {
                            ui.label(section_heading("Paused"));
                            let mut previous_bottom=None;
                            for (id, title, url, pct) in &paused {
                                paint_entry_boundary(ui,previous_bottom);
                                let mut action_rects=Vec::new();
                                let (title_cell,())=flexible_title_row(ui,("paused",id),title,&format!("{title}\n{url}"),&["✕ Cancel","▶ Resume"],|_|{},|ui|{
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if grid_action(ui.small_button("✕ Cancel"),&mut action_rects).on_hover_text("Cancel for good (partial file kept on disk)").clicked() {
                                            cancel = Some(*id);
                                        }
                                        if grid_action(ui.small_button("▶ Resume"),&mut action_rects).on_hover_text("Resume (continues the partial file)").clicked() {
                                            resume_job = Some(*id);
                                        }
                                    });
                                });
                                paint_grid_columns(ui,title_cell.rect,&action_rects);
                                let txt = if *pct > 0.0 {
                                    format!("Paused at {pct:.0}%")
                                } else {
                                    "Paused".to_string()
                                };
                                ui.label(egui::RichText::new(txt).color(TEXT_FAINT).small());
                                ui.add_space(4.0);
                                previous_bottom=Some(ui.min_rect().bottom());
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
                            let mut previous_bottom=None;
                            for row in &finished {
                                paint_entry_boundary(ui,previous_bottom);
                                job_row(ui, row, &mut open_target, &mut retry_finished, &mut remove_finished);
                                ui.add_space(2.0);
                                previous_bottom=Some(ui.min_rect().bottom());
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
                        let mut previous_bottom=None;
                        for e in history_entries.iter().take(40) {
                            paint_entry_boundary(ui,previous_bottom);
                            let mut action_rects=Vec::new();
                            let mut labels=Vec::new();
                            if e.file.is_some(){labels.push("📂");}
                            if e.status==HistoryStatus::Failed{labels.push("↻ Retry");}
                            let (title_cell,())=flexible_title_row(ui,("history",e.finished_at_unix,&e.url),&e.title,&format!("{}\n{}\n{}", e.title, e.url, e.error.clone().unwrap_or_default()),&labels,|ui|status_dot(ui,status_color(e.status)),|ui|{
                                if let Some(f) = &e.file {
                                    if grid_action(ui.small_button("📂"),&mut action_rects).on_hover_text("Show in Explorer").clicked() {
                                        open_target = f.parent().map(|p| p.to_path_buf());
                                    }
                                }
                                if e.status == HistoryStatus::Failed
                                    && grid_action(ui.small_button("↻ Retry"),&mut action_rects).on_hover_text("Download this link again with your current defaults").clicked()
                                {
                                    retry = Some((e.url.clone(), Preset::Best));
                                }
                            });
                            paint_grid_columns(ui,title_cell.rect,&action_rects);
                            let metadata=format!("{} · {}",local_time_hhmm(e.finished_at_unix),e.preset_label);
                            ui.add(egui::Label::new(egui::RichText::new(&metadata).color(TEXT_FAINT).small()).truncate()).on_hover_text(metadata);
                            previous_bottom=Some(ui.min_rect().bottom());
                        }
                        });
                    });
                });
            },
        );

        self.restore_new_window_geometry(WindowKind::Downloads);
        self.show_downloads = open;
        if !open {
            self.downloads_initial_size = None;
        }
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
        self.restore_new_window_geometry(WindowKind::About);
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
                .with_position(confirmation_position(ctx,egui::vec2(460.0,270.0)))
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
        platform::constrain_transient_window(TransientWindow::HideIconConfirmation);
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
                .with_position(confirmation_position(ctx,egui::vec2(470.0,330.0)))
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
        platform::constrain_transient_window(TransientWindow::CloseConfirmation);
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
    let (ch, color) = match state {
        JobState::Done => ("✓", OK_GREEN),
        JobState::Failed => ("✗", ACCENT),
        JobState::Cancelled => ("—", TEXT_FAINT),
        _ => ("…", TEXT_DIM),
    };
    let mut labels = Vec::new();
    if file.is_some() {
        labels.push("📂");
    }
    if matches!(state, JobState::Failed | JobState::Cancelled) {
        labels.push("↻ Retry");
    }
    labels.push("✕");
    let tooltip = format!(
        "{title}\n{url}{}",
        err.as_ref().map(|e| format!("\n{e}")).unwrap_or_default()
    );
    let mut action_rects = Vec::new();
    let (title_cell, ()) = flexible_title_row(
        ui,
        ("finished", id),
        title,
        &tooltip,
        &labels,
        |ui| {
            ui.label(egui::RichText::new(ch).color(color));
        },
        |ui| {
            if let Some(f) = file {
                if grid_action(ui.small_button("📂"), &mut action_rects)
                    .on_hover_text("Show in Explorer")
                    .clicked()
                {
                    *open_target = f.parent().map(|p| p.to_path_buf());
                }
            }
            if matches!(state, JobState::Failed | JobState::Cancelled)
                && grid_action(ui.small_button("↻ Retry"), &mut action_rects)
                    .on_hover_text("Download again with the same quality as the original attempt")
                    .clicked()
            {
                *retry_finished = Some(*id);
            }
            if grid_action(ui.small_button("✕"), &mut action_rects)
                .on_hover_text("Remove from this list (the file stays)")
                .clicked()
            {
                *remove_finished = Some(*id);
            }
        },
    );
    paint_grid_columns(ui, title_cell.rect, &action_rects);
}

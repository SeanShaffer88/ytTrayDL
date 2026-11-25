//! Shared themed controls and custom drawing helpers.
use super::*;

/// An opaque colour with its alpha replaced (egui 0.33's
/// Color32 has no `with_alpha`; all colours this is applied
/// to here are opaque, so rebuilding from unmultiplied
/// channels is exact).
pub fn with_alpha(c: egui::Color32, a: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// Per-channel linear interpolation between two colours.
pub fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| -> u8 { (x as f32 + (y as f32 - x as f32) * t).round() as u8 };
    egui::Color32::from_rgba_unmultiplied(
        ch(a.r(), b.r()),
        ch(a.g(), b.g()),
        ch(a.b(), b.b()),
        ch(a.a(), b.a()),
    )
}

/// Paint a vertical gradient inside a rounded rect (0.6.0).
/// egui has no gradient fill, so this draws thin horizontal
/// strips whose x-extent is clipped by the corner circles —
/// at 2px per strip the steps are invisible in practice. Used
/// for the header band, gradient buttons, the selected
/// segment/chip, and the popup's placeholder hero.
pub fn paint_vertical_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    top: egui::Color32,
    bottom: egui::Color32,
) {
    let h = rect.height();
    if h <= 0.0 || rect.width() <= 0.0 {
        return;
    }
    let strips = ((h / 2.0).ceil() as usize).clamp(4, 64);
    let r = radius.min(rect.width() / 2.0).min(h / 2.0);
    for i in 0..strips {
        let y0 = rect.top() + h * (i as f32 / strips as f32);
        let y1 = rect.top() + h * ((i + 1) as f32 / strips as f32);
        let yc = (y0 + y1) / 2.0;
        // Horizontal inset imposed by the corner circle at
        // this strip's centre height (0 outside the corners).
        let dy = if yc < rect.top() + r {
            r - (yc - rect.top())
        } else if yc > rect.bottom() - r {
            r - (rect.bottom() - yc)
        } else {
            0.0
        };
        let inset = if dy > 0.0 {
            r - (r * r - dy * dy).max(0.0).sqrt()
        } else {
            0.0
        };
        let strip = egui::Rect::from_min_max(
            egui::pos2(rect.left() + inset, y0),
            egui::pos2(rect.right() - inset, y1 + 0.75),
        );
        painter.rect_filled(
            strip,
            0.0,
            lerp_color(top, bottom, (i as f32 + 0.5) / strips as f32),
        );
    }
}

/// The 1px top-highlight line on a raised surface (0.6.0):
/// a short light stroke along the top edge, inset by the
/// corner radius so it never touches the corners.
pub fn paint_top_highlight(painter: &egui::Painter, rect: egui::Rect, radius: f32) {
    let y = rect.top() + 0.5;
    painter.line_segment(
        [
            egui::pos2(rect.left() + radius, y),
            egui::pos2(rect.right() - radius, y),
        ],
        egui::Stroke::new(1.0_f32, with_alpha(TOP_HIGHLIGHT, 110)),
    );
}

/// A soft fake shadow/glow (0.6.0): three nested rounded
/// rects of increasing size and decreasing alpha — egui's
/// painter has no blur, so this is the closest equivalent of
/// the mockups' blurred drop shadows / red button glow.
pub fn paint_soft_glow(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    color: egui::Color32,
) {
    for (expand, alpha) in [(6.0_f32, 22_u8), (4.0, 38), (2.0, 58)] {
        painter.rect_filled(
            rect.expand(expand),
            radius + expand,
            with_alpha(color, alpha),
        );
    }
}

/// A card frame (0.5.0): CARD fill, hairline outline, rounded
/// corners, 12px inner padding, 8px of space below so stacked
/// cards separate on the 4/8 grid. 0.6.0 adds a soft drop
/// shadow (Frame shadows are real egui; the matching top
/// highlight is painted by `premium_card`).
pub fn card_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0_f32, HAIRLINE))
        .corner_radius(RADIUS)
        .shadow(egui::epaint::Shadow {
            offset: [0, 2],
            blur: 6,
            spread: 0,
            color: egui::Color32::from_black_alpha(60),
        })
        .inner_margin(egui::Margin::same(12))
        .outer_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 0,
            bottom: 8,
        })
}

/// Show a card and paint its 1px top highlight (0.6.0) — the
/// standard raised container everywhere in the app.
pub fn premium_card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let shown = card_frame().show(ui, add_contents);
    paint_top_highlight(ui.painter(), shown.response.rect, RADIUS as f32);
    shown.inner
}

/// A recessed well frame (0.6.0): WELL fill (darker than the
/// panel), the darker stroke, no shadow — wells sink, cards
/// rise. Same padding/grid as `card_frame`.
pub fn well_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(WELL)
        .stroke(egui::Stroke::new(1.0_f32, STROKE_DARK))
        .corner_radius(RADIUS)
        .inner_margin(egui::Margin::same(12))
        .outer_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 0,
            bottom: 8,
        })
}

/// Show a well and paint its inset top line (0.6.0): wells get
/// a dark line just inside the top edge (the shadow a raised
/// neighbour would cast into the recess) instead of a card's
/// light highlight.
pub fn premium_well<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let shown = well_frame().show(ui, add_contents);
    let rect = shown.response.rect;
    let y = rect.top() + 1.5;
    ui.painter().line_segment(
        [
            egui::pos2(rect.left() + RADIUS as f32, y),
            egui::pos2(rect.right() - RADIUS as f32, y),
        ],
        egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(170)),
    );
    shown.inner
}

/// Section heading (0.5.0), restyled 0.6.0 as a micro-label:
/// small, strong, uppercase, faint — the mockups' section
/// voice ("ENGINE", "RECENT", "TRAY & STARTUP"). egui has no
/// letter-spacing API, so the mockups' letterspaced labels
/// are honestly approximated by case + size + weight alone.
pub fn section_heading(text: &str) -> egui::RichText {
    egui::RichText::new(text.to_uppercase())
        .strong()
        .color(TEXT_FAINT)
        .size(11.5)
}

/// A focus ring for custom-painted controls (0.6.0): an
/// accent outline just outside the rect, matching the ring
/// egui draws around focused stock widgets in spirit.
pub fn paint_focus_ring(painter: &egui::Painter, rect: egui::Rect, radius: f32) {
    painter.rect_stroke(
        rect.expand(1.5),
        egui::CornerRadius::same((radius + 1.5) as u8),
        egui::Stroke::new(2.0_f32, ACCENT),
        egui::epaint::StrokeKind::Outside,
    );
}

/// A primary action uses a bright accent with inverse text. Selected chips
/// reuse the painter with their own dark fill and light text.
/// Implements `Widget` so every 0.5.0 call site
/// (`ui.add`, `ui.add_enabled`, `ui.add_sized`, `.min_size`)
/// works unchanged; interaction (hover/press/focus, Space /
/// Enter activation) comes from the same `Sense::click`
/// machinery the stock Button uses.
pub struct GradientButton {
    text: String,
    min_size: egui::Vec2,
    top: egui::Color32,
    bottom: egui::Color32,
    glow: bool,
    inverse_text: bool,
    /// Label size override (chips render at 12.5, buttons at
    /// the theme's Button size).
    text_size: Option<f32>,
}

pub fn primary_button(text: &str) -> GradientButton {
    GradientButton {
        text: text.to_string(),
        min_size: egui::Vec2::ZERO,
        top: ACCENT,
        bottom: ACCENT,
        glow: true,
        inverse_text: true,
        text_size: None,
    }
}

impl GradientButton {
    pub fn min_size(mut self, min_size: egui::Vec2) -> Self {
        self.min_size = min_size;
        self
    }
}

impl egui::Widget for GradientButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let enabled = ui.is_enabled();
        let text_color = if !enabled {
            TEXT_FAINT
        } else if self.inverse_text {
            TEXT_INVERSE
        } else {
            TEXT
        };
        let rich = egui::RichText::new(&self.text).strong().color(text_color);
        let rich = match self.text_size {
            Some(s) => rich.size(s),
            None => rich,
        };
        let galley = egui::WidgetText::from(rich).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Button,
        );
        let padding = ui.spacing().button_padding;
        let desired = egui::vec2(
            galley.size().x + padding.x * 2.0,
            (galley.size().y + padding.y * 2.0).max(ui.spacing().interact_size.y),
        );
        let size = desired.max(self.min_size);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let radius = rect.height() / 2.0;
            let (mut top, mut bottom) = (self.top, self.bottom);
            if !enabled {
                top = CARD2;
                bottom = CARD2;
            } else if response.is_pointer_button_down_on() {
                let target = if self.inverse_text {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::BLACK
                };
                top = lerp_color(top, target, 0.12);
                bottom = lerp_color(bottom, target, 0.12);
            } else if response.hovered() {
                top = lerp_color(top, egui::Color32::WHITE, 0.10);
                bottom = lerp_color(bottom, egui::Color32::WHITE, 0.10);
            }
            let painter = ui.painter();
            if self.glow && enabled {
                paint_soft_glow(painter, rect, radius, ACCENT_DARK);
            }
            paint_vertical_gradient(painter, rect, radius, top, bottom);
            // Light top edge, inset like the mockup's.
            painter.line_segment(
                [
                    egui::pos2(rect.left() + radius, rect.top() + 0.5),
                    egui::pos2(rect.right() - radius, rect.top() + 0.5),
                ],
                egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgba_unmultiplied(0xFF, 0xB0, 0xB4, 110),
                ),
            );
            if response.has_focus() {
                paint_focus_ring(painter, rect, radius);
            }
            painter.galley(rect.center() - galley.size() / 2.0, galley, text_color);
        }
        response
    }
}

/// A selectable chip (0.6.0): the popup's format choices and
/// playlist choice, restyled from the old combo box /
/// selectable labels. Selected = deep-red gradient fill +
/// accent outline + white strong text; unselected = raised
/// ghost (CARD2 fill, hairline stroke, dim text).
pub fn chip(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    if selected {
        let resp = ui.add(GradientButton {
            text: label.to_string(),
            min_size: egui::vec2(0.0, 28.0),
            top: CHIP_GRAD_TOP,
            bottom: ACCENT_DARK,
            glow: false,
            inverse_text: false,
            text_size: Some(12.5),
        });
        // Accent outline over the gradient (mockup).
        ui.painter().rect_stroke(
            resp.rect,
            egui::CornerRadius::same((resp.rect.height() / 2.0) as u8),
            egui::Stroke::new(1.0_f32, ACCENT),
            egui::epaint::StrokeKind::Inside,
        );
        resp
    } else {
        let galley = egui::WidgetText::from(egui::RichText::new(label).color(TEXT_DIM).size(12.5))
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            );
        let size = egui::vec2(galley.size().x + 26.0, 28.0);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let radius = rect.height() / 2.0;
            let fill = if response.is_pointer_button_down_on() {
                CARD
            } else if response.hovered() {
                CONTROL_BG_HOVER
            } else {
                CARD2
            };
            let painter = ui.painter();
            painter.rect_filled(rect, radius, fill);
            painter.rect_stroke(
                rect,
                egui::CornerRadius::same(radius as u8),
                egui::Stroke::new(
                    1.0_f32,
                    if response.hovered() {
                        STROKE_SOFT
                    } else {
                        HAIRLINE
                    },
                ),
                egui::epaint::StrokeKind::Inside,
            );
            paint_top_highlight(painter, rect, radius);
            if response.has_focus() {
                paint_focus_ring(painter, rect, radius);
            }
            painter.galley(rect.center() - galley.size() / 2.0, galley, TEXT_DIM);
        }
        response
    }
}

/// A toggle switch (0.6.0), painter-drawn to the mockup's
/// geometry: 36×20 pill track (accent when on, #363638 when
/// off) with a white knob. Replaces checkboxes where the
/// mockups show switches; the response's `changed()` works
/// like the stock checkbox's.
pub fn switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = egui::vec2(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let mut track = if *on {
            ACCENT
        } else {
            egui::Color32::from_rgb(0x36, 0x36, 0x38)
        };
        if !ui.is_enabled() {
            track = lerp_color(track, egui::Color32::from_rgb(0x4A, 0x4A, 0x4C), 0.5);
        } else if response.hovered() {
            track = lerp_color(track, egui::Color32::WHITE, 0.08);
        }
        painter.rect_filled(rect, 10.0, track);
        let knob_x = if *on {
            rect.right() - 10.0
        } else {
            rect.left() + 10.0
        };
        let knob = egui::pos2(knob_x, rect.center().y);
        painter.circle_filled(
            knob + egui::vec2(0.0, 1.0),
            7.0,
            egui::Color32::from_black_alpha(70),
        );
        painter.circle_filled(knob, 7.0, egui::Color32::WHITE);
        if response.has_focus() {
            paint_focus_ring(painter, rect, 10.0);
        }
    }
    response
}

/// Drawn line icons (0.6.0) — the mockups replace the ⚙ / ⬇ /
/// 📂 text glyphs with simple stroke icons. `scale` 1.0 ≈ a
/// 16px icon box around `center`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Gear,
    Download,
    Folder,
}

pub fn paint_icon(
    painter: &egui::Painter,
    kind: IconKind,
    center: egui::Pos2,
    scale: f32,
    color: egui::Color32,
    tray_color: egui::Color32,
) {
    let line = |a: egui::Pos2, b: egui::Pos2, w: f32, c: egui::Color32| {
        painter.line_segment([a, b], egui::Stroke::new(w, c));
    };
    let at = |dx: f32, dy: f32| center + egui::vec2(dx * scale, dy * scale);
    match kind {
        IconKind::Gear => {
            for k in 0..6 {
                let a = (k as f32 * 60.0 + 30.0).to_radians();
                line(
                    at(5.2 * a.cos(), 5.2 * a.sin()),
                    at(8.0 * a.cos(), 8.0 * a.sin()),
                    2.4 * scale,
                    color,
                );
            }
            painter.circle_stroke(center, 5.4 * scale, egui::Stroke::new(1.9 * scale, color));
            painter.circle_filled(center, 1.8 * scale, color);
        }
        IconKind::Download => {
            line(at(0.0, -7.0), at(0.0, 3.0), 2.2 * scale, color);
            line(at(-4.5, -1.0), at(0.0, 3.5), 2.2 * scale, color);
            line(at(0.0, 3.5), at(4.5, -1.0), 2.2 * scale, color);
            line(at(-7.0, 7.0), at(7.0, 7.0), 2.2 * scale, tray_color);
        }
        IconKind::Folder => {
            let pts = [
                at(-8.0, -5.0),
                at(-2.0, -5.0),
                at(1.0, -2.4),
                at(8.0, -2.4),
                at(8.0, 6.0),
                at(-8.0, 6.0),
                at(-8.0, -5.0),
            ];
            painter.add(egui::Shape::line(
                pts.to_vec(),
                egui::Stroke::new(1.8 * scale, color),
            ));
        }
    }
}

/// A ghost button with a drawn line icon (0.6.0): raised
/// CARD2 fill, hairline stroke, top highlight, icon at the
/// left, truncating label — the panel's Settings / Downloads
/// / Download-folder trio. With an empty label the icon is
/// centred (used for the Recent rows' folder button).
pub fn icon_text_button(
    ui: &mut egui::Ui,
    kind: IconKind,
    label: &str,
    size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = 9.0_f32.min(rect.height() / 2.0);
        let fill = if !ui.is_enabled() || response.is_pointer_button_down_on() {
            CARD
        } else if response.hovered() {
            CONTROL_BG_HOVER
        } else {
            CARD2
        };
        painter.rect_filled(rect, radius, fill);
        painter.rect_stroke(
            rect,
            egui::CornerRadius::same(radius as u8),
            egui::Stroke::new(
                1.0_f32,
                if response.hovered() {
                    STROKE_SOFT
                } else {
                    HAIRLINE
                },
            ),
            egui::epaint::StrokeKind::Inside,
        );
        paint_top_highlight(painter, rect, radius);
        if response.has_focus() {
            paint_focus_ring(painter, rect, radius);
        }
        let icon_color = if response.hovered() { TEXT } else { TEXT_DIM };
        if label.is_empty() {
            paint_icon(painter, kind, rect.center(), 0.85, icon_color, icon_color);
        } else {
            paint_icon(
                painter,
                kind,
                egui::pos2(rect.left() + 17.0, rect.center().y),
                0.85,
                icon_color,
                icon_color,
            );
            let galley = egui::WidgetText::from(egui::RichText::new(label).color(TEXT))
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Truncate),
                    (rect.width() - 37.0).max(0.0),
                    egui::TextStyle::Button,
                );
            painter.galley(
                egui::pos2(rect.left() + 29.0, rect.center().y - galley.size().y / 2.0),
                galley,
                TEXT,
            );
        }
    }
    response
}

/// A visible focus ring for text fields (0.6.0): the mockup's
/// link field carries an accent outline plus a faint deep-red
/// outer glow while focused. Painted strokes only — the field
/// underneath is untouched.
pub fn paint_focus_ring_for_field(ui: &egui::Ui, resp: &egui::Response) {
    if resp.has_focus() {
        let painter = ui.painter();
        painter.rect_stroke(
            resp.rect.expand(3.0),
            egui::CornerRadius::same(RADIUS + 3),
            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0x4A, 0x1B, 0x1F)),
            egui::epaint::StrokeKind::Outside,
        );
        painter.rect_stroke(
            resp.rect.expand(1.0),
            egui::CornerRadius::same(RADIUS + 1),
            egui::Stroke::new(2.0_f32, ACCENT),
            egui::epaint::StrokeKind::Outside,
        );
    }
}

/// A small status dot, PAINTED as a filled circle (0.5.1) —
/// never the "●" character: U+25CF is missing from egui's
/// default font, so the text version rendered as a hollow
/// square on Windows (v0.5.0 field captures). Allocates a
/// 10×10 box and paints the circle inside it.
pub fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _resp) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// A rounded status pill (0.5.0): coloured dot + strong main
/// text + dim suffix, inside a card-fill pill outline. Used for
/// the panel's monitoring status line, replacing the plain
/// outlined box the status used to sit in.
///
/// Width budget (0.5.1): the pill may never be wider than the
/// row it sits in — the 0.5.0 version had no budget and its
/// natural width (~450px at the theme's sizes) overflowed the
/// 400px panel and painted under the Settings drawer. The
/// suffix gets whatever width remains after the dot and main
/// text, and truncates inside the pill; the full line is on
/// hover over the pill.
pub fn status_pill(ui: &mut egui::Ui, dot: egui::Color32, main: &str, suffix: &str, tip: &str) {
    let budget = ui.available_width();
    egui::Frame::NONE
        .fill(PILL_BG)
        .stroke(egui::Stroke::new(1.0_f32, PILL_STROKE))
        .corner_radius(14)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_max_width((budget - 20.0).max(60.0));
            ui.horizontal(|ui| {
                status_dot(ui, dot);
                ui.label(egui::RichText::new(main).strong().color(TEXT));
                ui.label(egui::RichText::new("·").color(TEXT_FAINT));
                let remaining = ui.available_width();
                if remaining > 24.0 {
                    ui.allocate_ui_with_layout(
                        egui::vec2(remaining, 20.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(suffix).color(TEXT_DIM))
                                    .truncate(),
                            );
                        },
                    );
                }
            });
        })
        .response
        .on_hover_text(tip);
}

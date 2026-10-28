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
pub const HAIRLINE: egui::Color32 = egui::Color32::from_rgb(0x2B, 0x2B, 0x2C);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xE9, 0xE9, 0xE9);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0xA0, 0xA0, 0xA0);
pub const TEXT_FAINT: egui::Color32 = egui::Color32::from_rgb(0x6E, 0x6E, 0x6E);
pub const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(0x4C, 0xC3, 0x6B);
/// Card fill: panels-within-the-panel (0.5.0 polish pass).
pub const CARD: egui::Color32 = egui::Color32::from_rgb(0x1A, 0x1A, 0x1A);
/// Control fill at rest / on hover (0.5.0; the rest fill is
/// CARD2 since 0.6.0 — controls sit one step above the cards).
pub const CONTROL_BG: egui::Color32 = CARD2;
pub const CONTROL_BG_HOVER: egui::Color32 = egui::Color32::from_rgb(0x28, 0x28, 0x2A);
/// Raised surface (0.6.0 premium pass): one step lighter than
/// CARD — buttons, chips, value readouts, anything that sits
/// ON a card instead of being the card.
pub const CARD2: egui::Color32 = egui::Color32::from_rgb(0x1E, 0x1E, 0x1F);
/// Recessed surface (0.6.0): darker than the panel itself —
/// wells, text fields, the segmented-control track, the panel
/// footer bar. Depth comes from pairing this with CARD/CARD2.
pub const WELL: egui::Color32 = egui::Color32::from_rgb(0x0B, 0x0B, 0x0C);
/// Darker outline for recessed surfaces (0.6.0).
pub const STROKE_DARK: egui::Color32 = egui::Color32::from_rgb(0x1D, 0x1D, 0x1E);
/// The 1px light line painted along a card's top edge
/// (0.6.0) — the cheapest depth cue egui can draw.
pub const TOP_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(0x42, 0x42, 0x44);
/// Gradient ends for primary buttons / selected segments
/// (0.6.0): light accent at the top, deep red at the bottom.
pub const ACCENT_LIGHT: egui::Color32 = egui::Color32::from_rgb(0xF0, 0x60, 0x66);
pub const ACCENT_DEEP: egui::Color32 = egui::Color32::from_rgb(0x94, 0x16, 0x1C);
/// Gradient top for the selected format chip (0.6.0); its
/// bottom is ACCENT_DARK.
pub const CHIP_GRAD_TOP: egui::Color32 = egui::Color32::from_rgb(0x96, 0x1E, 0x24);
/// Header-band gradient ends (0.6.0): near-black at the top,
/// a subtle deep-red tint at the bottom edge.
pub const BAND_TOP: egui::Color32 = egui::Color32::from_rgb(0x13, 0x12, 0x14);
pub const BAND_BOTTOM: egui::Color32 = egui::Color32::from_rgb(0x2B, 0x0F, 0x10);
pub const BAND_EDGE: egui::Color32 = egui::Color32::from_rgb(0x40, 0x22, 0x24);
/// Status pill colours inside the header band (0.6.0).
pub const PILL_BG: egui::Color32 = egui::Color32::from_rgb(0x12, 0x12, 0x14);
pub const PILL_STROKE: egui::Color32 = egui::Color32::from_rgb(0x46, 0x34, 0x36);
/// Slightly stronger outline than HAIRLINE, for interactive
/// controls on hover so they read as touchable (0.5.0).
pub const STROKE_SOFT: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3A, 0x3A);
/// Tinted fill for the panel failure banner (0.5.0; was an
/// inline colour in the panel before).
pub const DANGER_BG: egui::Color32 = egui::Color32::from_rgb(0x2A, 0x0E, 0x0E);
/// Corner-radius tokens (0.5.0): cards/windows/widgets share
/// the large radius; egui applies ONE radius per widget state,
/// so small controls inherit it too (noted in DESIGN.md).
pub const RADIUS: u8 = 8;
/// Default inner size of the standalone Settings window
/// (0.5.1; Settings was a slide-out drawer in 0.2.1–0.5.0).
pub const SETTINGS_WINDOW_SIZE: [f32; 2] = [600.0, 640.0];

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

/// A card with a section heading on top (0.5.0). Returns
/// whatever the body closure returns.
pub fn section_card<R>(
    ui: &mut egui::Ui,
    heading: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    premium_card(ui, |ui| {
        ui.label(section_heading(heading));
        ui.add_space(6.0);
        add_contents(ui)
    })
}

/// A focus ring for custom-painted controls (0.6.0): an
/// accent outline just outside the rect, matching the ring
/// egui draws around focused stock widgets in spirit.
pub fn paint_focus_ring(painter: &egui::Painter, rect: egui::Rect, radius: f32) {
    painter.rect_stroke(
        rect.expand(1.5),
        egui::CornerRadius::same((radius + 1.5) as u8),
        egui::Stroke::new(1.5_f32, ACCENT_LIGHT),
        egui::epaint::StrokeKind::Outside,
    );
}

/// The primary action button (0.5.0), rebuilt 0.6.0 as a
/// custom-painted pill: vertical gradient (ACCENT_LIGHT →
/// ACCENT_DEEP), a soft deep-red glow underneath, a light
/// top edge, strong white text — the one button in a view
/// that should draw the eye (Download, Hide the icon…).
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
    /// Label size override (chips render at 12.5, buttons at
    /// the theme's Button size).
    text_size: Option<f32>,
}

pub fn primary_button(text: &str) -> GradientButton {
    GradientButton {
        text: text.to_string(),
        min_size: egui::Vec2::ZERO,
        top: ACCENT_LIGHT,
        bottom: ACCENT_DEEP,
        glow: true,
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
        let rich = egui::RichText::new(&self.text)
            .strong()
            .color(egui::Color32::WHITE);
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
                let grey = egui::Color32::from_rgb(0x4A, 0x4A, 0x4C);
                top = lerp_color(top, grey, 0.55);
                bottom = lerp_color(bottom, grey, 0.55);
            } else if response.is_pointer_button_down_on() {
                top = lerp_color(top, egui::Color32::BLACK, 0.18);
                bottom = lerp_color(bottom, egui::Color32::BLACK, 0.18);
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
            painter.galley(
                rect.center() - galley.size() / 2.0,
                galley,
                egui::Color32::WHITE,
            );
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

/// One segment of a segmented control (0.6.0). Returns the
/// click response; painting matches the mockups: selected =
/// red gradient + top light edge, unselected = dim text that
/// brightens on hover.
fn segment(ui: &mut egui::Ui, label: &str, selected: bool, size: egui::Vec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = 9.0_f32.min(rect.height() / 2.0);
        if selected {
            paint_soft_glow(painter, rect, radius, ACCENT_DARK);
            let (mut top, mut bottom) = (ACCENT_LIGHT, ACCENT_DEEP);
            if response.is_pointer_button_down_on() {
                top = lerp_color(top, egui::Color32::BLACK, 0.15);
                bottom = lerp_color(bottom, egui::Color32::BLACK, 0.15);
            }
            paint_vertical_gradient(painter, rect, radius, top, bottom);
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
        } else if response.hovered() {
            painter.rect_filled(rect, radius, CONTROL_BG_HOVER);
        }
        if response.has_focus() && !selected {
            paint_focus_ring(painter, rect, radius);
        }
        let rich = egui::RichText::new(label)
            .color(if selected {
                egui::Color32::WHITE
            } else {
                TEXT_DIM
            })
            .size(12.5);
        let rich = if selected { rich.strong() } else { rich };
        let galley = egui::WidgetText::from(rich).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (rect.width() - 12.0).max(0.0),
            egui::TextStyle::Button,
        );
        painter.galley(
            rect.center() - galley.size() / 2.0,
            galley,
            if selected {
                egui::Color32::WHITE
            } else {
                TEXT_DIM
            },
        );
    }
    response
}

/// The Settings tab bar (0.6.0): a recessed well track with
/// four equal segments; the selected one is filled with the
/// red gradient. Returns true when the tab changed.
pub fn settings_tab_bar(ui: &mut egui::Ui, tab: &mut usize) -> bool {
    let labels = ["General", "Downloads", "Domains & rules", "yt-dlp"];
    let mut changed = false;
    egui::Frame::NONE
        .fill(WELL)
        .stroke(egui::Stroke::new(1.0_f32, STROKE_DARK))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(4))
        .outer_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 0,
            bottom: 8,
        })
        .show(ui, |ui| {
            let share =
                (ui.available_width() - 3.0 * ui.spacing().item_spacing.x) / labels.len() as f32;
            ui.horizontal(|ui| {
                for (i, name) in labels.iter().enumerate() {
                    if segment(ui, name, *tab == i, egui::vec2(share, 30.0))
                        .on_hover_text("Switch settings section")
                        .clicked()
                    {
                        *tab = i;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// A two-or-more-way segmented choice on a recessed track
/// (0.6.0) — the General tab's Default action, which the
/// mockup draws as a small segmented control instead of the
/// old radio pair. `tips` carries each option's tooltip, so
/// nothing the radios explained is lost. Returns the new
/// index when the user picks a different segment.
pub fn segmented_choice(
    ui: &mut egui::Ui,
    labels: &[&str],
    tips: &[&str],
    current: usize,
    width: f32,
) -> Option<usize> {
    let mut picked = None;
    egui::Frame::NONE
        .fill(WELL)
        .stroke(egui::Stroke::new(1.0_f32, STROKE_DARK))
        .corner_radius(9)
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.set_width(width);
            let share = (ui.available_width()
                - (labels.len() - 1) as f32 * ui.spacing().item_spacing.x)
                / labels.len() as f32;
            ui.horizontal(|ui| {
                for (i, name) in labels.iter().enumerate() {
                    let resp = segment(ui, name, current == i, egui::vec2(share, 22.0));
                    let resp = match tips.get(i) {
                        Some(tip) => resp.on_hover_text(*tip),
                        None => resp,
                    };
                    if resp.clicked() && current != i {
                        picked = Some(i);
                    }
                }
            });
        });
    picked
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
            egui::Stroke::new(1.5_f32, ACCENT),
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

/// The central theme (0.5.0 polish pass). Before 0.5.0 this
/// only swapped colours onto egui's defaults — square corners,
/// default spacing, default type sizes — which is why the app
/// read as "default egui". Now it sets the full style: a type
/// scale, a 4/8 spacing grid, corner radii, widget visuals for
/// every interaction state, window shadow/stroke, and slider /
/// scrollbar / progress styling (those widgets all read their
/// look from these visuals and spacing values).
pub fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    // Type scale: Heading 20 / Body 14 / Button 14 / Small 12,
    // keeping each style's existing font family.
    let resized = |styles: &std::collections::BTreeMap<egui::TextStyle, egui::FontId>,
                   key: &egui::TextStyle,
                   size: f32|
     -> egui::FontId {
        styles
            .get(key)
            .map(|f| egui::FontId::new(size, f.family.clone()))
            .unwrap_or_else(|| egui::FontId::new(size, egui::FontFamily::Proportional))
    };
    let old = &style.text_styles;
    style.text_styles = [
        (
            egui::TextStyle::Heading,
            resized(old, &egui::TextStyle::Heading, 20.0),
        ),
        (
            egui::TextStyle::Body,
            resized(old, &egui::TextStyle::Body, 14.0),
        ),
        (
            egui::TextStyle::Button,
            resized(old, &egui::TextStyle::Button, 14.0),
        ),
        (
            egui::TextStyle::Small,
            resized(old, &egui::TextStyle::Small, 12.0),
        ),
        (
            egui::TextStyle::Monospace,
            resized(old, &egui::TextStyle::Monospace, 13.0),
        ),
    ]
    .into();

    // Spacing on a 4/8 grid.
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size = egui::vec2(40.0, 24.0);
    style.spacing.indent = 18.0;
    style.spacing.icon_width = 18.0;
    style.spacing.icon_spacing = 6.0;
    style.spacing.slider_width = 150.0;
    // Thin slider track (0.6.0): the mockups' 4px rail with a
    // round thumb, instead of egui's chunky 8px default rail.
    style.spacing.slider_rail_height = 4.0;
    style.spacing.combo_width = 150.0;
    style.spacing.text_edit_width = 220.0;
    style.spacing.tooltip_width = 320.0;
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.handle_min_length = 24.0;

    // Widget + window visuals.
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL_BG;
    visuals.window_fill = PANEL_BG;
    // Recessed fields (text edits, combo boxes) sit in a well
    // (0.6.0) — darker than the panel around them.
    visuals.extreme_bg_color = WELL;
    visuals.faint_bg_color = CARD;
    visuals.code_bg_color = CARD;
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = egui::Color32::from_rgb(0xE5, 0xA0, 0x3C);
    visuals.error_fg_color = ACCENT;
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.menu_corner_radius = egui::CornerRadius::same(RADIUS);
    visuals.window_stroke = egui::Stroke::new(1.0_f32, HAIRLINE);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: egui::Color32::from_black_alpha(96),
    };
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: egui::Color32::from_black_alpha(80),
    };
    // Sliders show their filled portion in the selection
    // colour (the bright accent below, since 0.6.0).
    visuals.slider_trailing_fill = true;
    // Round slider thumbs (0.6.0).
    visuals.handle_shape = egui::style::HandleShape::Circle;

    let wv = |bg: egui::Color32, stroke: egui::Color32| egui::style::WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: bg,
        bg_stroke: egui::Stroke::new(1.0_f32, stroke),
        corner_radius: egui::CornerRadius::same(RADIUS),
        fg_stroke: egui::Stroke::new(1.0_f32, TEXT),
        expansion: 0.0,
    };
    visuals.widgets.noninteractive = wv(PANEL_BG, HAIRLINE);
    visuals.widgets.inactive = wv(CONTROL_BG, HAIRLINE);
    visuals.widgets.hovered = wv(CONTROL_BG_HOVER, STROKE_SOFT);
    visuals.widgets.active = wv(ACCENT_DARK, ACCENT_DARK);
    visuals.widgets.open = wv(CONTROL_BG_HOVER, STROKE_SOFT);
    // Selection is the bright accent since 0.6.0: it drives
    // the slider's filled trail, progress-bar fills, selected
    // combo entries and text selection — the mockups' red is
    // the bright one, not the deep red 0.5.0 used here.
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, TEXT);

    style.visuals = visuals;
    ctx.set_style(style);
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
/// window, about).
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
            tray_autostart_checked: None,
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
        // Autostart checkmark (0.5.1): owned here, in app
        // context, never in the tray handler (see
        // handle_tray_menu_direct). Read back from the registry
        // (the source of truth) and pushed only on change, so
        // a handler-side toggle shows up within a pass or two
        // once the app adopts the handler's settings save.
        let autostart_now = platform::Platform::autostart_enabled();
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

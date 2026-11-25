//! Shared palette, icon and egui theme.

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0xFB, 0x49, 0x34);
pub const ACCENT_DARK: egui::Color32 = egui::Color32::from_rgb(0x3B, 0x1D, 0x1B);
pub const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(0x1D, 0x20, 0x21);
pub const HAIRLINE: egui::Color32 = egui::Color32::from_rgb(0x3C, 0x38, 0x36);
pub const TEXT_INVERSE: egui::Color32 = egui::Color32::from_rgb(0x1D, 0x20, 0x21);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xEB, 0xDB, 0xB2);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0xD5, 0xC4, 0xA1);
pub const TEXT_FAINT: egui::Color32 = egui::Color32::from_rgb(0xA8, 0x99, 0x84);
pub const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(0xB8, 0xBB, 0x26);
/// Card fill: panels-within-the-panel (0.5.0 polish pass).
pub const CARD: egui::Color32 = egui::Color32::from_rgb(0x28, 0x28, 0x28);
/// Control fill at rest / on hover (0.5.0; the rest fill is
/// CARD2 since 0.6.0 — controls sit one step above the cards).
pub const CONTROL_BG: egui::Color32 = CARD2;
pub const CONTROL_BG_HOVER: egui::Color32 = egui::Color32::from_rgb(0x3C, 0x38, 0x36);
/// Raised surface (0.6.0 premium pass): one step lighter than
/// CARD — buttons, chips, value readouts, anything that sits
/// ON a card instead of being the card.
pub const CARD2: egui::Color32 = egui::Color32::from_rgb(0x32, 0x30, 0x2F);
/// Recessed surface (0.6.0): darker than the panel itself —
/// wells, text fields, the segmented-control track, the panel
/// footer bar. Depth comes from pairing this with CARD/CARD2.
pub const WELL: egui::Color32 = egui::Color32::from_rgb(0x17, 0x18, 0x19);
/// Darker outline for recessed surfaces (0.6.0).
pub const STROKE_DARK: egui::Color32 = egui::Color32::from_rgb(0x3C, 0x38, 0x36);
/// The 1px light line painted along a card's top edge
/// (0.6.0) — the cheapest depth cue egui can draw.
pub const TOP_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(0x50, 0x49, 0x45);
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
pub const STROKE_SOFT: egui::Color32 = egui::Color32::from_rgb(0x50, 0x49, 0x45);
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
    visuals.warn_fg_color = egui::Color32::from_rgb(0xFA, 0xBD, 0x2F);
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
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, TEXT_INVERSE);

    style.visuals = visuals;
    ctx.set_style(style);
}

#[cfg(test)]
mod theme_contrast_tests {
    use super::*;
    use crate::gui::lerp_color;
    fn luminance(color: egui::Color32) -> f64 {
        let linear = |v: u8| {
            let c = f64::from(v) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }
    fn contrast(a: egui::Color32, b: egui::Color32) -> f64 {
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn core_dark_text_pairs_and_primary_button_meet_body_contrast() {
        for (foreground, background) in [
            (TEXT, PANEL_BG),
            (TEXT, CARD),
            (TEXT, CARD2),
            (TEXT_DIM, CARD),
            (TEXT_FAINT, CARD2),
            (TEXT_INVERSE, ACCENT),
        ] {
            assert!(
                contrast(foreground, background) >= 4.5,
                "contrast {:.2}",
                contrast(foreground, background)
            );
        }
        for amount in [0.10, 0.12] {
            assert!(
                contrast(
                    TEXT_INVERSE,
                    lerp_color(ACCENT, egui::Color32::WHITE, amount)
                ) >= 4.5
            );
        }
    }
    #[test]
    fn theme_installs_shared_palette() {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let visuals = &ctx.style().visuals;
        assert_eq!(visuals.panel_fill, PANEL_BG);
        assert_eq!(visuals.widgets.inactive.bg_fill, CARD2);
        assert_eq!(visuals.selection.bg_fill, ACCENT);
        assert_eq!(visuals.selection.stroke.color, TEXT_INVERSE);
    }
}

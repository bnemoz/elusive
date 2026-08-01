//! The one place where design tokens become egui types.
//!
//! Widgets ask this module for colours and styles; they never construct a
//! `Color32` from a hex literal. That keeps `DESIGN_SYSTEM.md` rule #7 checkable
//! by grep, and means a token change propagates everywhere at once.

use crate::theme::{
    self, chart, color, control, radius, spacing, stroke, typography, Rgb, Rgba, Theme,
};
use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// Token → egui colour.
pub fn c(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

/// Token → egui colour, with alpha.
pub fn ca(rgba: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(rgba.r, rgba.g, rgba.b, rgba.a)
}

/// Same colour at a chosen alpha, for overlays that must not hide the trace.
pub fn c_alpha(rgb: Rgb, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(rgb.r, rgb.g, rgb.b, alpha)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
    /// Follow the operating system's preference.
    System,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Dark => "Dark",
            Mode::Light => "Light",
            Mode::System => "Follow OS",
        }
    }

    /// Resolve to a concrete theme, consulting the OS when asked to.
    pub fn resolve(self, ctx: &egui::Context) -> Theme {
        match self {
            Mode::Dark => theme::DARK,
            Mode::Light => theme::LIGHT,
            Mode::System => match ctx.system_theme() {
                Some(egui::Theme::Light) => theme::LIGHT,
                _ => theme::DARK,
            },
        }
    }
}

/// Apply the full EluSive style to a context.
pub fn apply(ctx: &egui::Context, t: Theme) {
    ctx.set_visuals(visuals(t));

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(spacing::SM, spacing::SM);
        style.spacing.button_padding = egui::vec2(spacing::MD, spacing::SM);
        style.spacing.interact_size.y = control::HEIGHT_COMPACT;
        style.spacing.window_margin = egui::Margin::same(spacing::LG as i8);
        style.text_styles = text_styles();
        // Rule #5: animation never delays an analytical action. Keep transitions
        // short enough that a click feels immediate.
        style.animation_time = 0.05;
    });
}

fn text_styles() -> std::collections::BTreeMap<TextStyle, FontId> {
    use std::collections::BTreeMap;
    let mut m = BTreeMap::new();
    m.insert(
        TextStyle::Heading,
        FontId::new(typography::H2_PX, FontFamily::Proportional),
    );
    m.insert(
        TextStyle::Body,
        FontId::new(typography::BODY_PX, FontFamily::Proportional),
    );
    m.insert(
        TextStyle::Button,
        FontId::new(typography::BODY_PX, FontFamily::Proportional),
    );
    m.insert(
        TextStyle::Small,
        FontId::new(typography::SMALL_PX, FontFamily::Proportional),
    );
    m.insert(
        TextStyle::Monospace,
        FontId::new(typography::CODE_PX, FontFamily::Monospace),
    );
    m
}

/// Font ids for the roles named in `DESIGN_SYSTEM.md` §2.
pub fn font_display() -> FontId {
    FontId::new(typography::DISPLAY_PX, FontFamily::Proportional)
}
pub fn font_h1() -> FontId {
    FontId::new(typography::H1_PX, FontFamily::Proportional)
}
pub fn font_h3() -> FontId {
    FontId::new(typography::H3_PX, FontFamily::Proportional)
}
pub fn font_micro() -> FontId {
    FontId::new(typography::MICRO_PX, FontFamily::Proportional)
}
/// Monospace, for paths, identifiers, and exact analytical values.
pub fn font_code() -> FontId {
    FontId::new(typography::CODE_PX, FontFamily::Monospace)
}

fn visuals(t: Theme) -> Visuals {
    let mut v = if t.is_dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.dark_mode = t.is_dark;
    v.panel_fill = c(t.app_bg);
    v.window_fill = c(t.panel_bg);
    v.faint_bg_color = c(t.panel_elevated);
    v.extreme_bg_color = c(if t.is_dark {
        color::INK_950
    } else {
        color::WHITE
    });
    v.override_text_color = Some(c(t.text_primary));
    v.hyperlink_color = c(t.accent);
    v.window_stroke = Stroke::new(stroke::HAIRLINE, c(t.border));
    v.window_corner_radius = CornerRadius::same(radius::MD as u8);
    v.selection.bg_fill = c_alpha(t.accent, 72);
    v.selection.stroke = Stroke::new(stroke::HAIRLINE, c(t.accent));

    // §5 says prefer a border plus surface contrast over shadows.
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;

    let radius = CornerRadius::same(radius::SM as u8);
    v.widgets.noninteractive.bg_stroke = Stroke::new(stroke::HAIRLINE, c(t.border));
    v.widgets.noninteractive.fg_stroke = Stroke::new(stroke::HAIRLINE, c(t.text_secondary));
    v.widgets.noninteractive.corner_radius = radius;

    v.widgets.inactive.bg_fill = c(t.panel_elevated);
    v.widgets.inactive.weak_bg_fill = c(t.panel_elevated);
    v.widgets.inactive.bg_stroke = Stroke::new(stroke::HAIRLINE, c(t.border));
    v.widgets.inactive.fg_stroke = Stroke::new(stroke::HAIRLINE, c(t.text_primary));
    v.widgets.inactive.corner_radius = radius;

    v.widgets.hovered.bg_fill = c(t.selection);
    v.widgets.hovered.weak_bg_fill = c(t.selection);
    v.widgets.hovered.bg_stroke = Stroke::new(stroke::HAIRLINE, c(t.accent_hover));
    v.widgets.hovered.fg_stroke = Stroke::new(stroke::HAIRLINE, c(t.text_primary));
    v.widgets.hovered.corner_radius = radius;

    v.widgets.active.bg_fill = c(t.accent);
    v.widgets.active.weak_bg_fill = c(t.accent);
    v.widgets.active.bg_stroke = Stroke::new(stroke::HAIRLINE, c(t.accent));
    v.widgets.active.fg_stroke = Stroke::new(
        stroke::HAIRLINE,
        c(if t.is_dark {
            color::INK_950
        } else {
            color::WHITE
        }),
    );
    v.widgets.active.corner_radius = radius;

    // Focus ring: 2 px, per §5.
    v.widgets.open.bg_stroke = Stroke::new(stroke::FOCUS, c(t.focus_ring));

    v
}

/// A card surface: border plus fill, no shadow (§5).
pub fn card(t: Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(c(t.panel_bg))
        .stroke(Stroke::new(stroke::HAIRLINE, c(t.border)))
        .corner_radius(CornerRadius::same(radius::MD as u8))
        .inner_margin(spacing::LG as i8)
}

/// The dark navigation rail, which stays `INK_900` in both themes (§6).
pub fn nav_frame(t: Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(c(t.nav_bg))
        .inner_margin(spacing::MD as i8)
}

/// Map a channel's dash pattern onto egui_plot's line style.
pub fn line_style(dash: chart::Dash) -> egui_plot::LineStyle {
    match dash {
        chart::Dash::Solid => egui_plot::LineStyle::Solid,
        chart::Dash::Dashed => egui_plot::LineStyle::Dashed { length: 6.0 },
        chart::Dash::Dotted => egui_plot::LineStyle::Dotted { spacing: 4.0 },
    }
}

/// Install Inter and JetBrains Mono when they are present in `assets/fonts/`.
///
/// The fonts are not vendored in this repository (they carry their own licences),
/// so this degrades to egui's bundled faces rather than failing to start. The
/// design system names a fallback stack for exactly this case (§2).
pub fn install_fonts(ctx: &egui::Context) -> Vec<String> {
    let mut fonts = egui::FontDefinitions::default();
    let mut missing = Vec::new();

    let candidates: [(&str, FontFamily, &[&str]); 2] = [
        (
            "Inter",
            FontFamily::Proportional,
            &["Inter-Regular.ttf", "Inter.ttf", "InterVariable.ttf"],
        ),
        (
            "JetBrainsMono",
            FontFamily::Monospace,
            &["JetBrainsMono-Regular.ttf", "JetBrainsMono.ttf"],
        ),
    ];

    for (name, family, files) in candidates {
        let mut loaded = false;
        for file in files {
            let path = std::path::Path::new("assets/fonts").join(file);
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            fonts.font_data.insert(
                name.to_string(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(family.clone())
                .or_default()
                .insert(0, name.to_string());
            loaded = true;
            break;
        }
        if !loaded {
            missing.push(name.to_string());
        }
    }

    ctx.set_fonts(fonts);
    missing
}

/// Format a number for an analytical column: fixed decimals so digits line up by
/// place value, not by chance (rule #4).
pub fn num(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    format!("{value:.decimals$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_conversion_preserves_the_channel_values() {
        assert_eq!(c(color::BLUE_600), Color32::from_rgb(0x32, 0x74, 0xBD));
        assert_eq!(
            ca(color::INTEGRATED_AREA),
            Color32::from_rgba_unmultiplied(0x9F, 0xC7, 0xEE, 0x66)
        );
    }

    #[test]
    fn integration_overlays_stay_translucent() {
        // Rule #2: the raw trace must remain visible beneath integrations.
        // Checked at compile time, so retuning a token cannot slip past.
        const { assert!(color::INTEGRATED_AREA.a < 128) };
        const { assert!(color::EXCLUDED_REGION.a < 128) };
    }

    #[test]
    fn numbers_format_with_stable_width() {
        assert_eq!(num(1.5, 3), "1.500");
        assert_eq!(num(-0.25, 2), "-0.25");
        assert_eq!(num(f64::NAN, 2), "—");
        assert_eq!(num(f64::INFINITY, 2), "—");
    }

    #[test]
    fn every_dash_token_maps_to_a_distinct_plot_style() {
        let styles = [
            line_style(chart::Dash::Solid),
            line_style(chart::Dash::Dashed),
            line_style(chart::Dash::Dotted),
        ];
        assert_ne!(styles[0], styles[1]);
        assert_ne!(styles[1], styles[2]);
    }
}

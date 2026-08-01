//! EluSive design tokens — the dependency-free equivalent of CSS variables.
//!
//! **Toolkit-neutral by design.** This module must not import egui; the mapping
//! onto egui types lives in [`crate::egui_adapter`]. That separation is what lets
//! `DESIGN_SYSTEM.md` stay the single source of truth: widgets consume named
//! tokens, never raw hex, so rule #7 ("new colours require a named token and a
//! documented purpose") is enforceable by reading this one file.
//!
//! Version: 1.1.0, tracking `DESIGN_SYSTEM.md`.

// This module is a catalogue, not a call graph. A token exists because
// `DESIGN_SYSTEM.md` defines it; an unused one is a token waiting for the widget
// that needs it, not dead code to delete.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// Parse a hand-typed sRGB colour: `#RRGGBB` or `RRGGBB`.
    ///
    /// Returns `None` on anything else rather than falling back to a colour, so a
    /// caller editing a text field can leave a half-typed value alone instead of
    /// flashing a shade the user never asked for.
    pub fn from_hex_str(s: &str) -> Option<Rgb> {
        let s = s.trim();
        let digits = s.strip_prefix('#').unwrap_or(s);
        // `u32::from_str_radix` accepts a leading `+`, so `+12345` would pass a
        // length-only check and parse to a colour the user did not type. The
        // digits are therefore validated explicitly.
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(digits, 16).ok()?;
        Some(Rgb::new(
            ((v >> 16) & 0xFF) as u8,
            ((v >> 8) & 0xFF) as u8,
            (v & 0xFF) as u8,
        ))
    }

    /// Canonical `#RRGGBB` form, for a hex field the user can read back and copy.
    pub fn hex_string(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Blend towards `other` by `t` in 0..=1, in linear-ish sRGB space.
    ///
    /// Used for the plate ramp, where the interpolation has to be monotonic in
    /// luminance for the map to stay colourblind-safe (`DESIGN_SYSTEM.md` §10.3).
    pub fn lerp(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| -> u8 {
            let a = (a as f32 / 255.0).powf(2.2);
            let b = (b as f32 / 255.0).powf(2.2);
            (((a + (b - a) * t).powf(1.0 / 2.2)) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Rgb::new(
            mix(self.r, other.r),
            mix(self.g, other.g),
            mix(self.b, other.b),
        )
    }

    /// WCAG relative luminance, for the contrast checks in [`chart::series_color`].
    pub fn relative_luminance(self) -> f64 {
        fn c(v: u8) -> f64 {
            let v = v as f64 / 255.0;
            if v <= 0.039_28 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * c(self.r) + 0.7152 * c(self.g) + 0.0722 * c(self.b)
    }

    pub fn contrast_ratio(self, other: Rgb) -> f64 {
        let (a, b) = (self.relative_luminance(), other.relative_luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

pub mod color {
    use super::{Rgb, Rgba};

    pub const INK_950: Rgb = Rgb::new(0x07, 0x11, 0x1F);
    pub const INK_900: Rgb = Rgb::new(0x0D, 0x1B, 0x2A);
    pub const INK_800: Rgb = Rgb::new(0x14, 0x28, 0x3B);
    pub const INK_700: Rgb = Rgb::new(0x20, 0x3A, 0x52);

    pub const BLUE_700: Rgb = Rgb::new(0x24, 0x5A, 0x9A);
    pub const BLUE_600: Rgb = Rgb::new(0x32, 0x74, 0xBD);
    pub const BLUE_500: Rgb = Rgb::new(0x4C, 0x8F, 0xD8);
    pub const BLUE_300: Rgb = Rgb::new(0x9F, 0xC7, 0xEE);

    pub const ICE_100: Rgb = Rgb::new(0xEA, 0xF4, 0xFC);
    pub const MIST_50: Rgb = Rgb::new(0xF7, 0xFA, 0xFD);
    pub const WHITE: Rgb = Rgb::new(0xFF, 0xFF, 0xFF);

    pub const SLATE_700: Rgb = Rgb::new(0x40, 0x56, 0x6C);
    pub const SLATE_500: Rgb = Rgb::new(0x6E, 0x81, 0x93);
    pub const SLATE_300: Rgb = Rgb::new(0xB8, 0xC5, 0xD1);
    pub const SLATE_200: Rgb = Rgb::new(0xD7, 0xE1, 0xEA);
    pub const SLATE_100: Rgb = Rgb::new(0xE8, 0xEE, 0xF3);

    pub const SUCCESS_600: Rgb = Rgb::new(0x26, 0x7B, 0x70);
    pub const WARNING_600: Rgb = Rgb::new(0xA9, 0x6B, 0x19);
    pub const DANGER_600: Rgb = Rgb::new(0xB4, 0x45, 0x55);
    pub const INFO_600: Rgb = Rgb::new(0x3B, 0x68, 0xB2);

    /// Kept translucent so the raw trace stays visible beneath it (rule #2).
    pub const INTEGRATED_AREA: Rgba = Rgba::new(0x9F, 0xC7, 0xEE, 0x66);
    pub const EXCLUDED_REGION: Rgba = Rgba::new(0xB4, 0x45, 0x55, 0x26);
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub app_bg: Rgb,
    pub panel_bg: Rgb,
    pub panel_elevated: Rgb,
    pub nav_bg: Rgb,
    pub nav_active: Rgb,
    pub text_primary: Rgb,
    pub text_secondary: Rgb,
    pub border: Rgb,
    pub accent: Rgb,
    pub accent_hover: Rgb,
    pub focus_ring: Rgb,
    pub grid: Rgb,
    pub axis: Rgb,
    /// Fill for a highlighted fraction span on the trace (`DESIGN_SYSTEM.md` §10.2).
    pub fraction_highlight: Rgb,
    pub selection: Rgb,
    pub is_dark: bool,
}

pub const LIGHT: Theme = Theme {
    app_bg: color::MIST_50,
    panel_bg: color::WHITE,
    panel_elevated: color::WHITE,
    nav_bg: color::INK_900,
    nav_active: color::INK_800,
    text_primary: color::INK_950,
    text_secondary: color::SLATE_700,
    border: color::SLATE_200,
    accent: color::BLUE_700,
    accent_hover: color::BLUE_600,
    focus_ring: color::BLUE_500,
    grid: color::SLATE_200,
    axis: color::SLATE_500,
    fraction_highlight: color::ICE_100,
    selection: color::ICE_100,
    is_dark: false,
};

pub const DARK: Theme = Theme {
    app_bg: color::INK_950,
    panel_bg: color::INK_900,
    panel_elevated: color::INK_800,
    nav_bg: color::INK_900,
    nav_active: color::INK_800,
    text_primary: color::WHITE,
    text_secondary: color::BLUE_300,
    border: color::INK_700,
    accent: color::BLUE_500,
    accent_hover: color::BLUE_300,
    focus_ring: color::BLUE_300,
    grid: color::INK_700,
    axis: color::SLATE_300,
    fraction_highlight: color::INK_800,
    selection: color::INK_800,
    is_dark: true,
};

pub mod typography {
    pub const UI_FONT_STACK: &[&str] = &["Inter", "Segoe UI", "Noto Sans", "Arial", "sans-serif"];
    pub const CODE_FONT_STACK: &[&str] = &[
        "JetBrains Mono",
        "Cascadia Mono",
        "SFMono-Regular",
        "Consolas",
        "monospace",
    ];

    pub const DISPLAY_PX: f32 = 32.0;
    pub const H1_PX: f32 = 24.0;
    pub const H2_PX: f32 = 20.0;
    pub const H3_PX: f32 = 16.0;
    pub const BODY_PX: f32 = 14.0;
    pub const SMALL_PX: f32 = 12.0;
    pub const MICRO_PX: f32 = 11.0;
    pub const CODE_PX: f32 = 13.0;
}

pub mod spacing {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
    pub const XXXL: f32 = 48.0;
}

/// Reading measures — how wide a block of prose or a label/value form may grow
/// before it stops being readable (`DESIGN_SYSTEM.md` §5).
///
/// A card that fills the viewport is fine for a chromatogram and wrong for a
/// form. On a 4K monitor a full-width card is ~3800 px, and a field whose label
/// sits at the left edge and whose value sits at the right edge puts twelve
/// pixels of meaning two feet apart: the eye has nothing to track along, so the
/// row stops reading as a pair. Capping the *content* width and centring the
/// remainder is the fix.
pub mod measure {
    /// Widest a label/value form may grow. Around 800 px is the conventional
    /// upper end of a comfortable measure for 14 px body text.
    pub const FORM_MAX: f32 = 800.0;

    /// Narrowest a form may be squeezed to and still hold its widest content —
    /// for the calibration panel that is the standard/MW/Ve point table. Below
    /// this width the form takes the whole window instead of being capped, so a
    /// small window degrades to full width rather than to a squeezed column.
    pub const FORM_MIN: f32 = 480.0;

    // A cap tighter than the usability floor would make the cap the bug.
    const _: () = assert!(FORM_MAX > FORM_MIN);

    /// Width reserved for the label side of a [`crate::widgets::panels::field`]
    /// row. Fixed rather than measured so every field in the app aligns on the
    /// same x, and so the column does not jitter as values change.
    pub const FIELD_LABEL: f32 = 168.0;

    /// Content width for a form, given the width its container offers.
    ///
    /// Width is the safe axis to constrain: unlike height it cannot feed back
    /// into a parent's size (see `widgets::chromatogram::data_y_range` for the
    /// loop this avoids). Never let a measured content *height* reach this.
    pub fn content_width(available: f32) -> f32 {
        if !available.is_finite() || available <= 0.0 {
            return 0.0;
        }
        available.min(FORM_MAX)
    }

    /// Space to insert ahead of the content so it sits centred in `available`.
    ///
    /// Centring rather than pinning left matters because the cap is visible: a
    /// left-pinned 800 px card on a 4K window reads as a rendering failure.
    pub fn leading_pad(available: f32) -> f32 {
        // Guarded before the subtraction, not after: an infinite `available`
        // would otherwise survive `max` as an infinite gutter.
        if !available.is_finite() {
            return 0.0;
        }
        ((available - content_width(available)) * 0.5).max(0.0)
    }
}

pub mod radius {
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const PILL: f32 = 999.0;
}

pub mod stroke {
    pub const HAIRLINE: f32 = 1.0;
    pub const CONTROL: f32 = 1.0;
    pub const FOCUS: f32 = 2.0;
    pub const TRACE: f32 = 1.5;
    pub const SELECTED_TRACE: f32 = 2.25;
}

pub mod control {
    pub const HEIGHT_COMPACT: f32 = 32.0;
    pub const HEIGHT_STANDARD: f32 = 36.0;
    pub const TABLE_ROW: f32 = 40.0;
}

pub mod chart {
    use super::{color, Rgb};

    pub const PRIMARY_TRACE: Rgb = Rgb::new(0x2F, 0x6F, 0xB3);
    pub const BASELINE: Rgb = color::SLATE_500;
    pub const GRID: Rgb = color::SLATE_200;
    pub const SELECTION_FILL: Rgb = color::ICE_100;
    pub const SELECTION_STROKE: Rgb = color::BLUE_600;

    /// Categorical sequence for overlaid channels or peak families.
    pub const SERIES: [Rgb; 8] = [
        Rgb::new(0x2F, 0x6F, 0xB3),
        Rgb::new(0x56, 0xA8, 0xD8),
        Rgb::new(0x6B, 0x70, 0xC8),
        Rgb::new(0x2E, 0x95, 0x99),
        Rgb::new(0x8A, 0x6F, 0xB8),
        Rgb::new(0xC4, 0x77, 0x3D),
        Rgb::new(0x3F, 0x8B, 0x63),
        Rgb::new(0xB4, 0x4D, 0x68),
    ];

    /// Stroke pattern used once the 8 series colours run out, so a tenth channel
    /// is distinguishable without relying on hue (`DESIGN_SYSTEM.md` §10.4).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Dash {
        Solid,
        /// 6 on, 4 off.
        Dashed,
        /// 2 on, 4 off.
        Dotted,
    }

    /// Colour for channel `i`: cycle the eight series colours.
    pub fn series_color(i: usize) -> Rgb {
        SERIES[i % SERIES.len()]
    }

    /// Dash pattern for channel `i`: solid for the first eight, then dashed, then
    /// dotted, so index and appearance stay in lockstep.
    pub fn series_dash(i: usize) -> Dash {
        match (i / SERIES.len()) % 3 {
            0 => Dash::Solid,
            1 => Dash::Dashed,
            _ => Dash::Dotted,
        }
    }

    /// Minimum contrast a colour must reach against the surface before it may be
    /// drawn as a trace. Matches the `BLUE_600`-on-white anchor in §3.
    pub const MIN_TRACE_CONTRAST: f64 = 3.0;

    /// A ChromLab legend colour may override [`series_color`] only when it is
    /// legible on the current surface; otherwise the design-system colour wins.
    pub fn legend_color_or_series(legend: Option<Rgb>, surface: Rgb, i: usize) -> Rgb {
        match legend {
            Some(c) if c.contrast_ratio(surface) >= MIN_TRACE_CONTRAST => c,
            _ => series_color(i),
        }
    }
}

/// The 96-well plate heatmap ramp (`DESIGN_SYSTEM.md` §10.3).
///
/// A single-hue, luminance-ordered sequential map, which is why it is
/// colourblind-safe: the ordering survives when hue information is lost.
pub mod plate {
    use super::{color, Rgb};

    /// Low → high stops. Interpolated, not stepped, so adjacent wells stay comparable.
    pub const RAMP: [Rgb; 4] = [
        color::MIST_50,
        color::BLUE_300,
        color::BLUE_500,
        color::BLUE_700,
    ];

    /// Perceptually-uniform alternative offered behind a toggle for users who
    /// prefer it; the on-brand blue stays the default.
    pub const VIRIDIS: [Rgb; 6] = [
        Rgb::new(0x44, 0x01, 0x54),
        Rgb::new(0x41, 0x44, 0x87),
        Rgb::new(0x2A, 0x78, 0x8E),
        Rgb::new(0x22, 0xA8, 0x84),
        Rgb::new(0x7A, 0xD1, 0x51),
        Rgb::new(0xFD, 0xE7, 0x25),
    ];

    /// Sample a ramp at `t` in 0..=1.
    pub fn sample(ramp: &[Rgb], t: f32) -> Rgb {
        if ramp.is_empty() {
            return color::MIST_50;
        }
        if ramp.len() == 1 {
            return ramp[0];
        }
        let t = t.clamp(0.0, 1.0);
        let scaled = t * (ramp.len() - 1) as f32;
        let idx = (scaled.floor() as usize).min(ramp.len() - 2);
        ramp[idx].lerp(ramp[idx + 1], scaled - idx as f32)
    }

    /// Minimum contrast a well's value text must reach against its background.
    pub const MIN_LABEL_CONTRAST: f64 = 4.5;

    /// How a well's label should be drawn on a given fill.
    ///
    /// Any continuous single-hue ramp has a middle band where *neither* white nor
    /// near-black text clears [`MIN_LABEL_CONTRAST`] — the two extremes are
    /// equidistant there. Because rule #3 requires every well to show its numeric
    /// value, the label gains a 1 px halo in that band; contrast is then against
    /// the halo, which is a true extreme, rather than against the ambiguous fill.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct LabelStyle {
        pub text: Rgb,
        /// Outline colour, drawn behind the glyphs. `None` when the fill alone
        /// already gives enough contrast.
        pub halo: Option<Rgb>,
    }

    impl LabelStyle {
        /// Contrast the reader actually experiences: against the halo when one is
        /// drawn, against the fill otherwise.
        pub fn effective_contrast(self, fill: Rgb) -> f64 {
            self.text.contrast_ratio(self.halo.unwrap_or(fill))
        }
    }

    /// Pick the label style for a ramp colour.
    pub fn label_on(fill: Rgb) -> LabelStyle {
        let on_ink = fill.contrast_ratio(color::INK_950);
        let on_white = fill.contrast_ratio(color::WHITE);
        let text = if on_ink >= on_white {
            color::INK_950
        } else {
            color::WHITE
        };
        let best = on_ink.max(on_white);
        let halo = (best < MIN_LABEL_CONTRAST).then(|| {
            if text == color::WHITE {
                color::INK_950
            } else {
                color::WHITE
            }
        });
        LabelStyle { text, halo }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encoding_is_stable() {
        assert_eq!(color::INK_950.hex(), 0x07111F);
        assert_eq!(color::BLUE_600.hex(), 0x3274BD);
    }

    #[test]
    fn documented_contrast_anchors_hold() {
        // DESIGN_SYSTEM.md §3 states these ratios; if a token is retuned, the
        // accessibility claim must be re-checked rather than silently broken.
        assert!(color::WHITE.contrast_ratio(color::INK_950) > 18.0);
        assert!(color::INK_950.contrast_ratio(color::MIST_50) > 18.0);
        assert!(color::BLUE_700.contrast_ratio(color::WHITE) > 6.9);
        assert!(color::BLUE_600.contrast_ratio(color::WHITE) > 4.5);
    }

    #[test]
    fn the_plate_ramp_is_monotonic_in_luminance() {
        // This is the property that makes the map colourblind-safe.
        let mut previous = f64::INFINITY;
        for step in 0..=20 {
            let l = plate::sample(&plate::RAMP, step as f32 / 20.0).relative_luminance();
            assert!(l <= previous + 1e-6, "luminance rose at step {step}");
            previous = l;
        }
    }

    #[test]
    fn plate_labels_stay_readable_across_the_whole_ramp() {
        // Rule #3: every well shows its value, so the value must be legible at
        // every point on the ramp — including the mid band where neither text
        // extreme clears the threshold against the fill alone.
        for ramp in [&plate::RAMP[..], &plate::VIRIDIS[..]] {
            for step in 0..=40 {
                let fill = plate::sample(ramp, step as f32 / 40.0);
                let style = plate::label_on(fill);
                assert!(
                    style.effective_contrast(fill) >= plate::MIN_LABEL_CONTRAST,
                    "step {step}: contrast {:.2}",
                    style.effective_contrast(fill)
                );
            }
        }
    }

    #[test]
    fn the_mid_ramp_dead_band_is_the_case_that_needs_a_halo() {
        // The ends of the ramp are unambiguous and must not pay for an outline.
        assert_eq!(plate::label_on(plate::RAMP[0]).halo, None);
        assert_eq!(plate::label_on(plate::RAMP[3]).halo, None);
        // Somewhere in between, no plain text colour is good enough.
        let needs_halo = (0..=40).any(|s| {
            plate::label_on(plate::sample(&plate::RAMP, s as f32 / 40.0))
                .halo
                .is_some()
        });
        assert!(needs_halo, "the dead band this guards against should exist");
    }

    #[test]
    fn channels_past_eight_change_dash_rather_than_repeating_silently() {
        assert_eq!(chart::series_dash(0), chart::Dash::Solid);
        assert_eq!(chart::series_dash(7), chart::Dash::Solid);
        assert_eq!(chart::series_dash(8), chart::Dash::Dashed);
        assert_eq!(chart::series_dash(16), chart::Dash::Dotted);
        // Colour repeats, but the pair (colour, dash) does not.
        assert_eq!(chart::series_color(0), chart::series_color(8));
        assert_ne!(chart::series_dash(0), chart::series_dash(8));
    }

    #[test]
    fn an_illegible_legend_colour_falls_back_to_the_series_palette() {
        // Near-white on a white surface fails the contrast anchor.
        let washed_out = Rgb::new(0xFA, 0xFB, 0xFC);
        assert_eq!(
            chart::legend_color_or_series(Some(washed_out), color::WHITE, 3),
            chart::series_color(3)
        );
        // A legible one is honoured.
        let legible = Rgb::new(0x20, 0x40, 0x60);
        assert_eq!(
            chart::legend_color_or_series(Some(legible), color::WHITE, 3),
            legible
        );
    }

    #[test]
    fn a_form_is_capped_on_a_wide_window_and_left_alone_on_a_narrow_one() {
        // The bug this exists to prevent: a 4K viewport stretching a field row.
        assert_eq!(measure::content_width(3840.0), measure::FORM_MAX);
        assert_eq!(
            measure::content_width(measure::FORM_MAX + 1.0),
            measure::FORM_MAX
        );
        // Below the cap the form keeps every pixel it is offered.
        assert_eq!(measure::content_width(640.0), 640.0);
        assert_eq!(measure::content_width(measure::FORM_MAX), measure::FORM_MAX);
    }

    #[test]
    fn a_window_narrower_than_the_usability_floor_still_gets_all_of_it() {
        // Degrade to full width rather than to a column too tight for the
        // calibration point table.
        for available in [1.0, 120.0, measure::FORM_MIN - 1.0, measure::FORM_MIN] {
            assert_eq!(measure::content_width(available), available);
            assert_eq!(measure::leading_pad(available), 0.0);
        }
    }

    #[test]
    fn hex_strings_round_trip_with_or_without_the_hash() {
        let teal = Rgb::new(0x2E, 0x95, 0x99);
        assert_eq!(teal.hex_string(), "#2E9599");
        assert_eq!(Rgb::from_hex_str("#2E9599"), Some(teal));
        assert_eq!(Rgb::from_hex_str("2e9599"), Some(teal));
        assert_eq!(Rgb::from_hex_str("  #2E9599  "), Some(teal));
    }

    #[test]
    fn malformed_hex_is_rejected_rather_than_guessed_at() {
        // A colour field is typed into character by character, so every
        // intermediate state must be a clean rejection, never a panic and never
        // a silently different colour.
        for bad in [
            "", "#", "#2E959", "#2E95999", "2E959G", "#+12345", "-123456", "#FFFF", "rebecca",
        ] {
            assert_eq!(Rgb::from_hex_str(bad), None, "{bad:?} should not parse");
        }
    }

    #[test]
    fn a_degenerate_width_produces_no_width_rather_than_a_negative_or_a_nan() {
        // egui hands out a zero or slightly negative available width during the
        // first frame of a collapsed panel; that must not become a NaN layout.
        for available in [0.0, -1.0, -4000.0, f32::NAN, f32::INFINITY] {
            let w = measure::content_width(available);
            assert!(
                w.is_finite() && w >= 0.0,
                "content_width({available}) = {w}"
            );
            let pad = measure::leading_pad(available);
            assert!(
                pad.is_finite() && pad >= 0.0,
                "leading_pad({available}) = {pad}"
            );
        }
        assert_eq!(measure::content_width(f32::INFINITY), 0.0);
    }

    #[test]
    fn the_capped_form_is_centred_in_what_is_left() {
        let available = 3840.0;
        let pad = measure::leading_pad(available);
        assert_eq!(pad, (available - measure::FORM_MAX) / 2.0);
        // Content plus both gutters accounts for the whole width.
        assert_eq!(pad * 2.0 + measure::content_width(available), available);
    }

    #[test]
    fn ramp_sampling_is_clamped_at_both_ends() {
        assert_eq!(plate::sample(&plate::RAMP, -1.0), plate::RAMP[0]);
        assert_eq!(plate::sample(&plate::RAMP, 2.0), plate::RAMP[3]);
    }
}

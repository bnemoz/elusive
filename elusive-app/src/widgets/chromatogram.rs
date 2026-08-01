//! The chromatogram pane: stacked, x-linked plots — one per axis group.
//!
//! Channels are grouped by [`AxisGroup`] and each group gets its own plot with its
//! own y-scale, all linked on the volume axis. The alternative — rescaling
//! conductivity onto the UV axis — would put a number on screen that is not the
//! number the instrument measured, so it is not on the table.

use crate::egui_adapter::{self as adapt, c, c_alpha, ca};
use crate::theme::{chart, color, spacing, stroke, Rgb, Theme};
use crate::view::{Interaction, View};
use egui::Ui;
use egui_plot::{Line, Plot, PlotPoints, Polygon};
use elusive_core::model::{AxisGroup, Channel, Color, Run};

/// Fraction of the pane's height given to the hero (UV) group.
const HERO_HEIGHT_SHARE: f32 = 0.55;
const MIN_GROUP_HEIGHT: f32 = 90.0;

/// Alpha of the per-fraction background zone.
///
/// Faint on purpose: the trace still has to dominate, but the user needs the
/// fraction windows to remain legible after vertical pan/zoom.
const FRACTION_ZONE_ALPHA: u8 = 18;

/// What the chromatogram pane observed this frame.
///
/// The pane *reports* rather than writes: several plots are stacked, and if each
/// one set the shared hover state directly the last would clear what the first
/// found. The caller resolves one answer per frame (see `app::linked_pane`).
#[derive(Clone, Debug, Default)]
pub struct ChartOutcome {
    pub interaction: Option<Interaction>,
    /// Elution volume under the pointer, when it is over one of the plots.
    pub hovered_volume: Option<f32>,
}

pub fn show(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) -> ChartOutcome {
    let groups = visible_groups(run, view);
    if groups.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("No channels are visible. Enable one in the legend.")
                    .color(c(t.text_secondary)),
            );
        });
        return ChartOutcome::default();
    }

    let mut outcome = ChartOutcome::default();
    let total = ui.available_height();
    let heights = group_heights(&groups, total);

    for (idx, (group, height)) in groups.iter().zip(heights).enumerate() {
        let (interaction, hovered) = plot_group(ui, run, view, t, *group, height, idx == 0);
        if interaction.is_some() {
            outcome.interaction = interaction;
        }
        // Only the plot actually under the pointer reports a volume, so a later
        // plot in the stack cannot erase an earlier one's hover.
        if hovered.is_some() {
            outcome.hovered_volume = hovered;
        }
    }
    outcome
}

/// Axis groups that currently have at least one visible channel, hero group first.
fn visible_groups(run: &Run, view: &View) -> Vec<AxisGroup> {
    let hero_group = view
        .hero_channel_id
        .as_ref()
        .and_then(|id| run.channel(id))
        .or_else(|| run.hero_channel())
        .map(|c| c.kind.axis_group());
    let mut groups: Vec<AxisGroup> = run
        .channels
        .iter()
        .filter(|c| !c.is_empty() && view.is_channel_visible(&c.id))
        .map(|c| c.kind.axis_group())
        .collect();
    groups.sort();
    groups.dedup();
    if let Some(hero) = hero_group {
        if let Some(pos) = groups.iter().position(|g| *g == hero) {
            groups.swap(0, pos);
        }
    }
    groups
}

/// Split the pane between axis groups.
///
/// The result always sums to at most `total`. Handing back more than we were
/// given would overflow the parent, and a parent that grows to fit its content is
/// exactly the feedback loop this module has to avoid.
fn group_heights(groups: &[AxisGroup], total: f32) -> Vec<f32> {
    let n = groups.len();
    if n == 0 {
        return Vec::new();
    }
    let total = total.max(0.0);
    if n == 1 {
        return vec![total];
    }
    // Too tight for everyone to get the minimum: split evenly and accept that
    // the plots are small, rather than overflowing.
    if total <= MIN_GROUP_HEIGHT * n as f32 {
        return vec![total / n as f32; n];
    }
    let hero = (total * HERO_HEIGHT_SHARE)
        .clamp(MIN_GROUP_HEIGHT, total - MIN_GROUP_HEIGHT * (n - 1) as f32);
    let rest = (total - hero) / (n - 1) as f32;
    std::iter::once(hero)
        .chain(std::iter::repeat_n(rest, n - 1))
        .collect()
}

/// The y-extent of the data in an axis group, in display units.
///
/// Overlays are drawn against *this*, never against `plot_ui.plot_bounds()`.
/// egui_plot recomputes auto-bounds from its items each frame and adds a 5%
/// margin, so an overlay sized from the current bounds re-enters the next frame's
/// bounds and inflates them compounding — the plot silently zooms out a little on
/// every repaint. Deriving from the data keeps overlays inside the bounds they
/// help produce, so the scale is stable.
fn data_y_range(channels: &[(usize, &Channel)]) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for (_, channel) in channels {
        if let Some((clo, chi)) = channel.display_value_range() {
            lo = lo.min(clo as f64);
            hi = hi.max(chi as f64);
        }
    }
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        return (0.0, 1.0);
    }
    (lo, hi)
}

fn plot_group(
    ui: &mut Ui,
    run: &Run,
    view: &mut View,
    t: Theme,
    group: AxisGroup,
    height: f32,
    is_hero: bool,
) -> (Option<Interaction>, Option<f32>) {
    let channels: Vec<(usize, &Channel)> = run
        .channels
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            !c.is_empty() && c.kind.axis_group() == group && view.is_channel_visible(&c.id)
        })
        .collect();
    if channels.is_empty() {
        return (None, None);
    }

    let unit = channels
        .first()
        .map(|(_, c)| c.display_unit.clone())
        .unwrap_or_default();

    // While integrating, dragging draws a selection instead of panning.
    let integrating = view.integrate_mode;

    let mut interaction = None;
    let mut hovered_volume = None;
    Plot::new(format!("chromatogram-{group:?}"))
        .height(height)
        .sense(egui::Sense::click_and_drag())
        .link_axis("chromatogram-x", [true, false])
        .allow_drag([!integrating, !integrating])
        .allow_boxed_zoom(!integrating)
        .show_grid([true, true])
        .custom_y_axes(vec![egui_plot::AxisHints::new_y()
            .label(format!("{} ({unit})", group.label()))
            // Defaults to 20..30 pt, which blanks every tick label on a short
            // plot — unreadable peak heights on the one trace that matters.
            .label_spacing(egui::Rangef::new(12.0, 20.0))
            .min_thickness(44.0)])
        .x_axis_label(if is_hero { "" } else { "Elution volume (mL)" })
        .legend(egui_plot::Legend::default().position(egui_plot::Corner::RightTop))
        .label_formatter(|pos| {
            let p = match pos {
                egui_plot::HoverPosition::NearDataPoint { position, .. } => position,
                egui_plot::HoverPosition::Elsewhere { position } => position,
            };
            Some(format!("{:.3} mL\n{:.3}", p.x, p.y))
        })
        .show(ui, |plot_ui| {
            // Fixed, data-derived extent. Deliberately NOT `plot_ui.plot_bounds()`
            // — see `data_y_range`.
            let (y_lo, y_hi) = data_y_range(&channels);

            // 1. Fraction bands sit *under* the traces so the signal stays on top.
            if is_hero {
                draw_fraction_zones(plot_ui, run, view, t, y_lo, y_hi);
                draw_highlighted_span(plot_ui, view, t, y_lo, y_hi);
                draw_excluded_regions(plot_ui, view, t, y_lo, y_hi);
            }

            // 2. Integrated peak regions, translucent (rule #2).
            draw_peak_regions(plot_ui, view, group, run, y_lo);

            // 3. The traces themselves.
            for (i, channel) in &channels {
                draw_channel(plot_ui, channel, *i, view, t);
            }

            // 4. The pending drag selection, so the user sees the window forming.
            if let Some((a, b)) = view.pending_selection {
                let fill = c_alpha(chart::SELECTION_STROKE, 40);
                plot_ui.polygon(
                    Polygon::new(
                        "",
                        PlotPoints::from(vec![
                            [a as f64, y_lo],
                            [b as f64, y_lo],
                            [b as f64, y_hi],
                            [a as f64, y_hi],
                        ]),
                    )
                    .fill_color(fill)
                    .stroke(egui::Stroke::new(
                        stroke::CONTROL,
                        c(chart::SELECTION_STROKE),
                    )),
                );
            }

            // --- interaction ------------------------------------------------
            let response = plot_ui.response();
            let pointer = plot_ui.pointer_coordinate();

            if response.hovered() {
                hovered_volume = pointer.map(|p| p.x as f32);
            }

            if integrating {
                if response.drag_started_by(egui::PointerButton::Primary) {
                    view.drag_anchor = pointer.map(|p| p.x as f32);
                }
                if response.dragged_by(egui::PointerButton::Primary) {
                    if let (Some(anchor), Some(p)) = (view.drag_anchor, pointer) {
                        view.pending_selection = Some((anchor, p.x as f32));
                    }
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let Some((a, b)) = view.pending_selection.take() {
                        if (b - a).abs() > f32::EPSILON {
                            interaction = Some(Interaction::IntegrateRange(a.min(b), a.max(b)));
                        }
                    }
                    view.drag_anchor = None;
                }
            }
        });

    (interaction, hovered_volume)
}

fn to_rgb(color: Color) -> Rgb {
    Rgb::new(color.r, color.g, color.b)
}

fn to_core_color(rgb: Rgb) -> Color {
    // Traces are lines, not fills: an override is always fully opaque.
    Color::new(rgb.r, rgb.g, rgb.b, 0xFF)
}

/// The colour a channel's trace is drawn in.
///
/// The single source of truth for both the plot and the legend swatch. The two
/// used to resolve the colour independently, which is a bug waiting to happen:
/// the moment they disagree the legend is documenting a colour the plot never
/// drew, and the reader has no way to tell which one is lying.
pub fn trace_color(channel: &Channel, index: usize, view: &View, t: Theme) -> Rgb {
    resolve_trace_color(
        view.channel_color(&channel.id).map(to_rgb),
        view.hero_channel_id.as_ref() == Some(&channel.id),
        channel.color.map(to_rgb),
        t.panel_bg,
        index,
    )
}

/// Precedence: user override → hero trace → legible ChromLab colour → palette.
///
/// The override outranks the contrast gate that
/// [`chart::legend_color_or_series`] applies to a ChromLab colour, and that
/// asymmetry is deliberate. A colour the instrument happened to record is a
/// default we are free to reject; a colour the user typed is an instruction, and
/// silently substituting a different one would make the hex field lie. Poor
/// legibility is reported next to the swatch instead (rule #3: never colour alone).
fn resolve_trace_color(
    user: Option<Rgb>,
    is_hero: bool,
    chromlab: Option<Rgb>,
    surface: Rgb,
    index: usize,
) -> Rgb {
    if let Some(rgb) = user {
        return rgb;
    }
    if is_hero {
        return chart::PRIMARY_TRACE;
    }
    chart::legend_color_or_series(chromlab, surface, index)
}

/// Whether a trace colour is too close to the surface to read reliably (§10.4).
fn is_low_contrast(rgb: Rgb, surface: Rgb) -> bool {
    rgb.contrast_ratio(surface) < chart::MIN_TRACE_CONTRAST
}

fn draw_channel(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    channel: &Channel,
    index: usize,
    view: &View,
    t: Theme,
) {
    // Each channel builds its own point list from its own samples — no shared
    // index is assumed anywhere (`model.rs` invariant 1).
    let points: Vec<[f64; 2]> = channel
        .samples
        .iter()
        .filter(|s| s.is_finite())
        .map(|s| [s.volume_ml as f64, (s.value * channel.display_scale) as f64])
        .collect();
    if points.len() < 2 {
        return;
    }

    let selected = view.selected_channel.as_ref() == Some(&channel.id);
    let rgb = trace_color(channel, index, view, t);

    let width = if selected {
        stroke::SELECTED_TRACE
    } else {
        stroke::TRACE
    };

    plot_ui.line(
        Line::new(channel.name.clone(), PlotPoints::from(points))
            .stroke(egui::Stroke::new(width, c(rgb)))
            .style(adapt::line_style(chart::series_dash(index))),
    );
}

/// Fraction windows as faint full-height zones.
///
/// Baseline ticks disappeared as soon as the user panned away from the baseline.
/// A very low-alpha zone keeps the windows visible without competing with the
/// chromatogram.
fn draw_fraction_zones(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    run: &Run,
    view: &View,
    t: Theme,
    y_lo: f64,
    y_hi: f64,
) {
    if !view.show_fractions || run.fractions.is_empty() {
        return;
    }
    for (idx, f) in run.fractions.iter().enumerate() {
        let (a, b) = f.volume_window();
        if !a.is_finite() || !b.is_finite() || b <= a {
            continue;
        }
        let alpha = if idx % 2 == 0 {
            FRACTION_ZONE_ALPHA
        } else {
            FRACTION_ZONE_ALPHA.saturating_add(6)
        };
        plot_ui.polygon(
            Polygon::new(
                "",
                PlotPoints::from(vec![
                    [a as f64, y_lo],
                    [b as f64, y_lo],
                    [b as f64, y_hi],
                    [a as f64, y_hi],
                ]),
            )
            .fill_color(c_alpha(t.fraction_highlight, alpha))
            .stroke(egui::Stroke::new(stroke::HAIRLINE, c_alpha(t.axis, 60)))
            .allow_hover(false),
        );
    }
}

/// The fraction currently hovered — on the plate or on the trace.
fn draw_highlighted_span(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    view: &View,
    t: Theme,
    y_lo: f64,
    y_hi: f64,
) {
    let Some((a, b)) = view.hovered_vol_range else {
        return;
    };
    plot_ui.polygon(
        Polygon::new(
            "",
            PlotPoints::from(vec![
                [a as f64, y_lo],
                [b as f64, y_lo],
                [b as f64, y_hi],
                [a as f64, y_hi],
            ]),
        )
        .fill_color(c_alpha(t.fraction_highlight, 90))
        .stroke(egui::Stroke::new(
            stroke::CONTROL,
            c(chart::SELECTION_STROKE),
        ))
        .allow_hover(false),
    );
}

fn draw_excluded_regions(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    view: &View,
    _t: Theme,
    y_lo: f64,
    y_hi: f64,
) {
    for region in &view.excluded_regions {
        let (a, b) = (region.v_start_ml as f64, region.v_end_ml as f64);
        plot_ui.polygon(
            Polygon::new(
                "",
                PlotPoints::from(vec![[a, y_lo], [b, y_lo], [b, y_hi], [a, y_hi]]),
            )
            .fill_color(ca(color::EXCLUDED_REGION))
            // §10.5: an excluded region carries a boundary stroke, not colour alone.
            .stroke(egui::Stroke::new(stroke::CONTROL, c(color::DANGER_600)))
            .allow_hover(false),
        );
    }
}

/// Shade each integrated peak between the signal and its baseline.
fn draw_peak_regions(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    view: &View,
    group: AxisGroup,
    run: &Run,
    y_lo: f64,
) {
    for peak in &view.peaks {
        let Some(channel) = run.channel(&peak.channel_id) else {
            continue;
        };
        if channel.kind.axis_group() != group {
            continue;
        }

        let mut outline: Vec<[f64; 2]> = channel
            .samples_in_volume(peak.v_start_ml, peak.v_end_ml)
            .iter()
            .filter(|s| s.is_finite())
            .map(|s| [s.volume_ml as f64, (s.value * channel.display_scale) as f64])
            .collect();
        if outline.len() < 2 {
            continue;
        }

        let baseline_y = baseline_points(peak, channel, y_lo);
        outline.extend(baseline_y.into_iter().rev());

        let selected = view.selected_peak == Some(peak.id);
        let alpha = if selected {
            0x99
        } else {
            color::INTEGRATED_AREA.a
        };
        plot_ui.polygon(
            Polygon::new("", PlotPoints::from(outline))
                .fill_color(c_alpha(
                    crate::theme::Rgb::new(
                        color::INTEGRATED_AREA.r,
                        color::INTEGRATED_AREA.g,
                        color::INTEGRATED_AREA.b,
                    ),
                    alpha,
                ))
                .stroke(egui::Stroke::new(
                    if selected {
                        stroke::SELECTED_TRACE
                    } else {
                        stroke::CONTROL
                    },
                    c(chart::SELECTION_STROKE),
                )),
        );
    }
}

/// The two endpoints of the peak's baseline, in display units.
fn baseline_points(
    peak: &elusive_core::model::PeakResult,
    channel: &Channel,
    y_lo: f64,
) -> Vec<[f64; 2]> {
    use elusive_core::model::BaselineMode;
    let scale = channel.display_scale as f64;
    let at = |v: f32| channel.value_at_volume(v).map(|y| y as f64 * scale);

    let (y0, y1) = match peak.baseline {
        BaselineMode::DropToZero => (0.0f64.max(y_lo), 0.0f64.max(y_lo)),
        BaselineMode::LinearEndpoints => (
            at(peak.v_start_ml).unwrap_or(0.0),
            at(peak.v_end_ml).unwrap_or(0.0),
        ),
        BaselineMode::ValleyToValley { left_ml, right_ml } => {
            // Extend the valley line out to the peak's own window.
            let (ya, yb) = (at(left_ml).unwrap_or(0.0), at(right_ml).unwrap_or(0.0));
            let span = (right_ml - left_ml) as f64;
            let interp = |v: f32| {
                if span.abs() < f64::EPSILON {
                    ya
                } else {
                    ya + ((v - left_ml) as f64 / span) * (yb - ya)
                }
            };
            (interp(peak.v_start_ml), interp(peak.v_end_ml))
        }
    };

    vec![[peak.v_start_ml as f64, y0], [peak.v_end_ml as f64, y1]]
}

/// Width of the legend swatch. The dash patterns below are laid out against it,
/// so they stay inside the rect.
const SWATCH_WIDTH: f32 = 24.0;
/// Taller than the 1.5 px line it contains: the swatch is a click target now, and
/// a 10 px-high one is not something a user can reliably hit.
const SWATCH_HEIGHT: f32 = 16.0;

/// Draw a channel's line sample: colour plus dash pattern, because channels past
/// the eighth are told apart by shape, not hue alone (§10.4).
fn paint_swatch(painter: &egui::Painter, rect: egui::Rect, rgb: Rgb, dash: chart::Dash) {
    let y = rect.center().y;
    let s = egui::Stroke::new(stroke::TRACE, c(rgb));
    let segment = |x0: f32, len: f32| {
        painter.line_segment([egui::pos2(x0, y), egui::pos2(x0 + len, y)], s);
    };
    match dash {
        chart::Dash::Solid => segment(rect.left(), rect.width()),
        chart::Dash::Dashed => {
            for seg in 0..2 {
                segment(rect.left() + seg as f32 * 12.0, 8.0);
            }
        }
        chart::Dash::Dotted => {
            for seg in 0..4 {
                segment(rect.left() + seg as f32 * 6.0, 2.0);
            }
        }
    }
}

/// Legend with per-channel visibility, colour swatch, and unit — the control
/// surface for Phase 2's show/hide requirement.
pub fn legend(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) {
    egui::ScrollArea::vertical()
        .id_salt("channel-legend")
        .show(ui, |ui| {
            for (i, channel) in run.channels.iter().enumerate() {
                if channel.is_empty() {
                    continue;
                }
                let mut visible = view.is_channel_visible(&channel.id);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut visible, "").changed() {
                        view.set_channel_visible(&channel.id, visible);
                    }

                    let rgb = trace_color(channel, i, view, t);

                    let swatch = ui.allocate_response(
                        egui::vec2(SWATCH_WIDTH, SWATCH_HEIGHT),
                        egui::Sense::click(),
                    );
                    paint_swatch(ui.painter(), swatch.rect, rgb, chart::series_dash(i));
                    let swatch = swatch
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Click to choose this trace's colour");

                    // Scratch buffer for the hex field, reset on every visit so a
                    // half-typed value from last time does not reappear.
                    let hex_id = swatch.id.with("hex-entry");
                    if swatch.clicked() {
                        ui.data_mut(|d| d.remove::<String>(hex_id));
                    }

                    if is_low_contrast(rgb, t.panel_bg) {
                        // Rule #3: the problem with a colour is never reported by
                        // colour alone, so this is a glyph with a tooltip.
                        ui.label(
                            egui::RichText::new("⚠")
                                .font(adapt::font_micro())
                                .color(c(color::WARNING_600)),
                        )
                        .on_hover_text(
                            "Low contrast against the plot background — this trace may be hard to see",
                        );
                    }

                    egui::Popup::from_toggle_button_response(&swatch)
                        // The default closes on any click, which would dismiss the
                        // popup the instant the user touched the colour wheel.
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .frame(adapt::card(t))
                        .show(|ui| color_editor(ui, channel, i, view, t, hex_id));

                    let label = ui.selectable_label(
                        view.selected_channel.as_ref() == Some(&channel.id),
                        egui::RichText::new(&channel.name).color(c(t.text_primary)),
                    );
                    if label.clicked() {
                        view.focus_channel(&channel.id);
                    }

                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {} pts",
                            channel.display_unit,
                            channel.samples.len()
                        ))
                        .font(adapt::font_micro())
                        .color(c(t.text_secondary)),
                    );
                });
            }
        });
}

/// The colour picker behind a legend swatch.
///
/// egui 0.35's `color_picker_color32` offers a wheel and R/G/B drag values but no
/// hex field, and a hex code is how a colour actually travels between a figure, a
/// protocol, and a colleague — so one is added here.
fn color_editor(
    ui: &mut Ui,
    channel: &Channel,
    index: usize,
    view: &mut View,
    t: Theme,
    hex_id: egui::Id,
) {
    ui.set_max_width(240.0);
    ui.label(
        egui::RichText::new(&channel.name)
            .font(adapt::font_h3())
            .color(c(t.text_primary)),
    );
    ui.add_space(spacing::SM);

    let current = trace_color(channel, index, view, t);
    let mut picked = c(current);
    // Opaque: a semi-transparent line reads as a different colour wherever it
    // crosses a fraction zone, so the swatch would stop matching the trace.
    if egui::color_picker::color_picker_color32(ui, &mut picked, egui::color_picker::Alpha::Opaque)
    {
        let rgb = Rgb::new(picked.r(), picked.g(), picked.b());
        view.set_channel_color(&channel.id, to_core_color(rgb));
        // Keep the hex field showing what the wheel just produced.
        ui.data_mut(|d| d.insert_temp(hex_id, rgb.hex_string()));
    }

    ui.add_space(spacing::SM);
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(hex_id))
        .unwrap_or_else(|| current.hex_string());
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Hex")
                .font(adapt::font_micro())
                .color(c(t.text_secondary)),
        );
        let edit = ui.add(
            egui::TextEdit::singleline(&mut text)
                .desired_width(90.0)
                .font(adapt::font_code())
                .hint_text("#RRGGBB"),
        );
        if edit.changed() {
            // Only a complete, well-formed value is applied. Anything else leaves
            // the trace alone while the user keeps typing.
            if let Some(rgb) = Rgb::from_hex_str(&text) {
                view.set_channel_color(&channel.id, to_core_color(rgb));
            }
        }
    });
    let parsed = Rgb::from_hex_str(&text);
    ui.data_mut(|d| d.insert_temp(hex_id, text));

    if parsed.is_none() {
        ui.label(
            egui::RichText::new("Enter six hex digits, e.g. #2F6FB3")
                .font(adapt::font_micro())
                .color(c(color::WARNING_600)),
        );
    } else if is_low_contrast(current, t.panel_bg) {
        // The user's choice still wins — this only tells them what they are
        // trading away (§10.4 rejects an *instrument* colour, not a chosen one).
        ui.label(
            egui::RichText::new("⚠ Low contrast against the plot background")
                .font(adapt::font_micro())
                .color(c(color::WARNING_600)),
        );
    }

    ui.add_space(spacing::SM);
    let overridden = view.channel_color(&channel.id).is_some();
    if ui
        .add_enabled(overridden, egui::Button::new("Reset to default"))
        .on_disabled_hover_text("This channel is already using its default colour")
        .clicked()
    {
        view.clear_channel_color(&channel.id);
        // Drop the buffer too, so the field redraws from the restored default.
        ui.data_mut(|d| d.remove::<String>(hex_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUPS: [AxisGroup; 3] = [AxisGroup::Uv, AxisGroup::Conductivity, AxisGroup::Ph];

    #[test]
    fn group_heights_never_exceed_the_space_available() {
        // Overflowing the parent is what lets a panel grow, so this is the
        // invariant that keeps the chromatogram pane stable.
        for total in [0.0f32, 40.0, 120.0, 300.0, 900.0, 2000.0] {
            for n in 1..=GROUPS.len() {
                let heights = group_heights(&GROUPS[..n], total);
                assert_eq!(heights.len(), n);
                let sum: f32 = heights.iter().sum();
                assert!(sum <= total + 1e-3, "n={n} total={total} sum={sum}");
                assert!(heights.iter().all(|h| *h >= 0.0));
            }
        }
    }

    #[test]
    fn the_hero_group_gets_the_most_height_when_there_is_room() {
        let heights = group_heights(&GROUPS, 900.0);
        assert!(heights[0] > heights[1]);
        assert!(heights[0] > heights[2]);
    }

    #[test]
    fn a_cramped_pane_splits_evenly_rather_than_overflowing() {
        let heights = group_heights(&GROUPS, 120.0);
        let sum: f32 = heights.iter().sum();
        assert!((sum - 120.0).abs() < 1e-3, "sum = {sum}");
        assert!(heights.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-3));
    }

    #[test]
    fn overlay_extent_comes_from_the_data_not_the_view() {
        use elusive_core::model::{Channel, ChannelKind, Sample};

        let mut uv = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        uv.display_scale = 1000.0;
        uv.samples = vec![
            Sample::new(0.0, 0.0, 0.0),
            Sample::new(60.0, 1.0, 0.5),
            Sample::new(120.0, 2.0, 0.25),
        ];
        let channels = vec![(0usize, &uv)];
        let (lo, hi) = data_y_range(&channels);

        // Display units: 0..500 mAU. If this ever tracked the plot's current
        // bounds instead, overlays would re-enter auto-bounds and the y-scale
        // would inflate on every repaint.
        assert!((lo - 0.0).abs() < 1e-6, "lo = {lo}");
        assert!((hi - 500.0).abs() < 1e-6, "hi = {hi}");
    }

    #[test]
    fn an_empty_group_still_yields_a_usable_extent() {
        let (lo, hi) = data_y_range(&[]);
        assert!(hi > lo, "a degenerate range would make overlays invisible");
    }

    // --- trace colour resolution -----------------------------------------

    const SURFACE: Rgb = crate::theme::color::WHITE;
    const PICKED: Rgb = Rgb::new(0xC4, 0x77, 0x3D);
    const CHROMLAB: Rgb = Rgb::new(0x20, 0x40, 0x60);

    #[test]
    fn a_chosen_colour_outranks_every_automatic_one() {
        // Including the hero trace and a legible ChromLab colour: an explicit
        // choice is an instruction, not a suggestion.
        assert_eq!(
            resolve_trace_color(Some(PICKED), true, Some(CHROMLAB), SURFACE, 3),
            PICKED
        );
        assert_eq!(
            resolve_trace_color(Some(PICKED), false, None, SURFACE, 3),
            PICKED
        );
    }

    #[test]
    fn an_illegible_choice_is_still_honoured() {
        // §10.4's contrast gate rejects an *instrument* colour. Substituting a
        // different colour for one the user typed would make the hex field lie;
        // the legend warns instead.
        let washed_out = Rgb::new(0xFA, 0xFB, 0xFC);
        assert_eq!(
            resolve_trace_color(Some(washed_out), false, None, SURFACE, 3),
            washed_out
        );
        assert!(is_low_contrast(washed_out, SURFACE));
        assert!(!is_low_contrast(chart::PRIMARY_TRACE, SURFACE));
    }

    #[test]
    fn without_an_override_the_documented_precedence_holds() {
        // hero → legible ChromLab colour → series palette.
        assert_eq!(
            resolve_trace_color(None, true, Some(CHROMLAB), SURFACE, 3),
            chart::PRIMARY_TRACE
        );
        assert_eq!(
            resolve_trace_color(None, false, Some(CHROMLAB), SURFACE, 3),
            CHROMLAB
        );
        assert_eq!(
            resolve_trace_color(None, false, None, SURFACE, 3),
            chart::series_color(3)
        );
        // An illegible instrument colour still loses to the palette.
        assert_eq!(
            resolve_trace_color(None, false, Some(Rgb::new(0xFA, 0xFB, 0xFC)), SURFACE, 3),
            chart::series_color(3)
        );
    }

    #[test]
    fn the_plot_and_the_legend_read_the_same_colour_for_a_channel() {
        // The regression this guards: two call sites resolving independently and
        // drifting, so the legend documents a colour the plot never drew. Both
        // now go through `trace_color`, which is what this exercises.
        use elusive_core::model::{Channel, ChannelKind, Sample};

        let mut uv = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        uv.samples = vec![Sample::new(0.0, 0.0, 0.0), Sample::new(60.0, 1.0, 0.5)];
        uv.color = Some(Color::new(0x20, 0x40, 0x60, 0xFF));

        let t = crate::theme::LIGHT;
        let mut view = View::default();
        view.hero_channel_id = Some(uv.id.clone());
        assert_eq!(trace_color(&uv, 0, &view, t), chart::PRIMARY_TRACE);

        view.set_channel_color(&uv.id, to_core_color(PICKED));
        assert_eq!(trace_color(&uv, 0, &view, t), PICKED);

        view.clear_channel_color(&uv.id);
        view.hero_channel_id = None;
        assert_eq!(trace_color(&uv, 0, &view, t), CHROMLAB);
    }

    #[test]
    fn a_colour_survives_the_trip_through_the_core_representation() {
        // The picker hands back an `Rgb`; the sidecar stores a core `Color`.
        assert_eq!(to_rgb(to_core_color(PICKED)), PICKED);
        assert_eq!(to_core_color(PICKED).a, 0xFF);
    }
}

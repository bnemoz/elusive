//! The chromatogram pane: stacked, x-linked plots — one per axis group.
//!
//! Channels are grouped by [`AxisGroup`] and each group gets its own plot with its
//! own y-scale, all linked on the volume axis. The alternative — rescaling
//! conductivity onto the UV axis — would put a number on screen that is not the
//! number the instrument measured, so it is not on the table.

use crate::egui_adapter::{self as adapt, c, c_alpha, ca};
use crate::theme::{chart, color, stroke, Theme};
use crate::view::{Interaction, View};
use egui::Ui;
use egui_plot::{Line, Plot, PlotPoints, Polygon};
use elusive_core::model::{AxisGroup, Channel, Fraction, PeakId, PeakResult, Run};

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
    let count = groups.len();

    for (idx, (group, height)) in groups.iter().zip(heights).enumerate() {
        let position = PlotPosition {
            is_hero: idx == 0,
            x_axis_label: x_axis_label_for(idx, count),
        };
        let (interaction, hovered) = plot_group(ui, run, view, t, *group, height, position);
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

/// Where a plot sits in the stack, and what that implies for its chrome.
///
/// `is_hero` (top plot, own y-axis overlays) and the x-axis label (bottom plot
/// only) are both derived from a plot's position but are otherwise unrelated,
/// so they are bundled here rather than passed as two more bare `plot_group`
/// arguments.
#[derive(Clone, Copy)]
struct PlotPosition {
    is_hero: bool,
    x_axis_label: &'static str,
}

/// The x-axis label for the plot at `idx` in a stack of `count` plots.
///
/// The stack is x-linked, so repeating "Elution volume (mL)" under every plot
/// is clutter — but it still has to appear *somewhere*, including in the
/// overwhelmingly common single-group (UV-only) view. Only the bottom-most
/// plot gets it.
fn x_axis_label_for(idx: usize, count: usize) -> &'static str {
    if count > 0 && idx == count - 1 {
        "Elution volume (mL)"
    } else {
        ""
    }
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

/// The peak facts the hover readout needs, lifted out of [`View`].
///
/// `label_formatter` holds its closure for as long as the `Plot` lives, which
/// spans the `show` body that mutates `View`. Copying three `Copy` fields per
/// peak up front settles that overlap without interior mutability.
#[derive(Clone, Copy, Debug, PartialEq)]
struct HoverPeak {
    id: PeakId,
    v_start_ml: f32,
    v_end_ml: f32,
}

impl HoverPeak {
    /// Peak windows arrive from a drag, so do not assume start precedes end.
    fn covers(&self, x: f64) -> bool {
        let (a, b) = if self.v_start_ml <= self.v_end_ml {
            (self.v_start_ml, self.v_end_ml)
        } else {
            (self.v_end_ml, self.v_start_ml)
        };
        a as f64 <= x && x <= b as f64
    }
}

/// The peaks that belong on one axis group's plot.
///
/// Same filter as `draw_peak_regions`: a conductivity peak must not be named
/// while the pointer is over the UV plot, where its window means nothing.
fn hover_peaks(run: &Run, peaks: &[PeakResult], group: AxisGroup) -> Vec<HoverPeak> {
    peaks
        .iter()
        .filter(|p| {
            run.channel(&p.channel_id)
                .is_some_and(|c| c.kind.axis_group() == group)
        })
        .map(|p| HoverPeak {
            id: p.id,
            v_start_ml: p.v_start_ml,
            v_end_ml: p.v_end_ml,
        })
        .collect()
}

/// The collected fraction covering `x`, if any.
///
/// Takes the first hit rather than asserting uniqueness: a malformed run can
/// report overlapping windows, and a tooltip is not the place to discover that.
/// `run.fractions` is not assumed sorted, so this is a scan.
fn fraction_at(run: &Run, x: f64) -> Option<&Fraction> {
    run.fractions.iter().find(|f| {
        let (a, b) = f.volume_window();
        f.has_usable_window() && a as f64 <= x && x <= b as f64
    })
}

/// The hover readout under the cursor.
///
/// Volume and value alone leave the user cross-referencing the plate and the peak
/// table by eye to answer "which tube is this, and did I integrate it?". The
/// fraction and peak lines are omitted when they do not apply — a placeholder line
/// is noise the eye still has to parse.
fn hover_label(run: &Run, peaks: &[HoverPeak], unit: &str, x: f64, y: f64) -> String {
    let mut out = format!("{} mL\n{}", adapt::num(x, 3), adapt::num(y, 3));
    if !unit.is_empty() {
        out.push(' ');
        out.push_str(unit);
    }
    if let Some(f) = fraction_at(run, x) {
        // Fall back to the tube number when the rack mapping is unresolved:
        // `well` is `None` for rack types the parser cannot lay out.
        let which = f
            .well
            .map(|w| w.label())
            .unwrap_or_else(|| format!("tube {}", f.tube));
        out.push_str(&format!("\nFraction {which}"));
    }
    if let Some(p) = peaks.iter().find(|p| p.covers(x)) {
        // Matches the "Peak {n}" wording of the peak-detail panel.
        out.push_str(&format!("\nPeak {}", p.id.0));
    }
    out
}

fn plot_group(
    ui: &mut Ui,
    run: &Run,
    view: &mut View,
    t: Theme,
    group: AxisGroup,
    height: f32,
    position: PlotPosition,
) -> (Option<Interaction>, Option<f32>) {
    let PlotPosition {
        is_hero,
        x_axis_label,
    } = position;
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

    // Detached before the plot is built so the hover closure borrows nothing the
    // `show` body below needs mutably.
    let readout_peaks = hover_peaks(run, &view.peaks, group);
    let readout_unit = unit.clone();

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
        .x_axis_label(x_axis_label)
        .legend(egui_plot::Legend::default().position(egui_plot::Corner::RightTop))
        .label_formatter(move |pos| {
            let p = match pos {
                egui_plot::HoverPosition::NearDataPoint { position, .. } => position,
                egui_plot::HoverPosition::Elsewhere { position } => position,
            };
            Some(hover_label(run, &readout_peaks, &readout_unit, p.x, p.y))
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

    let is_hero = view.hero_channel_id.as_ref() == Some(&channel.id);
    let selected = view.selected_channel.as_ref() == Some(&channel.id);

    let rgb = if is_hero {
        chart::PRIMARY_TRACE
    } else {
        let legend = channel.color.map(|c| crate::theme::Rgb::new(c.r, c.g, c.b));
        chart::legend_color_or_series(legend, t.panel_bg, index)
    };

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

                    let is_hero = view.hero_channel_id.as_ref() == Some(&channel.id);
                    let rgb = if is_hero {
                        chart::PRIMARY_TRACE
                    } else {
                        let legend = channel.color.map(|c| crate::theme::Rgb::new(c.r, c.g, c.b));
                        chart::legend_color_or_series(legend, t.panel_bg, i)
                    };

                    // A swatch plus the dash pattern drawn into it: channels past the
                    // eighth are told apart by shape, not hue alone (§10.4).
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(22.0, 10.0), egui::Sense::hover());
                    let painter = ui.painter();
                    let y = rect.center().y;
                    match chart::series_dash(i) {
                        chart::Dash::Solid => {
                            painter.line_segment(
                                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                                egui::Stroke::new(stroke::TRACE, c(rgb)),
                            );
                        }
                        chart::Dash::Dashed => {
                            for seg in 0..2 {
                                let x0 = rect.left() + seg as f32 * 12.0;
                                painter.line_segment(
                                    [egui::pos2(x0, y), egui::pos2(x0 + 8.0, y)],
                                    egui::Stroke::new(stroke::TRACE, c(rgb)),
                                );
                            }
                        }
                        chart::Dash::Dotted => {
                            for seg in 0..4 {
                                let x0 = rect.left() + seg as f32 * 6.0;
                                painter.line_segment(
                                    [egui::pos2(x0, y), egui::pos2(x0 + 2.0, y)],
                                    egui::Stroke::new(stroke::TRACE, c(rgb)),
                                );
                            }
                        }
                    }

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

    #[test]
    fn a_single_group_view_still_gets_the_x_axis_label() {
        // Regression: this is the overwhelmingly common UV-only view. The old
        // `is_hero` check blanked the label here because the one plot is both
        // the hero and the bottom of the stack.
        assert_eq!(x_axis_label_for(0, 1), "Elution volume (mL)");
    }

    #[test]
    fn a_stacked_view_labels_only_the_bottom_plot() {
        assert_eq!(x_axis_label_for(0, 3), "");
        assert_eq!(x_axis_label_for(1, 3), "");
        assert_eq!(x_axis_label_for(2, 3), "Elution volume (mL)");
    // --- hover readout ----------------------------------------------------

    use elusive_core::model::{
        BaselineMode, ChannelId, ChannelKind, RunMeta, Sample, SourceFormat, Well,
    };

    fn fraction(tube: u32, well: Option<Well>, start: f32, end: f32) -> Fraction {
        Fraction {
            tube,
            rack: 1,
            well,
            vol_start_ml: start,
            vol_end_ml: end,
            time_start_s: 0.0,
            time_end_s: 0.0,
            nominal_size_ml: Some(end - start),
            end_estimated: false,
            rack_type: "HEP96".into(),
            pattern: "Serpentine".into(),
        }
    }

    fn peak(id: u32, channel: &str, start: f32, end: f32) -> PeakResult {
        PeakResult {
            id: PeakId(id),
            channel_id: ChannelId::from(channel),
            v_start_ml: start,
            v_end_ml: end,
            baseline: BaselineMode::DropToZero,
            area: 1.0,
            height: 1.0,
            apex_volume_ml: (start + end) / 2.0,
            fwhm_ml: None,
            estimated_mw_kda: None,
        }
    }

    /// UV and conductivity channels, fractions over 10..13 mL, deliberately not
    /// stored in volume order.
    fn hover_run() -> Run {
        let mut uv = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        uv.samples = vec![Sample::new(0.0, 0.0, 0.0), Sample::new(600.0, 20.0, 1.0)];
        let mut cond = Channel::new("Cond", "Conductivity", ChannelKind::Conductivity);
        cond.samples = vec![Sample::new(0.0, 0.0, 0.0), Sample::new(600.0, 20.0, 1.0)];

        Run {
            meta: RunMeta::default(),
            source_format: SourceFormat::NgcAnalysis,
            source_path: std::path::PathBuf::from("t.ngcAnalysis"),
            channels: vec![uv, cond],
            fractions: vec![
                fraction(3, Some(Well::new(3, 7)), 12.0, 13.0),
                fraction(1, Some(Well::new(0, 0)), 10.0, 11.0),
                fraction(2, Some(Well::new(0, 1)), 11.0, 12.0),
            ],
            events: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn hover_inside_a_fraction_and_a_peak_reports_both() {
        let run = hover_run();
        let peaks = hover_peaks(&run, &[peak(3, "MWave2", 11.8, 12.6)], AxisGroup::Uv);
        let label = hover_label(&run, &peaks, "mAU", 12.48, 412.6);
        assert_eq!(label, "12.480 mL\n412.600 mAU\nFraction D8\nPeak 3");
    }

    #[test]
    fn hover_outside_every_window_reports_only_the_coordinates() {
        let run = hover_run();
        let peaks = hover_peaks(&run, &[peak(1, "MWave2", 11.0, 12.0)], AxisGroup::Uv);
        let label = hover_label(&run, &peaks, "mAU", 4.0, 1.5);
        // An "n/a" line would still cost the eye a read, so it is left out.
        assert_eq!(label, "4.000 mL\n1.500 mAU");
    }

    #[test]
    fn hover_inside_a_fraction_without_a_peak_omits_the_peak_line() {
        let run = hover_run();
        let label = hover_label(&run, &[], "mAU", 10.5, 22.0);
        assert_eq!(label, "10.500 mL\n22.000 mAU\nFraction A1");
    }

    #[test]
    fn a_peak_on_another_axis_group_is_not_reported() {
        let run = hover_run();
        let all = vec![peak(4, "Cond", 10.0, 11.0)];

        // Hovering UV: the conductivity peak's window is meaningless here.
        let uv = hover_peaks(&run, &all, AxisGroup::Uv);
        assert!(uv.is_empty());
        assert_eq!(
            hover_label(&run, &uv, "mAU", 10.5, 22.0),
            "10.500 mL\n22.000 mAU\nFraction A1"
        );

        // Hovering the conductivity plot: same peak, now in context.
        let cond = hover_peaks(&run, &all, AxisGroup::Conductivity);
        assert!(hover_label(&run, &cond, "mS/cm", 10.5, 3.0).ends_with("\nPeak 4"));
    }

    #[test]
    fn a_peak_whose_channel_is_gone_is_dropped_rather_than_shown_everywhere() {
        let run = hover_run();
        let orphan = vec![peak(9, "MWave0", 10.0, 11.0)];
        assert!(hover_peaks(&run, &orphan, AxisGroup::Uv).is_empty());
    }

    #[test]
    fn an_unmapped_rack_falls_back_to_the_tube_number() {
        let mut run = hover_run();
        run.fractions = vec![fraction(7, None, 10.0, 11.0)];
        let label = hover_label(&run, &[], "mAU", 10.5, 1.0);
        assert!(label.ends_with("\nFraction tube 7"), "{label}");
    }

    #[test]
    fn a_degenerate_fraction_window_is_ignored() {
        let mut run = hover_run();
        // A zero-width or reversed window cannot contain the pointer in any
        // meaningful sense; reporting it would name a tube at random.
        run.fractions = vec![
            fraction(1, Some(Well::new(0, 0)), 10.0, 10.0),
            fraction(2, Some(Well::new(0, 1)), f32::NAN, 11.0),
        ];
        assert_eq!(
            hover_label(&run, &[], "mAU", 10.0, 1.0),
            "10.000 mL\n1.000 mAU"
        );
    }

    #[test]
    fn a_channel_without_a_display_unit_still_reads_cleanly() {
        let run = hover_run();
        let label = hover_label(&run, &[], "", 4.0, 1.5);
        assert_eq!(
            label, "4.000 mL\n1.500",
            "no trailing space when unit is blank"
        );
    }
}

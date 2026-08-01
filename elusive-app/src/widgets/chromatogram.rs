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
use elusive_core::model::{AxisGroup, Channel, Run};

/// Fraction of the pane's height given to the hero (UV) group.
const HERO_HEIGHT_SHARE: f32 = 0.55;
const MIN_GROUP_HEIGHT: f32 = 90.0;

/// Height of a fraction boundary tick as a fraction of the plot's y-range.
/// Deliberately small: §10.2 requires the ticks not to span full height so the
/// raw trace stays dominant (rule #2).
const FRACTION_TICK_SHARE: f64 = 0.06;

pub fn show(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) -> Option<Interaction> {
    let groups = visible_groups(run, view);
    if groups.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("No channels are visible. Enable one in the legend.")
                    .color(c(t.text_secondary)),
            );
        });
        return None;
    }

    let mut interaction = None;
    let total = ui.available_height();
    let heights = group_heights(&groups, total);

    for (idx, (group, height)) in groups.iter().zip(heights).enumerate() {
        if let Some(action) = plot_group(ui, run, view, t, *group, height, idx == 0) {
            interaction = Some(action);
        }
    }
    interaction
}

/// Axis groups that currently have at least one visible channel, hero group first.
fn visible_groups(run: &Run, view: &View) -> Vec<AxisGroup> {
    let hero_group = run.hero_channel().map(|c| c.kind.axis_group());
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

fn group_heights(groups: &[AxisGroup], total: f32) -> Vec<f32> {
    if groups.len() == 1 {
        return vec![total];
    }
    let hero = (total * HERO_HEIGHT_SHARE).max(MIN_GROUP_HEIGHT);
    let rest = ((total - hero) / (groups.len() - 1) as f32).max(MIN_GROUP_HEIGHT);
    std::iter::once(hero)
        .chain(std::iter::repeat_n(rest, groups.len() - 1))
        .collect()
}

fn plot_group(
    ui: &mut Ui,
    run: &Run,
    view: &mut View,
    t: Theme,
    group: AxisGroup,
    height: f32,
    is_hero: bool,
) -> Option<Interaction> {
    let channels: Vec<(usize, &Channel)> = run
        .channels
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            !c.is_empty() && c.kind.axis_group() == group && view.is_channel_visible(&c.id)
        })
        .collect();
    if channels.is_empty() {
        return None;
    }

    let unit = channels
        .first()
        .map(|(_, c)| c.display_unit.clone())
        .unwrap_or_default();

    // While integrating, dragging draws a selection instead of panning.
    let integrating = view.integrate_mode;

    let mut interaction = None;
    let response = Plot::new(format!("chromatogram-{group:?}"))
        .height(height)
        .link_axis("chromatogram-x", [true, false])
        .allow_drag([!integrating, !integrating])
        .allow_boxed_zoom(!integrating)
        .show_grid([true, true])
        .y_axis_label(format!("{} ({unit})", group.label()))
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
            let bounds = plot_ui.plot_bounds();
            let (y_lo, y_hi) = (bounds.min()[1], bounds.max()[1]);
            let y_span = (y_hi - y_lo).max(f64::MIN_POSITIVE);

            // 1. Fraction bands sit *under* the traces so the signal stays on top.
            if is_hero {
                draw_fraction_ticks(plot_ui, run, view, t, y_lo, y_span);
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
                        "selection",
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

            if let Some(p) = pointer {
                let v = p.x as f32;
                view.hovered_volume = Some(v);
                // Hovering the trace highlights the fraction under the cursor,
                // which is the other half of the plate↔chart link.
                view.hovered_vol_range = run
                    .fractions
                    .iter()
                    .find(|f| {
                        let (a, b) = f.volume_window();
                        v >= a && v <= b
                    })
                    .map(|f| f.volume_window());
                if view.hovered_vol_range.is_none() {
                    view.hovered_well = None;
                } else {
                    view.hovered_well = run
                        .fractions
                        .iter()
                        .find(|f| {
                            let (a, b) = f.volume_window();
                            v >= a && v <= b
                        })
                        .and_then(|f| f.well);
                }
            } else if !response.hovered() {
                view.hovered_volume = None;
            }

            if integrating {
                if response.drag_started() {
                    view.drag_anchor = pointer.map(|p| p.x as f32);
                }
                if response.dragged() {
                    if let (Some(anchor), Some(p)) = (view.drag_anchor, pointer) {
                        view.pending_selection = Some((anchor, p.x as f32));
                    }
                }
                if response.drag_stopped() {
                    if let Some((a, b)) = view.pending_selection.take() {
                        if (b - a).abs() > f32::EPSILON {
                            interaction = Some(Interaction::IntegrateRange(a.min(b), a.max(b)));
                        }
                    }
                    view.drag_anchor = None;
                }
            }
        })
        .response;

    if !response.hovered() && view.pending_selection.is_none() {
        // Leaving the plot clears the link so a stale band does not linger.
        if view.hovered_well.is_none() {
            view.hovered_vol_range = None;
        }
    }

    interaction
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

/// Fraction boundaries as short ticks at the baseline (§10.2).
fn draw_fraction_ticks(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    run: &Run,
    view: &View,
    t: Theme,
    y_lo: f64,
    y_span: f64,
) {
    if !view.show_fractions || run.fractions.is_empty() {
        return;
    }
    let tick_top = y_lo + y_span * FRACTION_TICK_SHARE;
    let tick_color = c_alpha(t.axis, 160);

    for f in &run.fractions {
        let (a, _) = f.volume_window();
        if !a.is_finite() {
            continue;
        }
        plot_ui.line(
            Line::new(
                "",
                PlotPoints::from(vec![[a as f64, y_lo], [a as f64, tick_top]]),
            )
            .stroke(egui::Stroke::new(stroke::HAIRLINE, tick_color))
            .allow_hover(false),
        );
    }
    // Close the last fraction so the final window reads as bounded.
    if let Some(last) = run.fractions.last() {
        let (_, b) = last.volume_window();
        if b.is_finite() {
            plot_ui.line(
                Line::new(
                    "",
                    PlotPoints::from(vec![[b as f64, y_lo], [b as f64, tick_top]]),
                )
                .stroke(egui::Stroke::new(stroke::HAIRLINE, tick_color))
                .allow_hover(false),
            );
        }
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
            "fraction",
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
                "excluded",
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
            Polygon::new(peak.id.to_string(), PlotPoints::from(outline))
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
            let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 10.0), egui::Sense::hover());
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
                view.selected_channel = Some(channel.id.clone());
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
}

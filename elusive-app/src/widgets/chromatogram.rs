//! The chromatogram pane: stacked, x-linked plots — one per axis group.
//!
//! Channels are grouped by [`AxisGroup`] and each group gets its own plot with its
//! own y-scale, all linked on the shared x axis. The alternative — rescaling
//! conductivity onto the UV axis — would put a number on screen that is not the
//! number the instrument measured, so it is not on the table.
//!
//! The x axis shows either elution volume or elution time, per [`XAxis`]. That
//! choice stops at the edge of this module: everything entering it (peaks,
//! fractions, excluded regions, hover state) and everything leaving it
//! ([`ChartOutcome`], [`Interaction`]) is in mL. See [`PlotTransform`].

use crate::egui_adapter::{self as adapt, c, c_alpha, ca};
use crate::theme::{chart, color, spacing, stroke, Rgb, Theme};
use crate::view::{Interaction, View, XAxis};
use egui::Ui;
use egui_plot::{Line, Plot, PlotPoints, Polygon};
use elusive_core::model::{AxisGroup, Channel, Color, Fraction, PeakId, PeakResult, Run, Sample};

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
    /// Screen rect the stacked plots occupy, in logical points.
    ///
    /// Reported for the same reason as everything else here: the pane does not
    /// know that anyone wants to photograph it. `app` uses this to crop a
    /// framebuffer capture down to the chart. `None` when nothing was drawn.
    pub rect: Option<egui::Rect>,
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
            x_axis_label: x_axis_label_for(idx, count, view.x_axis),
        };
        let (interaction, hovered, rect) = plot_group(ui, run, view, t, *group, height, position);
        if interaction.is_some() {
            outcome.interaction = interaction;
        }
        // Only the plot actually under the pointer reports a volume, so a later
        // plot in the stack cannot erase an earlier one's hover.
        if hovered.is_some() {
            outcome.hovered_volume = hovered;
        }
        if rect.is_positive() {
            outcome.rect = Some(match outcome.rect {
                Some(sofar) => sofar.union(rect),
                None => rect,
            });
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
/// The stack is x-linked, so repeating the axis name under every plot is
/// clutter — but it still has to appear *somewhere*, including in the
/// overwhelmingly common single-group (UV-only) view. Only the bottom-most
/// plot gets it. The wording follows the axis the user selected, because a plot
/// labelled in mL while it is drawn in minutes is worse than no label at all.
fn x_axis_label_for(idx: usize, count: usize, axis: XAxis) -> &'static str {
    if count > 0 && idx == count - 1 {
        axis.label()
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

/// Everything a drawing function needs to turn stored data into plot coordinates.
///
/// The two axes are bundled rather than threaded separately because every overlay
/// needs both, and because they are wanted at different times: the x half is the
/// unit toggle, the y half is the place the planned per-group y-scaling will
/// attach. Adding the pair once keeps the six drawing functions' signatures stable
/// across both features.
#[derive(Clone, Copy)]
struct PlotTransform<'a> {
    x: XMap<'a>,
    y: YMap,
}

impl<'a> PlotTransform<'a> {
    fn new(run: &'a Run, view: &View) -> Self {
        Self {
            x: XMap::new(run, view),
            y: YMap::IDENTITY,
        }
    }
}

/// Placeholder for the per-group y transform.
///
/// Today every axis group draws in its own plot at its own natural scale, so
/// there is nothing to remap and [`YMap::apply`] hands its argument straight
/// back — deliberately, so current rendering is bit-identical to the code before
/// the transform existed. The planned multi-y-scale feature (several groups
/// sharing one plot, each with its own scale) fills this in with the offset and
/// gain for a group; every call site that will need it already routes through
/// here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct YMap;

impl YMap {
    const IDENTITY: YMap = YMap;

    /// Display value → plot y. Exactly the identity: no arithmetic, so no
    /// rounding is introduced on the default path.
    #[inline]
    fn apply(self, y: f64) -> f64 {
        y
    }

    /// The group's y-extent, given the extent of its data.
    ///
    /// Separate from [`YMap::apply`] because a rescaling map does not
    /// necessarily produce its axis extent by mapping the data extent — a
    /// normalising one draws on a fixed unitless span regardless of the data.
    /// Overlays are sized from the value this returns, so `data_y_range`'s
    /// no-feedback guarantee still holds either way.
    #[inline]
    fn extent(self, data: (f64, f64)) -> (f64, f64) {
        data
    }
}

/// Translates between stored volume (mL) and the x coordinate on screen.
///
/// The stored model is always in mL. Time is a *display* transform applied on the
/// way into the plot and undone on the way out, so an integration dragged while
/// the time axis is active still produces the volume window the user pointed at,
/// and a sidecar written afterwards is identical either way.
///
/// Traces never need this: every [`Sample`] already carries `time_s` alongside
/// `volume_ml`, so a trace plots a different field of the same point. Only
/// overlays that know a volume and nothing else (fractions, peaks, excluded
/// regions, the hovered span) need a channel to interpolate against.
#[derive(Clone, Copy)]
struct XMap<'a> {
    axis: XAxis,
    /// Channel supplying the volume↔time relation. `None` when the run has no
    /// samples to interpolate from, in which case time-axis overlays are skipped
    /// rather than drawn in the wrong place.
    reference: Option<&'a Channel>,
}

impl<'a> XMap<'a> {
    fn new(run: &'a Run, view: &View) -> Self {
        // The hero channel is the one the user is reading, so its time base is the
        // one to map against; any channel with samples will do as a fallback,
        // because time and volume are properties of the run, not of a detector.
        let reference = view
            .hero_channel_id
            .as_ref()
            .and_then(|id| run.channel(id))
            .filter(|c| !c.is_empty())
            .or_else(|| run.hero_channel())
            .or_else(|| run.channels.iter().find(|c| !c.is_empty()));
        Self {
            axis: view.x_axis,
            reference,
        }
    }

    /// x for a sample of a plotted trace — no interpolation, the point knows both.
    fn sample_x(self, s: &Sample) -> f64 {
        match self.axis {
            XAxis::Volume => s.volume_ml as f64,
            XAxis::Time => s.time_s as f64 / 60.0,
        }
    }

    /// x for a stored volume, or `None` when time mode has nothing to map with.
    fn to_x(self, volume_ml: f32) -> Option<f64> {
        match self.axis {
            XAxis::Volume => Some(volume_ml as f64),
            XAxis::Time => clamped_time_min(self.reference?, volume_ml).map(|t| t as f64),
        }
    }

    /// Both ends of a window, or `None` if either end cannot be mapped. Overlays
    /// are all-or-nothing: half a fraction band is worse than none.
    fn to_window(self, a: f32, b: f32) -> Option<(f64, f64)> {
        Some((self.to_x(a)?, self.to_x(b)?))
    }

    /// A pointer coordinate back to the volume the model speaks, refusing to
    /// answer outside the run.
    ///
    /// Used for hover linking, where "past the end of the data" must stay
    /// distinguishable from "at the last sample" — otherwise pointing into the
    /// empty space right of the trace would light up the final fraction.
    fn to_volume(self, x: f32) -> Option<f32> {
        match self.axis {
            XAxis::Volume => x.is_finite().then_some(x),
            XAxis::Time => self.reference?.volume_ml_at_time_min(x),
        }
    }

    /// The same inverse, clamped, for turning a finished drag into an integration
    /// window.
    ///
    /// A drag that overshoots the trace means "to the end of the run", which is
    /// what `integrate::integrate_peak` already does with an out-of-range
    /// endpoint. Refusing the conversion here would silently drop the whole
    /// integration instead.
    fn to_volume_clamped(self, x: f32) -> Option<f32> {
        match self.axis {
            XAxis::Volume => x.is_finite().then_some(x),
            XAxis::Time => clamped_volume_ml(self.reference?, x),
        }
    }

    /// The pointer's position expressed in the *other* unit, for the hover
    /// readout. Strict, so the second line simply disappears off the ends of the
    /// data rather than repeating the last sample.
    fn counterpart(self, x: f32) -> Option<f64> {
        let reference = self.reference?;
        match self.axis {
            XAxis::Volume => reference.time_min_at_volume(x).map(|t| t as f64),
            XAxis::Time => reference.volume_ml_at_time_min(x).map(|v| v as f64),
        }
    }
}

/// Time in minutes at a volume, clamped to the channel's sampled range.
///
/// Mirrors `integrate::endpoint_value`'s reasoning: a fraction window that ends a
/// hair past the last UV sample, or a drag that overshoots the trace, should pin
/// to the end of the run rather than make the overlay disappear.
fn clamped_time_min(channel: &Channel, volume_ml: f32) -> Option<f32> {
    if let Some(t) = channel.time_min_at_volume(volume_ml) {
        return Some(t);
    }
    if !volume_ml.is_finite() {
        return None;
    }
    // Non-finite samples are padding in some exports, so clamp to the first and
    // last *usable* points rather than to `samples[0]` and `samples[len - 1]`.
    let first = channel.samples.iter().find(|s| s.is_finite())?;
    let last = channel.samples.iter().rev().find(|s| s.is_finite())?;
    Some(if volume_ml <= first.volume_ml {
        first.time_s / 60.0
    } else {
        last.time_s / 60.0
    })
}

/// Volume at a time in minutes, clamped to the channel's sampled range.
fn clamped_volume_ml(channel: &Channel, time_min: f32) -> Option<f32> {
    if let Some(v) = channel.volume_ml_at_time_min(time_min) {
        return Some(v);
    }
    if !time_min.is_finite() {
        return None;
    }
    let first = channel.samples.iter().find(|s| s.is_finite())?;
    let last = channel.samples.iter().rev().find(|s| s.is_finite())?;
    Some(if time_min * 60.0 <= first.time_s {
        first.volume_ml
    } else {
        last.volume_ml
    })
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
///
/// `x` is in whatever unit `axis` names and `counterpart` is the same position in
/// the other unit, which `DESIGN_SYSTEM.md` §10.1 asks to keep in view. That
/// second value is also what makes the fraction and peak lines work off the
/// volume axis: in time mode the counterpart *is* the volume, and its strictness
/// off the ends of the data is exactly the behaviour those lookups want.
fn hover_label(
    run: &Run,
    peaks: &[HoverPeak],
    unit: &str,
    axis: XAxis,
    x: f64,
    y: f64,
    counterpart: Option<f64>,
) -> String {
    let mut out = format!("{} {}", adapt::num(x, 3), axis.unit());
    if let Some(other) = counterpart {
        out.push_str(&format!(
            "\n{} {}",
            adapt::num(other, 3),
            axis.other().unit()
        ));
    }
    out.push('\n');
    out.push_str(&adapt::num(y, 3));
    if !unit.is_empty() {
        out.push(' ');
        out.push_str(unit);
    }

    // Fractions and peaks are stored in mL, so they are looked up in mL.
    let volume_ml = match axis {
        XAxis::Volume => Some(x),
        XAxis::Time => counterpart,
    };
    let Some(v) = volume_ml else {
        return out;
    };
    if let Some(f) = fraction_at(run, v) {
        // Fall back to the tube number when the rack mapping is unresolved:
        // `well` is `None` for rack types the parser cannot lay out.
        let which = f
            .well
            .map(|w| w.label())
            .unwrap_or_else(|| format!("tube {}", f.tube));
        out.push_str(&format!("\nFraction {which}"));
    }
    if let Some(p) = peaks.iter().find(|p| p.covers(v)) {
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
) -> (Option<Interaction>, Option<f32>, egui::Rect) {
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
        return (None, None, egui::Rect::NOTHING);
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

    let tf = PlotTransform::new(run, view);
    let axis = view.x_axis;
    let readout_x = tf.x;

    let mut interaction = None;
    let mut hovered_volume = None;
    // The axis key is part of both ids on purpose. egui_plot remembers pan/zoom
    // per plot id and shares bounds per link group, and those bounds are bare
    // numbers: switching unit under a remembered `0..38` window would leave the
    // plot showing 0–38 *minutes* of a 75-minute run. Keying by axis gives each
    // mode its own remembered view, so a switch lands on auto-fit bounds and a
    // switch back restores where the user was.
    let plotted = Plot::new(format!("chromatogram-{group:?}-{}", axis.key()))
        .height(height)
        .sense(egui::Sense::click_and_drag())
        .link_axis(format!("chromatogram-x-{}", axis.key()), [true, false])
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
            Some(hover_label(
                run,
                &readout_peaks,
                &readout_unit,
                axis,
                p.x,
                p.y,
                readout_x.counterpart(p.x as f32),
            ))
        })
        .show(ui, |plot_ui| {
            // Fixed, data-derived extent. Deliberately NOT `plot_ui.plot_bounds()`
            // — see `data_y_range`. The drawers below receive it already mapped,
            // so none of them re-applies the y transform to it.
            let (y_lo, y_hi) = tf.y.extent(data_y_range(&channels));

            // 1. Fraction bands sit *under* the traces so the signal stays on top.
            if is_hero {
                draw_fraction_zones(plot_ui, run, view, t, tf, y_lo, y_hi);
                draw_highlighted_span(plot_ui, view, t, tf, y_lo, y_hi);
                draw_excluded_regions(plot_ui, view, t, tf, y_lo, y_hi);
            }

            // 2. Integrated peak regions, translucent (rule #2).
            draw_peak_regions(plot_ui, view, group, run, tf, y_lo);

            // 3. The traces themselves.
            for (i, channel) in &channels {
                draw_channel(plot_ui, channel, *i, view, t, tf);
            }

            // 4. The pending drag selection, so the user sees the window forming.
            //    Already in display units — see the drag handling below — so it
            //    needs no x mapping.
            if let Some((a, b)) = view.pending_selection {
                let (a, b) = (a as f64, b as f64);
                let fill = c_alpha(chart::SELECTION_STROKE, 40);
                plot_ui.polygon(
                    Polygon::new(
                        "",
                        PlotPoints::from(vec![[a, y_lo], [b, y_lo], [b, y_hi], [a, y_hi]]),
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

            // Reported in mL whatever the axis shows: the plate, the fraction
            // table and every other pane are keyed to volume.
            if response.hovered() {
                hovered_volume = pointer.and_then(|p| tf.x.to_volume(p.x as f32));
            }

            if integrating {
                // `drag_anchor` and `pending_selection` stay in display units —
                // they only exist to draw the band being dragged, and `View` drops
                // them if the axis changes underneath.
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
                        // Back to mL before the interaction leaves this module:
                        // `Interaction::IntegrateRange` and `integrate_peak` are
                        // defined in volume, so raising a range in minutes would
                        // integrate a plausible-looking but wrong window.
                        if let (Some(v0), Some(v1)) =
                            (tf.x.to_volume_clamped(a), tf.x.to_volume_clamped(b))
                        {
                            if (v1 - v0).abs() > f32::EPSILON {
                                interaction =
                                    Some(Interaction::IntegrateRange(v0.min(v1), v0.max(v1)));
                            }
                        }
                    }
                    view.drag_anchor = None;
                }
            }
        });

    // The plot's own rect, not `ui.min_rect()`: that would also cover whatever
    // padding the parent card contributes, which is not part of the chart.
    (interaction, hovered_volume, plotted.response.rect)
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
    tf: PlotTransform<'_>,
) {
    // Each channel builds its own point list from its own samples — no shared
    // index is assumed anywhere (`model.rs` invariant 1). That extends to the x
    // axis: a channel sampled at its own rate reads its *own* time base, so
    // nothing is interpolated through another channel to draw a trace.
    let points: Vec<[f64; 2]> = channel
        .samples
        .iter()
        .filter(|s| s.is_finite())
        .map(|s| {
            [
                tf.x.sample_x(s),
                tf.y.apply((s.value * channel.display_scale) as f64),
            ]
        })
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
    tf: PlotTransform<'_>,
    y_lo: f64,
    y_hi: f64,
) {
    if !view.show_fractions || run.fractions.is_empty() {
        return;
    }
    for (idx, f) in run.fractions.iter().enumerate() {
        let (v0, v1) = f.volume_window();
        if !v0.is_finite() || !v1.is_finite() || v1 <= v0 {
            continue;
        }
        // A fraction records both a volume and a time window, but only the volume
        // one is reconciled and corrected by the parser, so the displayed band is
        // always the volume window mapped onto the current axis.
        let Some((a, b)) = tf.x.to_window(v0, v1) else {
            continue;
        };
        let alpha = if idx % 2 == 0 {
            FRACTION_ZONE_ALPHA
        } else {
            FRACTION_ZONE_ALPHA.saturating_add(6)
        };
        plot_ui.polygon(
            Polygon::new(
                "",
                PlotPoints::from(vec![[a, y_lo], [b, y_lo], [b, y_hi], [a, y_hi]]),
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
    tf: PlotTransform<'_>,
    y_lo: f64,
    y_hi: f64,
) {
    // Shared hover state is in mL — the plate writes it too — so it maps like any
    // other overlay.
    let Some((v0, v1)) = view.hovered_vol_range else {
        return;
    };
    let Some((a, b)) = tf.x.to_window(v0, v1) else {
        return;
    };
    plot_ui.polygon(
        Polygon::new(
            "",
            PlotPoints::from(vec![[a, y_lo], [b, y_lo], [b, y_hi], [a, y_hi]]),
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
    tf: PlotTransform<'_>,
    y_lo: f64,
    y_hi: f64,
) {
    for region in &view.excluded_regions {
        // Stored in mL and saved that way; only the drawing moves.
        let Some((a, b)) = tf.x.to_window(region.v_start_ml, region.v_end_ml) else {
            continue;
        };
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
    tf: PlotTransform<'_>,
    y_lo: f64,
) {
    for peak in &view.peaks {
        let Some(channel) = run.channel(&peak.channel_id) else {
            continue;
        };
        if channel.kind.axis_group() != group {
            continue;
        }

        // The window is selected in volume — the unit the peak was integrated in —
        // and only the resulting points are moved onto the display axis.
        let mut outline: Vec<[f64; 2]> = channel
            .samples_in_volume(peak.v_start_ml, peak.v_end_ml)
            .iter()
            .filter(|s| s.is_finite())
            .map(|s| {
                [
                    tf.x.sample_x(s),
                    tf.y.apply((s.value * channel.display_scale) as f64),
                ]
            })
            .collect();
        if outline.len() < 2 {
            continue;
        }

        let Some((x_start, x_end)) = tf.x.to_window(peak.v_start_ml, peak.v_end_ml) else {
            continue;
        };
        let baseline_y = baseline_points(peak, channel, tf, y_lo, x_start, x_end);
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
///
/// `x_start`/`x_end` are the window's edges already mapped onto the current axis;
/// the baseline *values* are still worked out in volume, because that is the
/// geometry the integration used and it must not change with the view.
fn baseline_points(
    peak: &elusive_core::model::PeakResult,
    channel: &Channel,
    tf: PlotTransform<'_>,
    y_lo: f64,
    x_start: f64,
    x_end: f64,
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

    vec![[x_start, tf.y.apply(y0)], [x_end, tf.y.apply(y1)]]
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

    #[test]
    fn a_single_group_view_still_gets_the_x_axis_label() {
        // Regression: this is the overwhelmingly common UV-only view. The old
        // `is_hero` check blanked the label here because the one plot is both
        // the hero and the bottom of the stack.
        assert_eq!(x_axis_label_for(0, 1, XAxis::Volume), "Elution volume (mL)");
    }

    #[test]
    fn a_stacked_view_labels_only_the_bottom_plot() {
        assert_eq!(x_axis_label_for(0, 3, XAxis::Volume), "");
        assert_eq!(x_axis_label_for(1, 3, XAxis::Volume), "");
        assert_eq!(x_axis_label_for(2, 3, XAxis::Volume), "Elution volume (mL)");
    }

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
        let label = hover_label(&run, &peaks, "mAU", XAxis::Volume, 12.48, 412.6, None);
        assert_eq!(label, "12.480 mL\n412.600 mAU\nFraction D8\nPeak 3");
    }

    #[test]
    fn hover_outside_every_window_reports_only_the_coordinates() {
        let run = hover_run();
        let peaks = hover_peaks(&run, &[peak(1, "MWave2", 11.0, 12.0)], AxisGroup::Uv);
        let label = hover_label(&run, &peaks, "mAU", XAxis::Volume, 4.0, 1.5, None);
        // An "n/a" line would still cost the eye a read, so it is left out.
        assert_eq!(label, "4.000 mL\n1.500 mAU");
    }

    #[test]
    fn hover_inside_a_fraction_without_a_peak_omits_the_peak_line() {
        let run = hover_run();
        let label = hover_label(&run, &[], "mAU", XAxis::Volume, 10.5, 22.0, None);
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
            hover_label(&run, &uv, "mAU", XAxis::Volume, 10.5, 22.0, None),
            "10.500 mL\n22.000 mAU\nFraction A1"
        );

        // Hovering the conductivity plot: same peak, now in context.
        let cond = hover_peaks(&run, &all, AxisGroup::Conductivity);
        assert!(
            hover_label(&run, &cond, "mS/cm", XAxis::Volume, 10.5, 3.0, None).ends_with("\nPeak 4")
        );
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
        let label = hover_label(&run, &[], "mAU", XAxis::Volume, 10.5, 1.0, None);
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
            hover_label(&run, &[], "mAU", XAxis::Volume, 10.0, 1.0, None),
            "10.000 mL\n1.000 mAU"
        );
    }

    #[test]
    fn a_channel_without_a_display_unit_still_reads_cleanly() {
        let run = hover_run();
        let label = hover_label(&run, &[], "", XAxis::Volume, 4.0, 1.5, None);
        assert_eq!(
            label, "4.000 mL\n1.500",
            "no trailing space when unit is blank"
        );
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

    // --- x-axis mapping ---------------------------------------------------

    /// Two minutes at 1 mL/min then two at 0.5 mL/min, so a constant-rate
    /// assumption and an interpolation disagree everywhere past 2 mL.
    fn mapping_run() -> Run {
        let mut uv = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        uv.samples = vec![
            Sample::new(0.0, 0.0, 0.0),
            Sample::new(120.0, 2.0, 1.0),
            Sample::new(240.0, 3.0, 0.5),
        ];
        Run {
            meta: RunMeta::default(),
            source_format: SourceFormat::NgcAnalysis,
            source_path: std::path::PathBuf::from("test.ngcAnalysis"),
            channels: vec![uv],
            fractions: Vec::new(),
            events: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn map_for(run: &Run, axis: XAxis) -> XMap<'_> {
        let mut view = View::default();
        view.adopt_run(run);
        view.set_x_axis(axis);
        XMap::new(run, &view)
    }

    #[test]
    fn the_volume_axis_is_the_identity() {
        let run = mapping_run();
        let map = map_for(&run, XAxis::Volume);
        assert_eq!(map.to_x(2.5), Some(2.5));
        assert_eq!(map.to_volume(2.5), Some(2.5));
        assert_eq!(map.to_volume_clamped(99.0), Some(99.0));
        assert_eq!(map.to_volume(f32::NAN), None);
    }

    #[test]
    fn the_time_axis_maps_overlays_through_the_reference_channel() {
        let run = mapping_run();
        let map = map_for(&run, XAxis::Time);
        // 2.5 mL falls in the slow half, so it is 3 min in, not 2.5.
        let x = map.to_x(2.5).expect("inside the run");
        assert!((x - 3.0).abs() < 1e-4, "x = {x}");
        let (a, b) = map.to_window(0.0, 2.0).expect("inside the run");
        assert!((a - 0.0).abs() < 1e-4 && (b - 2.0).abs() < 1e-4, "{a}..{b}");
    }

    #[test]
    fn a_trace_reads_its_own_time_base_rather_than_being_interpolated() {
        let run = mapping_run();
        let map = map_for(&run, XAxis::Time);
        // 90 s is 1.5 min whatever the reference channel says about volume.
        assert_eq!(map.sample_x(&Sample::new(90.0, 0.4, 0.0)), 1.5);
        let x = map_for(&run, XAxis::Volume).sample_x(&Sample::new(90.0, 0.4, 0.0));
        assert!((x - 0.4).abs() < 1e-6, "x = {x}");
    }

    #[test]
    fn a_drag_in_minutes_becomes_the_volume_window_the_user_pointed_at() {
        // The regression this whole feature risks: integrating in time mode must
        // hand `integrate_peak` mL, and the mL the pointer was actually over.
        let run = mapping_run();
        let map = map_for(&run, XAxis::Time);
        let v0 = map.to_volume_clamped(1.0).expect("inside the run");
        let v1 = map.to_volume_clamped(3.0).expect("inside the run");
        assert!((v0 - 1.0).abs() < 1e-4, "v0 = {v0}");
        assert!((v1 - 2.5).abs() < 1e-4, "v1 = {v1}");
    }

    #[test]
    fn a_drag_past_the_end_of_the_run_integrates_to_the_end_of_the_run() {
        let run = mapping_run();
        let map = map_for(&run, XAxis::Time);
        assert_eq!(map.to_volume_clamped(99.0), Some(3.0));
        assert_eq!(map.to_volume_clamped(-5.0), Some(0.0));
        // Hover stays strict: past the data is not "at the last fraction".
        assert_eq!(map.to_volume(99.0), None);
    }

    #[test]
    fn overlays_are_pinned_to_the_ends_rather_than_disappearing() {
        // A fraction that closes just after the last UV sample must still draw.
        let run = mapping_run();
        let map = map_for(&run, XAxis::Time);
        let (a, b) = map.to_window(2.5, 3.5).expect("clamped, not dropped");
        assert!((a - 3.0).abs() < 1e-4, "a = {a}");
        assert!((b - 4.0).abs() < 1e-4, "b = {b}");
    }

    #[test]
    fn a_run_without_samples_maps_nothing_instead_of_panicking() {
        let mut run = mapping_run();
        run.channels[0].samples.clear();
        let map = map_for(&run, XAxis::Time);
        assert!(map.reference.is_none());
        assert_eq!(map.to_x(1.0), None);
        assert_eq!(map.to_volume(1.0), None);
        assert_eq!(map.to_volume_clamped(1.0), None);
        assert_eq!(map.counterpart(1.0), None);
    }

    #[test]
    fn the_hover_readout_names_its_unit_and_keeps_the_other_one_in_view() {
        let run = mapping_run();
        let volume = map_for(&run, XAxis::Volume);
        let label = hover_label(
            &run,
            &[],
            "",
            XAxis::Volume,
            2.5,
            0.75,
            volume.counterpart(2.5),
        );
        assert_eq!(label, "2.500 mL\n3.000 min\n0.750");

        let time = map_for(&run, XAxis::Time);
        let label = hover_label(&run, &[], "", XAxis::Time, 3.0, 0.75, time.counterpart(3.0));
        assert_eq!(label, "3.000 min\n2.500 mL\n0.750");

        // Off the ends of the data the secondary line is dropped rather than
        // invented.
        let label = hover_label(
            &run,
            &[],
            "",
            XAxis::Volume,
            9.0,
            0.1,
            volume.counterpart(9.0),
        );
        assert_eq!(label, "9.000 mL\n0.100");
    }

    #[test]
    fn the_time_axis_still_names_the_fraction_and_peak_under_the_pointer() {
        // The fraction and peak lines are looked up in mL, so they must survive
        // the pointer being reported in minutes.
        let mut run = mapping_run();
        run.fractions = vec![fraction(1, Some(Well::new(0, 0)), 2.4, 2.6)];
        let map = map_for(&run, XAxis::Time);
        let peaks = hover_peaks(&run, &[peak(2, "MWave2", 2.4, 2.6)], AxisGroup::Uv);

        // 3 min is 2.5 mL, inside both windows.
        let label = hover_label(
            &run,
            &peaks,
            "mAU",
            XAxis::Time,
            3.0,
            0.75,
            map.counterpart(3.0),
        );
        assert_eq!(label, "3.000 min\n2.500 mL\n0.750 mAU\nFraction A1\nPeak 2");

        // Past the data there is no volume to look anything up with, so neither
        // line is invented.
        let label = hover_label(
            &run,
            &peaks,
            "mAU",
            XAxis::Time,
            99.0,
            0.1,
            map.counterpart(99.0),
        );
        assert_eq!(label, "99.000 min\n0.100 mAU");
    }

    // --- y transform ------------------------------------------------------

    #[test]
    fn the_y_map_is_exactly_the_identity_for_now() {
        // Exact equality, not an epsilon: the placeholder must introduce no
        // arithmetic at all, so today's rendering is bit-identical to the code
        // before `PlotTransform` existed. `feat/multi-y-scales` replaces the body
        // of `YMap::apply`; when it does, this test is the one to rewrite.
        for y in [
            0.0f64,
            -0.0,
            1.0,
            -412.6,
            f64::MIN_POSITIVE,
            f64::MAX,
            0.1 + 0.2,
        ] {
            assert!(
                YMap::IDENTITY.apply(y) == y,
                "y = {y} came back as {}",
                YMap::IDENTITY.apply(y)
            );
        }
        assert!(YMap::IDENTITY.apply(f64::NAN).is_nan());

        // The extent hook is likewise exactly pass-through, so overlays are still
        // sized from `data_y_range` and nothing re-enters the plot's auto-bounds.
        let data = data_y_range(&[]);
        assert!(YMap::IDENTITY.extent(data) == data);
        assert!(YMap::IDENTITY.extent((-412.6, 1e300)) == (-412.6, 1e300));

        assert_eq!(PlotTransform::new(&mapping_run(), &View::default()).y, YMap);
    }
}

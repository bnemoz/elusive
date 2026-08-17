//! The chromatogram pane: stacked, x-linked plots — one per axis group.
//!
//! Channels are grouped by [`AxisGroup`] and each group gets its own plot with its
//! own y-scale, all linked on the shared x axis. The alternative — rescaling
//! conductivity onto the UV axis — would put a number on screen that is not the
//! number the instrument measured, so it is not on the table.
//!
//! Within a group the same tension reappears for a legitimate reason: a 280 nm
//! trace at 2000 mAU and a 260 nm trace at 40 mAU share an axis *and* a unit, but
//! the small one is a flat line along the bottom and cannot be read.
//! [`YScaleMode`] answers that by remapping each trace onto a shared **unitless**
//! axis — and the module keeps its principle by refusing to label that axis in
//! mAU once it no longer means mAU. When per-trace scaling is on, the axis reads
//! "relative", the hover readout says so, and every scaled channel carries a text
//! badge in the legend. A normalized overlay that still looks like a true
//! comparison is exactly the kind of plausible-but-wrong picture this tool exists
//! to avoid.
//!
//! The x axis shows either elution volume or elution time, per [`XAxis`]. That
//! choice stops at the edge of this module: everything entering it (peaks,
//! fractions, excluded regions, hover state) and everything leaving it
//! ([`ChartOutcome`], [`Interaction`]) is in mL. See [`PlotTransform`].

use crate::egui_adapter::{self as adapt, c, c_alpha, ca};
use crate::overlay::Overlay;
use crate::theme::{chart, color, spacing, stroke, Rgb, Theme};
use crate::view::{Interaction, View, XAxis, YScaleMode};
use egui::Ui;
use egui_plot::{Line, Plot, PlotPoints, Polygon};
use elusive_core::model::{
    AxisGroup, Channel, ChannelId, Color, Fraction, PeakId, PeakResult, Run, Sample,
};

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
    /// Screen rect the stacked plots occupy, in logical points, unioned with the
    /// relative-axis caveat when that is showing.
    ///
    /// Reported for the same reason as everything else here: the pane does not
    /// know that anyone wants to photograph it. `app` uses this to crop a
    /// framebuffer capture down to the chart. `None` when nothing was drawn.
    ///
    /// The caveat is inside the crop deliberately: a PNG of normalized traces
    /// that leaves the "heights are not comparable" line behind is precisely the
    /// plausible-but-wrong picture the mode exists to prevent.
    pub rect: Option<egui::Rect>,
}

pub fn show(
    ui: &mut Ui,
    run: &Run,
    overlays: &[Overlay],
    view: &mut View,
    t: Theme,
) -> ChartOutcome {
    let groups = visible_groups(run, overlays, view);
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
    if view.y_scale_mode.is_per_trace() {
        let note = relative_axis_note(ui, t);
        if note.is_positive() {
            outcome.rect = Some(note);
        }
    }
    if view.x_axis == XAxis::Time && overlays.iter().any(|o| o.visible && o.x_offset_ml != 0.0) {
        let note = offset_ignored_note(ui, t);
        if note.is_positive() {
            outcome.rect = Some(match outcome.rect {
                Some(sofar) => sofar.union(note),
                None => note,
            });
        }
    }
    // Read *after* the note, so the plots are laid out in what is actually left
    // rather than in space the note has already taken.
    let total = ui.available_height();
    let heights = group_heights(&groups, total);
    let count = groups.len();

    for (idx, (group, height)) in groups.iter().zip(heights).enumerate() {
        let position = PlotPosition {
            is_hero: idx == 0,
            x_axis_label: x_axis_label_for(idx, count, view.x_axis),
        };
        let (interaction, hovered, rect) =
            plot_group(ui, run, overlays, view, t, *group, height, position);
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

/// The standing caveat that per-trace scaling removes the one thing a shared
/// axis gives you: comparable heights.
///
/// Always on screen while the mode is active rather than shown once and
/// dismissed — someone reading over a colleague's shoulder, or a screenshot in a
/// notebook, has to carry the caveat with the picture. Text carries the meaning,
/// so it survives colour-blind vision and a greyscale print (rule #3).
///
/// Returns the rect it drew into, which joins [`ChartOutcome::rect`] so the PNG
/// export crops around it rather than cutting it off.
fn relative_axis_note(ui: &mut Ui, t: Theme) -> egui::Rect {
    let drawn = ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new("Relative y-axis")
                .font(adapt::font_micro())
                .color(c(color::WARNING_600)),
        );
        ui.label(
            egui::RichText::new(
                "— each trace is scaled to its own range, so heights are not comparable \
                 between traces. Per-trace ranges are in the channel legend.",
            )
            .font(adapt::font_micro())
            .color(c(t.text_secondary)),
        );
    });
    ui.add_space(spacing::XS);
    drawn.response.rect
}

/// The standing caveat that a comparison run's x-offset is not being applied.
///
/// Shown while the time axis is active and any visible overlay carries a
/// nonzero offset. Same always-on-screen reasoning as [`relative_axis_note`]:
/// a screenshot of misaligned traces must carry the reason for the
/// misalignment with it.
fn offset_ignored_note(ui: &mut Ui, t: Theme) -> egui::Rect {
    let drawn = ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new("X offsets not applied")
                .font(adapt::font_micro())
                .color(c(color::WARNING_600)),
        );
        ui.label(
            egui::RichText::new(
                "— comparison-run offsets are in mL and apply on the volume axis only; \
                 on the time axis every trace is at its own recorded time.",
            )
            .font(adapt::font_micro())
            .color(c(t.text_secondary)),
        );
    });
    ui.add_space(spacing::XS);
    drawn.response.rect
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
fn visible_groups(run: &Run, overlays: &[Overlay], view: &View) -> Vec<AxisGroup> {
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
    // A comparison run can bring a channel kind the primary never recorded —
    // its plot still deserves to exist, or the overlay would silently not show.
    for overlay in overlays.iter().filter(|o| o.visible) {
        groups.extend(
            overlay
                .run
                .channels
                .iter()
                .filter(|c| !c.is_empty() && overlay.is_channel_visible(&c.id))
                .map(|c| c.kind.axis_group()),
        );
    }
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

/// One comparison-run channel resolved for drawing on a group's plot.
///
/// Resolved once per group rather than inside the plot closure, so the same
/// facts feed the y-extent, the drawing and the legend name. Identity is the
/// dash pattern plus the run-qualified name — never colour alone (rule #3);
/// colour deliberately follows the same resolution a primary channel gets, so
/// UV 280 shares a hue across runs and the dash is what says "other run".
struct OverlayTrace<'a> {
    channel: &'a Channel,
    /// The channel's index within its own run, for colour cycling.
    channel_index: usize,
    dash: chart::Dash,
    /// Run-qualified legend name, e.g. `2026-08-02 prep · UV 280 nm`.
    name: String,
    /// Display-only x shift in mL; ignored on the time axis.
    offset_ml: f32,
}

/// The overlay channels that belong on one axis group's plot.
///
/// Dash identity keys off each overlay's position in the full list, not off
/// how many are currently visible, so hiding one run never restyles another.
fn overlay_traces(overlays: &[Overlay], group: AxisGroup) -> Vec<OverlayTrace<'_>> {
    overlays
        .iter()
        .enumerate()
        .filter(|(_, o)| o.visible)
        .flat_map(|(run_index, o)| {
            o.run
                .channels
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    !c.is_empty() && c.kind.axis_group() == group && o.is_channel_visible(&c.id)
                })
                .map(move |(channel_index, channel)| OverlayTrace {
                    channel,
                    channel_index,
                    dash: crate::overlay::overlay_dash(run_index),
                    name: format!("{} · {}", o.label(), channel.name),
                    offset_ml: o.x_offset_ml,
                })
        })
        .collect()
}

/// Grow a group's y-extent to enclose its overlay traces.
///
/// Same no-feedback rule as [`data_y_range`]: derived from the data, never from
/// the plot's current bounds. A taller comparison trace must widen the axis, or
/// it would leave the top of the plot and invite reading the crop as a plateau.
fn extend_y_range(range: (f64, f64), traces: &[OverlayTrace<'_>]) -> (f64, f64) {
    let (mut lo, mut hi) = range;
    for t in traces {
        if let Some((clo, chi)) = t.channel.display_value_range() {
            lo = lo.min(clo as f64);
            hi = hi.max(chi as f64);
        }
    }
    (lo, hi)
}

/// x coordinate for an overlay sample.
///
/// The offset applies on the volume axis only: it corrects a system-volume
/// difference in mL, which has no constant time equivalent under gradient
/// flow, so on the time axis the sample plots at its own recorded time and
/// [`show`] posts a standing note instead.
fn overlay_sample_x(axis: XAxis, s: &Sample, offset_ml: f32) -> f64 {
    match axis {
        XAxis::Volume => (s.volume_ml + offset_ml) as f64,
        XAxis::Time => s.time_s as f64 / 60.0,
    }
}

/// Draw one comparison trace. Kept apart from [`draw_channel`] because none of
/// the primary's per-channel view state (selection, colour overrides, custom
/// y-ranges) applies here, and pretending it might would invite id collisions —
/// `MWave2` names a different channel in every run.
fn draw_overlay_trace(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    trace: &OverlayTrace<'_>,
    axis: XAxis,
    normalized: bool,
    surface: Rgb,
) {
    let own_range = trace
        .channel
        .display_value_range()
        .and_then(|(lo, hi)| usable_range(lo as f64, hi as f64));
    let points: Vec<[f64; 2]> = trace
        .channel
        .samples
        .iter()
        .filter(|s| s.is_finite())
        .map(|s| {
            let y = (s.value * trace.channel.display_scale) as f64;
            let y = if normalized {
                // Remapped against the overlay channel's own range — not through
                // `YMap`, whose `ChannelId` keys collide across runs.
                remap(y, own_range.unwrap_or((0.0, 0.0)), (NORM_LO, NORM_HI))
            } else {
                y
            };
            [overlay_sample_x(axis, s, trace.offset_ml), y]
        })
        .collect();
    if points.len() < 2 {
        return;
    }

    let rgb = chart::legend_color_or_series(
        trace.channel.color.map(to_rgb),
        surface,
        trace.channel_index,
    );
    plot_ui.line(
        Line::new(trace.name.clone(), PlotPoints::from(points))
            .stroke(egui::Stroke::new(stroke::TRACE, c(rgb)))
            .style(adapt::line_style(trace.dash)),
    );
}

/// The unitless axis a per-trace-scaled group is drawn on.
///
/// 0..1 rather than 0..100: the tick numbers are all the axis has left, and a
/// fraction reads as "relative" more plainly than a percentage, which invites the
/// reader to look for a total.
const NORM_LO: f64 = 0.0;
const NORM_HI: f64 = 1.0;

/// Linear remap of one y value from a source range onto a target range.
///
/// Deliberately **unclamped**. A value above the user's maximum has to leave the
/// top of the plot: clipping it to the edge would draw a flat top and imply a
/// plateau the instrument never measured. A trace that runs off the plot is
/// obviously cropped; a trace that flattens against the frame is a lie.
fn remap(value: f64, from: (f64, f64), to: (f64, f64)) -> f64 {
    let mid = 0.5 * (to.0 + to.1);
    let span = from.1 - from.0;
    // A flat trace has no dynamic range to spread over the axis. Mid-height is
    // the neutral answer; dividing by the zero span gives NaN, and pinning it to
    // the floor would suggest the signal sat at its minimum.
    if !value.is_finite() || !span.is_finite() || span == 0.0 {
        return mid;
    }
    let mapped = to.0 + (value - from.0) / span * (to.1 - to.0);
    // A span too small to divide by (denormal, or a hand-edited sidecar) can
    // overflow to an infinity that egui_plot would then try to lay out.
    if mapped.is_finite() {
        mapped
    } else {
        mid
    }
}

/// A range usable as a remap source: finite, and strictly increasing.
fn usable_range(lo: f64, hi: f64) -> Option<(f64, f64)> {
    (lo.is_finite() && hi.is_finite() && hi > lo).then_some((lo, hi))
}

/// Where one channel's values are remapped *from*, in display units.
///
/// Falls back to the channel's own data range whenever the custom range is
/// unusable, so a half-typed minimum or a hand-edited sidecar degrades to the
/// honest default rather than a division by zero. The legend reports the
/// fallback so the user is not left wondering why their number had no effect.
fn source_range(channel: &Channel, view: &View) -> Option<(f64, f64)> {
    let data = channel
        .display_value_range()
        .and_then(|(lo, hi)| usable_range(lo as f64, hi as f64));
    match view.y_scale_mode {
        YScaleMode::AutoAll | YScaleMode::AutoEach => data,
        YScaleMode::Custom => view
            .channel_y_range(&channel.id)
            .and_then(|(lo, hi)| usable_range(lo as f64, hi as f64))
            .or(data),
    }
}

/// One channel's remap source for this frame. The inner `None` marks a channel
/// with no usable range of its own — a flat trace, or one with no finite samples.
type ChannelSource = (ChannelId, Option<(f64, f64)>);

/// The per-channel remap sources for one axis group, or `None` on the shared
/// scale where there is nothing to remap.
///
/// Resolved once per frame and handed to a single [`YMap`], so the traces, their
/// peak shading, their baselines and the full-height overlays cannot disagree
/// about where the top of the plot is. Shading that stayed in mAU while its trace
/// was normalized would simply detach from the curve.
fn y_sources(channels: &[(usize, &Channel)], view: &View) -> Option<Vec<ChannelSource>> {
    view.y_scale_mode.is_per_trace().then(|| {
        channels
            .iter()
            .map(|(_, c)| (c.id.clone(), source_range(c, view)))
            .collect()
    })
}

/// Everything a drawing function needs to turn stored data into plot coordinates.
///
/// The two axes are bundled rather than threaded separately because every overlay
/// needs both, and because they are wanted at different times: the x half is the
/// unit toggle, the y half is the per-trace y-scaling. Having the pair keeps the
/// six drawing functions' signatures stable across both features.
#[derive(Clone, Copy)]
struct PlotTransform<'a> {
    x: XMap<'a>,
    y: YMap<'a>,
}

impl<'a> PlotTransform<'a> {
    fn new(run: &'a Run, view: &View, y: YMap<'a>) -> Self {
        Self {
            x: XMap::new(run, view),
            y,
        }
    }
}

/// The per-group y transform: display value → plot y, per channel.
///
/// On the shared scale ([`YScaleMode::AutoAll`]) it holds no sources and
/// [`YMap::apply`] hands its argument straight back — deliberately, with no
/// arithmetic at all, so the default rendering is bit-identical to the code
/// before per-trace scaling existed. In the per-trace modes it borrows the
/// sources resolved by [`y_sources`] for this frame and remaps each channel onto
/// the unitless `NORM_LO..NORM_HI` axis.
///
/// Borrowed rather than owned so the whole [`PlotTransform`] stays `Copy` and can
/// keep being passed by value to the six drawing functions.
#[derive(Clone, Copy, Debug, PartialEq)]
struct YMap<'a> {
    sources: Option<&'a [ChannelSource]>,
}

impl<'a> YMap<'a> {
    /// `None` is the shared scale — the identity. See [`y_sources`], which
    /// produces `None` for exactly [`YScaleMode::AutoAll`].
    fn new(sources: Option<&'a [ChannelSource]>) -> Self {
        Self { sources }
    }

    /// Whether the axis has been stripped of its unit.
    #[inline]
    fn is_normalized(self) -> bool {
        self.sources.is_some()
    }

    /// Display value → plot y for a given channel.
    ///
    /// The shared-scale path returns `y` itself, untouched: no multiply, no add,
    /// so no rounding is introduced on the overwhelmingly common path.
    #[inline]
    fn apply(self, id: &ChannelId, y: f64) -> f64 {
        let Some(sources) = self.sources else {
            return y;
        };
        match sources
            .iter()
            .find(|(cid, _)| cid == id)
            .and_then(|(_, r)| *r)
        {
            Some(from) => remap(y, from, (NORM_LO, NORM_HI)),
            // No usable source (a flat trace, one with no finite samples, or a
            // channel hidden this frame that a peak still references): hand a
            // zero-width range to `remap` and inherit its degenerate rule.
            None => remap(y, (0.0, 0.0), (NORM_LO, NORM_HI)),
        }
    }

    /// The group's y-extent, given the extent of its data.
    ///
    /// Separate from [`YMap::apply`] because a rescaling map does not
    /// necessarily produce its axis extent by mapping the data extent — a
    /// normalising one draws on a fixed unitless span regardless of the data.
    /// Overlays are sized from the value this returns, so `data_y_range`'s
    /// no-feedback guarantee still holds either way; in the per-trace modes the
    /// extent is a compile-time constant, so nothing about the current view can
    /// enter it at all.
    ///
    /// A deliberately clipping custom range does push its trace past the extent,
    /// and the auto-bounds grow to fit — once. The extent does not follow, so the
    /// overlays stay strictly inside the bounds they helped produce and the next
    /// frame computes the same numbers. That is the property that matters: not
    /// that nothing exceeds the extent, but that the extent never chases what
    /// does.
    #[inline]
    fn extent(self, data: (f64, f64)) -> (f64, f64) {
        match self.sources {
            None => data,
            Some(_) => (NORM_LO, NORM_HI),
        }
    }
}

/// Drag increment for a custom min/max field, sized to the range being edited.
///
/// A fixed step is wrong by orders of magnitude across the units this app shows
/// at once (mAU in the thousands, pH in single digits), so it is derived from the
/// span and floored so the field never becomes impossible to nudge.
fn drag_speed(lo: f64, hi: f64) -> f64 {
    let span = (hi - lo).abs();
    if !span.is_finite() || span <= 0.0 {
        return 0.01;
    }
    (span / 200.0).max(1e-4)
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

#[allow(clippy::too_many_arguments)]
fn plot_group(
    ui: &mut Ui,
    run: &Run,
    overlays: &[Overlay],
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
    let otraces = overlay_traces(overlays, group);
    if channels.is_empty() && otraces.is_empty() {
        return (None, None, egui::Rect::NOTHING);
    }

    // A group can exist purely for a comparison run, so the unit falls back to
    // the overlay channel's when the primary has nothing in this group.
    let unit = channels
        .first()
        .map(|(_, c)| c.display_unit.clone())
        .or_else(|| otraces.first().map(|o| o.channel.display_unit.clone()))
        .unwrap_or_default();

    // While integrating, dragging draws a selection instead of panning.
    let integrating = view.integrate_mode;

    // Resolved before the plot so every drawing helper below reads one mapping.
    let sources = y_sources(&channels, view);
    let tf = PlotTransform::new(run, view, YMap::new(sources.as_deref()));
    let normalized = tf.y.is_normalized();

    // Once each trace has its own source range the axis is no longer in `unit`,
    // so it must not keep saying so — that is the whole correctness argument for
    // this feature.
    let axis_label = if normalized {
        format!("{} (relative)", group.label())
    } else {
        format!("{} ({unit})", group.label())
    };

    // Detached before the plot is built so the hover closure borrows nothing the
    // `show` body below needs mutably. On a normalized axis the cursor is over a
    // shared unitless height that several traces reach at several different
    // numbers, so the readout names no instrument unit either.
    let readout_peaks = hover_peaks(run, &view.peaks, group);
    let readout_unit = if normalized {
        "rel.".to_string()
    } else {
        unit.clone()
    };

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
            .label(axis_label)
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
            // so none of them re-applies the y transform to it. Comparison traces
            // join the extent on the shared scale; on the normalized axis the
            // extent is the fixed unitless span either way.
            let (y_lo, y_hi) =
                tf.y.extent(extend_y_range(data_y_range(&channels), &otraces));

            // 1. Fraction bands sit *under* the traces so the signal stays on top.
            if is_hero {
                draw_fraction_zones(plot_ui, run, view, t, tf, y_lo, y_hi);
                draw_highlighted_span(plot_ui, view, t, tf, y_lo, y_hi);
                draw_excluded_regions(plot_ui, view, t, tf, y_lo, y_hi);
            }

            // 2. Integrated peak regions, translucent (rule #2).
            draw_peak_regions(plot_ui, view, group, run, tf, y_lo);

            // 3. Comparison traces first, so the primary run stays on top —
            //    its raw trace is the one every annotation refers to.
            for trace in &otraces {
                draw_overlay_trace(plot_ui, trace, axis, normalized, t.panel_bg);
            }

            // 4. The primary's traces themselves.
            for (i, channel) in &channels {
                draw_channel(plot_ui, channel, *i, view, t, tf);
            }

            // 5. The pending drag selection, so the user sees the window forming.
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
                tf.y.apply(&channel.id, (s.value * channel.display_scale) as f64),
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
///
/// The shading follows the *drawn* trace through `tf.y`, not the stored values.
/// The peak's area and height are untouched by any of this: they were computed
/// in `elusive-core` from the samples and are never re-derived from the picture.
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
                    tf.y.apply(&channel.id, (s.value * channel.display_scale) as f64),
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

/// The two endpoints of the peak's baseline, in plot coordinates.
///
/// `x_start`/`x_end` are the window's edges already mapped onto the current axis;
/// the baseline *values* are still worked out in volume, because that is the
/// geometry the integration used and it must not change with the view.
///
/// Every y goes through `tf.y` for the same reason the trace does: an unmapped
/// baseline would sit somewhere else entirely and the shaded region would stop
/// touching the curve it belongs to. The valley interpolation is unaffected by
/// the ordering — the remap is affine, so interpolating before or after it gives
/// the same line.
fn baseline_points(
    peak: &elusive_core::model::PeakResult,
    channel: &Channel,
    tf: PlotTransform<'_>,
    y_lo: f64,
    x_start: f64,
    x_end: f64,
) -> Vec<[f64; 2]> {
    use elusive_core::model::BaselineMode;
    let display_scale = channel.display_scale as f64;
    let at = |v: f32| {
        channel
            .value_at_volume(v)
            .map(|y| tf.y.apply(&channel.id, y as f64 * display_scale))
    };
    // Where a displayed zero lands on this axis. On the shared scale that is
    // plain 0.0, which is what this code assumed outright before per-trace
    // scaling existed — but `y_lo` is in *plot* coordinates, so on a normalized
    // axis comparing it against an unmapped 0.0 would mix two units.
    let zero = tf.y.apply(&channel.id, 0.0);

    let (y0, y1) = match peak.baseline {
        BaselineMode::DropToZero => (zero.max(y_lo), zero.max(y_lo)),
        BaselineMode::LinearEndpoints => (
            at(peak.v_start_ml).unwrap_or(zero),
            at(peak.v_end_ml).unwrap_or(zero),
        ),
        BaselineMode::ValleyToValley { left_ml, right_ml } => {
            // Extend the valley line out to the peak's own window.
            let (ya, yb) = (at(left_ml).unwrap_or(zero), at(right_ml).unwrap_or(zero));
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

    vec![[x_start, y0], [x_end, y1]]
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

/// The y-scale mode selector.
///
/// It sits with the legend rather than in the toolbar because the per-channel
/// min/max fields it governs are here; splitting a control from the fields it
/// enables makes the relationship guesswork.
fn y_scale_controls(ui: &mut Ui, view: &mut View, t: Theme) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new("Y scale")
                .font(adapt::font_micro())
                .color(c(t.text_secondary)),
        );
        for mode in YScaleMode::ALL {
            let selected = view.y_scale_mode == mode;
            if ui.selectable_label(selected, mode.label()).clicked() {
                view.set_y_scale_mode(mode);
            }
        }
    });
}

/// One channel's y range: read-only in `AutoEach`, editable in `Custom`.
///
/// Values are in display units — the same numbers the axis and the hover readout
/// use — so a user typing "500" for a UV channel means 500 mAU, not 500 AU.
fn channel_range_row(ui: &mut Ui, channel: &Channel, view: &mut View, t: Theme) {
    let data = channel.display_value_range();
    let effective = source_range(channel, view);
    let mode = view.y_scale_mode;

    ui.horizontal_wrapped(|ui| {
        // Indent past the checkbox and swatch so the numbers line up under the name.
        ui.add_space(spacing::XXL);

        match mode {
            // Never reached: the caller only draws this row in a per-trace mode.
            YScaleMode::AutoAll => {}
            YScaleMode::AutoEach => {
                let (lo, hi) = effective.unwrap_or((0.0, 0.0));
                ui.label(
                    egui::RichText::new("min")
                        .font(adapt::font_micro())
                        .color(c(t.text_secondary)),
                );
                ui.label(
                    egui::RichText::new(adapt::num(lo, 3))
                        .font(adapt::font_code())
                        .color(c(t.text_primary)),
                );
                ui.label(
                    egui::RichText::new("max")
                        .font(adapt::font_micro())
                        .color(c(t.text_secondary)),
                );
                ui.label(
                    egui::RichText::new(adapt::num(hi, 3))
                        .font(adapt::font_code())
                        .color(c(t.text_primary)),
                );
                ui.label(
                    egui::RichText::new(&channel.display_unit)
                        .font(adapt::font_micro())
                        .color(c(t.text_secondary)),
                );
            }
            YScaleMode::Custom => {
                let fallback = data.unwrap_or((0.0, 1.0));
                let (mut lo, mut hi) = view.channel_y_range(&channel.id).unwrap_or(fallback);
                let speed = drag_speed(lo as f64, hi as f64) as f32;

                ui.label(
                    egui::RichText::new("min")
                        .font(adapt::font_micro())
                        .color(c(t.text_secondary)),
                );
                let mut changed = ui
                    .add(egui::DragValue::new(&mut lo).speed(speed).max_decimals(4))
                    .changed();
                ui.label(
                    egui::RichText::new("max")
                        .font(adapt::font_micro())
                        .color(c(t.text_secondary)),
                );
                changed |= ui
                    .add(egui::DragValue::new(&mut hi).speed(speed).max_decimals(4))
                    .changed();
                ui.label(
                    egui::RichText::new(&channel.display_unit)
                        .font(adapt::font_micro())
                        .color(c(t.text_secondary)),
                );
                if changed {
                    view.set_channel_y_range(&channel.id, lo, hi);
                }
                if ui.small_button("Reset").clicked() {
                    view.clear_channel_y_range(&channel.id);
                }
            }
        }
    });

    // §6: a validation message states the problem *and* the corrective action.
    if mode == YScaleMode::Custom {
        if let Some((lo, hi)) = view.channel_y_range(&channel.id) {
            if usable_range(lo as f64, hi as f64).is_none() {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(spacing::XXL);
                    ui.label(
                        egui::RichText::new(
                            "Min must be below max — drawing this trace at its data range instead.",
                        )
                        .font(adapt::font_micro())
                        .color(c(color::WARNING_600)),
                    );
                });
            }
        }
    }
}

/// Legend with per-channel visibility, colour swatch, and unit — the control
/// surface for Phase 2's show/hide requirement, and for the y-scale mode.
/// Comparison runs get one group each below the primary's channels.
pub fn legend(ui: &mut Ui, run: &Run, overlays: &mut Vec<Overlay>, view: &mut View, t: Theme) {
    y_scale_controls(ui, view, t);
    ui.add_space(spacing::XS);

    egui::ScrollArea::vertical()
        .id_salt("channel-legend")
        .show(ui, |ui| {
            for (i, channel) in run.channels.iter().enumerate() {
                if channel.is_empty() {
                    continue;
                }
                let mut visible = view.is_channel_visible(&channel.id);
                let scaled = visible && view.y_scale_mode.is_per_trace();
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

                    // A word, not a tint: whoever reads this has to be told the
                    // trace's height is no longer on the group's scale (rule #3).
                    if scaled {
                        ui.label(
                            egui::RichText::new("scaled")
                                .font(adapt::font_micro())
                                .color(c(color::WARNING_600)),
                        )
                        .on_hover_text(
                            "This trace is drawn on its own range, so its height cannot be \
                             compared with the others",
                        );
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

                if scaled {
                    channel_range_row(ui, channel, view, t);
                }
            }

            overlay_legend_groups(ui, overlays, view, t);
        });
}

/// One legend group per comparison run: a header row (master toggle, run name,
/// x-offset, Remove) over the run's channels.
///
/// Swatches here are display-only — the primary's colour overrides are keyed by
/// [`ChannelId`], which is a per-run string (`MWave2` exists in every run), so
/// extending overrides to overlays would collide. Overlay colours follow the
/// same automatic resolution the primary gets, and the dash carries run
/// identity (spec §3).
fn overlay_legend_groups(ui: &mut Ui, overlays: &mut Vec<Overlay>, view: &mut View, t: Theme) {
    let mut remove: Option<usize> = None;

    for (idx, overlay) in overlays.iter_mut().enumerate() {
        ui.add_space(spacing::SM);
        ui.separator();

        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut overlay.visible, "")
                .on_hover_text("Show or hide every trace of this comparison run")
                .changed()
            {
                view.dirty = true;
            }
            ui.label(
                egui::RichText::new(overlay.label())
                    .font(adapt::font_h3())
                    .color(c(t.text_primary)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("Remove")
                    .on_hover_text("Stop comparing this run. Its file and sidecar are untouched.")
                    .clicked()
                {
                    remove = Some(idx);
                }
                if ui
                    .add(
                        egui::DragValue::new(&mut overlay.x_offset_ml)
                            .speed(0.05)
                            .max_decimals(3)
                            .suffix(" mL"),
                    )
                    .on_hover_text(
                        "Display-only x shift for this run, to correct a system-volume \
                         difference. Applies on the volume axis; never enters a result.",
                    )
                    .changed()
                {
                    view.dirty = true;
                }
                ui.label(
                    egui::RichText::new("offset")
                        .font(adapt::font_micro())
                        .color(c(t.text_secondary)),
                );
            });
        });

        let dash = crate::overlay::overlay_dash(idx);
        for (i, channel) in overlay.run.channels.iter().enumerate() {
            if channel.is_empty() {
                continue;
            }
            let mut visible = !overlay.hidden_channels.contains(&channel.id);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut visible, "").changed() {
                    if visible {
                        overlay.hidden_channels.remove(&channel.id);
                    } else {
                        overlay.hidden_channels.insert(channel.id.clone());
                    }
                    view.dirty = true;
                }

                let rgb = chart::legend_color_or_series(channel.color.map(to_rgb), t.panel_bg, i);
                let (swatch, _) = ui.allocate_exact_size(
                    egui::vec2(SWATCH_WIDTH, SWATCH_HEIGHT),
                    egui::Sense::hover(),
                );
                paint_swatch(ui.painter(), swatch, rgb, dash);

                ui.label(egui::RichText::new(&channel.name).color(c(t.text_primary)));
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

    if let Some(idx) = remove {
        overlays.remove(idx);
        view.dirty = true;
    }
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

    // --- comparison overlays ------------------------------------------------

    use crate::overlay::Overlay;

    fn run_with(name: &str, channels: Vec<Channel>) -> Run {
        Run {
            meta: RunMeta {
                run_name: name.to_string(),
                ..RunMeta::default()
            },
            source_format: SourceFormat::NgcAnalysis,
            source_path: std::path::PathBuf::from(format!("{name}.ngcAnalysis")),
            channels,
            fractions: Vec::new(),
            events: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn sampled(id: &str, kind: ChannelKind, peak_value: f32) -> Channel {
        let mut c = Channel::new(id, id, kind);
        c.samples = vec![
            Sample::new(0.0, 0.0, 0.0),
            Sample::new(60.0, 1.0, peak_value),
            Sample::new(120.0, 2.0, 0.0),
        ];
        c
    }

    fn overlay_around(run: Run) -> Overlay {
        Overlay {
            source_path: run.source_path.clone(),
            run,
            peaks: Vec::new(),
            visible: true,
            hidden_channels: Default::default(),
            x_offset_ml: 0.0,
        }
    }

    #[test]
    fn overlay_channels_extend_the_group_y_range() {
        let primary = [sampled("MWave2", ChannelKind::Uv, 10.0)];
        let channels: Vec<(usize, &Channel)> = primary.iter().enumerate().collect();
        let overlay = overlay_around(run_with(
            "o",
            vec![sampled("MWave2", ChannelKind::Uv, 25.0)],
        ));
        let traces = overlay_traces(std::slice::from_ref(&overlay), AxisGroup::Uv);

        let range = extend_y_range(data_y_range(&channels), &traces);
        assert!((range.1 - 25.0).abs() < 1e-6, "hi = {}", range.1);
        assert!((range.0 - 0.0).abs() < 1e-6, "lo = {}", range.0);
    }

    #[test]
    fn overlay_groups_appear_even_without_primary_channels() {
        let primary = run_with("p", vec![sampled("MWave2", ChannelKind::Uv, 1.0)]);
        let overlay = overlay_around(run_with(
            "o",
            vec![sampled("MD_Conductivity", ChannelKind::Conductivity, 1.0)],
        ));
        let groups = visible_groups(&primary, std::slice::from_ref(&overlay), &View::default());
        assert!(
            groups.contains(&AxisGroup::Conductivity),
            "groups = {groups:?}"
        );
    }

    #[test]
    fn offset_moves_volume_x_only() {
        let s = Sample::new(90.0, 1.5, 0.2);
        assert!((overlay_sample_x(XAxis::Volume, &s, 0.25) - 1.75).abs() < 1e-6);
        // A mL offset has no constant time equivalent: ignored on the time axis.
        assert!((overlay_sample_x(XAxis::Time, &s, 0.25) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn hidden_channels_and_invisible_overlays_contribute_no_traces() {
        let mut hidden_run =
            overlay_around(run_with("a", vec![sampled("MWave2", ChannelKind::Uv, 1.0)]));
        hidden_run.visible = false;
        let mut hidden_channel =
            overlay_around(run_with("b", vec![sampled("MWave2", ChannelKind::Uv, 1.0)]));
        hidden_channel.hidden_channels.insert("MWave2".into());
        let shown = overlay_around(run_with("c", vec![sampled("MWave2", ChannelKind::Uv, 1.0)]));

        let overlays = vec![hidden_run, hidden_channel, shown];
        let traces = overlay_traces(&overlays, AxisGroup::Uv);
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].name, "c · MWave2");
        // Dash identity keys off the overlay's position in the list, not off how
        // many happen to be visible, so hiding one never restyles another.
        assert_eq!(traces[0].dash, crate::theme::chart::Dash::Dashed);
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

    /// A channel whose displayed values run 0..`peak_display`, in `n` samples.
    fn ramp(id: &str, peak_display: f32, display_scale: f32, n: usize) -> Channel {
        let mut ch = Channel::new(id, id, ChannelKind::Uv);
        ch.display_scale = display_scale;
        ch.display_unit = "mAU".into();
        ch.samples = (0..n)
            .map(|i| {
                let t = i as f32 / (n - 1) as f32;
                Sample::new(t * 60.0, t * 2.0, t * peak_display / display_scale)
            })
            .collect();
        ch
    }

    fn view_in(mode: YScaleMode) -> View {
        let mut view = View::default();
        view.set_y_scale_mode(mode);
        view
    }

    /// The y half of the transform for one group, with the sources it borrows.
    ///
    /// The `Vec` has to outlive the [`YMap`] that points into it, so it is handed
    /// back alongside rather than dropped at the end of a helper.
    fn scale_for<'a>(
        channels: &[(usize, &Channel)],
        view: &View,
        keep: &'a mut Option<Vec<ChannelSource>>,
    ) -> YMap<'a> {
        *keep = y_sources(channels, view);
        YMap::new(keep.as_deref())
    }

    /// A `PlotTransform` around a bare y map, for the drawing helpers that take
    /// the whole pair. The x half is unused by everything tested here.
    fn tf_with(y: YMap<'_>) -> PlotTransform<'_> {
        PlotTransform {
            x: XMap {
                axis: XAxis::Volume,
                reference: None,
            },
            y,
        }
    }

    #[test]
    fn the_shared_scale_draws_every_value_exactly_as_it_stands() {
        // Exact equality, not an epsilon: `AutoAll` must be bit-identical to the
        // pre-feature rendering path, so the mapping is required to be the
        // literal identity — no arithmetic at all, and no rounding introduced.
        let uv = ramp("MWave2", 500.0, 1000.0, 5);
        let channels = vec![(0usize, &uv)];
        let mut keep = None;
        let y = scale_for(&channels, &view_in(YScaleMode::AutoAll), &mut keep);

        assert!(!y.is_normalized());
        assert_eq!(y, YMap::new(None), "the shared scale resolves no sources");
        for v in [
            0.0f64,
            -0.0,
            1.0,
            -412.6,
            123.456,
            f64::MIN_POSITIVE,
            f64::MAX,
            0.1 + 0.2,
            1e9,
        ] {
            assert!(
                y.apply(&uv.id, v) == v,
                "v = {v} came back as {}",
                y.apply(&uv.id, v)
            );
        }
        assert!(y.apply(&uv.id, f64::NAN).is_nan());

        // The extent hook is likewise exactly pass-through, so overlays are still
        // sized from `data_y_range` and nothing re-enters the plot's auto-bounds.
        let data = data_y_range(&channels);
        assert!(y.extent(data) == data);
        assert!(y.extent((-412.6, 1e300)) == (-412.6, 1e300));
    }

    #[test]
    fn remap_is_the_identity_when_the_ranges_match() {
        for v in [-3.0, 0.0, 0.5, 17.25, 1e6] {
            assert_eq!(remap(v, (0.0, 100.0), (0.0, 100.0)), v);
        }
    }

    #[test]
    fn remap_puts_the_midpoint_at_the_midpoint() {
        assert!((remap(1020.0, (20.0, 2020.0), (0.0, 1.0)) - 0.5).abs() < 1e-12);
        // Also with a target that does not start at zero, and an offset source.
        assert!((remap(6.0, (4.0, 8.0), (10.0, 30.0)) - 20.0).abs() < 1e-12);
    }

    #[test]
    fn remap_does_not_clamp_values_outside_the_source_range() {
        // A clipped trace has to visibly leave the plot. Flattening it against
        // the frame would draw a plateau the instrument never measured.
        assert!(remap(150.0, (0.0, 100.0), (0.0, 1.0)) > 1.0);
        assert!(remap(-50.0, (0.0, 100.0), (0.0, 1.0)) < 0.0);
    }

    #[test]
    fn a_degenerate_source_range_maps_to_mid_height_not_nan() {
        // A flat trace: every sample is the same number, so there is no dynamic
        // range to spread over the axis.
        let y = remap(5.0, (5.0, 5.0), (0.0, 1.0));
        assert!(y.is_finite(), "y = {y}");
        assert!((y - 0.5).abs() < 1e-12, "y = {y}");
    }

    #[test]
    fn remap_survives_inputs_no_division_can_cope_with() {
        // A span too small to divide by, and a non-finite value: both would
        // otherwise reach egui_plot as NaN or an infinity to lay out.
        assert!(remap(1.0, (0.0, f64::MIN_POSITIVE / 4.0), (0.0, 1.0)).is_finite());
        assert!(remap(f64::NAN, (0.0, 1.0), (0.0, 1.0)).is_finite());
        assert!(remap(1.0, (0.0, f64::NAN), (0.0, 1.0)).is_finite());
        assert!(remap(f64::INFINITY, (0.0, 1.0), (0.0, 1.0)).is_finite());
    }

    #[test]
    fn overlay_extent_is_the_relative_axis_when_traces_are_scaled() {
        // The counterpart to `overlay_extent_comes_from_the_data_not_the_view`:
        // in a per-trace mode the extent is a compile-time constant, which is
        // stability by construction — nothing about the current bounds, the
        // current data or the current window can enter it.
        let big = ramp("MWave2", 2000.0, 1000.0, 9);
        let small = ramp("MWave1", 40.0, 1000.0, 9);
        let channels = vec![(0usize, &big), (1usize, &small)];

        for mode in [YScaleMode::AutoEach, YScaleMode::Custom] {
            let mut keep = None;
            let y = scale_for(&channels, &view_in(mode), &mut keep);
            assert!(y.is_normalized());
            assert_eq!(y.extent(data_y_range(&channels)), (NORM_LO, NORM_HI));
            // Even a wildly wrong "data" extent cannot move it.
            assert_eq!(y.extent((-1e300, 1e300)), (NORM_LO, NORM_HI), "{mode:?}");
        }
    }

    #[test]
    fn the_extent_never_chases_a_trace_that_leaves_the_plot() {
        // A clipping custom range *does* push its trace past the extent, and the
        // auto-bounds grow once to fit. The bug this guards against is the
        // extent then following: the overlays would re-enter the next frame's
        // bounds and inflate them again, every repaint, forever.
        let uv = ramp("MWave2", 500.0, 1000.0, 9);
        let channels = vec![(0usize, &uv)];
        let mut view = view_in(YScaleMode::Custom);
        view.set_channel_y_range(&uv.id, 0.0, 100.0);

        let mut keep = None;
        let first = scale_for(&channels, &view, &mut keep);
        let first_extent = first.extent(data_y_range(&channels));
        assert!(first.apply(&uv.id, 500.0) > first_extent.1);

        // Re-resolving with the same inputs — i.e. the next frame — must not
        // have moved.
        let mut keep2 = None;
        let second = scale_for(&channels, &view, &mut keep2);
        assert_eq!(first_extent, second.extent(data_y_range(&channels)));
        assert_eq!(first_extent, (NORM_LO, NORM_HI));
    }

    #[test]
    fn overlay_extent_encloses_the_traces_on_the_ranges_it_derives_itself() {
        // Whenever the scale picks the range (both auto modes, and custom before
        // the user narrows anything), every point the traces contribute to
        // auto-bounds lies inside the extent the overlays are drawn from.
        let big = ramp("MWave2", 2000.0, 1000.0, 9);
        let small = ramp("MWave1", 40.0, 1000.0, 9);
        let channels = vec![(0usize, &big), (1usize, &small)];

        for mode in YScaleMode::ALL {
            let mut keep = None;
            let y = scale_for(&channels, &view_in(mode), &mut keep);
            let (lo, hi) = y.extent(data_y_range(&channels));
            for (_, channel) in &channels {
                for s in &channel.samples {
                    let mapped = y.apply(&channel.id, (s.value * channel.display_scale) as f64);
                    assert!(
                        mapped >= lo - 1e-9 && mapped <= hi + 1e-9,
                        "mode={mode:?} y={mapped}"
                    );
                }
            }
        }
    }

    #[test]
    fn auto_each_makes_a_small_trace_fill_the_plot() {
        // The whole point of the feature: 40 mAU beside 2000 mAU is a flat line
        // on a shared axis, and a full-height curve on its own.
        let big = ramp("MWave2", 2000.0, 1000.0, 9);
        let small = ramp("MWave1", 40.0, 1000.0, 9);
        let channels = vec![(0usize, &big), (1usize, &small)];

        let mut keep = None;
        let shared = scale_for(&channels, &view_in(YScaleMode::AutoAll), &mut keep);
        let shared_extent = shared.extent(data_y_range(&channels));
        let small_top_shared = shared.apply(&small.id, 40.0);
        assert!(
            small_top_shared / shared_extent.1 < 0.05,
            "the premise of the feature: {small_top_shared}"
        );

        let mut keep2 = None;
        let each = scale_for(&channels, &view_in(YScaleMode::AutoEach), &mut keep2);
        assert!((each.apply(&small.id, 40.0) - NORM_HI).abs() < 1e-9);
        assert!((each.apply(&big.id, 2000.0) - NORM_HI).abs() < 1e-9);
        assert!((each.apply(&small.id, 0.0) - NORM_LO).abs() < 1e-9);
    }

    #[test]
    fn a_custom_range_overrides_the_data_range() {
        let uv = ramp("MWave2", 500.0, 1000.0, 5);
        let channels = vec![(0usize, &uv)];
        let mut view = view_in(YScaleMode::Custom);
        view.set_channel_y_range(&uv.id, 0.0, 1000.0);

        let mut keep = None;
        let y = scale_for(&channels, &view, &mut keep);
        // Half the user's window, so half the plot height — not full height.
        assert!((y.apply(&uv.id, 500.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_custom_range_below_the_data_lets_the_trace_leave_the_plot() {
        let uv = ramp("MWave2", 500.0, 1000.0, 5);
        let channels = vec![(0usize, &uv)];
        let mut view = view_in(YScaleMode::Custom);
        view.set_channel_y_range(&uv.id, 0.0, 100.0);

        let mut keep = None;
        let y = scale_for(&channels, &view, &mut keep);
        assert!(
            y.apply(&uv.id, 500.0) > NORM_HI,
            "a clipped peak must run off the top, not flatten against it"
        );
    }

    #[test]
    fn an_unusable_custom_range_falls_back_to_the_data_range() {
        let uv = ramp("MWave2", 500.0, 1000.0, 5);
        let channels = vec![(0usize, &uv)];

        for (lo, hi) in [(500.0f32, 0.0f32), (7.0, 7.0)] {
            let mut view = view_in(YScaleMode::Custom);
            view.set_channel_y_range(&uv.id, lo, hi);
            let mut keep = None;
            let y = scale_for(&channels, &view, &mut keep);
            // Same as `AutoEach` would give: the data range, full height.
            assert!(
                (y.apply(&uv.id, 500.0) - NORM_HI).abs() < 1e-9,
                "({lo}, {hi}) should have fallen back to the data range"
            );
        }
    }

    #[test]
    fn a_channel_the_scale_has_never_heard_of_still_draws_finitely() {
        // Defensive: a peak can reference a channel that is hidden this frame.
        let uv = ramp("MWave2", 500.0, 1000.0, 5);
        let channels = vec![(0usize, &uv)];
        let mut keep = None;
        let y = scale_for(&channels, &view_in(YScaleMode::AutoEach), &mut keep);
        assert!(y.apply(&ChannelId::from("NoSuchChannel"), 42.0).is_finite());
    }

    #[test]
    fn a_flat_trace_sits_mid_plot_rather_than_producing_nan() {
        let mut flat = Channel::new("MWave3", "UV 214 nm", ChannelKind::Uv);
        flat.display_scale = 1000.0;
        flat.samples = vec![Sample::new(0.0, 0.0, 0.3), Sample::new(60.0, 2.0, 0.3)];
        let channels = vec![(0usize, &flat)];

        let mut keep = None;
        let y = scale_for(&channels, &view_in(YScaleMode::AutoEach), &mut keep);
        let mapped = y.apply(&flat.id, 300.0);
        assert!(
            mapped.is_finite() && (mapped - 0.5).abs() < 1e-12,
            "y = {mapped}"
        );
    }

    #[test]
    fn peak_shading_stays_attached_to_the_trace_it_belongs_to() {
        let uv = ramp("MWave2", 500.0, 1000.0, 9);
        let channels = vec![(0usize, &uv)];
        let peak = PeakResult {
            id: PeakId(1),
            channel_id: uv.id.clone(),
            v_start_ml: 0.5,
            v_end_ml: 1.5,
            baseline: BaselineMode::LinearEndpoints,
            area: 0.0,
            height: 0.0,
            apex_volume_ml: 1.0,
            fwhm_ml: None,
            estimated_mw_kda: None,
        };

        for mode in YScaleMode::ALL {
            let mut keep = None;
            let y = scale_for(&channels, &view_in(mode), &mut keep);
            let (lo, _) = y.extent(data_y_range(&channels));
            let pts = baseline_points(
                &peak,
                &uv,
                tf_with(y),
                lo,
                peak.v_start_ml as f64,
                peak.v_end_ml as f64,
            );
            for (i, v) in [(0usize, peak.v_start_ml), (1, peak.v_end_ml)] {
                // The baseline endpoint must land exactly where the drawn trace
                // is at that volume, or the shaded region detaches from it.
                let on_trace = y.apply(
                    &uv.id,
                    (uv.value_at_volume(v).expect("sampled volume") * uv.display_scale) as f64,
                );
                assert!(
                    (pts[i][1] - on_trace).abs() < 1e-9,
                    "mode={mode:?} baseline={} trace={on_trace}",
                    pts[i][1]
                );
            }
        }
    }

    #[test]
    fn a_drop_to_zero_baseline_follows_the_scale_too() {
        let mut uv = ramp("MWave2", 500.0, 1000.0, 9);
        // Shift the trace up so a displayed zero is genuinely below its data range.
        for s in &mut uv.samples {
            s.value += 0.1;
        }
        let channels = vec![(0usize, &uv)];
        let peak = PeakResult {
            id: PeakId(1),
            channel_id: uv.id.clone(),
            v_start_ml: 0.5,
            v_end_ml: 1.5,
            baseline: BaselineMode::DropToZero,
            area: 0.0,
            height: 0.0,
            apex_volume_ml: 1.0,
            fwhm_ml: None,
            estimated_mw_kda: None,
        };

        let mut keep = None;
        let y = scale_for(&channels, &view_in(YScaleMode::AutoEach), &mut keep);
        let (lo, _) = y.extent(data_y_range(&channels));
        let pts = baseline_points(
            &peak,
            &uv,
            tf_with(y),
            lo,
            peak.v_start_ml as f64,
            peak.v_end_ml as f64,
        );
        // Zero is below the trace's own minimum, so it clamps to the plot floor
        // exactly as the shared-scale path already did.
        assert!((pts[0][1] - NORM_LO).abs() < 1e-12, "y = {}", pts[0][1]);
        assert_eq!(pts[0][1], pts[1][1]);
    }

    #[test]
    fn drag_speed_tracks_the_magnitude_of_the_range_being_edited() {
        // A step that suits mAU in the thousands is useless for pH.
        assert!(drag_speed(0.0, 2000.0) > drag_speed(6.0, 8.0));
        // Never zero, or the field cannot be nudged at all.
        for (lo, hi) in [(0.0, 0.0), (1.0, f64::NAN), (5.0, 5.0)] {
            assert!(drag_speed(lo, hi) > 0.0);
        }
    }

    #[test]
    fn usable_range_refuses_everything_the_remap_cannot_divide_by() {
        assert_eq!(usable_range(0.0, 1.0), Some((0.0, 1.0)));
        assert_eq!(usable_range(1.0, 0.0), None);
        assert_eq!(usable_range(1.0, 1.0), None);
        assert_eq!(usable_range(f64::NAN, 1.0), None);
        assert_eq!(usable_range(0.0, f64::INFINITY), None);
    }

    #[test]
    fn a_normalized_axis_stops_claiming_the_instrument_s_unit() {
        // The honesty rule, checked on the two strings the user actually reads:
        // the axis title and the hover readout. `mAU` must not survive either.
        let uv = ramp("MWave2", 500.0, 1000.0, 5);
        let channels = vec![(0usize, &uv)];

        let mut shared_keep = None;
        let shared = scale_for(&channels, &view_in(YScaleMode::AutoAll), &mut shared_keep);
        assert!(!shared.is_normalized());

        let mut keep = None;
        let y = scale_for(&channels, &view_in(YScaleMode::AutoEach), &mut keep);
        assert!(y.is_normalized());

        // Same shape as `plot_group` builds them.
        let label = format!("{} (relative)", AxisGroup::Uv.label());
        assert!(!label.contains("mAU"), "{label}");
        assert!(label.ends_with("(relative)"), "{label}");

        let run = hover_run();
        let readout = hover_label(&run, &[], "rel.", XAxis::Volume, 4.0, 0.5, None);
        assert_eq!(readout, "4.000 mL\n0.500 rel.");
        assert!(!readout.contains("mAU"));
    }
}

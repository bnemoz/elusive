//! Mutable UI state, kept separate from the loaded [`Run`].
//!
//! Two reasons for the split. First, the borrow checker: widgets need `&Run` and
//! `&mut View` at the same time, which is only possible if they are different
//! fields. Second, honesty about ownership — the run is immutable input read from
//! a file we never write to, while everything here is the user's.
//!
//! Cross-pane hover linking lives in plain fields (`hovered_vol_range`,
//! `hovered_well`) rather than channels or callbacks, as
//! `IMPLEMENTATION_PLAN.md` Phase 4 requires: in an immediate-mode UI, shared
//! state read by every pane each frame is the simplest thing that works.

use elusive_core::calibration::{Calibration, CalibrationPoint, Extinction};
use elusive_core::integrate::PlateMetric;
use elusive_core::model::{BaselineMode, ChannelId, Color, PeakId, PeakResult, Run, Well};
use elusive_core::sidecar::{Annotation, ExcludedRegion, NamedCalibration, Sidecar, ViewState};
use std::collections::{BTreeMap, BTreeSet};

/// Something the user did in a widget that the app must act on.
#[derive(Clone, Debug, PartialEq)]
pub enum Interaction {
    /// A drag on the chromatogram finished over this volume window.
    IntegrateRange(f32, f32),
}

/// Sidebar sections, mirroring the brand mockup's navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Overview,
    Chromatograms,
    Peaks,
    Calibration,
    Results,
    Reports,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Overview,
        Section::Chromatograms,
        Section::Peaks,
        Section::Calibration,
        Section::Results,
        Section::Reports,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Overview => "Overview",
            Section::Chromatograms => "Chromatograms",
            Section::Peaks => "Peaks",
            Section::Calibration => "Calibration",
            Section::Results => "Results",
            Section::Reports => "Reports",
        }
    }

    /// A single glyph standing in for the section when the rail is collapsed.
    ///
    /// Deliberately drawn from a narrow set of characters: Inter and JetBrains
    /// Mono are not vendored (`assets/fonts/README.md`), so on most machines
    /// these render through egui's bundled fallback faces, where anything
    /// exotic comes out as tofu. `nav_icons_render_in_the_bundled_fonts` in
    /// `app.rs` checks the actual glyph coverage rather than trusting the
    /// choice. An icon never travels alone — the collapsed rail pairs it with a
    /// hover tooltip carrying [`Section::label`], because a control identified
    /// only by its appearance is the failure rule #3 exists to prevent.
    pub fn icon(self) -> &'static str {
        match self {
            Section::Overview => "☰",
            Section::Chromatograms => "🗠",
            Section::Peaks => "Λ",
            Section::Calibration => "⚖",
            Section::Results => "∑",
            Section::Reports => "🖹",
        }
    }
}

/// A card in the Overview section, in the order the user has arranged them.
///
/// The Overview is the one section whose layout the user controls, so its cards
/// need names that survive a restart. The variants are UI identity only — the
/// core knows nothing about them, and the sidecar stores [`PanelId::as_str`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PanelId {
    RunSummary,
    Warnings,
    Channels,
    Fractions,
}

impl PanelId {
    /// The declared default arrangement, and the order a reset returns to.
    pub const ALL: [PanelId; 4] = [
        PanelId::RunSummary,
        PanelId::Warnings,
        PanelId::Channels,
        PanelId::Fractions,
    ];

    /// Stable serialized form. Never rename one of these without a migration:
    /// an id this build does not recognise is dropped on load.
    pub fn as_str(self) -> &'static str {
        match self {
            PanelId::RunSummary => "run_summary",
            PanelId::Warnings => "warnings",
            PanelId::Channels => "channels",
            PanelId::Fractions => "fractions",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        PanelId::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

/// Move the panel at `from` to index `to`, shifting everything between.
///
/// Pure, because the drag-and-drop gesture that calls it cannot be exercised in
/// a headless test but this arithmetic can. Out-of-range indices are ignored
/// rather than clamped: a drop that does not name a real slot is not a reorder.
/// Returns whether the order actually changed.
pub fn reorder(order: &mut Vec<PanelId>, from: usize, to: usize) -> bool {
    if from >= order.len() || to >= order.len() || from == to {
        return false;
    }
    let panel = order.remove(from);
    order.insert(to, panel);
    true
}

/// Rebuild a panel order from the strings in a sidecar.
///
/// Tolerant in both directions on purpose. An id this build does not know is
/// dropped (a sidecar from a future version with an extra card), and any panel
/// the saved list never mentions is appended after the ones it does mention (a
/// sidecar from an older version, or one that lost an id). Either way every panel
/// this build can draw appears exactly once, so a stale file can never hide a
/// card the user would then have no way to get back.
pub fn restore_order(saved: &[String]) -> Vec<PanelId> {
    let mut order: Vec<PanelId> = Vec::with_capacity(PanelId::ALL.len());
    for id in saved {
        if let Some(panel) = PanelId::parse(id) {
            if !order.contains(&panel) {
                order.push(panel);
            }
        }
    }
    for panel in PanelId::ALL {
        if !order.contains(&panel) {
            order.push(panel);
        }
    }
    order
}

/// Which quantity the chromatogram's x-axis shows.
///
/// Display only. Everything stored — peaks, excluded regions, annotations, the
/// sidecar — stays in mL, because elution volume is the physically meaningful
/// axis for prep SEC and a saved analysis must not depend on a view preference.
/// `DESIGN_SYSTEM.md` §10.1 keeps volume as the axis and time as a secondary
/// readout; this makes the secondary readout promotable for users who think in
/// method minutes without moving the model under them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XAxis {
    Volume,
    Time,
}

impl XAxis {
    pub const ALL: [XAxis; 2] = [XAxis::Volume, XAxis::Time];

    /// Full axis title.
    pub fn label(self) -> &'static str {
        match self {
            XAxis::Volume => "Elution volume (mL)",
            XAxis::Time => "Elution time (min)",
        }
    }

    /// Unit suffix, also short enough to label the toolbar toggle.
    pub fn unit(self) -> &'static str {
        match self {
            XAxis::Volume => "mL",
            XAxis::Time => "min",
        }
    }

    /// The axis *not* selected, i.e. the one shown as a secondary hover readout.
    pub fn other(self) -> XAxis {
        match self {
            XAxis::Volume => XAxis::Time,
            XAxis::Time => XAxis::Volume,
        }
    }

    /// Stable sidecar key. Kept separate from `unit()` so renaming a label can
    /// never invalidate saved files.
    pub fn key(self) -> &'static str {
        match self {
            XAxis::Volume => "volume",
            XAxis::Time => "time",
        }
    }

    /// `None` for an unrecognised key, so a sidecar written by a future build
    /// with a third axis mode degrades to the default instead of failing.
    pub fn from_key(key: &str) -> Option<XAxis> {
        match key {
            "volume" => Some(XAxis::Volume),
            "time" => Some(XAxis::Time),
            _ => None,
        }
    }
}

/// How the chromatogram maps trace values onto an axis group's y-axis.
///
/// This is display-only state. Nothing in `elusive-core` reads it: areas, heights
/// and every exported number are computed from the stored samples, so switching
/// mode changes what the plot looks like and never what the run measured.
///
/// It lives here rather than in `elusive-core` for the same reason, which is why
/// the sidecar persists it as a string ([`YScaleMode::as_key`]) instead of the
/// core owning an enum it has no use for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum YScaleMode {
    /// One shared, data-derived range per axis group. The axis keeps the
    /// instrument's unit and heights are directly comparable.
    #[default]
    AutoAll,
    /// Every visible channel is normalized to its own data range, so a 40 mAU
    /// trace and a 2000 mAU trace are both readable at once.
    AutoEach,
    /// Every channel is normalized from a user-entered range, which defaults to
    /// its data range.
    Custom,
}

impl YScaleMode {
    pub const ALL: [YScaleMode; 3] = [
        YScaleMode::AutoAll,
        YScaleMode::AutoEach,
        YScaleMode::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            YScaleMode::AutoAll => "Shared",
            YScaleMode::AutoEach => "Auto each",
            YScaleMode::Custom => "Custom",
        }
    }

    /// Whether traces are remapped onto a unitless axis.
    ///
    /// The chromatogram keys the relative-axis label, the hover readout and the
    /// "scaled" badges off this: once it is true the y-axis no longer carries the
    /// instrument's unit and must not claim to.
    pub fn is_per_trace(self) -> bool {
        !matches!(self, YScaleMode::AutoAll)
    }

    /// Stable sidecar spelling. Kept separate from [`YScaleMode::label`] so the
    /// UI wording can change without invalidating saved analyses.
    pub fn as_key(self) -> &'static str {
        match self {
            YScaleMode::AutoAll => "auto-all",
            YScaleMode::AutoEach => "auto-each",
            YScaleMode::Custom => "custom",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_key() == key)
    }
}

/// Which baseline the next integration will use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BaselineChoice {
    DropToZero,
    LinearEndpoints,
    ValleyToValley,
}

impl BaselineChoice {
    pub const ALL: [BaselineChoice; 3] = [
        BaselineChoice::DropToZero,
        BaselineChoice::LinearEndpoints,
        BaselineChoice::ValleyToValley,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BaselineChoice::DropToZero => "Drop to zero",
            BaselineChoice::LinearEndpoints => "Linear (endpoints)",
            BaselineChoice::ValleyToValley => "Valley to valley",
        }
    }

    /// Resolve to a concrete mode for a window. Valley-to-valley defaults its
    /// anchors to the window edges, which the user can then widen.
    pub fn resolve(self, v_start: f32, v_end: f32) -> BaselineMode {
        match self {
            BaselineChoice::DropToZero => BaselineMode::DropToZero,
            BaselineChoice::LinearEndpoints => BaselineMode::LinearEndpoints,
            BaselineChoice::ValleyToValley => BaselineMode::ValleyToValley {
                left_ml: v_start,
                right_ml: v_end,
            },
        }
    }
}

/// Inputs for the A280 concentration calculation.
#[derive(Clone, Debug, PartialEq)]
pub struct ConcentrationInputs {
    pub use_molar: bool,
    /// A(1%, 1 cm)-style coefficient, in (mg/mL)⁻¹cm⁻¹.
    pub e_mg_per_ml: f64,
    pub epsilon_molar: f64,
    pub mw_da: f64,
    pub path_length_cm: f64,
}

impl Default for ConcentrationInputs {
    fn default() -> Self {
        Self {
            use_molar: false,
            // 1.0 is the honest "unknown" default: it makes the arithmetic a pass
            // through so a user who has not entered a coefficient can see that
            // the number is not yet a concentration.
            e_mg_per_ml: 1.0,
            epsilon_molar: 43_824.0,
            mw_da: 50_000.0,
            path_length_cm: 0.2,
        }
    }
}

impl ConcentrationInputs {
    pub fn extinction(&self) -> Extinction {
        if self.use_molar {
            Extinction::Molar {
                epsilon: self.epsilon_molar,
                mw_da: self.mw_da,
            }
        } else {
            Extinction::PerMgPerMl(self.e_mg_per_ml)
        }
    }
}

/// Everything the user can change about the current view and analysis.
#[derive(Clone, Debug)]
pub struct View {
    pub section: Section,
    /// Navigation rail reduced to icons only.
    pub nav_collapsed: bool,

    // --- channel display ---
    pub hidden_channels: BTreeSet<ChannelId>,
    pub selected_channel: Option<ChannelId>,
    pub hero_channel_id: Option<ChannelId>,
    pub show_fractions: bool,
    /// Trace colours the user picked from the legend, overriding every automatic
    /// choice. Stored as the core [`Color`] rather than a `theme::Rgb` so the
    /// override and the ChromLab legend colour it replaces share one
    /// representation, and the sidecar gains no second colour encoding.
    pub channel_colors: BTreeMap<ChannelId, Color>,
    /// How the chromatogram scales each axis group's y-axis. Display only — see
    /// [`YScaleMode`].
    pub y_scale_mode: YScaleMode,
    /// User-entered y range per channel, in *display* units, used by
    /// [`YScaleMode::Custom`]. Absent means "fall back to the data range".
    pub channel_y_ranges: BTreeMap<ChannelId, (f32, f32)>,

    /// Overview card order, left-to-right then top-to-bottom through the columns.
    pub overview_order: Vec<PanelId>,

    /// Display axis for the chromatogram. Never consulted by anything that
    /// computes or stores a result — see [`XAxis`].
    pub x_axis: XAxis,

    // --- linked hover state (Phase 4) ---
    pub hovered_vol_range: Option<(f32, f32)>,
    pub hovered_well: Option<Well>,
    pub hovered_volume: Option<f32>,

    // --- plate ---
    pub plate_channel: Option<ChannelId>,
    pub plate_metric: PlateMetric,
    /// Swap the on-brand blue ramp for a perceptually-uniform one (§10.3).
    pub plate_uniform_ramp: bool,

    // --- integration ---
    pub integrate_mode: bool,
    pub baseline_choice: BaselineChoice,
    pub pending_selection: Option<(f32, f32)>,
    pub drag_anchor: Option<f32>,
    pub peaks: Vec<PeakResult>,
    pub selected_peak: Option<PeakId>,
    next_peak_id: u32,

    pub excluded_regions: Vec<ExcludedRegion>,
    pub annotations: Vec<Annotation>,

    // --- calibration ---
    pub calibration: Option<Calibration>,
    pub calibration_name: String,
    pub cal_points: Vec<CalibrationPoint>,
    pub use_kav: bool,
    pub v0_ml: f32,
    pub vt_ml: f32,
    pub concentration: ConcentrationInputs,

    /// Set whenever the analysis diverges from the last saved sidecar.
    pub dirty: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            section: Section::Overview,
            nav_collapsed: false,
            hidden_channels: BTreeSet::new(),
            selected_channel: None,
            hero_channel_id: None,
            show_fractions: true,
            overview_order: PanelId::ALL.to_vec(),
            channel_colors: BTreeMap::new(),
            y_scale_mode: YScaleMode::default(),
            channel_y_ranges: BTreeMap::new(),
            x_axis: XAxis::Volume,
            hovered_vol_range: None,
            hovered_well: None,
            hovered_volume: None,
            plate_channel: None,
            plate_metric: PlateMetric::IntegratedArea,
            plate_uniform_ramp: false,
            integrate_mode: false,
            baseline_choice: BaselineChoice::LinearEndpoints,
            pending_selection: None,
            drag_anchor: None,
            peaks: Vec::new(),
            selected_peak: None,
            next_peak_id: 1,
            excluded_regions: Vec::new(),
            annotations: Vec::new(),
            calibration: None,
            calibration_name: "Bio-Rad GFS 1511901".to_string(),
            cal_points: Vec::new(),
            use_kav: false,
            v0_ml: 0.0,
            vt_ml: 0.0,
            concentration: ConcentrationInputs::default(),
            dirty: false,
        }
    }
}

impl View {
    /// Reset per-run state and adopt sensible defaults for a freshly opened run.
    pub fn adopt_run(&mut self, run: &Run) {
        self.hidden_channels.clear();
        self.channel_colors.clear();
        // Custom ranges are in one run's display units against one run's channel
        // ids; carrying them into the next run would silently clip a trace.
        // The *mode* is a viewing preference and survives, like `show_fractions`.
        self.channel_y_ranges.clear();
        self.peaks.clear();
        self.excluded_regions.clear();
        self.annotations.clear();
        self.calibration = None;
        self.cal_points.clear();
        self.selected_peak = None;
        self.pending_selection = None;
        self.drag_anchor = None;
        self.hovered_vol_range = None;
        self.hovered_well = None;
        self.next_peak_id = 1;
        self.dirty = false;
        // `overview_order` is deliberately not reset: the arrangement is a
        // working preference, not a property of the run, so it survives opening
        // the next file. A sidecar for that file may still override it.

        let hero = run.hero_channel().map(|c| c.id.clone());
        self.hero_channel_id = hero.clone();
        self.selected_channel = hero.clone();
        self.plate_channel = hero;

        // Start with the hero plus one context channel visible; a run can carry
        // fourteen traces and showing all of them at once is unreadable.
        let keep: BTreeSet<ChannelId> = run
            .channels
            .iter()
            .filter(|c| !c.is_empty())
            .filter(|c| {
                Some(&c.id) == self.hero_channel_id.as_ref()
                    || c.kind == elusive_core::model::ChannelKind::Conductivity
            })
            .map(|c| c.id.clone())
            .collect();
        for channel in &run.channels {
            if !keep.contains(&channel.id) {
                self.hidden_channels.insert(channel.id.clone());
            }
        }

        if let (Some(v0), Some(vt)) = (run.meta.v0_ml, run.meta.vt_ml) {
            self.v0_ml = v0;
            self.vt_ml = vt;
            self.use_kav = true;
        }
        if let Some(l) = run.meta.path_length_cm {
            self.concentration.path_length_cm = l as f64;
        }
    }

    pub fn is_channel_visible(&self, id: &ChannelId) -> bool {
        !self.hidden_channels.contains(id)
    }

    pub fn set_channel_visible(&mut self, id: &ChannelId, visible: bool) {
        let changed = if visible {
            self.hidden_channels.remove(id)
        } else {
            self.hidden_channels.insert(id.clone())
        };
        if changed {
            self.dirty = true;
        }
    }

    /// The user's chosen colour for a channel, if they set one.
    pub fn channel_color(&self, id: &ChannelId) -> Option<Color> {
        self.channel_colors.get(id).copied()
    }

    /// Override a channel's trace colour.
    ///
    /// Forced opaque: a trace is a line, not a fill, and a semi-transparent line
    /// over a fraction zone reads as a different colour than the legend swatch.
    pub fn set_channel_color(&mut self, id: &ChannelId, color: Color) {
        let color = Color::new(color.r, color.g, color.b, 0xFF);
        if self.channel_colors.insert(id.clone(), color) != Some(color) {
            self.dirty = true;
        }
    }

    /// Drop an override so the channel falls back to the automatic colour.
    pub fn clear_channel_color(&mut self, id: &ChannelId) {
        if self.channel_colors.remove(id).is_some() {
            self.dirty = true;
        }
    }

    pub fn set_y_scale_mode(&mut self, mode: YScaleMode) {
        if self.y_scale_mode != mode {
            self.y_scale_mode = mode;
            self.dirty = true;
        }
    }

    /// The user's y range for a channel, in display units. `None` means the
    /// chromatogram should fall back to the channel's own data range.
    pub fn channel_y_range(&self, id: &ChannelId) -> Option<(f32, f32)> {
        self.channel_y_ranges.get(id).copied()
    }

    /// Record a custom y range, in display units.
    ///
    /// Non-finite input is refused outright: a stored NaN would propagate into
    /// every y coordinate in the group. An inverted or zero-width range *is*
    /// stored, because rejecting it would fight the user halfway through typing
    /// a new minimum; the chromatogram falls back to the data range and the
    /// legend says so.
    pub fn set_channel_y_range(&mut self, id: &ChannelId, lo: f32, hi: f32) {
        if !lo.is_finite() || !hi.is_finite() {
            return;
        }
        if self.channel_y_ranges.insert(id.clone(), (lo, hi)) != Some((lo, hi)) {
            self.dirty = true;
        }
    }

    /// Drop a custom range, returning the channel to its data range.
    pub fn clear_channel_y_range(&mut self, id: &ChannelId) {
        if self.channel_y_ranges.remove(id).is_some() {
            self.dirty = true;
        }
    }

    pub fn set_show_fractions(&mut self, show: bool) {
        if self.show_fractions != show {
            self.show_fractions = show;
            self.dirty = true;
        }
    }

    /// Collapse or expand the navigation rail.
    ///
    /// Marked dirty like the other saved display preferences: the rail state
    /// rides in the sidecar because `persist_egui_memory` is off, so an unsaved
    /// toggle really would be lost on the next launch.
    pub fn set_nav_collapsed(&mut self, collapsed: bool) {
        if self.nav_collapsed != collapsed {
            self.nav_collapsed = collapsed;
            self.dirty = true;
        }
    }

    /// Drop the panel `dragged` into the slot `target` currently occupies.
    ///
    /// Takes panels rather than indices because the drag payload is a panel and
    /// the visible list may omit the Warnings card; resolving against
    /// `overview_order` here keeps the hidden card's saved position intact.
    pub fn move_overview_panel(&mut self, dragged: PanelId, target: PanelId) {
        let (Some(from), Some(to)) = (
            self.overview_order.iter().position(|p| *p == dragged),
            self.overview_order.iter().position(|p| *p == target),
        ) else {
            return;
        };
        if reorder(&mut self.overview_order, from, to) {
            self.dirty = true;
        }
    }

    pub fn overview_order_is_default(&self) -> bool {
        self.overview_order == PanelId::ALL
    }

    pub fn reset_overview_order(&mut self) {
        if !self.overview_order_is_default() {
            self.overview_order = PanelId::ALL.to_vec();
            self.dirty = true;
        }
    }

    /// Switch the chromatogram's display axis.
    ///
    /// Any in-flight drag selection is dropped: it was measured in the old axis'
    /// units, and reinterpreting those numbers as the new unit would hand the
    /// integrator a window the user never drew.
    pub fn set_x_axis(&mut self, axis: XAxis) {
        if self.x_axis != axis {
            self.x_axis = axis;
            self.pending_selection = None;
            self.drag_anchor = None;
            self.dirty = true;
        }
    }

    pub fn set_plate_channel(&mut self, channel: Option<ChannelId>) {
        if self.plate_channel != channel {
            self.plate_channel = channel;
            self.dirty = true;
        }
    }

    pub fn set_plate_metric(&mut self, metric: PlateMetric) {
        if self.plate_metric != metric {
            self.plate_metric = metric;
            self.dirty = true;
        }
    }

    pub fn set_plate_uniform_ramp(&mut self, enabled: bool) {
        if self.plate_uniform_ramp != enabled {
            self.plate_uniform_ramp = enabled;
            self.dirty = true;
        }
    }

    /// Make a channel the current focus of the chromatogram UI.
    ///
    /// Selection itself is transient UI state, but focusing a hidden channel is
    /// treated as intent to see it, so the channel is also revealed.
    pub fn focus_channel(&mut self, id: &ChannelId) {
        self.selected_channel = Some(id.clone());
        self.hero_channel_id = Some(id.clone());
        if self.hidden_channels.remove(id) {
            self.dirty = true;
        }
    }

    /// Allocate the next peak id. Ids never repeat within a session, so a peak
    /// deleted and re-created is distinguishable in an export.
    pub fn allocate_peak_id(&mut self) -> PeakId {
        let id = PeakId(self.next_peak_id);
        self.next_peak_id += 1;
        id
    }

    pub fn add_peak(&mut self, peak: PeakResult) {
        self.selected_peak = Some(peak.id);
        self.peaks.push(peak);
        self.peaks
            .sort_by(|a, b| a.v_start_ml.total_cmp(&b.v_start_ml));
        self.dirty = true;
    }

    pub fn remove_peak(&mut self, id: PeakId) {
        self.peaks.retain(|p| p.id != id);
        if self.selected_peak == Some(id) {
            self.selected_peak = None;
        }
        self.dirty = true;
    }

    pub fn selected_peak(&self) -> Option<&PeakResult> {
        let id = self.selected_peak?;
        self.peaks.iter().find(|p| p.id == id)
    }

    /// Total peak area on a channel, for the area-% column.
    pub fn total_area_on(&self, channel: &ChannelId) -> f64 {
        self.peaks
            .iter()
            .filter(|p| &p.channel_id == channel)
            .map(|p| p.area.abs())
            .sum()
    }

    /// Build a sidecar snapshot of the current analysis.
    pub fn to_sidecar(&self, run: &Run) -> Sidecar {
        let mut sidecar = Sidecar::for_run(run);
        sidecar.peaks = self.peaks.clone();
        sidecar.excluded_regions = self.excluded_regions.clone();
        sidecar.annotations = self.annotations.clone();
        sidecar.calibrations = self
            .calibration
            .as_ref()
            .map(|c| {
                vec![NamedCalibration {
                    name: self.calibration_name.clone(),
                    calibration: c.clone(),
                }]
            })
            .unwrap_or_default();
        sidecar.view = ViewState {
            visible_channels: run
                .channels
                .iter()
                .filter(|c| self.is_channel_visible(&c.id))
                .map(|c| c.id.0.clone())
                .collect(),
            dark_mode: None,
            plate_channel: self.plate_channel.as_ref().map(|c| c.0.clone()),
            plate_metric: Some(self.plate_metric),
            show_fractions: Some(self.show_fractions),
            plate_uniform_ramp: Some(self.plate_uniform_ramp),
            nav_collapsed: Some(self.nav_collapsed),
            overview_order: Some(
                self.overview_order
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect(),
            ),
            channel_colors: Some(
                self.channel_colors
                    .iter()
                    .map(|(id, color)| (id.0.clone(), *color))
                    .collect(),
            ),
            x_axis: Some(self.x_axis.key().to_string()),
            y_scale_mode: Some(self.y_scale_mode.as_key().to_string()),
            // Omitted entirely when empty, so the common case does not add a
            // dead object to every sidecar on disk.
            channel_y_ranges: (!self.channel_y_ranges.is_empty()).then(|| {
                self.channel_y_ranges
                    .iter()
                    .map(|(id, range)| (id.0.clone(), *range))
                    .collect()
            }),
            // Overlays live beside the run in the app, not in `View`; the app
            // fills this after `to_sidecar` (see `EluSiveApp::save_sidecar`).
            overlays: None,
        };
        sidecar
    }

    /// Restore an analysis from a sidecar. Returns any messages worth showing.
    pub fn apply_sidecar(&mut self, sidecar: &Sidecar, run: &Run) -> Vec<String> {
        let mut notes = Vec::new();

        let orphans = sidecar.orphaned_peaks(run).len();
        if orphans > 0 {
            notes.push(format!(
                "{orphans} saved peak(s) reference a channel this run does not have; they were not restored"
            ));
        }
        self.peaks = sidecar
            .peaks
            .iter()
            .filter(|p| run.channels.iter().any(|c| c.id == p.channel_id))
            .cloned()
            .collect();
        self.next_peak_id = self.peaks.iter().map(|p| p.id.0).max().unwrap_or(0) + 1;

        self.excluded_regions = sidecar.excluded_regions.clone();
        self.annotations = sidecar.annotations.clone();

        if let Some(named) = sidecar.calibrations.first() {
            self.calibration_name = named.name.clone();
            self.cal_points = named.calibration.points.clone();
            if let elusive_core::calibration::FitBasis::Kav { v0_ml, vt_ml } =
                named.calibration.basis
            {
                self.use_kav = true;
                self.v0_ml = v0_ml;
                self.vt_ml = vt_ml;
            }
            self.calibration = Some(named.calibration.clone());
            for peak in &mut self.peaks {
                peak.estimated_mw_kda = run
                    .channel(&peak.channel_id)
                    .filter(|channel| channel.kind == elusive_core::model::ChannelKind::Uv)
                    .and_then(|_| named.calibration.mw_for_volume(peak.apex_volume_ml));
            }
        } else {
            for peak in &mut self.peaks {
                peak.estimated_mw_kda = None;
            }
        }

        if !sidecar.view.visible_channels.is_empty() {
            let visible: BTreeSet<String> = sidecar.view.visible_channels.iter().cloned().collect();
            self.hidden_channels = run
                .channels
                .iter()
                .filter(|c| !visible.contains(&c.id.0))
                .map(|c| c.id.clone())
                .collect();
        }
        if let Some(id) = &sidecar.view.plate_channel {
            if run.channels.iter().any(|c| c.id.0 == *id) {
                self.plate_channel = Some(ChannelId(id.clone()));
            }
        }
        if let Some(metric) = sidecar.view.plate_metric {
            self.plate_metric = metric;
        }
        if let Some(show) = sidecar.view.show_fractions {
            self.show_fractions = show;
        }
        if let Some(enabled) = sidecar.view.plate_uniform_ramp {
            self.plate_uniform_ramp = enabled;
        }
        if let Some(collapsed) = sidecar.view.nav_collapsed {
            self.nav_collapsed = collapsed;
        }
        if let Some(saved) = &sidecar.view.overview_order {
            self.overview_order = restore_order(saved);
        }
        if let Some(colors) = &sidecar.view.channel_colors {
            // An override for a channel this run lacks is dropped without a note.
            // Unlike an orphaned peak it carries no measurement, so warning about
            // it would be noise on every reopen of a re-exported run.
            self.channel_colors = colors
                .iter()
                .filter(|(id, _)| run.channels.iter().any(|c| c.id.0 == **id))
                .map(|(id, color)| (ChannelId(id.clone()), *color))
                .collect();
        }
        // An unknown key leaves the default in place rather than reporting a
        // problem: the axis is a preference, not part of the analysis.
        if let Some(axis) = sidecar.view.x_axis.as_deref().and_then(XAxis::from_key) {
            self.x_axis = axis;
        }
        if let Some(key) = &sidecar.view.y_scale_mode {
            match YScaleMode::from_key(key) {
                Some(mode) => self.y_scale_mode = mode,
                // A mode this build does not know is a display preference, not
                // analysis: say so and carry on rather than refusing the file.
                // Unlike `x_axis` above, this one is reported — a y-scale the
                // file asked for and did not get changes how tall every trace
                // looks, which is worth a line in the load notes.
                None => notes.push(format!(
                    "unknown y-scale mode '{key}'; using the shared scale"
                )),
            }
        }
        if let Some(ranges) = &sidecar.view.channel_y_ranges {
            // Same rule as the colour overrides: a range for a channel this run
            // lacks is dropped silently, and a hand-edited non-finite one never
            // reaches the remap.
            self.channel_y_ranges = ranges
                .iter()
                .filter(|(id, _)| run.channels.iter().any(|c| c.id.0 == **id))
                .filter(|(_, (lo, hi))| lo.is_finite() && hi.is_finite())
                .map(|(id, range)| (ChannelId(id.clone()), *range))
                .collect();
        }

        self.dirty = false;
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use elusive_core::model::{Channel, ChannelKind, RunMeta, Sample, SourceFormat};

    fn test_run() -> Run {
        let mut uv = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        uv.wavelength_nm = Some(280);
        uv.samples = vec![Sample::new(0.0, 0.0, 1.0), Sample::new(60.0, 1.0, 2.0)];
        let mut cond = Channel::new("MD_Conductivity", "Conductivity", ChannelKind::Conductivity);
        cond.samples = vec![Sample::new(0.0, 0.0, 17.0), Sample::new(60.0, 1.0, 17.5)];
        let mut ph = Channel::new("ModulePH", "pH", ChannelKind::Ph);
        ph.samples = vec![Sample::new(0.0, 0.0, 8.1)];

        Run {
            meta: RunMeta {
                run_name: "test".into(),
                ..RunMeta::default()
            },
            source_format: SourceFormat::NgcAnalysis,
            source_path: std::path::PathBuf::from("test.ngcAnalysis"),
            channels: vec![uv, cond, ph],
            fractions: Vec::new(),
            events: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn adopting_a_run_defaults_to_the_uv_hero_trace() {
        let run = test_run();
        let mut view = View::default();
        view.adopt_run(&run);
        assert_eq!(view.hero_channel_id.as_ref().unwrap().as_str(), "MWave2");
        assert_eq!(view.plate_channel.as_ref().unwrap().as_str(), "MWave2");
    }

    #[test]
    fn a_fresh_run_does_not_show_every_channel_at_once() {
        let run = test_run();
        let mut view = View::default();
        view.adopt_run(&run);
        assert!(view.is_channel_visible(&ChannelId::from("MWave2")));
        assert!(view.is_channel_visible(&ChannelId::from("MD_Conductivity")));
        assert!(!view.is_channel_visible(&ChannelId::from("ModulePH")));
    }

    #[test]
    fn peak_ids_do_not_repeat_after_a_delete() {
        let mut view = View::default();
        let first = view.allocate_peak_id();
        let second = view.allocate_peak_id();
        assert_ne!(first, second);
        view.remove_peak(first);
        assert_ne!(view.allocate_peak_id(), first);
    }

    #[test]
    fn kav_is_enabled_only_when_the_method_supplies_both_volumes() {
        let mut run = test_run();
        let mut view = View::default();
        view.adopt_run(&run);
        assert!(
            !view.use_kav,
            "no V0/Vt in the method means volume-based fitting"
        );

        run.meta.v0_ml = Some(8.0);
        run.meta.vt_ml = Some(24.0);
        let mut view = View::default();
        view.adopt_run(&run);
        assert!(view.use_kav);
        assert_eq!(view.v0_ml, 8.0);
    }

    #[test]
    fn sidecar_round_trip_restores_peaks_and_plate_settings() {
        let run = test_run();
        let mut view = View::default();
        view.adopt_run(&run);
        view.set_plate_metric(PlateMetric::MaxValue);
        view.set_show_fractions(false);
        view.set_plate_uniform_ramp(true);
        view.set_nav_collapsed(true);
        view.set_channel_color(
            &ChannelId::from("MWave2"),
            Color::new(0xC4, 0x77, 0x3D, 0xFF),
        );
        view.set_x_axis(XAxis::Time);
        view.set_y_scale_mode(YScaleMode::Custom);
        view.set_channel_y_range(&ChannelId::from("MWave2"), -5.0, 250.0);
        let peak_id = view.allocate_peak_id();
        view.add_peak(PeakResult {
            id: peak_id,
            channel_id: ChannelId::from("MWave2"),
            v_start_ml: 0.0,
            v_end_ml: 1.0,
            baseline: BaselineMode::DropToZero,
            area: 1.5,
            height: 2.0,
            apex_volume_ml: 1.0,
            fwhm_ml: None,
            estimated_mw_kda: None,
        });
        let sidecar = view.to_sidecar(&run);

        let mut restored = View::default();
        restored.adopt_run(&run);
        let notes = restored.apply_sidecar(&sidecar, &run);
        assert!(notes.is_empty());
        assert_eq!(restored.peaks.len(), 1);
        assert_eq!(restored.plate_metric, PlateMetric::MaxValue);
        assert!(!restored.show_fractions);
        assert!(restored.plate_uniform_ramp);
        assert!(restored.nav_collapsed);
        assert_eq!(
            restored.channel_color(&ChannelId::from("MWave2")),
            Some(Color::new(0xC4, 0x77, 0x3D, 0xFF))
        );
        assert_eq!(restored.x_axis, XAxis::Time);
        assert_eq!(restored.y_scale_mode, YScaleMode::Custom);
        assert_eq!(
            restored.channel_y_range(&ChannelId::from("MWave2")),
            Some((-5.0, 250.0))
        );
        assert!(!restored.dirty, "a freshly loaded sidecar is not dirty");
    }

    #[test]
    fn a_sidecar_without_y_scale_fields_still_loads_on_the_shared_scale() {
        // Every sidecar written before this feature existed.
        let run = test_run();
        let mut view = View {
            y_scale_mode: YScaleMode::AutoEach,
            ..View::default()
        };
        let notes = view.apply_sidecar(&Sidecar::for_run(&run), &run);
        assert!(notes.is_empty());
        assert_eq!(
            view.y_scale_mode,
            YScaleMode::AutoEach,
            "an absent field means 'unstated', so the current preference stands"
        );
    }

    #[test]
    fn an_unknown_y_scale_mode_is_reported_rather_than_refusing_the_file() {
        let run = test_run();
        let mut sidecar = Sidecar::for_run(&run);
        sidecar.view.y_scale_mode = Some("logarithmic-per-decade".into());
        let mut view = View::default();
        let notes = view.apply_sidecar(&sidecar, &run);
        assert_eq!(view.y_scale_mode, YScaleMode::AutoAll);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("logarithmic-per-decade"));
    }

    #[test]
    fn y_scale_mode_keys_round_trip_and_do_not_collide() {
        for mode in YScaleMode::ALL {
            assert_eq!(YScaleMode::from_key(mode.as_key()), Some(mode));
        }
        assert_eq!(YScaleMode::from_key("nonsense"), None);
    }

    #[test]
    fn a_custom_range_for_a_channel_this_run_lacks_is_dropped_on_restore() {
        let run = test_run();
        let mut sidecar = Sidecar::for_run(&run);
        let mut ranges = BTreeMap::new();
        ranges.insert("MWave2".to_string(), (0.0f32, 100.0f32));
        ranges.insert("NoSuchChannel".to_string(), (0.0f32, 1.0f32));
        sidecar.view.channel_y_ranges = Some(ranges);

        let mut view = View::default();
        view.apply_sidecar(&sidecar, &run);
        assert_eq!(view.channel_y_ranges.len(), 1);
        assert!(view
            .channel_y_ranges
            .contains_key(&ChannelId::from("MWave2")));
    }

    #[test]
    fn a_non_finite_custom_range_is_refused_rather_than_stored() {
        let mut view = View::default();
        let id = ChannelId::from("MWave2");
        view.set_channel_y_range(&id, f32::NAN, 100.0);
        view.set_channel_y_range(&id, 0.0, f32::INFINITY);
        assert_eq!(view.channel_y_range(&id), None);
        assert!(!view.dirty);

        // An inverted range *is* kept — the user may be mid-edit — and the
        // chromatogram falls back to the data range rather than dividing by zero.
        view.set_channel_y_range(&id, 100.0, 0.0);
        assert_eq!(view.channel_y_range(&id), Some((100.0, 0.0)));
        assert!(view.dirty);

        view.clear_channel_y_range(&id);
        assert_eq!(view.channel_y_range(&id), None);
    }

    #[test]
    fn opening_another_run_does_not_inherit_the_previous_run_s_custom_ranges() {
        // The numbers are in one run's display units; carrying them over would
        // silently clip a trace in the new run.
        let run = test_run();
        let mut view = View::default();
        view.set_channel_y_range(&ChannelId::from("MWave2"), 0.0, 100.0);
        view.adopt_run(&run);
        assert!(view.channel_y_ranges.is_empty());
    }

    #[test]
    fn integration_results_do_not_depend_on_the_y_scale_mode() {
        // The y-scale is a drawing decision. If it ever leaked into the numbers,
        // the Results table and the CSV export would disagree with the instrument.
        use elusive_core::integrate::integrate_peak;

        let run = test_run();
        let channel = run.channel(&ChannelId::from("MWave2")).expect("uv channel");

        let mut results = Vec::new();
        for mode in YScaleMode::ALL {
            let mut view = View::default();
            view.adopt_run(&run);
            view.set_y_scale_mode(mode);
            // Absurd on purpose: a range this narrow would clip the trace off the
            // top of the plot, and must still leave the arithmetic untouched.
            view.set_channel_y_range(&ChannelId::from("MWave2"), 0.0, 0.001);

            let id = view.allocate_peak_id();
            let peak = integrate_peak(id, channel, 0.0, 1.0, BaselineMode::DropToZero)
                .expect("the window covers sampled volumes");
            results.push((peak.area, peak.height, peak.apex_volume_ml));
        }
        assert!(
            results.windows(2).all(|w| w[0] == w[1]),
            "area/height/apex changed with the display scale: {results:?}"
        );
    }

    #[test]
    fn a_sidecar_written_before_the_rail_existed_leaves_it_alone() {
        // `nav_collapsed` is optional precisely so an older file does not force
        // the rail open on someone who collapsed it.
        let run = test_run();
        let mut view = View::default();
        view.set_nav_collapsed(true);
        let mut sidecar = Sidecar::for_run(&run);
        sidecar.view.nav_collapsed = None;
        view.apply_sidecar(&sidecar, &run);
        assert!(view.nav_collapsed);
    }

    #[test]
    fn a_sidecar_written_before_colour_overrides_existed_still_loads() {
        // `channel_colors: None` is what an older file deserializes to (the
        // wire-format half of this is checked in `elusive-core`'s sidecar tests).
        let run = test_run();
        let mut sidecar = Sidecar::for_run(&run);
        sidecar.view.channel_colors = None;

        let mut view = View::default();
        view.adopt_run(&run);
        view.set_channel_color(&ChannelId::from("MWave2"), Color::new(1, 2, 3, 0xFF));
        view.apply_sidecar(&sidecar, &run);
        assert_eq!(
            view.channel_color(&ChannelId::from("MWave2")),
            Some(Color::new(1, 2, 3, 0xFF)),
            "a sidecar with no opinion must not silently clear the session's colours"
        );
    }

    #[test]
    fn a_colour_override_is_stored_opaque_and_marks_the_view_dirty() {
        let mut view = View::default();
        let id = ChannelId::from("MWave2");
        assert!(!view.dirty);

        // Traces are lines, not fills; the stored alpha is always 0xFF.
        view.set_channel_color(&id, Color::new(0x2E, 0x95, 0x99, 0x40));
        assert_eq!(
            view.channel_color(&id),
            Some(Color::new(0x2E, 0x95, 0x99, 0xFF))
        );
        assert!(view.dirty);

        // Re-picking the same colour is not a change worth re-saving for.
        view.dirty = false;
        view.set_channel_color(&id, Color::new(0x2E, 0x95, 0x99, 0xFF));
        assert!(!view.dirty);

        view.clear_channel_color(&id);
        assert_eq!(view.channel_color(&id), None);
        assert!(view.dirty);

        // Clearing an override that was never set is likewise a no-op.
        view.dirty = false;
        view.clear_channel_color(&id);
        assert!(!view.dirty);
    }

    #[test]
    fn a_sidecar_without_an_axis_preference_still_loads() {
        // Files written before the toggle existed have no `x_axis` key; they must
        // deserialize and fall back to the volume default.
        let run = test_run();
        let sidecar = Sidecar::for_run(&run);
        assert_eq!(sidecar.view.x_axis, None);

        let mut view = View::default();
        view.set_x_axis(XAxis::Time);
        view.apply_sidecar(&sidecar, &run);
        assert_eq!(view.x_axis, XAxis::Time, "an absent key changes nothing");

        assert_eq!(XAxis::from_key("who knows"), None);
    }

    #[test]
    fn switching_the_axis_drops_a_selection_measured_in_the_old_unit() {
        let mut view = View {
            pending_selection: Some((1.0, 2.0)),
            drag_anchor: Some(1.0),
            ..View::default()
        };
        view.set_x_axis(XAxis::Time);
        assert_eq!(view.pending_selection, None);
        assert_eq!(view.drag_anchor, None);
        assert!(view.dirty);
    }

    #[test]
    fn peaks_for_a_missing_channel_are_reported_rather_than_silently_dropped() {
        let run = test_run();
        let mut sidecar = Sidecar::for_run(&run);
        sidecar.peaks.push(PeakResult {
            id: PeakId(1),
            channel_id: ChannelId::from("NoSuchChannel"),
            v_start_ml: 0.0,
            v_end_ml: 1.0,
            baseline: BaselineMode::DropToZero,
            area: 1.0,
            height: 1.0,
            apex_volume_ml: 0.5,
            fwhm_ml: None,
            estimated_mw_kda: None,
        });

        let mut view = View::default();
        let notes = view.apply_sidecar(&sidecar, &run);
        assert!(view.peaks.is_empty());
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("not restored"));
    }

    #[test]
    fn restoring_a_sidecar_continues_peak_numbering_past_the_saved_ids() {
        let run = test_run();
        let mut sidecar = Sidecar::for_run(&run);
        sidecar.peaks.push(PeakResult {
            id: PeakId(9),
            channel_id: ChannelId::from("MWave2"),
            v_start_ml: 0.0,
            v_end_ml: 1.0,
            baseline: BaselineMode::DropToZero,
            area: 1.0,
            height: 1.0,
            apex_volume_ml: 0.5,
            fwhm_ml: None,
            estimated_mw_kda: None,
        });
        let mut view = View::default();
        view.apply_sidecar(&sidecar, &run);
        assert_eq!(view.allocate_peak_id(), PeakId(10));
    }

    #[test]
    fn moving_a_panel_forward_and_backward_shifts_the_rest() {
        let mut order = PanelId::ALL.to_vec();
        assert!(reorder(&mut order, 0, 2));
        assert_eq!(
            order,
            vec![
                PanelId::Warnings,
                PanelId::Channels,
                PanelId::RunSummary,
                PanelId::Fractions
            ]
        );
        assert!(reorder(&mut order, 3, 1));
        assert_eq!(
            order,
            vec![
                PanelId::Warnings,
                PanelId::Fractions,
                PanelId::Channels,
                PanelId::RunSummary
            ]
        );
    }

    #[test]
    fn dropping_a_panel_on_itself_or_off_the_end_is_not_a_reorder() {
        let mut order = PanelId::ALL.to_vec();
        assert!(!reorder(&mut order, 2, 2));
        assert!(!reorder(&mut order, 0, 9));
        assert!(!reorder(&mut order, 9, 0));
        assert_eq!(order, PanelId::ALL.to_vec());
    }

    #[test]
    fn a_reorder_marks_the_analysis_dirty() {
        let mut view = View::default();
        view.move_overview_panel(PanelId::Fractions, PanelId::RunSummary);
        assert_eq!(view.overview_order.first(), Some(&PanelId::Fractions));
        assert!(view.dirty);

        view.dirty = false;
        view.reset_overview_order();
        assert!(view.overview_order_is_default());
        assert!(view.dirty);
    }

    #[test]
    fn an_unknown_saved_panel_id_is_dropped_rather_than_shown() {
        let order = restore_order(&[
            "fractions".to_string(),
            "spectra_from_a_future_build".to_string(),
            "run_summary".to_string(),
        ]);
        assert_eq!(order[0], PanelId::Fractions);
        assert_eq!(order[1], PanelId::RunSummary);
        assert_eq!(order.len(), PanelId::ALL.len());
    }

    #[test]
    fn a_panel_missing_from_the_saved_order_is_still_shown() {
        let order = restore_order(&["channels".to_string(), "channels".to_string()]);
        assert_eq!(order[0], PanelId::Channels);
        let mut sorted = order.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), PanelId::ALL.len(), "every panel appears once");
    }

    #[test]
    fn sidecar_round_trip_restores_the_overview_arrangement() {
        let run = test_run();
        let mut view = View::default();
        view.adopt_run(&run);
        view.move_overview_panel(PanelId::Fractions, PanelId::RunSummary);
        let arranged = view.overview_order.clone();
        let sidecar = view.to_sidecar(&run);
        assert_eq!(
            sidecar.view.overview_order.as_deref(),
            Some(
                arranged
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect::<Vec<_>>()
                    .as_slice()
            ),
            "the sidecar stores the stable string form"
        );

        let mut restored = View::default();
        restored.apply_sidecar(&sidecar, &run);
        assert_eq!(restored.overview_order, arranged);
        assert!(!restored.dirty);
    }

    #[test]
    fn a_sidecar_without_an_overview_order_keeps_the_default() {
        let run = test_run();
        let sidecar = Sidecar::for_run(&run);
        assert!(sidecar.view.overview_order.is_none());

        let mut view = View::default();
        view.move_overview_panel(PanelId::Channels, PanelId::RunSummary);
        view.apply_sidecar(&sidecar, &run);
        assert_eq!(
            view.overview_order.first(),
            Some(&PanelId::Channels),
            "an older sidecar says nothing about the order, so it must not clear it"
        );
    }

    #[test]
    fn valley_baseline_defaults_its_anchors_to_the_dragged_window() {
        let mode = BaselineChoice::ValleyToValley.resolve(10.0, 12.0);
        assert_eq!(
            mode,
            BaselineMode::ValleyToValley {
                left_ml: 10.0,
                right_ml: 12.0
            }
        );
    }

    #[test]
    fn area_percentages_are_scoped_to_one_channel() {
        let mut view = View::default();
        for (id, area) in [("MWave2", 10.0), ("MWave2", 30.0), ("MD_Conductivity", 5.0)] {
            let peak_id = view.allocate_peak_id();
            view.add_peak(PeakResult {
                id: peak_id,
                channel_id: ChannelId::from(id),
                v_start_ml: 0.0,
                v_end_ml: 1.0,
                baseline: BaselineMode::DropToZero,
                area,
                height: 1.0,
                apex_volume_ml: 0.5,
                fwhm_ml: None,
                estimated_mw_kda: None,
            });
        }
        assert_eq!(view.total_area_on(&ChannelId::from("MWave2")), 40.0);
    }
}

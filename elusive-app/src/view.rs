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
use elusive_core::model::{BaselineMode, ChannelId, PeakId, PeakResult, Run, Well};
use elusive_core::sidecar::{Annotation, ExcludedRegion, NamedCalibration, Sidecar, ViewState};
use std::collections::BTreeSet;

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

    // --- channel display ---
    pub hidden_channels: BTreeSet<ChannelId>,
    pub selected_channel: Option<ChannelId>,
    pub hero_channel_id: Option<ChannelId>,
    pub show_fractions: bool,

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
            hidden_channels: BTreeSet::new(),
            selected_channel: None,
            hero_channel_id: None,
            show_fractions: true,
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

    pub fn set_show_fractions(&mut self, show: bool) {
        if self.show_fractions != show {
            self.show_fractions = show;
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
        assert!(!restored.dirty, "a freshly loaded sidecar is not dirty");
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

//! The format-agnostic run model.
//!
//! Both the native NGC parser and the CSV importer produce a [`Run`], so nothing
//! downstream (plotting, integration, plate metrics) needs to know where the data
//! came from.
//!
//! Two invariants matter enough to repeat here, because violating either has
//! produced real bugs in tools like this one:
//!
//! 1. **Channels do not share a sample index.** ChromLab samples pH at roughly
//!    twice the UV rate, so `channels[0].samples[i]` and `channels[1].samples[i]`
//!    are unrelated points in time. Always look values up by volume or time.
//! 2. **Stored values are raw.** UV arrives in AU while ChromLab displays mAU.
//!    The scale factor lives in [`Channel::display_scale`] and is applied at the
//!    point of display, never baked into `samples`.

use serde::{Deserialize, Serialize};

/// One acquired point. The NGC binary layout stores exactly this triplet per
/// record (`design.md` §3.1), and we keep all three even though plots key off
/// volume — time-domain analysis and round-tripping stay possible.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub time_s: f32,
    pub volume_ml: f32,
    pub value: f32,
}

impl Sample {
    pub fn new(time_s: f32, volume_ml: f32, value: f32) -> Self {
        Self {
            time_s,
            volume_ml,
            value,
        }
    }

    /// A sample is usable for math only if every component is finite; instrument
    /// exports do occasionally contain NaN padding.
    pub fn is_finite(&self) -> bool {
        self.time_s.is_finite() && self.volume_ml.is_finite() && self.value.is_finite()
    }
}

/// Endpoint equality that survives a unit conversion.
///
/// The plain `f32::EPSILON` test used by [`Channel::value_at_volume`] is an
/// *absolute* 1.2e-7, which is far below one ulp of a run time in seconds (a
/// 4551 s run has an ulp near 5e-4). A caller that asks for the last sample after
/// a seconds↔minutes round trip would therefore be told it is out of range. The
/// tolerance is scaled to the magnitude of the operands so the endpoints of the
/// time↔volume conversions stay reachable.
fn nearly_equal(a: f32, b: f32) -> bool {
    (a - b).abs() <= 4.0 * f32::EPSILON * a.abs().max(b.abs()).max(1.0)
}

/// Drives default axis grouping, units, and colour assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChannelKind {
    Uv,
    Conductivity,
    PercentB,
    Ph,
    Pressure,
    Flow,
    Temperature,
    Other,
}

impl ChannelKind {
    /// Classify from the trace entry name used inside the archive
    /// (`MWave0`, `MD_Conductivity`, `PercentB`, `ModulePH`, …).
    pub fn from_trace_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if n.contains("wave") || n.contains("uv") || n.contains("absorb") {
            ChannelKind::Uv
        } else if n.contains("conductivity") || n.contains("cond") {
            ChannelKind::Conductivity
        } else if n.contains("percentb") || n == "%b" || n.contains("percent_b") {
            ChannelKind::PercentB
        } else if n.contains("ph") && !n.contains("phase") {
            ChannelKind::Ph
        } else if n.contains("pressure") {
            ChannelKind::Pressure
        } else if n.contains("flow") {
            ChannelKind::Flow
        } else if n.contains("temperature") || n.contains("temp") {
            ChannelKind::Temperature
        } else {
            ChannelKind::Other
        }
    }

    /// Channels sharing an axis group are drawn against the same y-axis.
    /// Anything not explicitly grouped gets its own axis rather than being
    /// silently squeezed onto an unrelated scale.
    pub fn axis_group(self) -> AxisGroup {
        match self {
            ChannelKind::Uv => AxisGroup::Uv,
            ChannelKind::Conductivity => AxisGroup::Conductivity,
            ChannelKind::PercentB => AxisGroup::Percent,
            ChannelKind::Ph => AxisGroup::Ph,
            ChannelKind::Pressure => AxisGroup::Pressure,
            ChannelKind::Flow => AxisGroup::Flow,
            ChannelKind::Temperature => AxisGroup::Temperature,
            ChannelKind::Other => AxisGroup::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ChannelKind::Uv => "UV",
            ChannelKind::Conductivity => "Conductivity",
            ChannelKind::PercentB => "%B",
            ChannelKind::Ph => "pH",
            ChannelKind::Pressure => "Pressure",
            ChannelKind::Flow => "Flow",
            ChannelKind::Temperature => "Temperature",
            ChannelKind::Other => "Other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AxisGroup {
    Uv,
    Conductivity,
    Percent,
    Ph,
    Pressure,
    Flow,
    Temperature,
    Other,
}

impl AxisGroup {
    pub fn label(self) -> &'static str {
        match self {
            AxisGroup::Uv => "UV",
            AxisGroup::Conductivity => "Conductivity",
            AxisGroup::Percent => "%B",
            AxisGroup::Ph => "pH",
            AxisGroup::Pressure => "Pressure",
            AxisGroup::Flow => "Flow",
            AxisGroup::Temperature => "Temperature",
            AxisGroup::Other => "Other",
        }
    }
}

/// An 8-bit RGBA colour carried through from a ChromLab legend.
///
/// Deliberately not an egui type: `elusive-core` never imports a UI toolkit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Parse ChromLab's `#AARRGGBB` (and tolerate `#RRGGBB` / missing `#`).
    pub fn parse_argb(s: &str) -> Option<Self> {
        let hex = s.trim().trim_start_matches('#');
        let v = u32::from_str_radix(hex, 16).ok()?;
        match hex.len() {
            8 => Some(Color::new(
                ((v >> 16) & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                (v & 0xFF) as u8,
                ((v >> 24) & 0xFF) as u8,
            )),
            6 => Some(Color::new(
                ((v >> 16) & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                (v & 0xFF) as u8,
                0xFF,
            )),
            _ => None,
        }
    }

    /// Relative luminance per WCAG 2.x, used to check the design-system contrast
    /// anchors before a legend colour is allowed to override `chart::SERIES`
    /// (`DESIGN_SYSTEM.md` §10.4).
    pub fn relative_luminance(self) -> f64 {
        fn channel(c: u8) -> f64 {
            let c = c as f64 / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// WCAG contrast ratio against another colour, ignoring alpha.
    pub fn contrast_ratio(self, other: Color) -> f64 {
        let (a, b) = (self.relative_luminance(), other.relative_luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }
}

/// Stable identity for a channel, used by peaks, sidecars and CSV exports.
/// This is the archive entry name (`MWave2`, `MD_Conductivity`, …) or the CSV
/// column stem, so it survives a reopen.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChannelId(pub String);

impl ChannelId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ChannelId {
    fn from(s: &str) -> Self {
        ChannelId(s.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    pub id: ChannelId,
    /// Human-readable name for the legend, e.g. `UV 280 nm`.
    pub name: String,
    /// Unit of the *stored* value.
    pub unit: String,
    pub kind: ChannelKind,
    /// Legend colour from the archive or Traces CSV, when present.
    pub color: Option<Color>,
    /// Multiplier taking stored value → displayed value (AU → mAU is 1000.0).
    pub display_scale: f32,
    /// Unit matching `display_scale`; equals `unit` when the scale is 1.
    pub display_unit: String,
    /// Detector wavelength for UV channels, once resolved.
    pub wavelength_nm: Option<u16>,
    /// Independent per-channel series. Never index-align this with another channel.
    pub samples: Vec<Sample>,
}

impl Channel {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: ChannelKind) -> Self {
        let id = id.into();
        Self {
            id: ChannelId(id),
            name: name.into(),
            unit: String::new(),
            kind,
            color: None,
            display_scale: 1.0,
            display_unit: String::new(),
            wavelength_nm: None,
            samples: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Volume span covered by this channel, if it has any finite samples.
    pub fn volume_range(&self) -> Option<(f32, f32)> {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for s in self.samples.iter().filter(|s| s.is_finite()) {
            lo = lo.min(s.volume_ml);
            hi = hi.max(s.volume_ml);
        }
        (lo <= hi).then_some((lo, hi))
    }

    /// Displayed value range (i.e. with `display_scale` applied).
    pub fn display_value_range(&self) -> Option<(f32, f32)> {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for s in self.samples.iter().filter(|s| s.is_finite()) {
            lo = lo.min(s.value);
            hi = hi.max(s.value);
        }
        (lo <= hi).then_some((lo * self.display_scale, hi * self.display_scale))
    }

    /// Linearly interpolate the stored value at a given elution volume.
    ///
    /// Returns `None` outside the sampled range rather than clamping, so callers
    /// can distinguish "no data here" from "flat here".
    pub fn value_at_volume(&self, volume_ml: f32) -> Option<f32> {
        let s = &self.samples;
        if s.is_empty() || !volume_ml.is_finite() {
            return None;
        }
        // Samples are monotonic in volume (the instrument pumps forwards), so a
        // binary search is valid and keeps hover-linking cheap on long runs.
        let idx = s.partition_point(|p| p.volume_ml < volume_ml);
        if idx == 0 {
            let first = s[0];
            return ((first.volume_ml - volume_ml).abs() <= f32::EPSILON).then_some(first.value);
        }
        if idx >= s.len() {
            let last = s[s.len() - 1];
            return ((volume_ml - last.volume_ml).abs() <= f32::EPSILON).then_some(last.value);
        }
        let (a, b) = (s[idx - 1], s[idx]);
        let span = b.volume_ml - a.volume_ml;
        if span.abs() < f32::EPSILON {
            return Some(a.value);
        }
        let t = (volume_ml - a.volume_ml) / span;
        Some(a.value + t * (b.value - a.value))
    }

    /// Elapsed time in **minutes** at a given elution volume.
    ///
    /// Interpolates the recorded sample pairs. Flow rate is not constant across a
    /// method — gradients, wash steps and pauses all change it — so scaling a
    /// volume by an average rate would misplace anything drawn on a time axis.
    ///
    /// Like [`Channel::value_at_volume`], returns `None` outside the sampled range
    /// rather than clamping, so a caller can tell "no data here" from "flat here".
    pub fn time_min_at_volume(&self, volume_ml: f32) -> Option<f32> {
        let s = &self.samples;
        if s.is_empty() || !volume_ml.is_finite() {
            return None;
        }
        // Monotonic in volume (the instrument pumps forwards), so binary search
        // is valid — same assumption `value_at_volume` relies on.
        let idx = s.partition_point(|p| p.volume_ml < volume_ml);
        if idx == 0 {
            let first = s[0];
            return nearly_equal(first.volume_ml, volume_ml).then_some(first.time_s / 60.0);
        }
        if idx >= s.len() {
            let last = s[s.len() - 1];
            return nearly_equal(last.volume_ml, volume_ml).then_some(last.time_s / 60.0);
        }
        let (a, b) = (s[idx - 1], s[idx]);
        let span = b.volume_ml - a.volume_ml;
        if span.abs() < f32::EPSILON {
            return Some(a.time_s / 60.0);
        }
        let t = (volume_ml - a.volume_ml) / span;
        Some((a.time_s + t * (b.time_s - a.time_s)) / 60.0)
    }

    /// Elution volume in mL at a given elapsed time in **minutes**.
    ///
    /// The inverse of [`Channel::time_min_at_volume`], and interpolating for the
    /// same reason: the two are only linearly related while the flow rate holds.
    pub fn volume_ml_at_time_min(&self, time_min: f32) -> Option<f32> {
        let s = &self.samples;
        if s.is_empty() || !time_min.is_finite() {
            return None;
        }
        let time_s = time_min * 60.0;
        // Monotonic in time for the same reason it is monotonic in volume: the
        // record is a forward-running acquisition.
        let idx = s.partition_point(|p| p.time_s < time_s);
        if idx == 0 {
            let first = s[0];
            return nearly_equal(first.time_s, time_s).then_some(first.volume_ml);
        }
        if idx >= s.len() {
            let last = s[s.len() - 1];
            return nearly_equal(last.time_s, time_s).then_some(last.volume_ml);
        }
        let (a, b) = (s[idx - 1], s[idx]);
        let span = b.time_s - a.time_s;
        if span.abs() < f32::EPSILON {
            return Some(a.volume_ml);
        }
        let t = (time_s - a.time_s) / span;
        Some(a.volume_ml + t * (b.volume_ml - a.volume_ml))
    }

    /// Samples whose volume lies inside `[v0, v1]`, inclusive.
    pub fn samples_in_volume(&self, v0: f32, v1: f32) -> &[Sample] {
        let (v0, v1) = if v0 <= v1 { (v0, v1) } else { (v1, v0) };
        let start = self.samples.partition_point(|p| p.volume_ml < v0);
        let end = self.samples.partition_point(|p| p.volume_ml <= v1);
        &self.samples[start..end.max(start)]
    }
}

/// Zero-based plate coordinate. `A1` is `{row: 0, col: 0}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Well {
    pub row: u8,
    pub col: u8,
}

impl Well {
    pub const fn new(row: u8, col: u8) -> Self {
        Self { row, col }
    }

    /// `A1`, `H12`, … Rows beyond `Z` fall back to a numeric label rather than
    /// emitting nonsense characters.
    pub fn label(&self) -> String {
        if self.row < 26 {
            format!("{}{}", (b'A' + self.row) as char, self.col as u16 + 1)
        } else {
            format!("R{}C{}", self.row as u16 + 1, self.col as u16 + 1)
        }
    }
}

impl std::fmt::Display for Well {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fraction {
    /// 1-based tube number as reported by the collector.
    pub tube: u32,
    pub rack: u32,
    /// Resolved plate position; `None` when the rack type or pattern is unknown.
    pub well: Option<Well>,
    pub vol_start_ml: f32,
    pub vol_end_ml: f32,
    pub time_start_s: f32,
    pub time_end_s: f32,
    /// Nominal fraction size in mL as configured in the method.
    pub nominal_size_ml: Option<f32>,
    /// True when `vol_end_ml` was derived from `nominal_size_ml` because no
    /// `FractionDone` record was present. An inferred boundary must never be
    /// mistaken for a measured one — it decides which duplicate fraction trace
    /// wins reconciliation, and the plate marks such windows as provisional.
    pub end_estimated: bool,
    pub rack_type: String,
    pub pattern: String,
}

impl Fraction {
    pub fn volume_window(&self) -> (f32, f32) {
        if self.vol_start_ml <= self.vol_end_ml {
            (self.vol_start_ml, self.vol_end_ml)
        } else {
            (self.vol_end_ml, self.vol_start_ml)
        }
    }

    /// A fraction with a positive, finite volume window can carry a metric.
    pub fn has_usable_window(&self) -> bool {
        let (a, b) = self.volume_window();
        a.is_finite() && b.is_finite() && b > a
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEvent {
    pub time_s: f32,
    pub volume_ml: Option<f32>,
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunMeta {
    pub run_name: String,
    pub method_name: String,
    pub technique: String,
    pub started: Option<String>,
    pub ended: Option<String>,
    pub column: Option<String>,
    /// Column void volume, if the method records it (enables Kav-based SEC fits).
    pub v0_ml: Option<f32>,
    /// Column total volume, likewise.
    pub vt_ml: Option<f32>,
    /// UV flow-cell path length in cm, for Beer–Lambert concentration.
    pub path_length_cm: Option<f32>,
}

/// Non-fatal diagnostics raised while parsing. These surface in the UI instead of
/// being logged and forgotten, because most of them ("wavelengths assumed") change
/// how the numbers should be read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    pub scope: String,
    pub message: String,
}

impl Warning {
    pub fn new(scope: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            message: message.into(),
        }
    }
}

/// Where a [`Run`] came from — drives the "fractions unavailable" messaging that
/// `IMPLEMENTATION_PLAN.md` Phase 3 and 8 require for CSV-only runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceFormat {
    NgcAnalysis,
    NgcMethodruns,
    AnalysisCsv,
    /// Cytiva/GE ÄKTA, UNICORN 6/7 result container. Recognised and inventoried,
    /// but its curves are not decoded yet — see `parse::unicorn`.
    UnicornResult,
}

impl SourceFormat {
    /// Whether this format is capable of carrying fraction records at all.
    pub fn supports_fractions(self) -> bool {
        matches!(
            self,
            SourceFormat::NgcAnalysis | SourceFormat::NgcMethodruns | SourceFormat::UnicornResult
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            SourceFormat::NgcAnalysis => "NGC analysis archive",
            SourceFormat::NgcMethodruns => "NGC method-runs archive",
            SourceFormat::AnalysisCsv => "Analysis CSV",
            SourceFormat::UnicornResult => "UNICORN result (ÄKTA)",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Run {
    pub meta: RunMeta,
    pub source_format: SourceFormat,
    pub source_path: std::path::PathBuf,
    pub channels: Vec<Channel>,
    pub fractions: Vec<Fraction>,
    pub events: Vec<LogEvent>,
    pub warnings: Vec<Warning>,
}

impl Run {
    pub fn channel(&self, id: &ChannelId) -> Option<&Channel> {
        self.channels.iter().find(|c| &c.id == id)
    }

    /// Volume span of the whole run: the union across channels. Channels are
    /// sampled independently, so this is a union rather than any one channel's range.
    pub fn volume_range(&self) -> Option<(f32, f32)> {
        self.channels.iter().filter_map(|c| c.volume_range()).fold(
            None,
            |acc: Option<(f32, f32)>, (lo, hi)| match acc {
                None => Some((lo, hi)),
                Some((a, b)) => Some((a.min(lo), b.max(hi))),
            },
        )
    }

    /// The channel a fresh view should show first: UV 280 when present, else the
    /// UV channel closest to 280 nm, else the first UV channel, else the first
    /// channel with data (`IMPLEMENTATION_PLAN.md` Phase 2).
    pub fn hero_channel(&self) -> Option<&Channel> {
        let uv: Vec<&Channel> = self
            .channels
            .iter()
            .filter(|c| c.kind == ChannelKind::Uv && !c.is_empty())
            .collect();
        if !uv.is_empty() {
            let with_nm: Vec<&&Channel> = uv.iter().filter(|c| c.wavelength_nm.is_some()).collect();
            if let Some(best) = with_nm
                .iter()
                .min_by_key(|c| (c.wavelength_nm.unwrap() as i32 - 280).abs())
            {
                return Some(**best);
            }
            return Some(uv[0]);
        }
        self.channels.iter().find(|c| !c.is_empty())
    }

    /// Axis groups present in the run, in a stable order.
    pub fn axis_groups(&self) -> Vec<AxisGroup> {
        let mut groups: Vec<AxisGroup> = self
            .channels
            .iter()
            .filter(|c| !c.is_empty())
            .map(|c| c.kind.axis_group())
            .collect();
        groups.sort();
        groups.dedup();
        groups
    }

    /// Fractions whose volume window overlaps `[v0, v1]`.
    pub fn fractions_in_volume(&self, v0: f32, v1: f32) -> Vec<&Fraction> {
        let (v0, v1) = if v0 <= v1 { (v0, v1) } else { (v1, v0) };
        self.fractions
            .iter()
            .filter(|f| {
                let (a, b) = f.volume_window();
                b >= v0 && a <= v1
            })
            .collect()
    }

    /// Fractions that actually *received* part of the eluate between `v0` and `v1`.
    ///
    /// This is deliberately stricter than [`Self::fractions_in_volume`], which
    /// uses closed intervals so that a cursor parked on a boundary still lights
    /// up the neighbouring fraction. Collection tiles the elution without gaps —
    /// fraction *n* ends exactly where *n+1* begins — so under a closed test a
    /// peak integrated over `[10, 12]` would report the fractions on both sides
    /// of it as well, even though none of that peak went into either tube. Here
    /// the intersection must have positive width: touching a boundary does not
    /// count.
    ///
    /// Fractions without a usable window are skipped, because a zero-width or
    /// non-finite window cannot be said to have caught any part of the peak.
    ///
    /// The result is in **collection order** — ascending start volume, tube
    /// number as the tie-break — which is the order a user reads off the
    /// chromatogram from left to right. On a serpentine rack that is not the
    /// same as alphabetical well order.
    pub fn fractions_overlapping(&self, v0: f32, v1: f32) -> Vec<&Fraction> {
        let (v0, v1) = if v0 <= v1 { (v0, v1) } else { (v1, v0) };
        let mut hits: Vec<&Fraction> = self
            .fractions
            .iter()
            .filter(|f| f.has_usable_window())
            .filter(|f| {
                let (a, b) = f.volume_window();
                b > v0 && a < v1
            })
            .collect();
        hits.sort_by(|x, y| {
            x.volume_window()
                .0
                .total_cmp(&y.volume_window().0)
                .then(x.tube.cmp(&y.tube))
        });
        hits
    }

    /// Plate positions of [`Self::fractions_overlapping`], in collection order.
    ///
    /// A fraction whose rack type or collection pattern could not be resolved has
    /// no well and is dropped, so this list can be shorter than the fraction list
    /// it came from — compare the two lengths before telling a user that nothing
    /// was collected. Repeated wells are collapsed: the two `Trace_Fractions_*`
    /// streams can describe the same tube twice.
    pub fn wells_in_volume(&self, v0: f32, v1: f32) -> Vec<Well> {
        let mut wells: Vec<Well> = Vec::new();
        for well in self
            .fractions_overlapping(v0, v1)
            .into_iter()
            .filter_map(|f| f.well)
        {
            if !wells.contains(&well) {
                wells.push(well);
            }
        }
        wells
    }

    /// Default sidecar location: `<run>.elusive.json` beside the source file.
    pub fn sidecar_path(&self) -> std::path::PathBuf {
        crate::sidecar::sidecar_path_for(&self.source_path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BaselineMode {
    /// Baseline y = 0.
    DropToZero,
    /// Straight line between the signal at the window endpoints.
    LinearEndpoints,
    /// Straight line between two user-chosen valley volumes.
    ValleyToValley { left_ml: f32, right_ml: f32 },
}

impl BaselineMode {
    pub fn label(self) -> &'static str {
        match self {
            BaselineMode::DropToZero => "Drop to zero",
            BaselineMode::LinearEndpoints => "Linear (endpoints)",
            BaselineMode::ValleyToValley { .. } => "Valley to valley",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeakId(pub u32);

impl std::fmt::Display for PeakId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "P{}", self.0)
    }
}

/// Output of a manual integration over a volume window.
///
/// All quantities are in *display* units — `area` is signal-display-unit × mL
/// (e.g. mAU·mL), matching what a user would read off the plot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeakResult {
    pub id: PeakId,
    pub channel_id: ChannelId,
    pub v_start_ml: f32,
    pub v_end_ml: f32,
    pub baseline: BaselineMode,
    pub area: f64,
    pub height: f64,
    pub apex_volume_ml: f32,
    /// Full width at half maximum, in mL. `None` when the peak does not fall back
    /// to half height inside the window on both sides.
    pub fwhm_ml: Option<f32>,
    /// Filled in by [`crate::calibration`] once a curve is applied.
    pub estimated_mw_kda: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wells::{well_for_tube, CollectionPattern, RackGeometry};

    /// One serpentine HEP96 fraction, `size` mL wide, starting at `start`.
    fn fraction(tube: u32, start: f32, size: f32) -> Fraction {
        Fraction {
            tube,
            rack: 1,
            well: well_for_tube(tube, RackGeometry::HEP96, CollectionPattern::Serpentine),
            vol_start_ml: start,
            vol_end_ml: start + size,
            time_start_s: 0.0,
            time_end_s: 0.0,
            nominal_size_ml: Some(size),
            end_estimated: false,
            rack_type: "HEP96".into(),
            pattern: "Serpentine".into(),
        }
    }

    fn run_with(fractions: Vec<Fraction>) -> Run {
        Run {
            meta: RunMeta::default(),
            source_format: SourceFormat::NgcAnalysis,
            source_path: std::path::PathBuf::from("run.ngcAnalysis"),
            channels: Vec::new(),
            fractions,
            events: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Tubes 1..=n tiling the elution from 10 mL in 1 mL steps: tube 1 is
    /// `[10, 11)`, tube 2 `[11, 12)`, and so on.
    fn tiled_run(n: u32) -> Run {
        run_with((1..=n).map(|t| fraction(t, 9.0 + t as f32, 1.0)).collect())
    }

    fn labels(run: &Run, v0: f32, v1: f32) -> Vec<String> {
        run.wells_in_volume(v0, v1)
            .iter()
            .map(Well::label)
            .collect()
    }

    #[test]
    fn a_peak_inside_one_fraction_reports_only_that_fraction() {
        assert_eq!(labels(&tiled_run(6), 12.2, 12.8), ["A3"]);
    }

    #[test]
    fn a_peak_spanning_four_fractions_reports_all_four_in_collection_order() {
        assert_eq!(labels(&tiled_run(8), 12.5, 15.5), ["A3", "A4", "A5", "A6"]);
    }

    #[test]
    fn a_peak_eluting_before_collection_started_reports_nothing() {
        assert!(tiled_run(6).wells_in_volume(2.0, 5.0).is_empty());
    }

    #[test]
    fn a_run_with_no_fractions_reports_nothing() {
        assert!(run_with(Vec::new()).wells_in_volume(0.0, 100.0).is_empty());
    }

    #[test]
    fn a_fraction_with_an_unusable_window_is_never_reported() {
        // A zero-width window and a NaN window both fail `has_usable_window`, and
        // neither can honestly be said to hold part of the peak.
        let collapsed = fraction(1, 12.0, 0.0);
        let mut nonfinite = fraction(2, 12.0, 1.0);
        nonfinite.vol_end_ml = f32::NAN;

        let run = run_with(vec![collapsed, nonfinite]);
        assert!(run.fractions_overlapping(10.0, 14.0).is_empty());
    }

    #[test]
    fn touching_a_fraction_boundary_does_not_count_as_overlap() {
        // Fractions tile without gaps, so a peak integrated over exactly one
        // fraction's window must not drag in its two neighbours.
        assert_eq!(labels(&tiled_run(6), 12.0, 13.0), ["A3"]);
        // And a zero-width window touches nothing at all.
        assert!(tiled_run(6).wells_in_volume(12.0, 12.0).is_empty());
    }

    #[test]
    fn the_permissive_hover_test_still_includes_boundary_neighbours() {
        // `fractions_in_volume` keeps its closed-interval behaviour: the plate
        // highlight wants the neighbours, the peak report does not.
        let run = tiled_run(6);
        assert_eq!(run.fractions_in_volume(12.0, 13.0).len(), 3);
        assert_eq!(run.fractions_overlapping(12.0, 13.0).len(), 1);
    }

    #[test]
    fn a_reversed_window_is_normalised_rather_than_returning_nothing() {
        assert_eq!(labels(&tiled_run(8), 15.5, 12.5), ["A3", "A4", "A5", "A6"]);
    }

    #[test]
    fn wells_come_back_in_collection_order_even_when_records_are_shuffled() {
        // Serpentine row B runs B12 → B1, so collection order and alphabetical
        // well order disagree; the records themselves arrive out of order here.
        let run = run_with(vec![
            fraction(14, 22.0, 1.0),
            fraction(12, 20.0, 1.0),
            fraction(13, 21.0, 1.0),
        ]);
        assert_eq!(labels(&run, 20.5, 22.5), ["A12", "B12", "B11"]);
    }

    #[test]
    fn a_fraction_without_a_resolved_well_drops_out_of_the_well_list() {
        let mut unplaced = fraction(3, 12.0, 1.0);
        unplaced.well = None;
        let run = run_with(vec![unplaced, fraction(4, 13.0, 1.0)]);
        // The fraction is still counted; only its plate position is missing.
        assert_eq!(run.fractions_overlapping(12.0, 14.0).len(), 2);
        assert_eq!(labels(&run, 12.0, 14.0), ["A4"]);
    }

    #[test]
    fn a_well_described_twice_is_listed_once() {
        // The two `Trace_Fractions_*` streams can both describe the same tube.
        let run = run_with(vec![fraction(3, 12.0, 1.0), fraction(3, 12.0, 1.0)]);
        assert_eq!(labels(&run, 12.0, 13.0), ["A3"]);
    }

    // --- volume ↔ time conversion ----------------------------------------

    /// A run whose flow rate halves half way through: 1 mL/min for the first two
    /// minutes, then 0.5 mL/min. Anything assuming a constant rate would place the
    /// last point at 4 mL rather than 3 mL.
    fn variable_flow() -> Channel {
        let mut ch = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        ch.samples = vec![
            Sample::new(0.0, 0.0, 0.0),
            Sample::new(60.0, 1.0, 1.0),
            Sample::new(120.0, 2.0, 2.0),
            Sample::new(240.0, 3.0, 3.0),
        ];
        ch
    }

    #[test]
    fn time_at_volume_interpolates_the_actual_sample_pairs() {
        let ch = variable_flow();
        assert_eq!(ch.time_min_at_volume(0.5), Some(0.5));
        // In the slow segment 0.5 mL costs a full minute, not half of one.
        let t = ch
            .time_min_at_volume(2.5)
            .expect("inside the sampled range");
        assert!((t - 3.0).abs() < 1e-5, "t = {t}");
    }

    #[test]
    fn volume_at_time_interpolates_the_actual_sample_pairs() {
        let ch = variable_flow();
        assert_eq!(ch.volume_ml_at_time_min(1.5), Some(1.5));
        let v = ch
            .volume_ml_at_time_min(3.0)
            .expect("inside the sampled range");
        assert!((v - 2.5).abs() < 1e-5, "v = {v}");
    }

    #[test]
    fn the_conversions_round_trip_including_at_the_endpoints() {
        let ch = variable_flow();
        for v in [0.0f32, 0.25, 1.0, 2.75, 3.0] {
            let t = ch.time_min_at_volume(v).expect("inside the sampled range");
            let back = ch
                .volume_ml_at_time_min(t)
                .expect("a time derived from a sampled volume is in range");
            assert!((back - v).abs() < 1e-4, "v = {v}, back = {back}");
        }
    }

    #[test]
    fn a_long_run_still_resolves_its_last_sample_after_a_unit_conversion() {
        // Regression guard for the endpoint tolerance: 4551 s is a realistic run
        // length and its f32 ulp is far larger than `f32::EPSILON`, so an absolute
        // epsilon test would report the final sample as out of range.
        let mut ch = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        ch.samples = vec![Sample::new(0.0, 0.0, 0.0), Sample::new(4551.0, 37.9, 1.0)];
        let t = ch
            .time_min_at_volume(37.9)
            .expect("the last sample is in range");
        assert!(ch.volume_ml_at_time_min(t).is_some());
    }

    #[test]
    fn out_of_range_lookups_report_no_data_rather_than_clamping() {
        let ch = variable_flow();
        assert_eq!(ch.time_min_at_volume(-0.1), None);
        assert_eq!(ch.time_min_at_volume(3.1), None);
        assert_eq!(ch.time_min_at_volume(f32::NAN), None);
        assert_eq!(ch.volume_ml_at_time_min(-0.1), None);
        assert_eq!(ch.volume_ml_at_time_min(5.0), None);
        assert_eq!(ch.volume_ml_at_time_min(f32::NAN), None);
    }

    #[test]
    fn a_single_sample_channel_maps_only_that_sample() {
        let mut ch = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        ch.samples = vec![Sample::new(90.0, 1.5, 0.2)];
        assert_eq!(ch.time_min_at_volume(1.5), Some(1.5));
        assert_eq!(ch.volume_ml_at_time_min(1.5), Some(1.5));
        assert_eq!(ch.time_min_at_volume(1.6), None);
        assert_eq!(ch.volume_ml_at_time_min(0.0), None);
    }

    #[test]
    fn an_empty_channel_has_nothing_to_map() {
        let ch = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        assert_eq!(ch.time_min_at_volume(1.0), None);
        assert_eq!(ch.volume_ml_at_time_min(1.0), None);
    }
}

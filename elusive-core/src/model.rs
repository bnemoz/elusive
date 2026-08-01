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
}

impl SourceFormat {
    /// Whether this format is capable of carrying fraction records at all.
    pub fn supports_fractions(self) -> bool {
        matches!(
            self,
            SourceFormat::NgcAnalysis | SourceFormat::NgcMethodruns
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            SourceFormat::NgcAnalysis => "NGC analysis archive",
            SourceFormat::NgcMethodruns => "NGC method-runs archive",
            SourceFormat::AnalysisCsv => "Analysis CSV",
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

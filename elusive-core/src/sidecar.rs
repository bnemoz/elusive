//! `<run>.elusive.json` — where the user's analysis lives.
//!
//! The source archive is **never** modified (`design.md` §12). Everything a user
//! adds — integrations, excluded regions, calibrations, notes — is written to a
//! human-readable JSON file beside it, so the annotations stay portable and
//! outlive this application.
//!
//! # Schema (version 1)
//!
//! ```json
//! {
//!   "version": 1,
//!   "source": { "file_name": "run.ngcAnalysis", "run_name": "SEC 2026-07-31", "channel_ids": ["MWave2"] },
//!   "peaks": [ { "id": 1, "channel_id": "MWave2", "v_start_ml": 12.0, "v_end_ml": 14.0,
//!                "baseline": { "LinearEndpoints": null }, "area": 1234.5, "height": 890.0,
//!                "apex_volume_ml": 13.0, "fwhm_ml": 0.8, "estimated_mw_kda": null } ],
//!   "excluded_regions": [ { "v_start_ml": 0.0, "v_end_ml": 2.0, "note": "void" } ],
//!   "calibrations": [ { "name": "Bio-Rad GFS", "basis": "ElutionVolume", "slope": -0.2,
//!                       "intercept": 3.0, "r_squared": 0.999, "points": [] } ],
//!   "annotations": [ { "volume_ml": 13.0, "text": "monomer" } ],
//!   "view": { "visible_channels": ["MWave2"], "dark_mode": true,
//!             "plate_channel": "MWave2", "plate_metric": "IntegratedArea",
//!             "y_scale_mode": "custom", "channel_y_ranges": { "MWave2": [0.0, 500.0] } }
//! }
//! ```
//!
//! Unknown *future* versions are refused rather than partially read: silently
//! dropping a field the user relied on is worse than an error message.

use crate::error::{Error, Result};
use crate::integrate::PlateMetric;
use crate::model::{Color, PeakResult, Run};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Schema version written by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// Identity of the run a sidecar belongs to.
///
/// Deliberately not an absolute path: the whole point of this tool is that a run
/// travels on a USB stick, so the sidecar must still match after the folder moves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub file_name: String,
    pub run_name: String,
    /// Channel ids present when the sidecar was written, used to warn about a
    /// mismatch instead of silently dropping peaks that reference a missing channel.
    #[serde(default)]
    pub channel_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExcludedRegion {
    pub v_start_ml: f32,
    pub v_end_ml: f32,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub volume_ml: f32,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedCalibration {
    pub name: String,
    #[serde(flatten)]
    pub calibration: crate::calibration::Calibration,
}

/// Display state worth restoring. Everything here is optional by design: losing
/// a view preference must never make a sidecar unreadable.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    #[serde(default)]
    pub visible_channels: Vec<String>,
    #[serde(default)]
    pub dark_mode: Option<bool>,
    #[serde(default)]
    pub plate_channel: Option<String>,
    #[serde(default)]
    pub plate_metric: Option<PlateMetric>,
    #[serde(default)]
    pub show_fractions: Option<bool>,
    #[serde(default)]
    pub plate_uniform_ramp: Option<bool>,
    /// Navigation rail reduced to icons only.
    #[serde(default)]
    pub nav_collapsed: Option<bool>,
    /// Overview card order, as the app's stable panel-id strings.
    ///
    /// Strings rather than an enum on purpose: the core has no opinion about
    /// which cards the UI shows, and a sidecar written by a build with more
    /// panels than this one must still load. The app is responsible for dropping
    /// ids it does not know and appending panels the list does not mention.
    #[serde(default)]
    pub overview_order: Option<Vec<String>>,
    /// Trace colours the user chose by hand, keyed by channel id.
    ///
    /// Absent in sidecars written before this field existed, hence the `Option`:
    /// "this build never had the feature" and "the user cleared every override"
    /// are different facts, and only the second should be allowed to wipe a
    /// colour a future merge might want to keep.
    #[serde(default)]
    pub channel_colors: Option<BTreeMap<String, Color>>,
    /// Which quantity the chromatogram's x-axis was showing, as a stable key
    /// (`"volume"` / `"time"`).
    ///
    /// A string rather than an enum on purpose: this is a pure UI preference that
    /// core has no other use for, and an unrecognised key can be ignored by an
    /// older or newer build without failing the whole sidecar.
    #[serde(default)]
    pub x_axis: Option<String>,
    /// How the chromatogram scales its y-axes, as the app's stable key
    /// (`auto-all`, `auto-each`, `custom`).
    ///
    /// A string rather than an enum on purpose: which y-scale modes exist is a
    /// property of the viewer, not of the run, and `elusive-core` has no use for
    /// the distinction. Keeping it opaque here means the UI can add a mode
    /// without a core release, and an older build reading a newer sidecar can
    /// report the unknown key instead of failing to deserialise the file.
    #[serde(default)]
    pub y_scale_mode: Option<String>,
    /// Per-channel y range in *display* units, keyed by channel id. Only
    /// meaningful in the `custom` mode.
    ///
    /// `Option` for the same reason as `channel_colors`: "this build never had
    /// the feature" and "the user cleared every range" are different facts.
    #[serde(default)]
    pub channel_y_ranges: Option<BTreeMap<String, (f32, f32)>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sidecar {
    pub version: u32,
    pub source: SourceIdentity,
    #[serde(default)]
    pub peaks: Vec<PeakResult>,
    #[serde(default)]
    pub excluded_regions: Vec<ExcludedRegion>,
    #[serde(default)]
    pub calibrations: Vec<NamedCalibration>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub view: ViewState,
}

impl Default for Sidecar {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            source: SourceIdentity::default(),
            peaks: Vec::new(),
            excluded_regions: Vec::new(),
            calibrations: Vec::new(),
            annotations: Vec::new(),
            view: ViewState::default(),
        }
    }
}

impl Sidecar {
    /// A fresh sidecar stamped with the run's identity.
    pub fn for_run(run: &Run) -> Self {
        Sidecar {
            source: SourceIdentity {
                file_name: run
                    .source_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string(),
                run_name: run.meta.run_name.clone(),
                channel_ids: run.channels.iter().map(|c| c.id.0.clone()).collect(),
            },
            ..Sidecar::default()
        }
    }

    /// Peaks whose channel is missing from `run`. The caller shows these rather
    /// than dropping them, so a user reopening the wrong file finds out why their
    /// peaks vanished.
    pub fn orphaned_peaks(&self, run: &Run) -> Vec<&PeakResult> {
        self.peaks
            .iter()
            .filter(|p| !run.channels.iter().any(|c| c.id == p.channel_id))
            .collect()
    }

    /// Whether this sidecar names the same run file.
    pub fn matches(&self, run: &Run) -> bool {
        let name = run
            .source_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        self.source.file_name == name
            && (self.source.run_name.is_empty() || self.source.run_name == run.meta.run_name)
    }
}

/// `run.ngcAnalysis` → `run.ngcAnalysis.elusive.json`.
///
/// The full original name is kept (rather than replacing the extension) so an
/// `.ngcAnalysis` and a `.csv` export of the same run get separate sidecars.
pub fn sidecar_path_for(source: &Path) -> PathBuf {
    let mut name = source
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "run".to_string());
    name.push_str(".elusive.json");
    source.with_file_name(name)
}

/// Read a sidecar, refusing schema versions this build does not understand.
pub fn load(path: impl AsRef<Path>) -> Result<Sidecar> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    from_json(&text)
}

/// Parse sidecar JSON. Split out from [`load`] so the version gate is testable
/// without touching the filesystem.
pub fn from_json(text: &str) -> Result<Sidecar> {
    // Read the version before the body, so a future schema fails with a clear
    // message instead of a confusing field-level deserialisation error.
    #[derive(Deserialize)]
    struct VersionProbe {
        version: u32,
    }
    let probe: VersionProbe = serde_json::from_str(text).map_err(|e| Error::Sidecar {
        detail: format!("could not read the schema version: {e}"),
    })?;
    if probe.version > SCHEMA_VERSION {
        return Err(Error::SidecarVersion {
            found: probe.version,
            supported: SCHEMA_VERSION,
        });
    }

    serde_json::from_str(text).map_err(|e| Error::Sidecar {
        detail: format!("malformed sidecar: {e}"),
    })
}

/// Write a sidecar as pretty JSON.
///
/// Writes to a temporary file and renames, so an interrupted save cannot leave a
/// half-written sidecar where a complete one used to be.
pub fn save(path: impl AsRef<Path>, sidecar: &Sidecar) -> Result<()> {
    let path = path.as_ref();
    let json = serde_json::to_string_pretty(sidecar).map_err(|e| Error::Sidecar {
        detail: format!("could not serialise sidecar: {e}"),
    })?;

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json.as_bytes()).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))?;
    Ok(())
}

/// CSV export of the peak table.
/// Schema fixed by `IMPLEMENTATION_PLAN.md` Phase 5.
///
/// `fractions` lists every well the peak's window overlaps, spelled out rather
/// than ranged so a script does not have to expand `D5–D8` itself. It is empty
/// both when no fraction overlaps and when the source format carries no
/// fractions at all — the two are distinguished in the UI, not here.
pub fn peaks_to_csv(run: &Run, peaks: &[PeakResult]) -> String {
    let mut out = String::from(
        "peak_id,channel_id,v_start_ml,v_end_ml,apex_volume_ml,area,height,fwhm,estimated_mw,fractions\n",
    );
    for p in peaks {
        let fractions =
            crate::wells::join_well_labels(&run.wells_in_volume(p.v_start_ml, p.v_end_ml));
        out.push_str(&format!(
            "{},{},{:.4},{:.4},{:.4},{:.6},{:.6},{},{},{}\n",
            p.id.0,
            csv_escape(p.channel_id.as_str()),
            p.v_start_ml,
            p.v_end_ml,
            p.apex_volume_ml,
            p.area,
            p.height,
            p.fwhm_ml.map(|v| format!("{v:.4}")).unwrap_or_default(),
            p.estimated_mw_kda
                .map(|v| format!("{v:.3}"))
                .unwrap_or_default(),
            csv_escape(&fractions),
        ));
    }
    out
}

/// Shown for a value the run does not carry. An empty Markdown cell reads as an
/// oversight; an em dash reads as "not measured".
const EM_DASH: &str = "—";

/// The peak table as a GitHub-flavoured Markdown table.
///
/// Same columns and same numeric precision as [`peaks_to_csv`]: this is the CSV
/// rendered for a human reader — an electronic notebook entry or an issue comment
/// — so the two must never disagree about a value. The one deliberate difference
/// is the peak column, which carries the `P1` label the plot annotates rather than
/// the bare integer a script would parse. Numeric columns get the `---:` alignment
/// marker, which is how Markdown expresses rule #4.
pub fn peaks_to_markdown(peaks: &[PeakResult]) -> String {
    let mut out = String::from(
        "| Peak | Channel | Start (mL) | End (mL) | Ve (mL) | Area | Height | FWHM (mL) | \
         Est. MW (kDa) |\n| ---: | :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for p in peaks {
        out.push_str(&format!(
            "| {} | {} | {:.4} | {:.4} | {:.4} | {:.6} | {:.6} | {} | {} |\n",
            p.id,
            md_escape(p.channel_id.as_str()),
            p.v_start_ml,
            p.v_end_ml,
            p.apex_volume_ml,
            p.area,
            p.height,
            p.fwhm_ml
                .map(|v| format!("{v:.4}"))
                .unwrap_or_else(|| EM_DASH.to_string()),
            p.estimated_mw_kda
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| EM_DASH.to_string()),
        ));
    }
    out
}

/// One row per collected well.
/// Schema fixed by `IMPLEMENTATION_PLAN.md` Phase 5.
pub fn wells_to_csv(rows: &[(crate::model::Well, String, PlateMetric, Option<f64>)]) -> String {
    let mut out = String::from("well_id,row,col,channel_id,metric,value\n");
    for (well, channel_id, metric, value) in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            well.label(),
            well.row + 1,
            well.col + 1,
            csv_escape(channel_id),
            metric.label(),
            value.map(|v| format!("{v:.6}")).unwrap_or_default(),
        ));
    }
    out
}

/// Keep a channel name inside its Markdown cell.
///
/// A bare `|` would split the row into extra columns and silently shift every
/// value one cell to the right; newlines would end the row outright.
fn md_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\n', '\r'], " ")
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::{fit, CalibrationPoint, FitBasis};
    use crate::model::{BaselineMode, ChannelId, Fraction, PeakId, SourceFormat, Well};

    fn sample_run(source_format: SourceFormat, fractions: Vec<Fraction>) -> Run {
        Run {
            meta: crate::model::RunMeta::default(),
            source_format,
            source_path: PathBuf::from("run.ngcAnalysis"),
            channels: Vec::new(),
            fractions,
            events: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// A fraction occupying `[start, start + 1)` in the given well.
    fn sample_fraction(tube: u32, start: f32, well: Well) -> Fraction {
        Fraction {
            tube,
            rack: 1,
            well: Some(well),
            vol_start_ml: start,
            vol_end_ml: start + 1.0,
            time_start_s: 0.0,
            time_end_s: 0.0,
            nominal_size_ml: Some(1.0),
            end_estimated: false,
            rack_type: "HEP96".into(),
            pattern: "Serpentine".into(),
        }
    }

    fn fractionless_run() -> Run {
        sample_run(SourceFormat::NgcAnalysis, Vec::new())
    }

    fn sample_peak() -> PeakResult {
        PeakResult {
            id: PeakId(1),
            channel_id: ChannelId::from("MWave2"),
            v_start_ml: 12.0,
            v_end_ml: 14.0,
            baseline: BaselineMode::LinearEndpoints,
            area: 1234.5,
            height: 890.0,
            apex_volume_ml: 13.0,
            fwhm_ml: Some(0.8),
            estimated_mw_kda: None,
        }
    }

    #[test]
    fn sidecar_round_trips_through_json() {
        let mut s = Sidecar {
            source: SourceIdentity {
                file_name: "run.ngcAnalysis".into(),
                run_name: "SEC test".into(),
                channel_ids: vec!["MWave2".into()],
            },
            ..Sidecar::default()
        };
        s.peaks.push(sample_peak());
        s.excluded_regions.push(ExcludedRegion {
            v_start_ml: 0.0,
            v_end_ml: 2.0,
            note: "void".into(),
        });
        s.annotations.push(Annotation {
            volume_ml: 13.0,
            text: "monomer".into(),
        });
        s.view.dark_mode = Some(true);
        s.view.plate_metric = Some(PlateMetric::MaxValue);

        let json = serde_json::to_string_pretty(&s).unwrap();
        let back = from_json(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn user_trace_colours_survive_a_round_trip_and_are_optional_on_the_wire() {
        let mut s = Sidecar::default();
        s.view.channel_colors = Some(BTreeMap::from([(
            "MWave2".to_string(),
            Color::new(0xC4, 0x77, 0x3D, 0xFF),
        )]));
        let back = from_json(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.view.channel_colors, s.view.channel_colors);

        // A sidecar written before the field existed must still load; refusing it
        // would strand every analysis saved by an earlier build.
        let legacy = r#"{
            "version": 1,
            "source": { "file_name": "run.ngcAnalysis", "run_name": "SEC", "channel_ids": [] },
            "view": { "visible_channels": ["MWave2"], "dark_mode": true }
        }"#;
        let old = from_json(legacy).expect("a pre-colour sidecar should still parse");
        assert_eq!(old.view.channel_colors, None);
        assert_eq!(old.view.visible_channels, vec!["MWave2".to_string()]);
    }

    #[test]
    fn valley_to_valley_baselines_survive_a_round_trip() {
        let mut s = Sidecar::default();
        let mut p = sample_peak();
        p.baseline = BaselineMode::ValleyToValley {
            left_ml: 11.5,
            right_ml: 14.5,
        };
        s.peaks.push(p);
        let back = from_json(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.peaks[0].baseline, s.peaks[0].baseline);
    }

    #[test]
    fn calibrations_survive_a_round_trip() {
        let mut s = Sidecar::default();
        let cal = fit(
            &[
                CalibrationPoint {
                    mw_kda: 670.0,
                    ve_ml: 8.0,
                },
                CalibrationPoint {
                    mw_kda: 44.0,
                    ve_ml: 14.0,
                },
            ],
            FitBasis::Kav {
                v0_ml: 7.0,
                vt_ml: 24.0,
            },
        )
        .unwrap();
        s.calibrations.push(NamedCalibration {
            name: "Bio-Rad GFS".into(),
            calibration: cal.clone(),
        });

        let back = from_json(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.calibrations[0].calibration, cal);
    }

    #[test]
    fn a_newer_schema_version_is_refused_with_both_numbers() {
        let json = r#"{"version": 99, "source": {"file_name":"x","run_name":"y"}}"#;
        match from_json(json) {
            Err(Error::SidecarVersion { found, supported }) => {
                assert_eq!(found, 99);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected a version error, got {other:?}"),
        }
    }

    #[test]
    fn a_minimal_sidecar_loads_with_defaults_for_every_optional_section() {
        let json = r#"{"version": 1, "source": {"file_name":"run.ngcAnalysis","run_name":"r"}}"#;
        let s = from_json(json).unwrap();
        assert!(s.peaks.is_empty());
        assert!(s.calibrations.is_empty());
        assert_eq!(s.view, ViewState::default());
    }

    #[test]
    fn a_sidecar_written_before_overview_order_existed_still_loads() {
        let json = r#"{"version": 1, "source": {"file_name":"run.ngcAnalysis","run_name":"r"},
            "view": {"visible_channels":["MWave2"],"show_fractions":true}}"#;
        let s = from_json(json).unwrap();
        assert_eq!(s.view.visible_channels, vec!["MWave2".to_string()]);
        assert!(s.view.overview_order.is_none());
    }

    #[test]
    fn y_scale_state_survives_a_round_trip_and_is_optional_on_the_wire() {
        let mut s = Sidecar::default();
        s.view.y_scale_mode = Some("custom".into());
        s.view.channel_y_ranges =
            Some(BTreeMap::from([("MWave2".to_string(), (0.0f32, 500.0f32))]));
        let back = from_json(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.view.y_scale_mode, s.view.y_scale_mode);
        assert_eq!(back.view.channel_y_ranges, s.view.channel_y_ranges);

        // Absent in everything written before the feature existed; the core must
        // read those files unchanged rather than refusing them.
        let legacy = r#"{"version": 1, "source": {"file_name":"run.ngcAnalysis","run_name":"r"},
            "view": {"visible_channels":["MWave2"]}}"#;
        let old = from_json(legacy).expect("a pre-y-scale sidecar should still parse");
        assert_eq!(old.view.y_scale_mode, None);
        assert_eq!(old.view.channel_y_ranges, None);
    }

    #[test]
    fn malformed_json_reports_a_sidecar_error_not_a_panic() {
        assert!(matches!(from_json("{not json"), Err(Error::Sidecar { .. })));
        assert!(matches!(from_json("{}"), Err(Error::Sidecar { .. })));
    }

    #[test]
    fn sidecar_identity_requires_matching_file_name_and_run_name() {
        let mut run = Run {
            meta: crate::model::RunMeta {
                run_name: "run-a".into(),
                ..crate::model::RunMeta::default()
            },
            source_format: crate::model::SourceFormat::NgcAnalysis,
            source_path: PathBuf::from("run-a.ngcAnalysis"),
            channels: Vec::new(),
            fractions: Vec::new(),
            events: Vec::new(),
            warnings: Vec::new(),
        };

        let sidecar = Sidecar::for_run(&run);
        assert!(sidecar.matches(&run));

        run.meta.run_name = "run-b".into();
        assert!(!sidecar.matches(&run));
    }

    #[test]
    fn sidecar_path_keeps_the_original_extension() {
        assert_eq!(
            sidecar_path_for(Path::new("/data/run.ngcAnalysis")),
            PathBuf::from("/data/run.ngcAnalysis.elusive.json")
        );
        // A CSV export of the same run gets its own sidecar.
        assert_eq!(
            sidecar_path_for(Path::new("/data/run.csv")),
            PathBuf::from("/data/run.csv.elusive.json")
        );
    }

    #[test]
    fn peak_csv_matches_the_agreed_schema() {
        let csv = peaks_to_csv(&fractionless_run(), &[sample_peak()]);
        let mut lines = csv.lines();
        assert_eq!(
            lines.next().unwrap(),
            "peak_id,channel_id,v_start_ml,v_end_ml,apex_volume_ml,area,height,fwhm,estimated_mw,\
             fractions"
        );
        let row = lines.next().unwrap();
        assert!(row.starts_with("1,MWave2,12.0000,14.0000,13.0000,"));
        // An unset MW must be an empty cell, never a zero that reads as a result;
        // with no fractions collected the last cell is empty too.
        assert!(row.ends_with(",,"), "row = {row}");
    }

    #[test]
    fn the_fraction_cell_is_quoted_because_it_holds_a_comma_separated_list() {
        let run = sample_run(
            SourceFormat::NgcAnalysis,
            vec![
                sample_fraction(1, 12.0, Well::new(3, 4)),
                sample_fraction(2, 13.0, Well::new(3, 5)),
            ],
        );
        let csv = peaks_to_csv(&run, &[sample_peak()]);
        let row = csv.lines().nth(1).expect("one data row");
        assert!(row.ends_with(",\"D5, D6\""), "row = {row}");

        // Round trip: a naive split on commas outside quotes must recover the
        // field intact, commas and all.
        assert_eq!(unquote_last_field(row), "D5, D6");
    }

    #[test]
    fn a_csv_import_exports_an_empty_fraction_cell_rather_than_a_guess() {
        // `SourceFormat::AnalysisCsv` cannot carry fractions at all, so there is
        // nothing honest to write here.
        let run = sample_run(SourceFormat::AnalysisCsv, Vec::new());
        assert!(!run.source_format.supports_fractions());
        let row = peaks_to_csv(&run, &[sample_peak()])
            .lines()
            .nth(1)
            .map(str::to_owned)
            .expect("one data row");
        assert!(row.ends_with(",,"), "row = {row}");
    }

    /// Minimal RFC-4180 reader for the final field of a row, used to prove the
    /// quoting survives a parse rather than merely looking right.
    fn unquote_last_field(row: &str) -> String {
        let mut fields = vec![String::new()];
        let mut in_quotes = false;
        let mut chars = row.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '"' if in_quotes && chars.peek() == Some(&'"') => {
                    chars.next();
                    if let Some(last) = fields.last_mut() {
                        last.push('"');
                    }
                }
                '"' => in_quotes = !in_quotes,
                ',' if !in_quotes => fields.push(String::new()),
                _ => {
                    if let Some(last) = fields.last_mut() {
                        last.push(ch);
                    }
                }
            }
        }
        fields.pop().unwrap_or_default()
    }

    #[test]
    fn well_csv_matches_the_agreed_schema() {
        let csv = wells_to_csv(&[(
            Well::new(1, 11),
            "MWave2".into(),
            PlateMetric::IntegratedArea,
            Some(12.5),
        )]);
        let mut lines = csv.lines();
        assert_eq!(
            lines.next().unwrap(),
            "well_id,row,col,channel_id,metric,value"
        );
        assert_eq!(
            lines.next().unwrap(),
            "B12,2,12,MWave2,Integrated area,12.500000"
        );
    }

    #[test]
    fn an_uncollected_well_exports_an_empty_value_not_a_zero() {
        let csv = wells_to_csv(&[(
            Well::new(0, 0),
            "MWave2".into(),
            PlateMetric::MaxValue,
            None,
        )]);
        assert!(csv.lines().nth(1).unwrap().ends_with(','));
    }

    #[test]
    fn peak_markdown_has_a_header_an_alignment_row_and_one_row_per_peak() {
        let md = peaks_to_markdown(&[sample_peak()]);
        let lines: Vec<&str> = md.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "| Peak | Channel | Start (mL) | End (mL) | Ve (mL) | Area | Height | FWHM (mL) | \
             Est. MW (kDa) |"
        );
        // Numeric columns are right-aligned; only the channel name is not.
        assert_eq!(
            lines[1],
            "| ---: | :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
        );
        assert_eq!(
            lines[2],
            "| P1 | MWave2 | 12.0000 | 14.0000 | 13.0000 | 1234.500000 | 890.000000 | 0.8000 | — |"
        );
        // Every row has the same cell count, or a Markdown renderer drops cells.
        for line in &lines {
            assert_eq!(line.matches('|').count(), 10, "line = {line}");
        }
    }

    #[test]
    fn peak_markdown_with_no_peaks_is_still_a_valid_table() {
        let md = peaks_to_markdown(&[]);
        assert_eq!(md.lines().count(), 2);
        assert!(md.ends_with("|\n"));
    }

    #[test]
    fn a_pipe_in_a_channel_name_cannot_forge_a_column() {
        let mut p = sample_peak();
        p.channel_id = ChannelId::from("UV|280");
        let md = peaks_to_markdown(&[p]);
        let row = md.lines().nth(2).unwrap();
        assert!(row.contains("UV\\|280"), "row = {row}");
        // The escaped pipe still counts as a `|` character, so compare against the
        // header instead of a bare count.
        let header_cells = md.lines().next().unwrap().matches('|').count();
        assert_eq!(row.matches("\\|").count(), 1);
        assert_eq!(row.matches('|').count() - 1, header_cells);
    }

    #[test]
    fn channel_ids_containing_commas_are_quoted() {
        let mut p = sample_peak();
        p.channel_id = ChannelId::from("UV,weird");
        assert!(peaks_to_csv(&fractionless_run(), &[p]).contains("\"UV,weird\""));
    }
}

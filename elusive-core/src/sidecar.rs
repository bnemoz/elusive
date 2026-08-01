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
//!             "plate_channel": "MWave2", "plate_metric": "IntegratedArea" }
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
    /// Trace colours the user chose by hand, keyed by channel id.
    ///
    /// Absent in sidecars written before this field existed, hence the `Option`:
    /// "this build never had the feature" and "the user cleared every override"
    /// are different facts, and only the second should be allowed to wipe a
    /// colour a future merge might want to keep.
    #[serde(default)]
    pub channel_colors: Option<BTreeMap<String, Color>>,
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
pub fn peaks_to_csv(peaks: &[PeakResult]) -> String {
    let mut out = String::from(
        "peak_id,channel_id,v_start_ml,v_end_ml,apex_volume_ml,area,height,fwhm,estimated_mw\n",
    );
    for p in peaks {
        out.push_str(&format!(
            "{},{},{:.4},{:.4},{:.4},{:.6},{:.6},{},{}\n",
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
    use crate::model::{BaselineMode, ChannelId, PeakId, Well};

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
        let csv = peaks_to_csv(&[sample_peak()]);
        let mut lines = csv.lines();
        assert_eq!(
            lines.next().unwrap(),
            "peak_id,channel_id,v_start_ml,v_end_ml,apex_volume_ml,area,height,fwhm,estimated_mw"
        );
        let row = lines.next().unwrap();
        assert!(row.starts_with("1,MWave2,12.0000,14.0000,13.0000,"));
        // An unset MW must be an empty cell, never a zero that reads as a result.
        assert!(row.ends_with(','), "row = {row}");
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
    fn channel_ids_containing_commas_are_quoted() {
        let mut p = sample_peak();
        p.channel_id = ChannelId::from("UV,weird");
        assert!(peaks_to_csv(&[p]).contains("\"UV,weird\""));
    }
}

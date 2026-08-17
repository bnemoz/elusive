//! Comparison runs overlaid on the primary run's chromatogram.
//!
//! The primary run behaves exactly as it always has — it owns the sidecar, the
//! plate, and every edit. An [`Overlay`] is a *reference*: a fully parsed
//! [`Run`] plus display settings, and the peaks its own sidecar happens to
//! carry, loaded read-only. Nothing here ever writes an overlay's sidecar
//! (spec: `docs/superpowers/specs/2026-08-17-multi-run-overlay-design.md`).

use crate::theme::chart;
use crate::view::View;
use elusive_core::model::{ChannelId, ChannelKind, PeakResult, Run};
use elusive_core::{sidecar, wells};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

/// One comparison run: a parsed [`Run`] plus its display settings.
///
/// The `run` is immutable input, exactly like the primary; everything mutable
/// here is the user's viewing state, persisted through the *primary's* sidecar
/// as a [`sidecar::OverlayRef`].
pub struct Overlay {
    pub run: Run,
    pub source_path: PathBuf,
    /// Peaks read from the overlay run's own sidecar, if one exists. Read-only:
    /// they feed the comparison table and are never edited or written back.
    pub peaks: Vec<PeakResult>,
    /// Master toggle for the whole overlay.
    pub visible: bool,
    pub hidden_channels: BTreeSet<ChannelId>,
    /// Display-only shift on the volume axis, in mL. Never enters a stored or
    /// computed result, and is ignored on the time axis, where a constant mL
    /// offset has no constant time equivalent.
    pub x_offset_ml: f32,
}

impl Overlay {
    /// The name the legend, hover readout and comparison table call this run.
    pub fn label(&self) -> &str {
        if !self.run.meta.run_name.is_empty() {
            return &self.run.meta.run_name;
        }
        self.source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("comparison run")
    }

    pub fn is_channel_visible(&self, id: &ChannelId) -> bool {
        !self.hidden_channels.contains(id)
    }
}

/// Open a run as an overlay, picking up the peaks its own sidecar carries.
///
/// Errors come back as display strings rather than [`elusive_core::Error`]
/// because the only consumer is a status message: a failed overlay must never
/// take the primary run down with it. A sidecar that is missing, unreadable, or
/// for a different run degrades to "no saved analysis", not to a failure — the
/// traces still compare fine without it.
pub fn load_overlay(path: &Path) -> Result<Overlay, String> {
    let run = elusive_core::parse::open(path)
        .map_err(|e| format!("Could not open {}: {e}", path.display()))?;

    let mut peaks = Vec::new();
    let sidecar_path = run.sidecar_path();
    if sidecar_path.is_file() {
        if let Ok(s) = sidecar::load(&sidecar_path) {
            if s.matches(&run) {
                peaks = s.peaks;
            }
        }
    }

    Ok(Overlay {
        source_path: path.to_path_buf(),
        run,
        peaks,
        visible: true,
        hidden_channels: BTreeSet::new(),
        x_offset_ml: 0.0,
    })
}

/// The channels of a new overlay that should *start* hidden.
///
/// An overlay channel starts visible iff the primary is currently showing a
/// channel of the same kind — and, for UV, of the same wavelength, because
/// "UV" alone would pair a 280 nm trace with a 495 nm one and invite a
/// comparison that means nothing. Everything else starts hidden: comparing
/// UV 280 across five productions must not require unticking dozens of boxes.
pub fn default_hidden_channels(overlay: &Run, primary: &Run, view: &View) -> BTreeSet<ChannelId> {
    overlay
        .channels
        .iter()
        .filter(|c| {
            let shown = !c.is_empty()
                && primary.channels.iter().any(|p| {
                    !p.is_empty()
                        && view.is_channel_visible(&p.id)
                        && p.kind == c.kind
                        && (c.kind != ChannelKind::Uv || p.wavelength_nm == c.wavelength_nm)
                });
            !shown
        })
        .map(|c| c.id.clone())
        .collect()
}

/// Dash pattern identifying overlay run `overlay_index` on the chart.
///
/// Never [`chart::Dash::Solid`] — solid is the primary's look, and run identity
/// must not rest on colour alone (design-system rule #3). With two non-solid
/// patterns the identity repeats from the third overlay on; the legend and the
/// hover readout still carry the run name, which is the authoritative label.
pub fn overlay_dash(overlay_index: usize) -> chart::Dash {
    if overlay_index.is_multiple_of(2) {
        chart::Dash::Dashed
    } else {
        chart::Dash::Dotted
    }
}

/// One row of the Results "Run comparison" table and of `comparison.csv`.
pub struct ComparisonRow {
    pub run: String,
    /// Human-readable channel name for the table; the CSV uses the stable id
    /// carried inside `peak`.
    pub channel_name: String,
    pub peak: PeakResult,
    /// This peak's share of the total area on its own run's channel, in
    /// percent. `None` when the total is zero or the peak is alone with zero
    /// area — a percentage of nothing is not 100%.
    pub area_pct: Option<f64>,
    /// Wells the peak's window overlaps *in its own run*, spelled out.
    pub fractions: String,
}

/// Assemble the cross-run peak rows: the primary's live peaks first, then each
/// overlay's saved peaks, in overlay order.
pub fn comparison_rows(
    primary: &Run,
    primary_peaks: &[PeakResult],
    overlays: &[Overlay],
) -> Vec<ComparisonRow> {
    let mut rows = Vec::new();
    push_run_rows(&mut rows, run_label(primary), primary, primary_peaks);
    for overlay in overlays {
        push_run_rows(
            &mut rows,
            overlay.label().to_string(),
            &overlay.run,
            &overlay.peaks,
        );
    }
    rows
}

/// The primary's display name, mirroring [`Overlay::label`]'s fallback.
fn run_label(run: &Run) -> String {
    if !run.meta.run_name.is_empty() {
        return run.meta.run_name.clone();
    }
    run.source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("run")
        .to_string()
}

fn push_run_rows(rows: &mut Vec<ComparisonRow>, label: String, run: &Run, peaks: &[PeakResult]) {
    for p in peaks {
        let total: f64 = peaks
            .iter()
            .filter(|q| q.channel_id == p.channel_id)
            .map(|q| q.area)
            .sum();
        let area_pct = (total > 0.0).then(|| 100.0 * p.area / total);
        let channel_name = run
            .channel(&p.channel_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| p.channel_id.0.clone());
        let fractions = wells::join_well_labels(&run.wells_in_volume(p.v_start_ml, p.v_end_ml));
        rows.push(ComparisonRow {
            run: label.clone(),
            channel_name,
            peak: p.clone(),
            area_pct,
            fractions,
        });
    }
}

/// `comparison.csv`: the peak-table schema with a leading `run` column and a
/// trailing `area_pct`. Numeric precision matches `sidecar::peaks_to_csv`
/// exactly, so a value never disagrees between the two exports.
pub fn comparison_to_csv(rows: &[ComparisonRow]) -> String {
    let mut out = String::from(
        "run,peak_id,channel_id,v_start_ml,v_end_ml,apex_volume_ml,area,height,fwhm,\
         estimated_mw,area_pct,fractions\n",
    );
    for r in rows {
        let p = &r.peak;
        out.push_str(&format!(
            "{},{},{},{:.4},{:.4},{:.4},{:.6},{:.6},{},{},{},{}\n",
            csv_escape(&r.run),
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
            r.area_pct.map(|v| format!("{v:.2}")).unwrap_or_default(),
            csv_escape(&r.fractions),
        ));
    }
    out
}

/// Quote a CSV field when it holds a comma, quote or newline (RFC 4180).
/// Mirrors the private helper in `elusive_core::sidecar`.
fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// How `target` is written into the primary's sidecar.
///
/// Relative to `base_dir` whenever the two share any path prefix, so a folder
/// of runs copied to another machine keeps its comparisons; the absolute path
/// only when there is nothing to be relative to (say, different Windows
/// drives).
pub fn relative_or_absolute(base_dir: &Path, target: &Path) -> String {
    let base: Vec<Component<'_>> = base_dir.components().collect();
    let tgt: Vec<Component<'_>> = target.components().collect();
    let common = base
        .iter()
        .zip(tgt.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return target.display().to_string();
    }
    let mut rel = PathBuf::new();
    for _ in common..base.len() {
        rel.push("..");
    }
    for c in &tgt[common..] {
        rel.push(c.as_os_str());
    }
    rel.display().to_string()
}

/// Resolve a stored [`sidecar::OverlayRef::path`] against the primary's
/// directory. Relative refs are joined and normalised lexically; absolute refs
/// pass through untouched.
pub fn resolve_overlay_path(base_dir: &Path, stored: &str) -> PathBuf {
    let stored = Path::new(stored);
    if stored.is_absolute() {
        return stored.to_path_buf();
    }
    lexical_normalize(&base_dir.join(stored))
}

/// Collapse `.` and `..` components without touching the filesystem.
///
/// Lexical on purpose: the file may not exist yet (that is precisely the error
/// being reported), and `canonicalize` would fail on it. `..` at the root is
/// dropped rather than kept, matching how every shell resolves `/..`.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out: Vec<Component<'_>> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => out.push(c),
            },
            other => out.push(other),
        }
    }
    out.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::chart;
    use crate::view::View;
    use elusive_core::model::{
        BaselineMode, Channel, ChannelKind, PeakId, PeakResult, Run, RunMeta, Sample, SourceFormat,
    };
    use std::path::{Path, PathBuf};

    fn channel(id: &str, kind: ChannelKind, wavelength_nm: Option<u16>) -> Channel {
        let mut c = Channel::new(id, id, kind);
        c.wavelength_nm = wavelength_nm;
        c.samples = vec![Sample::new(0.0, 0.0, 0.1), Sample::new(60.0, 1.0, 0.2)];
        c
    }

    /// A run with UV 215, UV 280 and conductivity channels.
    fn mini_run(name: &str) -> Run {
        Run {
            meta: RunMeta {
                run_name: name.to_string(),
                ..RunMeta::default()
            },
            source_format: SourceFormat::NgcAnalysis,
            source_path: PathBuf::from(format!("{name}.ngcAnalysis")),
            channels: vec![
                channel("MWave0", ChannelKind::Uv, Some(215)),
                channel("MWave2", ChannelKind::Uv, Some(280)),
                channel("MD_Conductivity", ChannelKind::Conductivity, None),
            ],
            fractions: Vec::new(),
            events: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn peak(id: u32, channel: &str, apex: f32, area: f64) -> PeakResult {
        PeakResult {
            id: PeakId(id),
            channel_id: channel.into(),
            v_start_ml: apex - 1.0,
            v_end_ml: apex + 1.0,
            baseline: BaselineMode::LinearEndpoints,
            area,
            height: 1.0,
            apex_volume_ml: apex,
            fwhm_ml: None,
            estimated_mw_kda: None,
        }
    }

    fn overlay_of(run: Run) -> Overlay {
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
    fn matching_visible_channels_start_visible() {
        let primary = mini_run("primary");
        let mut view = View::default();
        // The user is looking at UV 280 and conductivity; UV 215 is hidden.
        view.set_channel_visible(&"MWave0".into(), false);

        let hidden = default_hidden_channels(&mini_run("other"), &primary, &view);
        assert!(
            hidden.contains(&"MWave0".into()),
            "UV 215 should start hidden"
        );
        assert!(
            !hidden.contains(&"MWave2".into()),
            "UV 280 should start visible"
        );
        assert!(
            !hidden.contains(&"MD_Conductivity".into()),
            "conductivity should start visible"
        );
    }

    #[test]
    fn uv_match_requires_equal_wavelength() {
        let primary = mini_run("primary");
        let view = View::default();
        let mut other = mini_run("other");
        // The overlay's second UV detector monitored 260 nm, not 280.
        other.channels[1].wavelength_nm = Some(260);

        let hidden = default_hidden_channels(&other, &primary, &view);
        assert!(
            hidden.contains(&"MWave2".into()),
            "a UV channel at a different wavelength must not silently pair up"
        );
    }

    #[test]
    fn overlay_dash_is_never_solid_and_alternates() {
        assert_eq!(overlay_dash(0), chart::Dash::Dashed);
        assert_eq!(overlay_dash(1), chart::Dash::Dotted);
        assert_eq!(overlay_dash(2), chart::Dash::Dashed);
    }

    #[test]
    fn area_pct_is_within_one_runs_channel() {
        let primary = mini_run("primary");
        let primary_peaks = vec![peak(1, "MWave2", 13.0, 3.0), peak(2, "MWave2", 16.0, 1.0)];
        let mut overlay = overlay_of(mini_run("other"));
        overlay.peaks = vec![peak(1, "MWave2", 13.2, 8.0)];

        let rows = comparison_rows(&primary, &primary_peaks, &[overlay]);
        assert_eq!(rows.len(), 3);
        // Primary rows come first and their percentages sum within the run.
        assert_eq!(rows[0].run, "primary");
        assert_eq!(rows[0].area_pct, Some(75.0));
        assert_eq!(rows[1].area_pct, Some(25.0));
        // The overlay's lone peak is 100% of its own run, not of the union.
        assert_eq!(rows[2].run, "other");
        assert_eq!(rows[2].area_pct, Some(100.0));
    }

    #[test]
    fn comparison_csv_has_run_column_then_peak_schema() {
        let primary = mini_run("primary");
        let rows = comparison_rows(&primary, &[peak(1, "MWave2", 13.0, 3.0)], &[]);
        let csv = comparison_to_csv(&rows);
        assert_eq!(
            csv.lines().next().unwrap(),
            "run,peak_id,channel_id,v_start_ml,v_end_ml,apex_volume_ml,area,height,fwhm,\
             estimated_mw,area_pct,fractions"
        );
        let row = csv.lines().nth(1).unwrap();
        assert!(
            row.starts_with("primary,1,MWave2,12.0000,14.0000,13.0000,"),
            "row = {row}"
        );
    }

    #[test]
    fn a_run_name_with_a_comma_is_quoted_in_the_csv() {
        let mut primary = mini_run("primary");
        primary.meta.run_name = "SEC, redo".into();
        let rows = comparison_rows(&primary, &[peak(1, "MWave2", 13.0, 3.0)], &[]);
        let csv = comparison_to_csv(&rows);
        assert!(csv.lines().nth(1).unwrap().starts_with("\"SEC, redo\","));
    }

    #[test]
    fn relative_path_when_shared_prefix_absolute_otherwise() {
        assert_eq!(
            relative_or_absolute(Path::new("/data/runs"), Path::new("/data/std.ngcAnalysis")),
            "../std.ngcAnalysis"
        );
        assert_eq!(
            relative_or_absolute(
                Path::new("/data/runs"),
                Path::new("/data/runs/b.ngcAnalysis")
            ),
            "b.ngcAnalysis"
        );
        // Relative refs resolve back against the primary's directory.
        assert_eq!(
            resolve_overlay_path(Path::new("/data/runs"), "../std.ngcAnalysis"),
            PathBuf::from("/data/std.ngcAnalysis")
        );
        // Absolute refs pass through untouched.
        assert_eq!(
            resolve_overlay_path(Path::new("/data/runs"), "/elsewhere/std.ngcAnalysis"),
            PathBuf::from("/elsewhere/std.ngcAnalysis")
        );
    }

    /// The committed instrument fixture, overlaid on itself. At offset zero the
    /// two must be byte-for-byte the same traces; every channel of the overlay
    /// must pair with its own twin and start visible.
    #[test]
    fn the_real_fixture_overlays_on_itself() {
        let path = Path::new("../testdata/sec-run.ngcAnalysis");
        let primary = elusive_core::parse::open(path).expect("fixture parses");
        let overlay = load_overlay(path).expect("fixture loads as an overlay");

        assert_eq!(overlay.run.channels.len(), primary.channels.len());
        assert_eq!(overlay.x_offset_ml, 0.0);
        assert!(overlay.visible);

        // Same file, same channels: every non-empty channel matches its twin,
        // so none of them may start hidden under the default view.
        let hidden = default_hidden_channels(&overlay.run, &primary, &View::default());
        let hidden_nonempty: Vec<_> = hidden
            .iter()
            .filter(|id| overlay.run.channel(id).is_some_and(|c| !c.is_empty()))
            .collect();
        assert!(hidden_nonempty.is_empty(), "hidden = {hidden_nonempty:?}");

        // Trace payloads are identical, so the overlay draws exactly on top.
        let a = primary.channel(&"MWave0".into()).expect("primary MWave0");
        let b = overlay
            .run
            .channel(&"MWave0".into())
            .expect("overlay MWave0");
        assert_eq!(a.samples.len(), b.samples.len());
        assert_eq!(a.samples.first(), b.samples.first());
        assert_eq!(a.samples.last(), b.samples.last());
    }

    #[test]
    fn overlay_label_prefers_the_run_name_and_falls_back_to_the_file_stem() {
        let named = overlay_of(mini_run("prep 2026-08-02"));
        assert_eq!(named.label(), "prep 2026-08-02");

        let mut anonymous = overlay_of(mini_run("x"));
        anonymous.run.meta.run_name = String::new();
        anonymous.source_path = PathBuf::from("/data/std-run.ngcAnalysis");
        assert_eq!(anonymous.label(), "std-run");
    }
}

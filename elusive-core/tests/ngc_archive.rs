//! End-to-end tests over a synthetic `.ngcAnalysis` archive.
//!
//! No real ChromLab export is committed to this repository (instrument files are
//! not ours to redistribute), so these build an archive that matches the layout
//! and payload encodings verified in `design.md` §3 and push it through the whole
//! parser. They cover the acceptance criteria of `IMPLEMENTATION_PLAN.md` Phase 1:
//! metadata, uneven sampling, fraction windows, and errors that carry context.
//!
//! When a real archive is available, drop it in `testdata/` and add a test that
//! opens it — these synthetic ones check the logic, not the reality of the format.

use base64::Engine as _;
use elusive_core::model::{ChannelKind, SourceFormat};
use elusive_core::parse::ngc;
use std::io::{Cursor, Write};
use std::path::Path;

/// Encode samples in the documented binary layout: `u32` version, then
/// `[time_s, value, volume_mL]` little-endian f32 triplets.
fn signal_payload(samples: &[(f32, f32, f32)]) -> String {
    let mut blob = 1u32.to_le_bytes().to_vec();
    for (time_s, value, volume_ml) in samples {
        blob.extend(time_s.to_le_bytes());
        blob.extend(value.to_le_bytes());
        blob.extend(volume_ml.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(blob)
}

fn trace_xml(name: &str, unit: &str, payload: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Trace xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <Name>{name}</Name>
  <Unit>{unit}</Unit>
  <Color>#FF2F6FB3</Color>
  <TraceData>{payload}</TraceData>
</Trace>"#
    )
}

/// The fractions payload is base64 of an inner `RootNodeOfCFCData` document.
fn fractions_xml(inner: &str) -> String {
    let payload = base64::engine::general_purpose::STANDARD.encode(inner.as_bytes());
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Trace><Name>Fractions</Name><TraceData>{payload}</TraceData></Trace>"#
    )
}

fn cfc_record(tube: u32, event: &str, extra: &str) -> String {
    format!(
        r#"<CFCData>
      <Event>{event}</Event>
      <TubeNumber>{tube}</TubeNumber>
      <TubeNumberNotMinusOne>{tube}</TubeNumberNotMinusOne>
      <RackNumber>1</RackNumber>
      <RackType>HEP96</RackType>
      <CollectionPattern>Serpentine</CollectionPattern>
      <FractionSize>0.4</FractionSize>
      {extra}
    </CFCData>"#
    )
}

fn build_archive(entries: &[(&str, String)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, contents) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

/// A run with two UV channels, conductivity at a different sample rate, and two
/// fraction traces (a complete stream and a summary) to exercise reconciliation.
fn realistic_archive() -> Vec<u8> {
    // UV: 200 samples over 40 mL, a Gaussian peak centred at 12 mL.
    let uv: Vec<(f32, f32, f32)> = (0..200)
        .map(|i| {
            let v = 40.0 * i as f32 / 199.0;
            let z: f32 = (v - 12.0) / 0.8;
            (v * 120.0, 0.9 * (-0.5 * z * z).exp(), v)
        })
        .collect();
    // Conductivity: 401 samples — deliberately not the same count as UV.
    let cond: Vec<(f32, f32, f32)> = (0..401)
        .map(|i| {
            let v = 40.0 * i as f32 / 400.0;
            (v * 120.0, 17.0 + 0.5 * (v / 40.0), v)
        })
        .collect();

    let full_stream = format!(
        "<RootNodeOfCFCData>{}</RootNodeOfCFCData>",
        (1..=6)
            .map(|tube| {
                let start = 10.0 + 0.4 * (tube - 1) as f32;
                format!(
                    "{}{}",
                    cfc_record(
                        tube,
                        "FractionStart",
                        &format!(
                            "<VolumeStartSec>{start}</VolumeStartSec><TimeStartSec>{}</TimeStartSec>",
                            start * 120.0
                        ),
                    ),
                    cfc_record(
                        tube,
                        "FractionDone",
                        &format!(
                            "<VolumeEndSec>{}</VolumeEndSec><TimeEndSec>{}</TimeEndSec>",
                            start + 0.4,
                            (start + 0.4) * 120.0
                        ),
                    ),
                )
            })
            .collect::<String>()
    );

    // The summary trace: same tubes, start volumes only, plus one extra tube.
    let summary = format!(
        "<RootNodeOfCFCData>{}</RootNodeOfCFCData>",
        (1..=7)
            .map(|tube| {
                let start = 10.0 + 0.4 * (tube - 1) as f32;
                cfc_record(
                    tube,
                    "FractionStart",
                    &format!("<VolumeStartSec>{start}</VolumeStartSec>"),
                )
            })
            .collect::<String>()
    );

    build_archive(&[
        (
            "Method/MethodData.xml",
            r#"<?xml version="1.0"?>
<Method>
  <MethodName>SEC Superdex 200</MethodName>
  <Technique>SEC</Technique>
  <Column>Superdex 200 Increase 10/300</Column>
  <VoidVolume>8.0</VoidVolume>
  <ColumnVolume>24.0</ColumnVolume>
  <PathLength>0.2</PathLength>
  <Detector><Wavelength1>215</Wavelength1></Detector>
  <Detector><Wavelength2>255</Wavelength2></Detector>
  <Detector><Wavelength3>280</Wavelength3></Detector>
  <Detector><Wavelength4>495</Wavelength4></Detector>
</Method>"#
                .to_string(),
        ),
        (
            "Runs/Run1.xml",
            r#"<?xml version="1.0"?>
<Run><RunName>EluSive smoke run</RunName><StartTime>2026-07-31T09:15:00</StartTime></Run>"#
                .to_string(),
        ),
        (
            "Runs/Run1/Trace_MWave2_1.xml",
            trace_xml("UV 3", "AU", &signal_payload(&uv)),
        ),
        (
            "Runs/Run1/Trace_MWave0_2.xml",
            trace_xml("UV 1", "AU", &signal_payload(&uv)),
        ),
        (
            "Runs/Run1/Trace_MD_Conductivity_3.xml",
            trace_xml("Conductivity", "mS/cm", &signal_payload(&cond)),
        ),
        (
            "Runs/Run1/Trace_Fractions_4.xml",
            fractions_xml(&full_stream),
        ),
        ("Runs/Run1/Trace_Fractions_5.xml", fractions_xml(&summary)),
    ])
}

fn open_realistic() -> elusive_core::Run {
    ngc::from_reader(
        Cursor::new(realistic_archive()),
        Path::new("smoke.ngcAnalysis"),
        SourceFormat::NgcAnalysis,
    )
    .expect("synthetic archive should parse")
}

#[test]
fn opens_an_archive_and_reports_channels_and_fractions() {
    let run = open_realistic();
    assert_eq!(run.meta.run_name, "EluSive smoke run");
    assert_eq!(run.meta.technique, "SEC");
    assert_eq!(
        run.meta.column.as_deref(),
        Some("Superdex 200 Increase 10/300")
    );
    assert_eq!(run.channels.len(), 3);
    assert_eq!(run.source_format, SourceFormat::NgcAnalysis);
}

#[test]
fn channel_sample_counts_stay_independent() {
    let run = open_realistic();
    let uv = run
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::Uv)
        .unwrap();
    let cond = run
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::Conductivity)
        .unwrap();

    assert_eq!(uv.samples.len(), 200);
    assert_eq!(cond.samples.len(), 401);
    assert_ne!(
        uv.samples.len(),
        cond.samples.len(),
        "the fixture must exercise uneven sampling"
    );
}

#[test]
fn conductivity_values_land_in_the_physically_expected_range() {
    // The same sanity check performed against the real export in design.md §3.1.
    let run = open_realistic();
    let cond = run
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::Conductivity)
        .unwrap();
    let (lo, hi) = cond.display_value_range().unwrap();
    assert!((17.0..=18.0).contains(&lo), "lo = {lo}");
    assert!((17.0..=18.0).contains(&hi), "hi = {hi}");
    assert_eq!(cond.display_scale, 1.0, "only UV is rescaled");
}

#[test]
fn declared_au_units_are_displayed_as_mau() {
    let run = open_realistic();
    let uv = run.hero_channel().unwrap();
    assert_eq!(uv.unit, "AU");
    assert_eq!(uv.display_unit, "mAU");
    assert_eq!(uv.display_scale, 1000.0);

    // Sampled every 0.2 mL, so the recorded maximum sits just below the true
    // 900 mAU apex; the point of the assertion is the x1000 scale, not the apex.
    let (_, peak) = uv.display_value_range().unwrap();
    assert!((peak - 900.0).abs() / 900.0 < 0.01, "peak = {peak} mAU");
}

#[test]
fn wavelengths_come_from_the_method_and_pick_the_hero_trace() {
    let run = open_realistic();
    let hero = run.hero_channel().unwrap();
    // MWave2 maps to the third detector wavelength, 280 nm.
    assert_eq!(hero.wavelength_nm, Some(280));
    assert_eq!(hero.name, "UV 280 nm");

    let other = run
        .channels
        .iter()
        .find(|c| c.id.as_str() == "MWave0")
        .unwrap();
    assert_eq!(other.wavelength_nm, Some(215));
}

#[test]
fn fraction_windows_and_wells_are_decoded() {
    let run = open_realistic();
    assert_eq!(
        run.fractions.len(),
        7,
        "six complete plus the summary-only tube"
    );

    let f1 = &run.fractions[0];
    assert_eq!(f1.tube, 1);
    assert!((f1.vol_start_ml - 10.0).abs() < 1e-4);
    assert!((f1.vol_end_ml - 10.4).abs() < 1e-4);
    assert_eq!(f1.well.unwrap().label(), "A1");
    assert_eq!(run.fractions[1].well.unwrap().label(), "A2");
    assert_eq!(f1.rack_type, "HEP96");
}

#[test]
fn duplicate_fraction_traces_reconcile_to_the_complete_stream() {
    let run = open_realistic();
    // Tubes 1..6 come from the full stream with real end volumes; tube 7 exists
    // only in the summary and falls back to the nominal 0.4 mL size.
    for tube in 1..=6 {
        let f = run.fractions.iter().find(|f| f.tube == tube).unwrap();
        let expected_start = 10.0 + 0.4 * (tube - 1) as f32;
        assert!(
            (f.vol_start_ml - expected_start).abs() < 1e-3,
            "tube {tube}"
        );
        assert!(
            (f.vol_end_ml - (expected_start + 0.4)).abs() < 1e-3,
            "tube {tube}"
        );
    }
    for tube in 1..=6 {
        let f = run.fractions.iter().find(|f| f.tube == tube).unwrap();
        assert!(!f.end_estimated, "tube {tube} has a recorded FractionDone");
    }

    // Tube 7 exists only in the summary, so its end is inferred from the
    // nominal size and must be flagged as such rather than passed off as measured.
    let t7 = run.fractions.iter().find(|f| f.tube == 7).unwrap();
    assert!(t7.has_usable_window());
    assert!(t7.end_estimated, "an inferred window must be marked");

    assert!(run.warnings.iter().any(|w| w.scope == "fractions"));
}

#[test]
fn fraction_lookup_by_volume_finds_the_overlapping_tubes() {
    let run = open_realistic();
    // Tubes span 0.4 mL from 10.0: t2 = 10.4..10.8, t3 = 10.8..11.2, and t4
    // starts at 11.2 — past the end of the query, so it must not be returned.
    let tubes: Vec<u32> = run
        .fractions_in_volume(10.5, 11.0)
        .iter()
        .map(|f| f.tube)
        .collect();
    assert_eq!(tubes, vec![2, 3]);

    let wide: Vec<u32> = run
        .fractions_in_volume(10.5, 11.3)
        .iter()
        .map(|f| f.tube)
        .collect();
    assert_eq!(wide, vec![2, 3, 4]);
}

#[test]
fn plate_metrics_light_up_the_wells_under_the_peak() {
    use elusive_core::integrate::{metrics_for_fractions, PlateMetric};
    let run = open_realistic();
    let uv = run.hero_channel().unwrap();
    let values = metrics_for_fractions(uv, &run.fractions, PlateMetric::IntegratedArea);

    assert_eq!(values.len(), run.fractions.len());
    assert!(values.iter().all(|v| v.is_some()), "every window has data");

    // The Gaussian is centred at 12 mL, which is tube 6 (10.0 + 0.4*5 = 12.0).
    let peak_idx = values
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.unwrap().total_cmp(&b.1.unwrap()))
        .map(|(i, _)| i)
        .unwrap();
    assert_eq!(run.fractions[peak_idx].tube, 6);
}

#[test]
fn integration_over_the_peak_recovers_the_synthetic_area() {
    use elusive_core::integrate::integrate_peak;
    use elusive_core::model::{BaselineMode, PeakId};

    let run = open_realistic();
    let uv = run.hero_channel().unwrap();
    let peak = integrate_peak(PeakId(1), uv, 9.0, 15.0, BaselineMode::DropToZero).unwrap();

    // Amplitude 0.9 AU = 900 mAU, sigma 0.8 mL → area = 900 * 0.8 * sqrt(2π).
    let analytic = 900.0 * 0.8 * (2.0 * std::f64::consts::PI).sqrt();
    assert!(
        (peak.area - analytic).abs() / analytic < 0.02,
        "area {} vs {analytic}",
        peak.area
    );
    assert!((peak.apex_volume_ml - 12.0).abs() < 0.3);
}

#[test]
fn a_run_with_no_recognisable_entries_fails_with_context() {
    let archive = build_archive(&[("readme.txt", "not a chromatogram".to_string())]);
    let err = ngc::from_reader(
        Cursor::new(archive),
        Path::new("empty.ngcAnalysis"),
        SourceFormat::NgcAnalysis,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("no run data"),
        "unhelpful error: {err}"
    );
}

#[test]
fn a_corrupt_trace_is_reported_as_a_warning_without_losing_the_run() {
    let mut entries: Vec<(&str, String)> = vec![
        (
            "Runs/Run1.xml",
            "<Run><RunName>Partly broken</RunName></Run>".to_string(),
        ),
        (
            "Runs/Run1/Trace_MWave2_1.xml",
            trace_xml(
                "UV 3",
                "AU",
                &signal_payload(&[(0.0, 1.0, 0.0), (1.0, 2.0, 0.1)]),
            ),
        ),
    ];
    // A payload whose body is not a multiple of the 12-byte record size.
    let mut broken = 1u32.to_le_bytes().to_vec();
    broken.extend([0u8; 7]);
    entries.push((
        "Runs/Run1/Trace_MD_Conductivity_2.xml",
        trace_xml(
            "Conductivity",
            "mS/cm",
            &base64::engine::general_purpose::STANDARD.encode(broken),
        ),
    ));

    let run = ngc::from_reader(
        Cursor::new(build_archive(&entries)),
        Path::new("partly.ngcAnalysis"),
        SourceFormat::NgcAnalysis,
    )
    .expect("one bad trace must not sink the whole run");

    assert_eq!(run.channels.len(), 1, "the good channel still loads");
    assert!(
        run.warnings
            .iter()
            .any(|w| w.message.contains("record size")),
        "the bad trace must be reported: {:?}",
        run.warnings
    );
}

#[test]
fn a_missing_wavelength_map_falls_back_and_says_so() {
    let entries = vec![
        (
            "Runs/Run1.xml",
            "<Run><RunName>No method</RunName></Run>".to_string(),
        ),
        (
            "Runs/Run1/Trace_MWave3_1.xml",
            // No <Name> hint and no method XML: only the documented default order left.
            format!(
                r#"<Trace><Unit>AU</Unit><TraceData>{}</TraceData></Trace>"#,
                signal_payload(&[(0.0, 1.0, 0.0), (1.0, 2.0, 0.1)])
            ),
        ),
    ];
    let run = ngc::from_reader(
        Cursor::new(build_archive(&entries)),
        Path::new("nomethod.ngcAnalysis"),
        SourceFormat::NgcAnalysis,
    )
    .unwrap();

    assert_eq!(run.channels[0].wavelength_nm, Some(495));
    assert!(
        run.warnings
            .iter()
            .any(|w| w.scope == "wavelengths" && w.message.contains("assuming")),
        "an assumed wavelength map must be surfaced: {:?}",
        run.warnings
    );
}

#[test]
fn the_source_archive_is_never_written_to() {
    let dir = std::env::temp_dir().join("elusive-readonly-check");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("run.ngcAnalysis");
    std::fs::write(&path, realistic_archive()).unwrap();
    let before = std::fs::metadata(&path).unwrap().len();

    let run = ngc::open(&path).unwrap();
    assert!(!run.channels.is_empty());

    let after = std::fs::metadata(&path).unwrap().len();
    assert_eq!(before, after, "opening a run must not modify it");
    assert_eq!(
        run.sidecar_path().file_name().unwrap().to_str().unwrap(),
        "run.ngcAnalysis.elusive.json"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_non_archive_extension_is_rejected_before_reading() {
    let err = elusive_core::parse::open(Path::new("run.txt")).unwrap_err();
    assert!(err.to_string().contains("not a supported run format"));
}

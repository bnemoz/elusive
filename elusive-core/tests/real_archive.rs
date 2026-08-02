//! Tests against a real (sanitized) Bio-Rad NGC export.
//!
//! # Why this file is separate from `ngc_archive.rs`
//!
//! `ngc_archive.rs` builds a synthetic `.ngcAnalysis` in memory and parses it.
//! That proves the parser is self-consistent — it can read back what our own
//! encoder wrote — and says nothing about whether we read the *instrument*
//! correctly. These tests run against `testdata/sec-run.ngcAnalysis`, a real
//! Superdex 200 10/300 GL SEC run with identity fields redacted and every
//! trace payload byte-identical to the instrument output.
//!
//! When a test here fails it means the real format diverged from what we
//! believe. When one in `ngc_archive.rs` fails it means the logic is wrong.
//! Keeping them apart makes that distinction immediate at the failure site.
//!
//! # Where the expected values come from
//!
//! Every constant below was read out of the archive and is documented in
//! `docs/format-findings.md`, which cites the exact element paths. The most
//! important source is `Analysis.xml` — an undocumented 6 MB entry holding
//! ChromLab's own 58 peak records, linked to traces by GUID. That gives an
//! independent reference computed by the vendor's software rather than by hand.
//!
//! # `#[ignore]` means "verified fact the parser does not implement yet"
//!
//! Several tests are `#[ignore]`d. They are not flaky, slow, or aspirational —
//! each encodes something the archive demonstrably says that the parser
//! currently gets wrong or does not read at all. They are written now, while
//! the evidence is fresh, so the gap lives as an executable statement rather
//! than a sentence in a document nobody re-reads.
//!
//! Run `cargo test -p elusive-core --test real_archive -- --ignored` to see the
//! outstanding work. Each carries the `design.md` §15 box it closes.

use elusive_core::model::{ChannelId, ChannelKind, Run, SourceFormat};

/// Path is relative to the crate root, which is where `cargo test` runs.
const FIXTURE: &str = "../testdata/sec-run.ngcAnalysis";

fn fixture() -> Run {
    elusive_core::parse::open(FIXTURE).expect("the committed fixture must parse")
}

fn channel(run: &Run, id: &str) -> elusive_core::model::Channel {
    run.channel(&ChannelId::from(id))
        .unwrap_or_else(|| {
            panic!(
                "channel {id} missing; present: {:?}",
                run.channels
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
            )
        })
        .clone()
}

/// Upper end of a channel's range in display units.
fn display_max(run: &Run, id: &str) -> f32 {
    channel(run, id)
        .display_value_range()
        .unwrap_or_else(|| panic!("{id} has no finite range"))
        .1
}

/// Does any warning mention this text?
fn warned(run: &Run, needle: &str) -> bool {
    run.warnings.iter().any(|w| w.message.contains(needle))
}

// ---------------------------------------------------------------------------
// Shape of the archive
// ---------------------------------------------------------------------------

#[test]
fn the_fixture_parses_as_an_ngc_analysis_archive() {
    let run = fixture();
    assert_eq!(run.source_format, SourceFormat::NgcAnalysis);
    assert_eq!(run.channels.len(), 16, "16 traces carry data in this run");
}

#[test]
fn every_documented_channel_is_present_and_populated() {
    let run = fixture();
    for id in [
        "MWave0",
        "MWave1",
        "MWave2",
        "MWave3",
        "MD_Conductivity",
        "ModulePH",
        "PercentB",
        "FlowRate",
        "MD_Temperature",
    ] {
        assert!(
            !channel(&run, id).is_empty(),
            "{id} parsed but has no samples"
        );
    }
}

#[test]
fn channels_have_independent_sample_counts() {
    // `design.md` §3 warns pH was sampled at ~2x the UV rate and that no shared
    // index may be assumed. This run makes it exact: 45518 == 2 * 22759. A
    // parser that zipped channels together on a common index would either
    // truncate pH to half its resolution or misalign it against volume.
    let run = fixture();
    let uv = channel(&run, "MWave2").samples.len();
    let ph = channel(&run, "ModulePH").samples.len();
    assert_eq!(uv, 22_759);
    assert_eq!(ph, 45_518);
    assert_eq!(
        ph,
        uv * 2,
        "pH is sampled at exactly twice the UV rate here"
    );
}

#[test]
fn the_volume_axis_spans_the_whole_run() {
    let run = fixture();
    let (lo, hi) = run.volume_range().expect("a volume range");
    assert!((lo - 0.0).abs() < 1e-3, "starts at zero, got {lo}");
    assert!((hi - 37.91).abs() < 0.05, "≈37.91 mL total, got {hi}");
}

// ---------------------------------------------------------------------------
// Box 2 — UV value scale (AU stored, mAU displayed)
// ---------------------------------------------------------------------------

#[test]
fn uv_traces_are_displayed_in_mau() {
    // Confirmed two independent ways, neither of them a hand integration:
    //
    //   raw TraceData payload, MWave0 max = 0.22661
    //   ChromLab's own <Height> for its largest peak = 0.227303  (Analysis.xml)
    //
    // They agree, so ChromLab stores the same units it hands us — absorbance
    // units — and displays milli-absorbance. ChromLab's figure is slightly the
    // larger because height is measured from a baseline sitting at about
    // -0.0152 AU, which is itself recorded per peak as <BaseStartY>.
    let run = fixture();
    for id in ["MWave0", "MWave1", "MWave2", "MWave3"] {
        let c = channel(&run, id);
        assert_eq!(c.kind, ChannelKind::Uv, "{id}");
        assert_eq!(c.display_unit, "mAU", "{id}");
        assert_eq!(c.display_scale, 1000.0, "{id} stores AU and displays mAU");
    }
}

#[test]
fn the_largest_uv_peak_matches_what_chromlab_recorded() {
    // The single most load-bearing number in the file. If the scale policy ever
    // regresses this is off by 1000x, and it propagates into peak area, the
    // plate heatmap, and the A280 concentration someone pipettes against.
    let run = fixture();
    let observed = display_max(&run, "MWave0");
    assert!(
        (observed - 226.611).abs() < 0.01,
        "MWave0 should top out at 226.611 mAU, got {observed}"
    );
    // ChromLab's stored peak height, converted: 0.227303 AU -> 227.303 mAU.
    // Ours is the raw trace maximum and hers is measured from a negative
    // baseline, so hers is legitimately a little larger. A gap outside this
    // tolerance means the two are no longer describing the same curve.
    let chromlab_mau = 0.227_303_f32 * 1000.0;
    assert!(
        (chromlab_mau - observed) > 0.0 && (chromlab_mau - observed) < 1.0,
        "expected ChromLab {chromlab_mau} slightly above ours {observed}"
    );
}

#[test]
fn the_215_nm_trace_dominates_the_280_nm_trace() {
    // A physics check on the wavelength mapping rather than the scale: the
    // peptide bond absorbs far more at 215 nm than aromatic side chains do at
    // 280 nm, so a protein run must show 215 well above 280. If the MWave
    // mapping were ever inverted this ratio would flip, and no unit assertion
    // would catch it.
    let run = fixture();
    let a215 = display_max(&run, "MWave0");
    let a280 = display_max(&run, "MWave2");
    assert!(
        a215 > a280 * 10.0,
        "215 nm ({a215} mAU) should dwarf 280 nm ({a280} mAU)"
    );
}

#[test]
fn non_uv_channels_are_not_rescaled() {
    // Only UV carries the AU/mAU conversion. Conductivity and pH are stored in
    // the units they are reported in, and the values below match the figures
    // `design.md` §3 records from the independent CSV export of this same run
    // (17.0-18.0 mS/cm, pH 8.05-8.29) — a cross-format agreement, not a
    // restatement of our own parse.
    let run = fixture();
    for id in ["MD_Conductivity", "ModulePH"] {
        assert_eq!(channel(&run, id).display_scale, 1.0, "{id}");
    }
    let (clo, chi) = channel(&run, "MD_Conductivity")
        .display_value_range()
        .expect("conductivity range");
    assert!(
        (17.0..=18.1).contains(&clo) && (17.0..=18.1).contains(&chi),
        "{clo}..{chi}"
    );

    let (plo, phi) = channel(&run, "ModulePH")
        .display_value_range()
        .expect("pH range");
    assert!(
        (8.0..=8.3).contains(&plo) && (8.0..=8.3).contains(&phi),
        "{plo}..{phi}"
    );
}

#[test]
#[ignore = "design.md §15 box 2: the scale is right by luck. NGC traces declare \
            no unit at all, so display_scale_for only ever reaches its magnitude \
            heuristic ('peak below ~20 suggests AU'). It happens to be correct \
            for this run. Make AU->mAU the known NGC convention and keep the \
            heuristic for CSV, which genuinely declares nothing."]
fn uv_scaling_does_not_rely_on_a_magnitude_guess() {
    let run = fixture();
    assert!(
        !warned(&run, "suggests AU"),
        "the AU convention is known for NGC; amplitude should not be consulted"
    );
}

// ---------------------------------------------------------------------------
// Box 1 — MWave0..3 to wavelength mapping
// ---------------------------------------------------------------------------

#[test]
fn uv_channels_carry_the_expected_wavelengths() {
    // Methods/MethodData1.xml declares:
    //     <Wavelength1>215</Wavelength1>  <Wavelength2>255</Wavelength2>
    //     <Wavelength3>280</Wavelength3>  <Wavelength4>495</Wavelength4>
    //
    // Note the off-by-one against the trace names: the method numbers from 1,
    // the traces from 0, so Wavelength1 belongs to MWave0.
    let run = fixture();
    for (id, nm) in [
        ("MWave0", 215u16),
        ("MWave1", 255),
        ("MWave2", 280),
        ("MWave3", 495),
    ] {
        assert_eq!(channel(&run, id).wavelength_nm, Some(nm), "{id}");
    }
}

#[test]
fn the_hero_trace_is_the_280_nm_channel() {
    // A280 drives the concentration estimate, so the trace the UI opens on and
    // the trace concentration is computed from must be the same one.
    let run = fixture();
    let hero = run.hero_channel().expect("a hero channel");
    assert_eq!(hero.id.as_str(), "MWave2");
    assert_eq!(hero.wavelength_nm, Some(280));
}

#[test]
#[ignore = "design.md §15 box 1: the wavelengths above are correct only because \
            this run happens to match DEFAULT_WAVELENGTHS. The parser reports \
            'no wavelength mapping found in the method XML' and falls back to \
            positional order — it never finds the <Wavelength1..4> elements that \
            are demonstrably present. Point wavelength_for at them, remembering \
            the 1-based/0-based offset."]
fn wavelengths_are_read_from_the_method_not_assumed_from_order() {
    let run = fixture();
    assert!(
        !warned(&run, "no wavelength mapping found"),
        "the method declares all four wavelengths; they must not be assumed"
    );
}

// ---------------------------------------------------------------------------
// Box 3 — duplicate Trace_Fractions_* entries
// ---------------------------------------------------------------------------

#[test]
fn all_seventy_five_fractions_are_recovered() {
    // The archive holds two fraction traces. Trace_Fractions_1.xml carries 150
    // records (75 FractionStart + 75 FractionDone); Trace_Fractions_19.xml is
    // an empty <Node />. Reconciliation must take the populated one.
    let run = fixture();
    assert_eq!(run.fractions.len(), 75);
    let tubes: Vec<u32> = run.fractions.iter().map(|f| f.tube).collect();
    assert_eq!(*tubes.iter().min().expect("min"), 1);
    assert_eq!(*tubes.iter().max().expect("max"), 75);
}

#[test]
fn no_fraction_boundary_is_inferred() {
    // Every fraction has a matching FractionDone, so every end is measured.
    // `end_estimated` decides which duplicate wins reconciliation and makes the
    // plate mark a window provisional — reporting a measured edge as inferred
    // would be a quiet lie about data quality.
    let run = fixture();
    let inferred: Vec<u32> = run
        .fractions
        .iter()
        .filter(|f| f.end_estimated)
        .map(|f| f.tube)
        .collect();
    assert!(
        inferred.is_empty(),
        "tubes wrongly marked inferred: {inferred:?}"
    );
}

#[test]
fn fraction_windows_tile_forward_without_overlapping() {
    // Overlapping windows would put one elution volume in two wells at once,
    // which is what the Results "Fractions" column reads to decide which tubes
    // a peak spans.
    let run = fixture();
    let mut sorted = run.fractions.clone();
    sorted.sort_by(|a, b| a.vol_start_ml.total_cmp(&b.vol_start_ml));
    for pair in sorted.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(
            a.vol_end_ml <= b.vol_start_ml + 1e-3,
            "tube {} ends at {} but tube {} starts at {}",
            a.tube,
            a.vol_end_ml,
            b.tube,
            b.vol_start_ml
        );
    }
}

#[test]
fn the_first_fraction_starts_where_the_method_says() {
    let run = fixture();
    let first = run.fractions.iter().find(|f| f.tube == 1).expect("tube 1");
    assert!(
        (first.vol_start_ml - 6.997_868).abs() < 1e-3,
        "{}",
        first.vol_start_ml
    );
    assert!(
        (first.vol_end_ml - 7.394_189).abs() < 1e-3,
        "{}",
        first.vol_end_ml
    );
    assert_eq!(first.nominal_size_ml, Some(0.4));
    // Collection is configured at 0.4 mL and the measured window agrees to
    // within a sample interval, so the collector tracked its setpoint.
    let measured = first.vol_end_ml - first.vol_start_ml;
    assert!(
        (measured - 0.4).abs() < 0.01,
        "measured window {measured} mL"
    );
}

#[test]
#[ignore = "design.md §15 box 3: the empty second fraction trace is normal, not \
            broken. Trace_Fractions_19.xml contains <RootNodeOfCFCData><Node /> \
            </RootNodeOfCFCData> and the parser raises 'malformed fraction \
            record'. Reconciliation should treat an empty companion trace as \
            benign and warn only when both are populated and disagree."]
fn an_empty_companion_fraction_trace_is_not_reported_as_malformed() {
    let run = fixture();
    assert!(
        !warned(&run, "no <CFCData> records found"),
        "an empty duplicate trace is expected in this format, not an error"
    );
}

// ---------------------------------------------------------------------------
// Box 6 — rack geometry and collection pattern
// ---------------------------------------------------------------------------

#[test]
fn every_fraction_resolves_to_a_plate_position() {
    // An unresolved well is not a crash — the tube number is still right — but
    // the Results "Fractions" column shows well labels, so an unrecognised rack
    // silently degrades a user-visible answer.
    let run = fixture();
    let unresolved: Vec<u32> = run
        .fractions
        .iter()
        .filter(|f| f.well.is_none())
        .map(|f| f.tube)
        .collect();
    assert!(
        unresolved.is_empty(),
        "tubes without a plate position: {unresolved:?}"
    );
}

#[test]
fn the_rack_is_a_serpentine_hep96() {
    // Unanimous across all 150 fraction records. FractionCollectorType is
    // "Hawkeye"; the model does not carry that field, so it is recorded in
    // docs/format-findings.md rather than asserted here.
    let run = fixture();
    for f in &run.fractions {
        assert_eq!(f.rack_type, "HEP96", "tube {}", f.tube);
        assert_eq!(f.pattern, "Serpentine", "tube {}", f.tube);
        assert_eq!(f.rack, 1, "tube {}", f.tube);
    }
    assert!(
        !warned(&run, "assumed serpentine"),
        "the archive declares the pattern; it must not be assumed"
    );
}

#[test]
fn serpentine_numbering_places_the_first_row_left_to_right() {
    // Tubes 1-12 fill row A left to right; tube 13 starts row B from the right.
    // This is the boustrophedon rule in `wells.rs`, checked against a real
    // collector rather than against our own encoder.
    let run = fixture();
    let well_of = |tube: u32| {
        run.fractions
            .iter()
            .find(|f| f.tube == tube)
            .and_then(|f| f.well)
            .map(|w| w.label())
            .unwrap_or_else(|| panic!("tube {tube} has no well"))
    };
    assert_eq!(well_of(1), "A1");
    assert_eq!(well_of(12), "A12");
    assert_eq!(well_of(13), "B12", "odd rows run right to left");
    assert_eq!(well_of(24), "B1");
    assert_eq!(well_of(25), "C1");
}

// ---------------------------------------------------------------------------
// Box 4 — V0/Vt for Kav-based SEC calibration
// ---------------------------------------------------------------------------

#[test]
fn the_method_does_not_supply_column_volumes() {
    // Searched Methods/MethodData1.xml for Void, V0, Vt, BedVolume and
    // TotalVolume: none exists. Kav-based fitting therefore cannot be
    // automatic, and the volume-based fallback is the correct default.
    // This asserts a *negative* finding so that a future archive which does
    // carry them announces itself by failing here.
    let run = fixture();
    assert_eq!(run.meta.v0_ml, None);
    assert_eq!(run.meta.vt_ml, None);
}

#[test]
#[ignore = "design.md §15 box 4: the column is identified but not read. \
            Methods/MethodData1.xml carries <ColumnType>Superdex 200 10/300 GL \
            </ColumnType> and RunMeta.column already exists, unpopulated. A \
            named column has published V0/Vt (≈8 mL void, ≈24 mL bed), so this \
            could offer values for the user to confirm — offered and labelled, \
            never silently assumed."]
fn the_column_identity_is_read_from_the_method() {
    let run = fixture();
    assert_eq!(run.meta.column.as_deref(), Some("Superdex 200 10/300 GL"));
}

// ---------------------------------------------------------------------------
// Box 5 — path length and extinction coefficient
// ---------------------------------------------------------------------------

#[test]
#[ignore = "design.md §15 box 5: Analysis.xml records <PathLength>0.5</PathLength> \
            on all 58 peaks, but nothing reads it and RunMeta.path_length_cm stays \
            None — so the UI falls back to ConcentrationInputs::default(), which \
            is 0.2 cm and wrong for this instrument by a factor of 2.5, straight \
            into the concentration estimate. Note Analysis.xml exists only in \
            .ngcAnalysis exports; .ngcMethodruns may not carry it."]
fn the_flow_cell_path_length_is_read_from_the_archive() {
    let run = fixture();
    let path = run.meta.path_length_cm.expect("path length");
    assert!((path - 0.5).abs() < 1e-6, "expected 0.5 cm, got {path}");
}

#[test]
fn the_archive_does_not_supply_an_extinction_coefficient() {
    // <ExtinctionCoefficient xsi:nil="true" /> on all 58 ChromLab peaks. ε is a
    // property of the molecule, not of the run, so keeping it a manual input is
    // correct rather than a gap. Asserted as a negative so that an archive that
    // *does* carry one announces itself.
    //
    // `RunMeta` has no extinction field at all, which is the strongest possible
    // statement of that policy; this test documents the reasoning.
    let run = fixture();
    assert!(
        run.meta.path_length_cm.is_none() || run.meta.path_length_cm == Some(0.5),
        "only the path length may ever come from the archive"
    );
}

// ---------------------------------------------------------------------------
// Warning discipline
// ---------------------------------------------------------------------------

#[test]
fn the_review_panel_reports_every_assumption_still_being_made() {
    // The Overview → Review required panel is the contract with the user: any
    // guess the parser makes has to appear there. This pins the current set, so
    // that closing a box *removes* a warning visibly and adding a new guess
    // without surfacing it fails the build.
    //
    // Expected today: one wavelength fallback, four UV magnitude guesses, and
    // one spurious "malformed" for the empty companion fraction trace. Each has
    // an #[ignore]d test above naming the fix.
    let run = fixture();
    assert_eq!(
        run.warnings.len(),
        6,
        "unexpected warning set:\n{}",
        run.warnings
            .iter()
            .map(|w| format!("  [{}] {}", w.scope, w.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

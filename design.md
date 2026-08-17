# EluSive — Design Document

> **EluSive** — *Precise analysis. Invisible by design.*
> Brand pillars: **Precise · Reliable · Pure · Invisible.**

A cross-platform (Windows + macOS) desktop viewer and light analysis tool for
Bio-Rad NGC / ChromLab chromatography runs. Opens exported runs, plots the traces,
shows fraction collection on a 96-well plate, supports manual peak integration, and
derives size / concentration from a standard run. Crates: `elusive-core` (logic) and
`elusive-app` (UI, binary `elusive`). Visual identity in §16.

Status: **design draft** (v0.1). Data formats below are verified against real
exported files (see §3). Everything else is proposed and open to iteration.

---

## 1. Goals and non-goals

### Goals
1. **Open a run offline, standalone.** No ChromLab, no network, no instrument.
   Grab one file from the cold-room instrument PC onto a USB stick, open it warm
   at a desk on Windows or Mac.
2. **Visualize** all channels (UV 215/255/280/495, conductivity, %B, pH,
   pressures, flow, temperature) on a shared volume axis.
3. **Show fractions where they were dispensed**, including a 96-well plate view
   whose wells are colored by a live-selectable channel + metric.
4. **Manual peak integration** with selectable baseline modes, reporting area,
   height, retention volume and width.
5. **Size / concentration** from a standard run: build a SEC MW calibration curve
   from a Bio-Rad gel-filtration standard and apply it to sample peaks; estimate
   concentration from A280.

### Non-goals (for now)
- No instrument control, no acquisition, no method editing.
- No automatic peak *detection*. Integration is manual by design; auto-picking
  can be a later addition but is explicitly out of scope for v1.
- No writing back into ChromLab's native files. We read them; we never mutate
  them. Our own annotations live in a sidecar (§12).

---

## 2. Context and the "why" behind native-first

The instrument is piloted by ChromLab on an old Windows Vista box in a cold room,
not on the network, so remote analysis is impossible — today you analyze standing
in the cold. The whole point of this tool is to move analysis off that machine.

ChromLab can export a run three ways that matter to us:
- **`.ngcAnalysis` / `.ngcMethodruns`** — the native archive. *Single self-contained
  file* that contains traces **and** fractions **and** method **and** metadata.
- **"Analysis" CSV** — trace data as text, but **no fractions**.
- **"Traces" CSV** — just the channel legend (names, units, colors, min/max).

Because the plate/fraction view *requires* fraction data, and because a single
file is the friendliest thing to copy onto a USB stick, the **native archive is
the primary input format**. CSV import is a secondary convenience for runs where
that's all that's available. This reverses the earlier "CSV now, native later"
plan — justified because the native format turned out to be fully decodable (§3).

---

## 3. File format reference (verified)

Both `.ngcAnalysis` and `.ngcMethodruns` are **ZIP archives**. Layout (from a real
Superdex SEC run, ChromLab export):

```
Version.txt                              (methodruns only)
Methods/MethodData1.xml                  method definition (incl. Wavelength1..4, ColumnType)
Methods/MethodInfo1.xml                  method info
Runs/Run1.xml                            run identity
Runs/RunInfo1.xml                        run metadata
Runs/AnalysisRunViewSettings1.xml        ChromLab's saved view state
Runs/Run1/Trace_<Name>_<idx>.xml         one XML per channel/trace (see below)
Analysis.xml                             (analysis export only) ChromLab's own peak results
```

*Corrected 2026-08-02 against a real archive.* The earlier listing said
`Method/MethodData.xml` — the directory is **plural** and the files are
**numbered**, and `AnalysisRunViewSettings1.xml` was missing entirely.

Trace file names seen: `MWave0..3` (the four UV wavelengths), `MD_Conductivity`,
`PercentB`, `ModulePH`, `MD_Temperature`, `FlowRate`, `SamplePumpFlowRate`,
several pressure channels, `Fractions` (x2), `NextGenEvents` (logbook),
`Annotations`.

The trailing `_<idx>` in a trace file name is **not** the wavelength index and
carries no meaning worth relying on: this run has `Trace_MWave0_8.xml`,
`Trace_MWave1_2.xml`, `Trace_MWave2_14.xml`, `Trace_MWave3_4.xml`.

`Analysis.xml` was previously described only as "integration/analysis state". It
is the largest entry (6.1 MB here) and holds ChromLab's **own 58 peak records** —
area, height, FWHM, baseline endpoints, asymmetry, path length — grouped into
`<AlgorithmParameters>` blocks whose `<RunDataId>` matches a trace's
`<OriginalRunDataId>`. Every value has a `Raw` twin, which is *not* a unit
conversion but a decimated-versus-full-rate pair (index 5 vs 84, 23 vs 376, a
consistent ~16×). See `docs/format-findings.md`.

### 3.1 Signal traces (MWave*, Conductivity, pH, pressures, …)

Each trace XML has a `<TraceData>` element containing **base64**. Decoded:

```
offset 0   : u32 little-endian  version  (observed value = 1)
offset 4   : N records, each 3 × f32 little-endian:
             [ time_seconds , value , volume_mL ]
```

- `time_seconds` — monotonic, identical across channels (0 … run length; e.g. 4551 s).
- `volume_mL`   — monotonic, identical time-base (0 … total volume; e.g. 37.9 mL).
- `value`       — the per-channel measurement in that channel's native unit.

**Verification performed:** conductivity decoded to 17.0–18.0 mS/cm and pH to
8.05–8.29, matching the CSV export of the same run. Record count is
`(len(blob) - 4) / 12`.

**Watch-outs (verify in implementation):**
- Channels have **independent sample counts** (pH was sampled at ~2× the UV rate).
  Never assume a shared index; each `Channel` owns its own sample vector.
- UV `value` appears to be in **AU**, while ChromLab displays **mAU** (×1000).
  Confirm the scale per channel from the legend/units and store a display scale.
- Confirm the `MWave0..3` → wavelength (215/255/280/495) mapping from
  `Method/MethodData.xml` or `RunInfo1.xml`; do not hard-code the order.

### 3.2 Fractions (`Trace_Fractions_*.xml`)

`<TraceData>` is **base64 of an inner XML** document (`RootNodeOfCFCData`),
a sequence of `<CFCData>` records. Key fields per record:

| Field | Meaning |
|---|---|
| `Event` | `FractionStart` / `FractionDone` |
| `TubeNumber` / `TubeNumberNotMinusOne` | 1-based fraction/well number |
| `RackNumber` | rack index |
| `VolumeStartSec` / `VolumeEndSec` | fraction start/end **in mL** (name is a misnomer) |
| `TimeStartSec` / `TimeEndSec` | fraction start/end in seconds |
| `FractionSize` | nominal size (mL), e.g. 0.4 |
| `RackType` | e.g. `HEP96` (96-well plate) |
| `CollectionPattern` | e.g. `Serpentine` |
| `FractionCollectorType` | e.g. `Hawkeye` |

This is everything needed to place each fraction on the plate and to define the
volume window used for per-well metrics (§8, §9).

### 3.3 CSV formats (secondary import)

- **Analysis CSV**: row 1 = run name repeated; row 2 = paired headers
  `"<Channel>_volume","<Channel>_<unit>"`; then data rows. Each channel is an
  **independent (volume, value) series** — columns are not row-aligned across
  channels; trailing cells go empty when a channel has fewer samples. Includes
  ChromLab's own `Baseline of UV*` columns (reference only; we compute our own).
  Encoding is ISO-8859-1 with mixed CRLF/LF — decode as latin-1, not UTF-8.
- **Traces CSV**: the channel legend — `Show, Type, Color (#AARRGGBB), Min Y,
  Max Y, Units, Method, Start Time, End Time, Technique`. Use it to seed
  channel display colors so we match ChromLab's palette.

---

## 4. Architecture

Two crates in one Cargo workspace, matching the ClonoDoc core+UI split:

```
elusive/
  elusive-core/     pure logic, zero UI — parsing, model, integration, calibration
  elusive-app/  eframe/egui desktop app on top of elusive-core (binary: `elusive`)
```

**Why the split.** All format risk and all math live in `elusive-core` and are unit-
testable headless. The UI can't hide a parsing bug. The same core can later back a
CLI or a batch pipeline (e.g. feeding your Claude Code analysis) without dragging
egui along.

---

## 5. Data model (`elusive-core::model`)

See `elusive-core/src/model.rs` for the concrete Rust. Summary:

- `Run { meta, channels: Vec<Channel>, fractions: Vec<Fraction>, events: Vec<LogEvent> }`
- `Channel { id, name, unit, kind, color, display_scale, samples: Vec<Sample> }`
- `Sample { time_s: f32, volume_ml: f32, value: f32 }`
- `Fraction { tube: u32, rack: u32, well: Well, vol_start_ml, vol_end_ml, time_start_s, time_end_s, rack_type, pattern }`
- `Well { row: u8, col: u8 }` (0-based; A1 = {0,0})
- `RunMeta { run_name, method_name, technique, started, ended, column }`
- `PeakResult { channel_id, v_start, v_end, baseline, area, height, apex_volume, fwhm }`

Design choices:
- Store the full `(time, value, volume)` triplet per sample even though plots key
  off volume — keeps time-domain analysis and round-tripping possible.
- `ChannelKind` enum (Uv, Conductivity, PercentB, Ph, Pressure, Flow, Temperature,
  Other) drives default axis grouping and units.

---

## 6. Parsers (`elusive-core::parse`)

### `parse::ngc` (primary)
1. Open ZIP.
2. Read `RunInfo1.xml` / `Run1.xml` → `RunMeta`; `MethodInfo`/`MethodData` for
   wavelength mapping and column info.
3. For each `Trace_*.xml`: base64-decode `<TraceData>`; if it's a signal trace,
   parse the binary triplets (§3.1) into `Channel.samples`; if it's the fractions
   trace, base64→inner XML→`Fraction` records (§3.2).
4. Deduplicate the two `Trace_Fractions_*` files (one is the full stream, one is a
   short summary — reconcile by tube number).

### `parse::csv` (secondary)
- Parse Analysis CSV into channels (latin-1; split paired columns; trim trailing
  empties per channel). Parse Traces CSV into a legend and merge colors/units.
- Fractions are absent from CSV — a CSV-only run simply has no plate view, and the
  UI should say so rather than show an empty plate.

Both parsers return the same `Run`, so everything downstream is format-agnostic.

---

## 7. Manual peak integration (`elusive-core::integrate`)

Input: a channel, a volume window `[v0, v1]`, a baseline mode. Output: `PeakResult`.

Baseline modes:
- **Drop-to-zero** — baseline y = 0.
- **Linear (endpoints)** — straight line between the signal at `v0` and `v1`.
- **Valley-to-valley** — linear between two user-chosen valley points (generalizes
  the endpoint mode; the UI supplies the valley volumes).

Computation:
- Restrict samples to `[v0, v1]`.
- `area = Σ trapezoid( (value - baseline) )` integrated over **volume** (units:
  signal-unit × mL, e.g. mAU·mL).
- `height = max(value - baseline)` over the window.
- `apex_volume = volume at that max`.
- `fwhm` = width where `(value - baseline) = height/2`, via linear interpolation on
  each side of the apex.

All pure functions over slices; property tests: area of a synthetic triangle/
Gaussian matches analytic value within tolerance; empty/edge windows handled.

---

## 8. Fraction → well mapping (`elusive-core::wells`)

Given `tube` (1-based), `rack_type` (`HEP96` = 8 rows × 12 cols) and
`CollectionPattern`:
- **Serpentine**: row-major but every other row reversed (boustrophedon). Row =
  `(tube-1) / cols`; position in row `p = (tube-1) % cols`; `col = p` on even rows,
  `cols-1-p` on odd rows.
- Provide a `pattern` trait/enum so `Columns`/`Rows`/other patterns can be added.
- Unit-test the first ~20 tube numbers against a hand-computed serpentine grid.

Each fraction thus gets a `Well{row,col}` and a volume window `[vol_start, vol_end]`
used for per-well metrics.

---

## 9. 96-well plate view + live picker

A plate is 8×12 cells (A–H × 1–12). For each fraction/well, compute a scalar from:
- **Channel** — user-selected (any signal channel; default UV 280).
- **Metric** — user-selected, computed over the well's volume window:
  - Integrated area (∫ value dV)
  - Max value
  - Mean value
  - Value at window center
- Color-map the scalar (sequential map; shared scale across wells; legend shown).

Live picker: two dropdowns (channel, metric) recompute all wells instantly.
This is the "heatmap matching the peaks" — with UV 280 + integrated area it lights
up exactly the wells under each eluting peak.

**Linking (the payoff of one pane):**
- Hover a well → highlight its `[vol_start, vol_end]` band on the chromatogram.
- Hover / drag a region on the chromatogram → highlight the wells it spans.
- Select a peak (§7) → outline all wells overlapping that peak's window.

---

## 10. Size and concentration (`elusive-core::calibration`)

### Size (SEC MW calibration)
- Load a **standard run**. Confirmed standard: **Bio-Rad Gel Filtration Standard,
  Cat# 1511901** — five markers eluting largest→smallest (A thyroglobulin 670 kDa,
  B γ-globulin 158 kDa, C ovalbumin 44 kDa, D myoglobin 17 kDa, E vitamin B12
  1.35 kDa). Encoded in `calibration::BIORAD_GFS` with per-vial amounts.
- User assigns each standard peak's elution volume (pick apexes, or reuse §7). The
  UI can pre-fill A..E by descending elution volume since order is known; myoglobin
  (brown) and B12 (pink) are colored and often clearest on UV4 (495 nm).
- Fit `log10(MW)` vs elution volume, or vs `Kav = (Ve - V0)/(Vt - V0)` if V0/Vt are
  provided (preferred — column-geometry independent).
- Persist the calibration; apply it to sample-run peak apexes → estimated MW.

### Concentration
- Separate from size. Beer–Lambert on A280: `c = A / (ε · l)`.
- Inputs: extinction coefficient ε (per mg/mL or per M), path length l (UV cell,
  from method/hardware), and either peak-apex absorbance or a concentration from
  integrated area vs a known standard.
- Keep the two clearly distinct in the UI; they answer different questions.

---

## 11. UI and layout (`elusive-app`)

**eframe/egui**, native on Windows/macOS/Linux from one codebase, styled with the
EluSive theme (§16, `elusive-app/src/theme.rs`).

Layout follows the brand mockup, adapted to EluSive's prep-chromatography domain:
- **Dark navy sidebar** (`INK_900`) with the wordmark + tagline and the nav sections:
  Overview · Chromatograms · Peaks · Calibration · Results · Reports.
  Settings/Help pinned at the bottom.
- **Light content area** with white cards (dark mode: deep-navy cards). On big
  screens, a **single linked pane**: chromatogram (top, most height) + HEP96 plate
  (bottom); plate poppable to its own window for multi-monitor.
- **Right rail**: peak-detail card (RT/elution vol, area, height, area %, width) and
  a peak-shape mini-view — same information architecture as the mockup, in volume
  units and with our fraction/plate additions.
- **Themes**: EluSive dark (default) and light, toggle + "follow OS". Colors come from the
  design-system tokens (via `theme.rs` / `egui_adapter.rs`), not ad-hoc per widget.
- Chromatogram interactions: pan/zoom, multi-y-axis grouping (UV vs conductivity vs
  pressure), drag-to-integrate (§7), hover-to-link (§9). Default UV trace uses
  `chart::PRIMARY_TRACE`; additional channels cycle `chart::SERIES` with dash
  patterns past eight (DESIGN_SYSTEM.md §10.4), or their ChromLab legend color when
  it meets the contrast anchors.
- **Multi-run comparison** (v0.4): further runs open as read-only *overlays* on the
  primary — traces join the axis-group stack behind the primary's, identified by a
  per-run dash pattern plus the run-qualified legend name (never color alone), and
  each run's saved peaks appear side by side in Results. One run stays primary and
  owns integration, calibration, plate, and the sidecar; the comparison set (paths,
  visibility, per-run mL x-offset) persists in the primary's sidecar. Design:
  `docs/superpowers/specs/2026-08-17-multi-run-overlay-design.md`.

**Domain note:** the mockup is a generic HPLC analysis screen (time axis, USP/tailing).
EluSive adopts its *visual language* but keeps the prep-SEC feature set — volume axis,
fractions, and the 96-well plate — which the generic mockup does not show.

---

## 12. Persistence

- Never mutate the source file. Peak selections, integration results, calibrations,
  and view settings save to a **sidecar** `"<run>.elusive.json"` next to the source.
- Human-readable JSON — your annotations stay yours and portable, no app lock-in.
- Export: peaks table → CSV; plate metrics → CSV; optional PNG/SVG of the view.

---

## 13. Cross-platform build

- `cargo build --release` produces a native binary per OS.
- CI: GitHub Actions matrix (windows-latest, macos-latest, ubuntu-latest) →
  artifacts. Consider `cargo-bundle` / `cargo-dist` for `.app` and `.exe`/installer.
- Signing/notarization: macOS notarization and Windows signing are the only fiddly
  parts; unsigned local builds work for personal use, sign later for distribution.
- Dependencies to pin: `eframe`/`egui`, `egui_plot`, `zip`, `quick-xml`,
  `base64`, `serde`/`serde_json`, `rfd` (file dialog). All pure-Rust / permissively
  licensed; no C toolchain needed → clean cross-compiles.

---

## 14. Milestones

- **M0** — Workspace builds; `parse::ngc` opens a `.ngcAnalysis`, prints channel
  names + sample counts + fraction count. (Format is known — this is mechanical.)
- **M1** — Chromatogram plot: multi-channel, legend, show/hide, y-grouping.
- **M2** — Fraction overlay on the trace + fraction table.
- **M3** — 96-well plate, linked hover both directions, live channel/metric picker.
- **M4** — Manual integration tool + peak results table + CSV export.
- **M5** — Sidecar save/load; light/dark polish.
- **M6** — SEC MW calibration from a standard run; concentration from A280.
- **M7** — CSV import path; packaging + CI for Win/Mac.

---

## 15. Open questions / verification checklist

All six format questions were **answered** against a real export on 2026-08-02;
the evidence is in `docs/format-findings.md` and the assertions in
`elusive-core/tests/real_archive.rs`. A box is ticked only when the *parser*
does the right thing, not merely when the format is understood — four are
answered but not yet implemented, and each carries an `#[ignore]`d test naming
the fix. Run `cargo test -p elusive-core --test real_archive -- --ignored`.

- [x] `MWave0..3` → wavelength mapping. **Answered and implemented.**
      `Methods/MethodData1.xml` declares `<Wavelength1..4>` = 215/255/280/495,
      numbered from 1 while traces number from 0, so `Wavelength1` belongs to
      `MWave0`. The mapping code was always right; `is_method_entry` matched only
      `method/` while the archive uses `Methods/`, so the method XML was never
      read. Fixed 2026-08-02.
- [x] UV value scale: AU vs mAU. **Answered and implemented.** Stored in **AU**,
      displayed as **mAU** (×1000). Confirmed twice over — the raw payload peaks
      at 0.22661 and ChromLab's own stored `Height` is 0.227303, the difference
      being a −0.0152 AU baseline. Now applied as a property of the format
      rather than inferred per trace from amplitude: an NGC header declares no
      unit at all, so the old magnitude test ran on every UV trace and would
      have scaled a very dilute run differently from a saturated one. A declared
      unit still wins, and an implausible result is flagged without changing the
      value.
- [x] Reconcile the two `Trace_Fractions_*` files. **Answered and implemented.**
      It is not "full stream vs summary" — `Trace_Fractions_19.xml` is an empty
      `<Node />`. The populated entry wins; all 75 fractions carry a measured
      `FractionDone`, so no boundary is inferred. An empty companion is now
      benign, and a warning fires only if *every* source is empty.
- [x] V0/Vt for Kav-based SEC calibration. **Answered.** V0 is genuinely absent,
      so Kav cannot be automatic and the volume-based fit stays. Vt *is*
      declared — but twice, as `1` and `23.5619449019234` (exactly π·0.5²·30 for
      the declared Superdex 200 10/300 GL). The parser refuses an ambiguous
      value and says so rather than resolving it by document order.
      `<ColumnType>` is now read into `RunMeta.column`.
- [x] Path length + ε for concentration. **Answered and implemented.**
      `Analysis.xml` records `<PathLength>0.5</PathLength>` on all 58 peaks and
      `<ExtinctionCoefficient xsi:nil="true"/>` on all of them, so ε is genuinely
      absent and stays a manual input by design. Path length is now read from the
      run-side leaves; previously the UI fell back to 0.2 cm, wrong for this
      instrument by 2.5× and straight into the concentration estimate. Note
      `Analysis.xml` exists only in `.ngcAnalysis` exports.
- [x] Plate heatmap colormap + colorblind option — resolved in DESIGN_SYSTEM.md §10.3.
- [x] Channel overflow beyond 8 series — resolved in DESIGN_SYSTEM.md §10.4.
- [x] `HEP96` geometry (8×12). **Answered and verified:** all 150 fraction
      records declare `RackType=HEP96`, `CollectionPattern=Serpentine`,
      `FractionCollectorType=Hawkeye`, 0.4 mL fractions, tubes 1–75, and every
      tube resolves to a well (A1…A12, then B12…B1). Whether *other* rack types
      appear in practice is a question about lab workflow, not about the format.

---

## 16. Brand and visual design

**Authority:** `DESIGN_SYSTEM.md` (v1.1.0) is the single source of truth for color,
typography, spacing, components, and the non-negotiable rules; the tokens live in code
in `elusive-app/src/theme.rs` (toolkit-neutral constants) with an `egui_adapter.rs`
mapping. This section only records identity + EluSive-specific notes — it does **not**
re-list tokens, to avoid drift.

**Name & voice.** EluSive — *"Precise analysis. Invisible by design."* A play on
*elute* / *elusive* fitting a size-exclusion tool. Pillars: **Precise, Reliable,
Pure, Invisible**. Voice is concise scientific microcopy (DESIGN_SYSTEM.md §7).

**Logo.** A three-peak chromatogram (center peak tallest) on a quiet baseline, in a
light→blue gradient. `assets/logo.svg`; app icon = mark on an `INK_900` rounded tile;
splash = wordmark + tagline on `INK_950`. NOTE: the current SVGs still use the old
brand-sheet blues and must be retuned to the design-system ramp
(`ICE_100`/`BLUE_300` → `BLUE_500`); the `#1E3A5F` "NAVY" from the sheet is **not** a
design-system token, so the wordmark uses `WHITE` + `BLUE_500`.

**Typography.** Inter for UI, JetBrains Mono for paths/identifiers/exact values; sizes
and weights per DESIGN_SYSTEM.md §2. Tabular numerals for all analytical values. Load
the `.ttf`s from `assets/fonts/` in the egui adapter.

**Iconography.** Thin single-stroke line icons; tint with text tokens
(`text_primary`/`text_secondary`), never a bespoke hex.

**EluSive-specific extensions** (volume axis, fraction bands, 96-well plate colormap,
channel overflow, status-by-shape) are defined in **DESIGN_SYSTEM.md §10** — the plate
`plate` ramp and the trace `SERIES`+dash overflow scheme are the parts unique to this
app. The plate colormap and channel-overflow items in §15 are resolved there.

**Motion.** Loading state = the peak mark at low contrast with deterministic text
("Analyzing 4 of 12 traces"), not a spinner (DESIGN_SYSTEM.md §6). Animation never
delays an analytical action (rule #5).

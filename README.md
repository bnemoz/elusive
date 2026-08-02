# EluSive

> *Precise analysis. Invisible by design.*

A cross-platform desktop application for reviewing Bio-Rad NGC / ChromLab
chromatography runs. Open an exported run offline, inspect every channel, follow
fractions onto a 96-well plate, integrate peaks, and estimate size and
concentration from a standard run.

No instrument, no ChromLab, no network. Copy one file off the cold-room PC and
open it warm at your desk.

EluSive is a released desktop product. It is deliberately offline-first: your run
archive stays on your computer and is never uploaded or modified.

## Install

Grab a build from the [Releases](../../releases) page.

**Windows** — download `elusive-windows-x86_64.zip`, unzip it anywhere, run
`elusive.exe`. Keep the `assets/` folder beside the binary. Builds are unsigned, so
SmartScreen warns on first launch: *More info → Run anyway*.

**macOS** — download the `.tar.gz` for your chip, extract, and run `elusive`.
Unsigned, so the first launch needs *Right-click → Open*, or
`xattr -d com.apple.quarantine elusive`.

**From source** — `cargo run --release -p elusive-app`. Requires Rust 1.92+; on
Linux also `libgtk-3-dev libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev
libxrandr-dev libxi-dev libgl1-mesa-dev`.

## What you can do

Open a `.ngcAnalysis` or `.ngcMethodruns` archive, or drag one onto the window. A
ChromLab "Analysis" CSV also works, but CSV exports carry no fraction records, so
the plate view is unavailable for them and the app says so rather than showing an
empty grid.

- **Overview** — run metadata, channel inventory, fractions, and parser review
  notes in responsive cards you can rearrange.
- **Chromatograms** — linked UV, conductivity, pH, pressure, and other traces;
  switch the x axis between elution volume and time; choose a shared, automatic,
  or custom y scale for each trace; and set a trace colour directly from its
  legend. Hover readouts identify the fraction and integrated peak under the
  cursor.
- **Fractions and plates** — see collected fractions on the plate, hover from
  chart to well (and back), and include the overlapping fractions in Results and
  CSV exports.
- **Peaks** — choose a baseline mode and drag across a peak. Area, height, apex
  volume, FWHM, area percentage, and applicable size/concentration estimates are
  available in Results and Reports.
- **Calibration** — assign the Bio-Rad gel-filtration standards to peak apexes,
  fit against elution volume or Kav, and read R² alongside the curve. Weak fits
  and extrapolated sizes are labelled as such.
- **Reports** — review a readable peak table, export peak and plate-metric CSVs,
  copy the peak table as Markdown, or export the chromatogram exactly as shown to
  a PNG.

Your source archive is **never modified**. Peaks, excluded regions, calibrations,
trace colours, chart scales, panel arrangement, and view settings are written to a
human-readable `<run>.elusive.json` sidecar beside the run and restored when you
open it again.

## Quick start

1. Open a `.ngcAnalysis` or `.ngcMethodruns` archive, or drag it onto the window.
2. Use **Chromatograms** to select traces, adjust scales, and inspect collection.
3. Turn on **Integrate**, choose a baseline, and drag over a peak.
4. Review fractions and calculated values in **Results**, then export from
   **Reports** when you are ready to share.

A ChromLab “Analysis” CSV also opens. CSV exports do not contain fraction records,
so plate and fraction features are shown as unavailable rather than as empty data.

## Interpreting imported runs

ChromLab exports vary by instrument and software version. When an import requires
an assumption, EluSive records it openly in **Overview → Review required**. Common
examples include

- `MWave0..3` → wavelength mapping, when the method XML does not name it
- UV storage scale (AU vs mAU) when the unit is not declared
- which of the two `Trace_Fractions_*` entries is authoritative
- rack geometry for anything other than `HEP96`
- availability of `V0` / `Vt` for Kav-based SEC fits
- extinction coefficient and path length for concentration

If a run shows a review note, verify it against the original method or report. A
saved sidecar never changes the source archive.

## Repository layout

```text
elusive-core/     parsing, model, integration, calibration, sidecar I/O — no UI
elusive-app/      egui/eframe desktop app (binary: elusive)
assets/           brand assets; drop Inter and JetBrains Mono in assets/fonts/
design.md         product and file-format source of truth
DESIGN_SYSTEM.md  visual and interaction source of truth
IMPLEMENTATION_PLAN.md   repository structure, phases, acceptance criteria
```

`elusive-core` must never depend on a UI toolkit — all format risk and all
analysis math live there so they can be tested headless. CI enforces this.

## Development

```bash
cargo build
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets
```

No real instrument export is committed here — those are not ours to redistribute.
The parser is exercised against synthetic archives built to the layout documented
in `design.md` §3 (`elusive-core/tests/ngc_archive.rs`). If you have a run you can
share, drop it in `testdata/` and add a test that opens it.

## Licence

MIT OR Apache-2.0.

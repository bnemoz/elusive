# EluSive

> *Precise analysis. Invisible by design.*

A cross-platform desktop viewer and light analysis tool for Bio-Rad NGC / ChromLab
chromatography runs. Open an exported run offline, plot every channel against
elution volume, see where each fraction landed on a 96-well plate, integrate peaks
by hand, and derive size and concentration from a standard run.

No instrument, no ChromLab, no network. Copy one file off the cold-room PC and
open it warm at your desk.

## Status

Early. The workspace, the native parser, the analysis math, and the desktop UI are
implemented and tested; several file-format details are still assumptions flagged
at runtime rather than confirmed facts — see [Verify against your files](#verify-against-your-files).

## Install

Grab a build from the [Releases](../../releases) page.

**Windows** — download `elusive-windows-x86_64.zip`, unzip it anywhere, run
`elusive.exe`. Keep the `assets/` folder beside the binary. Builds are unsigned, so
SmartScreen warns on first launch: *More info → Run anyway*.

**macOS** — download the `.tar.gz` for your chip, extract, and run `elusive`.
Unsigned, so the first launch needs *Right-click → Open*, or
`xattr -d com.apple.quarantine elusive`.

**From source** — `cargo run --release -p elusive-app`. Requires Rust 1.82+; on
Linux also `libgtk-3-dev libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev
libxrandr-dev libxi-dev libgl1-mesa-dev`.

## Using it

Open a `.ngcAnalysis` or `.ngcMethodruns` archive, or drag one onto the window. A
ChromLab "Analysis" CSV also works, but CSV exports carry no fraction records, so
the plate view is unavailable for them and the app says so rather than showing an
empty grid.

- **Overview** — run metadata, the channel inventory, the fraction table, and a
  *Review required* card listing anything the parser had to assume.
- **Chromatograms** — one plot per axis group (UV, conductivity, pH, pressure…),
  linked on the volume axis so pan and zoom stay in step. Hovering a fraction on
  the trace highlights its well, and hovering a well highlights its span.
- **Peaks** — turn on *Integrate*, pick a baseline mode, and drag
  across a peak. Area, height, apex volume, FWHM and area-% appear in the table
  and the right-hand detail card.
- **Calibration** — assign the Bio-Rad gel-filtration standards to peak apexes,
  fit against elution volume or Kav, and read R² alongside the curve. Weak fits
  and extrapolated sizes are labelled as such.
- **Reports** — CSV export of the peak table and the plate metrics.

Your source archive is **never modified**. Peaks, excluded regions, calibrations
and view settings are written to a human-readable `<run>.elusive.json` sidecar
beside the run, and reloaded automatically the next time you open it.

## Verify against your files

Some format details in `design.md` are decoded but not yet confirmed across real
exports. Where EluSive has to guess, it guesses in the open: the guess and its
reasoning appear in **Overview → Review required**. The open items are

- `MWave0..3` → wavelength mapping, when the method XML does not name it
- UV storage scale (AU vs mAU) when the unit is not declared
- which of the two `Trace_Fractions_*` entries is authoritative
- rack geometry for anything other than `HEP96`
- availability of `V0` / `Vt` for Kav-based SEC fits
- extinction coefficient and path length for concentration

If your runs produce warnings here, that output is the most useful bug report you
can file.

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

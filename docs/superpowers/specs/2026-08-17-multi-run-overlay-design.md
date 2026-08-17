# Multi-run overlay — design

Date: 2026-08-17
Status: approved design, pre-implementation
Branch: `feat/multi-run-overlay`

## Problem

A user wants to open several runs at once and compare them on one plot —
different productions of the same protein, checked against each other for
yield (peak area), conformation (peak shape), and size (elution position /
estimated MW). Today EluSive holds exactly one `Run`; comparing two
productions means two app windows and a ruler.

## Scope decisions (made with the user)

- **Overlay + peak comparison.** Traces from several runs plot on the shared
  chromatogram, and each run's saved peak results appear side by side in
  Results. Editing (integration, calibration, plate) stays on one run at a
  time.
- **Primary + read-only overlays.** The run opened first behaves exactly as
  today and owns the sidecar, the plate, and all editing. Additional runs are
  comparison references. Equal-peers editing is explicitly out of scope.
- **Raw shared volume axis by default** — same instrument and column is the
  assumed common case. Optional, off by default: per-trace y-normalization
  (for shape comparison across concentrations) and a per-run x-offset in mL
  (for small system-volume differences).

## Approach

Approach A of the three considered: overlays live in the app beside the
existing `run`, as fully parsed `Run`s wrapped in display settings.
`elusive-core`'s model, parsers, and analysis stay untouched; the only core
change is a tolerant sidecar `ViewState` addition. Rejected alternatives: a
multi-run `Session` in core (large diff for flexibility the chosen run model
deliberately defers) and merging overlay channels into the primary `Run`
under namespaced ids (pollutes the model, lets the primary's sidecar acquire
peaks keyed to other files' channels).

## 1. Data model and loading

New app-side module `elusive-app/src/overlay.rs`:

```rust
pub struct Overlay {
    pub run: Run,                        // fully parsed, immutable, read-only
    pub source_path: PathBuf,
    pub peaks: Vec<PeakResult>,          // from that run's own .elusive.json, if present
    pub visible: bool,                   // master toggle
    pub hidden_channels: BTreeSet<ChannelId>,
    pub x_offset_ml: f32,                // default 0.0
}
```

`EluSiveApp` gains `overlays: Vec<Overlay>` beside `run: Option<Run>`.
Loading reuses `parse::open` unchanged. An overlay's own sidecar is loaded
read-only for its peaks and calibration results and is **never written** by
the comparison session.

Entry points:

- an "Add comparison…" button in the chromatogram legend;
- dropping a file while a run is already open prompts
  "Replace run / Add as comparison" (today a drop replaces silently).

CSV runs are valid overlays — traces are all a comparison needs; the plate
and fraction features already degrade per source format.

**Default channel visibility.** An overlay channel starts visible iff it
matches a currently visible primary channel: same `ChannelKind`, and same
`wavelength_nm` for UV. Everything else starts hidden. Comparing UV 280
across five productions must not require deselecting dozens of channels.

## 2. Chart rendering and alignment

Overlay traces join the existing axis-group stack — UV plots with UV,
conductivity with conductivity — and draw **before** the primary's traces, so
the primary stays on top (the raw-trace-visibility rule extends naturally:
the primary's raw trace stays the topmost signal).

**Identity is never color alone** (design-system rule #3): overlay run *k*
renders with dash pattern *k* from the §10.4 dash vocabulary, and hover and
legend text carry the run name, e.g. "2026-08-02 prep · UV 280 nm".

- **Raw shared mL axis is the default.** Traces plot exactly as stored.
- **Y-normalization** is delivered by the existing per-trace relative
  y-scale mode (`YScaleMode`, "auto-each"), extended to cover overlay
  traces. No new mode is added; the standing "heights are not comparable"
  caveat note already carries the honesty requirement, including into PNG
  exports.
- **Per-run x-offset** applies as `volume_ml + x_offset_ml` at display time
  only; nothing stored or computed ever includes it. Editable from the
  overlay's legend row. In Time-axis mode offsets are **ignored** — a mL
  offset has no constant time equivalent under gradient flow — and a
  micro-note states so whenever any offset is nonzero.

Unchanged, deliberately: integration dragging targets the primary only;
fraction bands, plate, and hover-to-well linking remain primary-only.

## 3. Legend and overlay management

The legend gains one group per overlay: a header row with the run name, a
master show/hide toggle, the x-offset value (drag-value in mL), and a remove
button; beneath it, the overlay's channels reuse the existing per-channel
legend rows (visibility toggle, color swatch, per-trace range when the
relative mode is active). Removing an overlay drops it from the vec; nothing
on disk changes.

Overlay color swatches are **display-only** in this version. The primary's
user color overrides are keyed by `ChannelId`, which is a per-run string
(`"MWave2"` exists in every run), so extending overrides to overlays needs a
(run, channel) key and a second sidecar representation — deferred until
someone asks. Overlay trace colors follow the same automatic resolution as
the primary (archive legend color, else series cycling), with the dash
pattern carrying run identity.

## 4. Peak comparison (Results)

When at least one overlay is open, Results gains a **Run comparison** table:
the primary's live peaks plus each overlay's sidecar peaks, grouped by run.

Columns: run, channel, apex (mL), area, height, FWHM, area-% (computed
within that peak's own run and channel), estimated MW (kDa) when the run's
saved analysis carries one.

Overlay rows are read-only. An overlay without a sidecar contributes an
explicit "no saved analysis for this run" row rather than silently nothing.

Export: a new `comparison.csv` whose schema is the existing peak-table
schema with a leading `run` column. The existing single-run peak CSV schema
is untouched.

## 5. Persistence

The primary's sidecar `ViewState` (core `sidecar.rs`) gains one field, with
the same tolerance conventions as its neighbours (`#[serde(default)]`,
absent-vs-empty distinction preserved via `Option`):

```json
"overlays": [
  { "path": "../std-run.ngcAnalysis",
    "visible": true,
    "x_offset_ml": 0.0,
    "hidden_channels": ["MWave3"] }
]
```

- Paths are stored **relative to the primary's directory** when possible
  (runs travel together on a USB stick), absolute as a fallback.
- On reopen, a missing or unparseable overlay file degrades to a status
  message; it never fails the sidecar load or the run open.
- Sidecars written before this field load exactly as today.
- The sidecar schema version does not change: old builds ignore the unknown
  field, and this build treats its absence as "no overlays".

## 6. Errors and edge cases

- Overlay parse failure → status message; the overlay list is unchanged.
- Adding a file already open (same canonicalized path, primary or overlay)
  → no-op with a status message.
- Closing or replacing the primary clears all overlays.
- Runs with different volume ranges → the plot takes the union, as
  `Run::volume_range` already does across channels.
- Memory is a non-concern at this scale (a full 16-channel SEC run is a few
  MB parsed); no lazy loading.

## 7. Testing

Headless unit tests for every pure decision, in the style the app crate
already uses for overview layout arithmetic:

- default-visibility matching (kind + wavelength, UV vs non-UV);
- dash-pattern assignment per overlay index;
- x-offset application and its Time-axis suppression;
- comparison-row assembly: grouping, ordering, per-run area-% math,
  "no saved analysis" placeholder;
- relative-vs-absolute path resolution for sidecar overlay refs.

Core tests, matching existing sidecar tests: round-trip with the new field;
a legacy sidecar without it loads with `overlays: None`; a sidecar with
unknown extra fields still loads.

Manual smoke: open `testdata/sec-run.ngcAnalysis` as primary and add the
same file as an overlay — traces must coincide exactly at offset 0, shift by
exactly the offset otherwise, and the comparison table must show both runs.

## Files touched

- `elusive-core/src/sidecar.rs` — `ViewState.overlays` + tests
- `elusive-app/src/overlay.rs` — new: `Overlay`, matching/assignment logic
- `elusive-app/src/app.rs` — overlay vec, open/drop/replace flow, status
- `elusive-app/src/view.rs` — persistence glue for overlay view state
- `elusive-app/src/widgets/chromatogram.rs` — overlay trace rendering,
  offset, hover naming
- `elusive-app/src/widgets/panels.rs` — legend groups, comparison table,
  `comparison.csv` export
- `design.md` / `README.md` — feature notes

## Out of scope (recorded so they stay deliberate)

- Editing an overlay's analysis; switching which run is primary.
- Overlay fractions or plates on screen.
- Normalization to injection amount (needs metadata the format may not
  carry; revisit with a real request).
- Importing ChromLab's own peaks from `Analysis.xml` as comparison rows —
  noted in `docs/format-findings.md` as a separate feature.

# EluSive Implementation Plan

This file is the working implementation plan for bringing the repository into line
with the product design in `design.md` and the visual rules in `DESIGN_SYSTEM.md`.

It does not replace either document:

- `design.md` remains the product and format-spec source of truth.
- `DESIGN_SYSTEM.md` remains the visual and interaction-system source of truth.
- This file defines repository structure, engineering milestones, acceptance
  criteria, and the unresolved details that must be nailed down before the app is
  considered reliable.

## 1. Current state

The repository now matches §2. The workspace, both crates, the module tree, the
tests, and CI all exist; the duplicate theme implementation has been removed and
the brand assets moved under `assets/`.

Phases 0–8 below have a working first implementation. What is *not* done is
verification against real instrument exports: every format assumption the parser
makes is surfaced at runtime as a `model::Warning` and shown in the app's
Overview → Review required panel, rather than being silently baked in. §6's
data-format questions stay open until a real file settles them.

Two deviations from the module list in §2, both deliberate:

- `elusive-core/src/parse/xml.rs` — a name-based XML flattener. `design.md` §3
  documents the trace payloads exactly but not the wrapper elements, so locating
  `TraceData` and `CFCData` by name is the only approach that does not encode a
  guessed hierarchy.
- `elusive-app/src/view.rs` — all mutable UI state, separated from the loaded
  `Run` so widgets can borrow `&Run` and `&mut View` at once. This is also where
  the Phase 4 shared hover state (`hovered_vol_range`, `hovered_well`) lives.

Still absent: a redistributable sample archive in `testdata/`, and the Inter /
JetBrains Mono font files (see `assets/fonts/README.md`).

## 2. Target repository layout

The repository should move to a workspace layout like this:

```text
EluSive/
  Cargo.toml
  Cargo.lock
  design.md
  DESIGN_SYSTEM.md
  IMPLEMENTATION_PLAN.md
  CLAUDE.md
  assets/
    app-icon.svg
    logo.svg
    fonts/
  elusive-core/
    Cargo.toml
    src/
      lib.rs
      error.rs
      model.rs
      parse/
        mod.rs
        ngc.rs
        csv.rs
      integrate.rs
      calibration.rs
      sidecar.rs
      wells.rs
    tests/
  elusive-app/
    Cargo.toml
    src/
      main.rs
      app.rs
      theme.rs
      egui_adapter.rs
      widgets/
  testdata/          # only if legal and practical for the sample files
  .github/
    workflows/
```

Notes:

- Keep `design.md` and `DESIGN_SYSTEM.md` at the repo root unless a later cleanup
  moves all docs into a `docs/` directory consistently.
- Keep one canonical theme implementation only.
- `CLAUDE.md` should be reduced to a concise implementation guide that matches the
  actual repository state.

## 3. Documentation responsibilities

The documents should have explicit boundaries:

### `design.md`

Owns:

- product goals and non-goals
- verified file format knowledge
- domain model
- UX flows
- milestone intent

Should not pretend code exists when it does not.

### `DESIGN_SYSTEM.md`

Owns:

- brand tokens
- visual hierarchy
- chart and plate display rules
- accessibility and interaction constraints

Should remain independent of toolkit-specific implementation details except where
adapter guidance is useful.

### `IMPLEMENTATION_PLAN.md`

Owns:

- repository layout
- engineering phases
- acceptance criteria
- unresolved technical questions
- testing, CI, packaging, and sidecar plans
- reconciled implementation decisions from gap-analysis notes

### `CLAUDE.md`

Owns:

- short implementation guidance for coding agents
- build/test commands
- constraints and invariants

Should not duplicate the full product spec or describe nonexistent files/modules as
already implemented.

## 4. Immediate cleanup decisions

These changes should happen before feature work starts:

1. Create the Cargo workspace.
2. Create `elusive-core` and `elusive-app`.
3. Move the app shell into `elusive-app/src/app.rs`.
4. Create a single canonical theme path:
   - `elusive-app/src/theme.rs`
   - `elusive-app/src/egui_adapter.rs`
5. Remove the duplicate theme implementation after the surviving approach is chosen.
6. Rewrite `CLAUDE.md` so it matches real repository state.
7. Treat `claude_implementation.md` as review input, not as a competing source of truth.

Decision already made for the theme direction:

- keep the tokenized design-system approach
- use one toolkit-neutral theme module plus one egui adapter
- remove the duplicate parallel theme file

## 5. Engineering phases

## Phase 0 — workspace and foundations

Goal: establish a repository that can be built, tested, and extended without
restructuring later.

Scope:

- create workspace root `Cargo.toml`
- create `elusive-core` library crate
- create `elusive-app` binary crate
- define module boundaries
- define shared error strategy
- define dependency policy
- add baseline CI

Recommended technical decisions:

- `elusive-core` stays UI-free
- `elusive-app` owns egui/eframe and desktop-only concerns
- use `thiserror` for library errors
- use `anyhow` only at application boundaries if needed
- use `serde` + `serde_json` for sidecar serialization
- commit Inter and JetBrains Mono font files under `assets/fonts/` with their license files

Initial module skeleton:

- `elusive-core/src/lib.rs`
- `elusive-core/src/error.rs`
- `elusive-core/src/model.rs`
- `elusive-core/src/parse/mod.rs`
- `elusive-core/src/parse/ngc.rs`
- `elusive-core/src/parse/csv.rs`
- `elusive-core/src/integrate.rs`
- `elusive-core/src/calibration.rs`
- `elusive-core/src/sidecar.rs`
- `elusive-core/src/wells.rs`
- `elusive-app/src/main.rs`
- `elusive-app/src/app.rs`
- `elusive-app/src/theme.rs`
- `elusive-app/src/egui_adapter.rs`

Initial error variants to define in `elusive_core::Error`:

- `Io`
- `Zip`
- `Xml`
- `Base64`
- `MalformedTrace`
- `MalformedFractions`
- `UnsupportedFormat`

Acceptance criteria:

- `cargo build` succeeds for the workspace
- `cargo test` runs cleanly
- `cargo fmt --check` and `cargo clippy` are wired into CI
- no UI dependency appears in `elusive-core`

## Phase 1 — primary NGC parser

Goal: open `.ngcAnalysis` and `.ngcMethodruns` reliably into a single internal model.

Scope:

- ZIP archive reading
- XML extraction
- signal-trace decoding
- fraction-trace decoding
- metadata extraction
- wavelength mapping
- fraction-trace reconciliation

Required implementation details:

- resolve `MWave0..3` mapping from method/run XML when possible
- if wavelength mapping is missing, fall back to `215 / 255 / 280 / 495` with a warning
- define a deterministic fraction deduplication rule:
  prefer the fraction stream containing complete `FractionDone` records and discard
  summary-only duplicates after reconciliation by tube number
- preserve raw UV values in storage and assign display scaling separately
- treat channel sample vectors as independent; never infer shared indices

Acceptance criteria:

- a real sample archive opens successfully
- channel names, units, sample counts, and fraction counts are extracted
- uneven sampling across channels is preserved
- fraction start/end volume windows are extracted correctly
- parser errors return context rather than panicking

Deferred from this phase:

- well placement rendering is not required to complete the parser phase

## Phase 2 — chromatogram viewer

Goal: render the run usefully before downstream analysis features are added.

Scope:

- multi-channel plotting
- legend and visibility toggles
- sensible axis grouping
- zoom/pan
- hero-channel default behavior

Required implementation details:

- group axes by `ChannelKind`
- UV channels share one axis and render with display scaling applied
- conductivity, pH, pressure, flow, and temperature do not share an axis unless
  their units and semantics match
- default the hero trace to UV 280 when available

Acceptance criteria:

- a loaded run renders multiple channels on volume axis
- channels can be shown and hidden
- axis grouping separates incompatible units
- no assumption of shared sample indices leaks into plotting behavior

## Phase 3 — fraction overlays and table

Goal: make fraction collection visible and inspectable.

Scope:

- fraction span indicators on the chromatogram
- fraction table
- selection state
- hover/selection linking between plot and table

Acceptance criteria:

- fraction boundaries correspond to decoded volume windows
- selected fraction is obvious without hiding the raw trace
- CSV-only runs clearly show that fractions are unavailable

## Phase 4 — 96-well plate heatmap

Goal: represent collected fractions in the physical rack layout.

Scope:

- HEP96 plate rendering
- serpentine placement
- channel + metric picker
- hover linking between plate and chromatogram
- empty-vs-zero distinction

Required implementation details:

- store shared immediate-mode hover state in app state, not background channels
- include at least:
  - `hovered_vol_range: Option<(f32, f32)>`
  - `hovered_well: Option<Well>`

Acceptance criteria:

- collected wells land in the correct positions
- hover on a well highlights the corresponding trace span
- at least these metrics work: integrated area, max, mean, value-at-center
- wells remain readable without relying on color alone

## Phase 5 — manual integration

Goal: support deliberate manual analysis without automatic peak picking.

Scope:

- drag/select region interaction
- baseline mode selection
- peak result generation
- editable peak definitions
- results table
- CSV export

Required implementation details:

- drag begins on pointer down in the plot region and commits on pointer release
- a visible pending selection is shown during drag
- baseline mode is explicit at creation/edit time
- existing integrations remain selectable for edit/delete
- CSV export schema is defined, not inferred ad hoc

Default CSV export targets:

- peak table:
  `peak_id,channel_id,v_start_ml,v_end_ml,apex_volume_ml,area,height,fwhm,estimated_mw,fractions`
  (`fractions` is the quoted, comma-separated list of wells the peak's window
  overlaps, in collection order; empty when the source format carries none)
- well table:
  `well_id,row,col,channel_id,metric,value`

Acceptance criteria:

- a user can create, edit, and remove manual integrations
- baseline mode differences are explicit
- area, height, apex volume, and FWHM are stable on test cases
- overlapping or adjacent integrations behave predictably

## Phase 6 — sidecar persistence

Goal: preserve user analysis without mutating source files.

Scope:

- save/load `.elusive.json`
- schema versioning
- persisted manual peaks
- persisted excluded regions
- persisted calibration data
- persisted selected display preferences where appropriate

Required implementation details:

- schema must be documented before implementation is considered complete
- source-run identity must be stored explicitly
- incompatible schema versions must fail with a clear message rather than guessing

Minimum sidecar fields:

- `version`
- `source_path` or equivalent source identity
- `peaks`
- `excluded_regions`
- `calibrations`
- `annotations`
- `view`

Acceptance criteria:

- reopening a run restores saved annotations and integrations
- sidecar schema version is explicit
- incompatible future schema versions fail clearly

## Phase 7 — SEC calibration and concentration

Goal: support standard-run-derived size and concentration outputs.

Scope:

- calibration curve fitting
- sample-peak application
- concentration estimate from A280
- fit statistics and validation messaging

Required implementation details:

- define UX for standard-peak assignment
- define UX for `V0` and `Vt` entry when Kav-based fitting is used
- define UX for extinction coefficient, path length, and concentration unit choice
- failed or low-confidence fits must be visible in UI state and exports

Acceptance criteria:

- calibration inputs and resulting fit are inspectable
- output includes estimated MW and concentration separately
- failed or weak fits are signaled explicitly

## Phase 8 — CSV import

Goal: support degraded import when native archives are unavailable.

Scope:

- Analysis CSV parsing
- Traces CSV legend merge
- latin-1 decoding
- trailing-empty handling

Acceptance criteria:

- channels load from CSV exports correctly
- colors and units merge from the legend when present
- the UI clearly communicates that fractions are unavailable for CSV-only runs

## Phase 9 — packaging and release

Goal: deliver usable desktop builds for target platforms.

Scope:

- Windows packaging
- macOS packaging
- icons and app metadata
- CI artifacts
- signing/notarization plan

Recommended packaging decision:

- prefer `cargo-dist` over `cargo-bundle`

Acceptance criteria:

- reproducible release builds exist for Windows and macOS
- packaging steps are documented
- platform-specific constraints are known, not implied

## 6. Missing details that must be specified

The current design is strong, but these items still need explicit answers.

### Data-format and domain questions

- exact `MWave0..3` to wavelength mapping source
- exact UV display scale policy: AU stored vs mAU shown
- whether multi-run archives exist and how they should be handled
- how to reconcile duplicate `Trace_Fractions_*` files deterministically
- supported rack types beyond HEP96, if any
- whether V0 and Vt are available for SEC-derived calculations
- extinction coefficient and path-length source for concentration

### UX questions

- how the default hero trace is selected
- how users switch axis grouping or pin channels to an axis
- how integration edits are represented after creation
- how overlapping manual peaks are allowed or restricted
- whether dark mode preference is persisted
- exact report/export formats: CSV, image, PDF, or all three
- exact standard-peak assignment workflow
- exact `V0` / `Vt` input workflow
- exact concentration inputs: extinction coefficient, path length, and units

### Engineering questions

- Rust edition and minimum supported toolchain
- exact dependency list and version policy
- test fixture policy for proprietary customer/sample files
- logging and debug-dump policy for parser diagnostics
- release-signing and notarization approach for Windows and macOS
- whether fonts are committed directly or fetched during packaging

## 7. Sidecar schema requirements

The sidecar needs its own defined contract rather than an implied one.

Minimum fields to plan for:

- schema version
- source file identity
- manual integrations
- baseline mode and control points
- excluded regions
- calibration selection and parameters
- user annotations
- optional display state worth restoring

Rules:

- source archives are never modified
- sidecar versioning is explicit
- migrations are deliberate, not best-effort guesswork

## 8. Testing strategy

Testing needs to be part of the plan, not an afterthought.

### Unit tests

- trace binary decoding
- fraction XML decoding
- well mapping
- integration math
- calibration math
- sidecar serialization round-trips

### Fixture-based tests

- real `.ngcAnalysis` sample
- real `.ngcMethodruns` sample
- matching Analysis CSV and Traces CSV exports
- malformed or partial files

### UI-level verification

- basic smoke test for app startup
- screenshot/manual review for theme and layout stability
- interaction checks for plot/table/plate linking

## 9. CI and quality gates

Minimum CI should include:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets`
- `cargo test --workspace`

Later additions:

- release build jobs for Windows and macOS
- artifact upload
- optional screenshot regression checks if UI stabilizes enough

## 10. Repo cleanup

Done: the workspace is scaffolded, the app shell lives in `elusive-app/src/app.rs`,
the tokenized theme is canonical (`theme.rs` + `egui_adapter.rs`), the duplicate
`theme_claude.rs` is deleted, the core module tree exists, and `CLAUDE.md` describes
the repository as it actually is.

The next action list is verification, not restructuring:

1. Open real `.ngcAnalysis` and `.ngcMethodruns` exports; record what the
   Review-required panel reports.
2. Turn each confirmed behaviour into a fact in `design.md` §3 and a fixture test,
   and remove the corresponding warning.
3. Tick the matching box in `design.md` §15 and §6 above.

## 11. Practical recommendation on document placement

This plan should live in its own file: `IMPLEMENTATION_PLAN.md`.

Reason:

- it avoids overloading `design.md`, which is already doing product-spec work
- it avoids polluting `DESIGN_SYSTEM.md`, which should stay visual-only
- it avoids turning `CLAUDE.md` into a long-lived planning document

If the repository is later reorganized, this file can move to `docs/IMPLEMENTATION_PLAN.md`,
but while the repo is still flat, keeping it at the root is the least ambiguous choice.

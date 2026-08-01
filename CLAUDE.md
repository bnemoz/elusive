# CLAUDE.md — implementation guide for EluSive

Read these documents in this order:

1. `design.md` — product and file-format source of truth
2. `DESIGN_SYSTEM.md` — visual and interaction-system source of truth
3. `IMPLEMENTATION_PLAN.md` — repository layout, engineering phases, and acceptance criteria

This file is intentionally short. It is not a second product spec and it should not
describe code as already existing when it does not.

## What this project is

**EluSive** is a cross-platform desktop viewer and light analysis tool for Bio-Rad
NGC / ChromLab runs.

Target architecture:

- `elusive-core` — parsing, model, integration, calibration, sidecar I/O
- `elusive-app` — egui/eframe desktop UI

## Current repository state

The Cargo workspace exists and both crates are implemented.

`elusive-core/src/` — `lib.rs`, `error.rs`, `model.rs`, `wells.rs`, `integrate.rs`,
`calibration.rs`, `sidecar.rs`, and `parse/{mod,ngc,csv,xml}.rs`. `parse/xml.rs` is
an addition to the planned module list: a name-based XML flattener, needed because
`design.md` §3 pins down the trace *payloads* but not the wrapper elements around
them, and no real archive is available to read the hierarchy from.

`elusive-app/src/` — `main.rs`, `app.rs`, `view.rs`, `theme.rs`, `egui_adapter.rs`,
and `widgets/{mod,chromatogram,plate,panels,overview}.rs`. `view.rs` holds all
mutable UI state so widgets can take `&Run` and `&mut View` simultaneously.
`widgets/overview.rs` owns the Overview section's responsive column flow and its
drag-to-rearrange cards; its layout arithmetic is pure and width-driven so it can
be tested without a window.

Tests: unit tests in every core module plus `elusive-core/tests/ngc_archive.rs`,
which builds a synthetic `.ngcAnalysis` in memory and parses it end to end.
CI runs fmt, clippy, and tests on Linux/Windows/macOS, and asserts `elusive-core`
has no UI toolkit in its dependency tree.

Still absent: a real instrument export in `testdata/` (none is redistributable),
and the Inter / JetBrains Mono font files (see `assets/fonts/README.md`).

## Document boundaries

- `design.md` owns product goals, verified format knowledge, and UX intent.
- `DESIGN_SYSTEM.md` owns tokens, chart/plate rules, accessibility, and visual behavior.
- `IMPLEMENTATION_PLAN.md` owns repo structure, phases, open questions, and acceptance criteria.
- `CLAUDE.md` owns implementation constraints and the working order of operations.

If these documents disagree:

1. `design.md` wins on product and format facts.
2. `DESIGN_SYSTEM.md` wins on visual rules.
3. `IMPLEMENTATION_PLAN.md` wins on repo structure and milestone sequencing.
4. `CLAUDE.md` should be updated rather than treated as a competing source of truth.

## Immediate implementation direction

Phase 0 (workspace) is done, and Phases 1–8 of `IMPLEMENTATION_PLAN.md` have a
working first implementation. The scaffolding steps that used to live here — create
the workspace, pick one theme path, delete the duplicate — are complete: the
tokenized `theme.rs` plus `egui_adapter.rs` pair survived and `theme_claude.rs`
was removed.

What matters next is confirmation, not construction:

1. Open real `.ngcAnalysis` and `.ngcMethodruns` files and read the
   **Overview → Review required** panel. Every assumption the parser makes is
   reported there; each one that a real file settles should become a fact in
   `design.md` §3 and lose its warning.
2. Add a fixture test per confirmed behaviour, ideally from a redistributable run.
3. Only then widen scope — extra rack types, auto peak detection, packaging polish.

Resist adding features while `design.md` §15 still has unchecked boxes. A parser
that is confidently wrong about a wavelength mapping produces plausible numbers,
which is worse than one that says it is guessing.

## Brand and UI constraints

Use the tokenized design-system approach.

Rules:

- keep one toolkit-neutral theme module plus one egui adapter
- do not hard-code hex colors in widgets
- raw trace must remain visible beneath integrations and annotations
- status must never rely on color alone
- numeric displays should use stable, right-aligned formatting

The intended canonical direction is the token-based structure described in
`DESIGN_SYSTEM.md`, not two parallel theme implementations.

## Engineering constraints

- `elusive-core` must never import egui or eframe
- source archives are read-only; user analysis goes into `<run>.elusive.json`
- native `.ngcAnalysis` / `.ngcMethodruns` support comes before CSV import
- parser and analysis code should be small, pure, and testable
- no `unwrap()` on user-supplied data paths or parsed run content

Recommended error boundary:

- `thiserror` in library crates
- `anyhow` only at app entrypoints if needed

Recommended serialization:

- `serde`
- `serde_json` for sidecars

## Known unresolved format questions

These are not optional polish items. Verify them against real files:

- `MWave0..3` to wavelength mapping source
- UV storage vs display scale policy
- duplicate `Trace_Fractions_*` reconciliation
- rack/well mapping confirmation for HEP96 serpentine layout
- availability of V0 and Vt for SEC calculations
- extinction coefficient and path-length source for concentration

## Build, test, and quality gates

Once the workspace exists, standard commands should be:

```bash
cargo build
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets
```

Add these to CI early. Do not wait for feature completion to establish quality gates.

## Implementation style

- prefer small pure functions in core
- write tests for format quirks and analysis math
- comments should explain why, especially around decoded file-format behavior
- keep UI concerns out of parser and analysis code

## What not to do

- do not treat this file as the full plan
- do not duplicate large sections of `design.md` here
- do not claim modules are already implemented unless they are present in the repo
- do not keep both theme implementations long-term

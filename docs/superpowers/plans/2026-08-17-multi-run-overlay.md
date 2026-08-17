# Multi-Run Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open several runs at once — overlay their traces on the shared chromatogram and compare their saved peak results side by side — with one primary run keeping all editing and the sidecar.

**Architecture:** Approach A from the spec: overlays live in `elusive-app` beside the existing `run: Option<Run>`, each a fully parsed `Run` wrapped in display settings plus that run's sidecar peaks loaded read-only. `elusive-core` changes only in `sidecar.rs` (a tolerant `ViewState.overlays` field). Rendering joins the existing axis-group stack; identity is dash pattern + run name, never color alone.

**Tech Stack:** Rust (MSRV 1.92), egui/eframe + egui_plot, serde/serde_json.

**Spec:** `docs/superpowers/specs/2026-08-17-multi-run-overlay-design.md`

## Global Constraints

- `elusive-core` must never depend on egui/eframe (CI asserts this).
- No `unwrap()` on user-supplied paths or parsed run content.
- Raw trace stays visible beneath annotations; status never by color alone.
- Source archives and **overlay sidecars are never written**; only the primary's sidecar is saved.
- Everything stored stays in mL; the x-offset is display-only.
- Run gates before pushing: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`.

---

### Task 1: `ViewState.overlays` in the core sidecar

**Files:**
- Modify: `elusive-core/src/sidecar.rs` (ViewState ends ~line 135; tests module at bottom)

**Interfaces:**
- Produces: `pub struct OverlayRef { pub path: String, pub visible: bool, pub x_offset_ml: f32, pub hidden_channels: Vec<String> }` and `ViewState.overlays: Option<Vec<OverlayRef>>`.

- [x] **Step 1: Write failing tests** in `sidecar.rs`'s tests module:

```rust
#[test]
fn overlay_refs_round_trip() {
    let mut sc = Sidecar::default();
    sc.view.overlays = Some(vec![OverlayRef {
        path: "../std-run.ngcAnalysis".into(),
        visible: true,
        x_offset_ml: 0.25,
        hidden_channels: vec!["MWave3".into()],
    }]);
    let json = serde_json::to_string(&sc).unwrap();
    let back = from_json(&json).unwrap();
    assert_eq!(back.view.overlays, sc.view.overlays);
}

#[test]
fn sidecar_without_overlays_field_loads_as_none() {
    let json = r#"{"version": 1, "source": {"file_name":"x","run_name":"y"}}"#;
    assert_eq!(from_json(json).unwrap().view.overlays, None);
}
```

- [x] **Step 2: Run** `cargo test -p elusive-core sidecar` — expect compile failure (no `OverlayRef`).
- [x] **Step 3: Implement.** Add above `ViewState`:

```rust
/// One comparison run remembered by the primary's sidecar. Read-only data:
/// loading it back never writes the overlay run's own sidecar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OverlayRef {
    /// Preferably relative to the primary's directory (runs travel together);
    /// absolute when no relative form exists.
    pub path: String,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub x_offset_ml: f32,
    #[serde(default)]
    pub hidden_channels: Vec<String>,
}

fn default_true() -> bool { true }
```

and to `ViewState` (with the absent-vs-empty doc comment style of `channel_colors`):

```rust
#[serde(default)]
pub overlays: Option<Vec<OverlayRef>>,
```

- [x] **Step 4: Run tests** — expect PASS (also fixes `ViewState { .. }` literal in `view.rs::to_sidecar`: add `overlays: None` there for now; Task 7 fills it).
- [x] **Step 5: Commit** `feat(core): remember comparison runs in the sidecar view state`

---

### Task 2: `overlay.rs` — the Overlay type and its pure logic

**Files:**
- Create: `elusive-app/src/overlay.rs`; register `mod overlay;` in `main.rs` next to the other modules.

**Interfaces (produced, used by Tasks 3–7):**

```rust
pub struct Overlay {
    pub run: Run,
    pub source_path: PathBuf,
    pub peaks: Vec<PeakResult>,        // read-only, from the overlay's own sidecar
    pub visible: bool,
    pub hidden_channels: BTreeSet<ChannelId>,
    pub x_offset_ml: f32,
}
impl Overlay {
    pub fn label(&self) -> &str;                        // run_name, else file stem
    pub fn is_channel_visible(&self, id: &ChannelId) -> bool;
}
pub fn load_overlay(path: &Path) -> Result<Overlay, String>;
pub fn default_hidden_channels(overlay: &Run, primary: &Run, view: &View) -> BTreeSet<ChannelId>;
pub fn overlay_dash(overlay_index: usize) -> chart::Dash;   // Dashed, Dotted, Dashed, …
pub struct ComparisonRow { pub run: String, pub channel: String, pub peak: PeakResult,
                           pub area_pct: Option<f64>, pub fractions: String }
pub fn comparison_rows(primary: &Run, primary_peaks: &[PeakResult], overlays: &[Overlay]) -> Vec<ComparisonRow>;
pub fn comparison_to_csv(rows: &[ComparisonRow]) -> String;  // peak schema + leading `run` column
pub fn relative_or_absolute(base_dir: &Path, target: &Path) -> String;
pub fn resolve_overlay_path(base_dir: &Path, stored: &str) -> PathBuf;
```

- [x] **Step 1: Failing tests** (same file, `#[cfg(test)]`): a `mini_run(name, channels)` helper building `Run`s with one UV 215 + one UV 280 + one conductivity channel; then:
  - `matching_visible_channels_start_visible`: primary shows UV 280 + conductivity, hides UV 215 → overlay's UV 280 and conductivity visible, UV 215 hidden.
  - `uv_match_requires_equal_wavelength`: overlay UV 260 vs primary UV 280 → hidden.
  - `overlay_dash_never_solid_and_alternates`: `overlay_dash(0) == Dashed`, `(1) == Dotted`, `(2) == Dashed`.
  - `area_pct_is_within_one_runs_channel`: two peaks 3.0 + 1.0 on one channel → 75/25; a peak on another run unaffected.
  - `comparison_csv_has_run_column_then_peak_schema`: header is `run,peak_id,channel_id,v_start_ml,v_end_ml,apex_volume_ml,area,height,fwhm,estimated_mw,fractions`.
  - `relative_path_when_shared_prefix`: base `/data/runs`, target `/data/std.ngcAnalysis` → `../std.ngcAnalysis`; disjoint roots → absolute string. `resolve_overlay_path` round-trips both.
- [x] **Step 2: Run** `cargo test -p elusive-app overlay` — expect FAIL (module missing).
- [x] **Step 3: Implement.** Key logic:
  - `default_hidden_channels`: a channel is *shown* iff `!c.is_empty()` and primary has a channel `p` with `view.is_channel_visible(&p.id) && p.kind == c.kind && (c.kind != ChannelKind::Uv || p.wavelength_nm == c.wavelength_nm)`; everything else goes into the hidden set.
  - `load_overlay`: `parse::open(path)` (map err to string); then if `run.sidecar_path().is_file()`, `sidecar::load` + `matches(&run)` → take `peaks` (else empty, mismatch/err becomes an empty list — the run still overlays).
  - `comparison_rows`: primary first (`run.meta.run_name`), then each overlay in order; `area_pct` = `100 * p.area / Σ area over same (run, channel_id)` when the sum is > 0; `fractions` via `run.wells_in_volume` + `wells::join_well_labels` per owning run.
  - `comparison_to_csv`: reuse the exact number formatting of `sidecar::peaks_to_csv` (`{:.4}` volumes, `{:.6}` area/height, `{:.3}` MW), CSV-escape run/channel/fractions (local `csv_escape` copy: quote when the field contains `,`, `"` or newline, doubling quotes).
  - `relative_or_absolute`: walk `components()` of both absolute paths, strip common prefix, emit `..` per remaining base component + the target remainder; if no common prefix (different roots), return the absolute target as a string. `resolve_overlay_path`: join+normalize when `stored` is relative, else `PathBuf::from(stored)`.
- [x] **Step 4: Run tests** — expect PASS.
- [x] **Step 5: Commit** `feat(app): overlay model, matching defaults, comparison rows`

---

### Task 3: chromatogram renders overlay traces

**Files:**
- Modify: `elusive-app/src/widgets/chromatogram.rs` — `show` (:66), `visible_groups` (:173), `data_y_range` (:231), `plot_group` (:687), `draw_channel` (:927); call sites in `app.rs::linked_pane` (:1052).

**Interfaces:**
- `show(ui, run, overlays: &[Overlay], view, t) -> ChartOutcome`; internal `draw_overlay_channel(plot_ui, overlay_label, channel, rgb, dash, tf, normalized_range)`.

- [x] **Step 1: Failing tests** for the pure pieces (chromatogram's tests module):
  - `overlay_channels_extend_the_group_y_range`: `data_y_range` over primary (0..10) + visible overlay channel (0..25) → hi = 25.
  - `overlay_groups_appear_even_without_primary_channels`: an overlay-only conductivity channel makes `AxisGroup::Conductivity` visible.
  - `offset_moves_volume_x_only`: sample x for an overlay sample = `volume_ml + offset` in Volume mode; equals `time_s/60` (offset ignored) in Time mode — factor this as `fn overlay_sample_x(axis: XAxis, s: &Sample, offset_ml: f32) -> f64` so it is testable.
- [x] **Step 2: Run** — expect FAIL.
- [x] **Step 3: Implement:**
  - Thread `overlays: &[Overlay]` through `show` → `plot_group`. In both, an overlay channel *counts* iff `overlay.visible && !c.is_empty() && overlay.is_channel_visible(&c.id)`.
  - `visible_groups`: extend the collected groups with qualifying overlay channels' groups before sort/dedup.
  - `plot_group`: build `overlay_channels: Vec<(usize /*run idx*/, usize /*chan idx*/, &Channel)>` for the group; extend `data_y_range` (accept a second slice or an iterator of `&Channel`).
  - Draw overlays between step 2 (peak regions) and step 3 (primary traces): color = `chart::legend_color_or_series(channel.color.map(to_rgb), t.panel_bg, chan_idx)` (same resolution a non-hero primary channel gets, so UV 280 shares a hue across runs); style = `adapt::line_style(overlay::overlay_dash(run_idx))`; line name = `format!("{} · {}", overlay.label(), channel.name)`.
  - Per-trace y mode: overlay traces remap through `remap(y, own_display_range, (NORM_LO, NORM_HI))` computed inline from `channel.display_value_range()` — do **not** feed them through `YMap`, whose `ChannelId` keys collide across runs.
  - Time-axis + nonzero offsets: in `show`, when `view.x_axis == XAxis::Time` and any visible overlay has `x_offset_ml != 0.0`, draw a micro-note in the `relative_axis_note` style: "X offsets apply on the volume axis only — traces below are at their own recorded times."
- [x] **Step 4: Run tests**; expect PASS. `cargo build -p elusive-app` compiles with `&[]` temporarily passed from `linked_pane`.
- [x] **Step 5: Commit** `feat(app): draw comparison overlays on the chromatogram`

---

### Task 4: legend groups for overlays

**Files:**
- Modify: `elusive-app/src/widgets/chromatogram.rs::legend` (:1352); call site `app.rs:975`.

**Interfaces:**
- `legend(ui, run, overlays: &mut Vec<Overlay>, view, t)` — mutates overlay settings directly; removal via `overlays.retain` after the loop using an index collected in the loop (`let mut remove: Option<usize> = None`).

- [x] **Step 1:** After the primary channel loop, per overlay: separator; header row with master `checkbox(&mut overlay.visible, "")`, bold run label, `DragValue::new(&mut overlay.x_offset_ml).speed(0.05).suffix(" mL")` labelled "offset", and a small "Remove" button setting `remove = Some(idx)`. Under it, per non-empty channel: visibility checkbox driving `hidden_channels`, display-only swatch via `paint_swatch(painter, rect, rgb, overlay_dash(idx))`, channel name + `display_unit · N pts` micro-label. Every mutation sets `view.dirty = true`.
- [x] **Step 2:** `cargo build -p elusive-app` compiles; `cargo test -p elusive-app` stays green.
- [x] **Step 3: Commit** `feat(app): manage comparison runs from the legend`

---

### Task 5: app wiring — add/replace flow

**Files:**
- Modify: `elusive-app/src/app.rs` — struct (:65), `open_path` (:130), `toolbar` (:547), drag-drop (:1205), `linked_pane` signature + `content` (:710).

**Interfaces:**
- `EluSiveApp { overlays: Vec<Overlay>, pending_drop: Option<PathBuf>, .. }`; `fn add_overlay(&mut self, ctx, path: &Path)`.

- [x] **Step 1: Implement:**
  - `add_overlay`: refuse when `self.run.is_none()`; duplicate check by canonicalized path against primary + existing overlays (`std::fs::canonicalize` falling back to the raw path) → note "already open"; else `overlay::load_overlay`, then `overlay.hidden_channels = default_hidden_channels(&overlay.run, run, &self.view)`, push, `view.dirty = true`, note "Comparing N runs".
  - Toolbar: after "Open run…", `ui.add_enabled(has_run, egui::Button::new("Add comparison…"))` → file dialog (same filters) → `add_overlay`.
  - `open_path` (primary replace): `self.overlays.clear()` on success.
  - Drag-drop: when `self.run.is_some()`, set `self.pending_drop = Some(path)` instead of opening; each frame, if `pending_drop` is set show a modal `egui::Window` ("Open dropped run") with three buttons: "Replace current run" → `open_path`; "Add as comparison" → `add_overlay`; "Cancel".
  - `content`/`linked_pane`: destructure `EluSiveApp { run, view, overlays, .. }` and pass `overlays` through to `chromatogram::show` and `legend`.
- [x] **Step 2:** Build + full app tests green.
- [x] **Step 3: Commit** `feat(app): open comparison runs beside the primary`

---

### Task 6: Results comparison table + CSV export

**Files:**
- Modify: `elusive-app/src/widgets/panels.rs` (new `comparison_table`), `app.rs` (`content` Results arm, `reports`, `ExportKind`/`DeferredAction`).

- [x] **Step 1:** `panels::comparison_table(ui, run, view, overlays, t)`: `heading("Run comparison")`; rows from `overlay::comparison_rows(run, &view.peaks, overlays)` in the `table_header_row`/`num_cell` style — columns Run, Channel, Ve (mL), Area, Area %, Height, FWHM (mL), Est. MW (kDa); `EM_DASH`-style "—" for absent values; for an overlay with no peaks, one full-width secondary-text row "no saved analysis for this run". Shown in the Results arm only when `!overlays.is_empty()`.
- [x] **Step 2:** Export: add `ExportKind::Comparison` + `DeferredAction::ComparisonCsv`; Reports gains "Run comparison (CSV)" (enabled when overlays exist) → default name `format!("{}-comparison.csv", stem(run))`, contents `overlay::comparison_to_csv(&rows)`.
- [x] **Step 3:** Build + tests green (csv content already unit-tested in Task 2).
- [x] **Step 4: Commit** `feat(app): cross-run peak comparison in Results and CSV export`

---

### Task 7: persistence round trip

**Files:**
- Modify: `elusive-app/src/app.rs::save_sidecar` (:198) and `open_path` (:130).

- [x] **Step 1:** `save_sidecar`: after `to_sidecar`, fill

```rust
let base = run.source_path.parent().unwrap_or(Path::new(""));
payload.view.overlays = Some(self.overlays.iter().map(|o| sidecar::OverlayRef {
    path: overlay::relative_or_absolute(base, &o.source_path),
    visible: o.visible,
    x_offset_ml: o.x_offset_ml,
    hidden_channels: o.hidden_channels.iter().map(|c| c.0.clone()).collect(),
}).collect());
```

- [x] **Step 2:** `open_path`: after `apply_sidecar`, for each `OverlayRef` resolve via `overlay::resolve_overlay_path(base, &r.path)`, `load_overlay`, restore `visible`/`x_offset_ml`/`hidden_channels`; each failure becomes a note naming the path, never an error.
- [x] **Step 3:** Manual round-trip check with the fixture (see Task 8 smoke); build + tests green.
- [x] **Step 4: Commit** `feat(app): comparison set survives in the primary's sidecar`

---

### Task 8: gates, docs, smoke, release tag

- [x] **Step 1:** `cargo test --workspace` / `cargo fmt --all --check` / `cargo clippy --workspace --all-targets -- -D warnings` all clean.
- [x] **Step 2:** Smoke: a headless check opening `testdata/sec-run.ngcAnalysis` twice via `overlay::load_overlay` + `comparison_rows` (integration test in `elusive-app/tests/` is fine — no window needed for the pure path).
- [x] **Step 3:** Docs: README "What you can do" bullet + `design.md` §11 note describing overlays; brief CLAUDE.md state note.
- [x] **Step 4:** Version: workspace `version = "0.4.0-beta.1"`. Commit `chore: bump to 0.4.0-beta.1 for the overlay test build`.
- [x] **Step 5:** Push branch; tag `v0.4.0-beta.1` on the branch head and push the tag → CI builds the Windows zip and publishes a pre-release.

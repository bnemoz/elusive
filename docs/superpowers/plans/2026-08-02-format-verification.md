# NGC Format Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the six unchecked boxes in `design.md` §15 by reading a real instrument export, so every parser assumption becomes either a verified fact with a fixture test or an explicit, still-warned guess.

**Architecture:** Three phases. **A** builds redistributable fixtures from a real run, because none of the rest is testable without them. **B** is evidence gathering — read the archive, write down what it says, change no behaviour. **C** locks each confirmed fact in with a fixture test and removes the corresponding warning. B and C alternate per box so that a box is never "closed" without a test.

**Tech Stack:** Rust 2021, `zip`, `quick-xml`, `base64`, `serde`. No new dependencies.

## Global Constraints

- `elusive-core` must never import `egui` or `eframe`. CI asserts this.
- No `unwrap()` on user-supplied data or parsed run content.
- Comments explain *why*, not *what*. Match the existing rationale-heavy house style.
- Quality gates, all must pass before any commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Baseline is **286 tests** on `feat/akta-unicorn-import`, **280** on `main`. Never let the count drop.
- **A box is only checked when a fixture test proves it.** Reading a value once is evidence; a passing test is verification.
- **Never delete a warning without replacing it with a test.** A silent parser is worse than a warned one.
- Real exports are not redistributable. Only sanitized fixtures may be committed.
- Prefer small pure functions in core; test format quirks and analysis math.

## What is already known

A read-only listing of `test/test_data.ngcAnalysis` (26 entries, 2.0 MB) established:

```
  6118890  Analysis.xml                              <- ChromLab's own analysis state
   728837  Runs/Run1/Trace_ModulePH_16.xml           <- pH sampled ~2x, as design.md §3 warns
   364701  Runs/Run1/Trace_MD_Conductivity_7.xml
   364693  Runs/Run1/Trace_MWave{0,1,2,3}_{8,2,14,4}.xml
   350074  Runs/Run1/Trace_NextGenEvents_6.xml
   350069  Runs/Run1/Trace_Fractions_1.xml           <- the FULL fraction stream
   155383  Methods/MethodData1.xml
    73462  Runs/Run1/Trace_FlowRate_13.xml           (+ pressures, %B, pumps)
    41807  Runs/RunInfo1.xml
     8589  Runs/AnalysisRunViewSettings1.xml         <- NOT in design.md's layout
      885  Runs/Run1/Trace_Fractions_19.xml          <- the SUMMARY
      832  Runs/Run1.xml
```

Three findings that shape the plan:

1. **The duplicate fraction trace is real and its asymmetry is stark** — 350,069 bytes versus 885. Box 3 is answerable by inspection, not inference.
2. **`design.md` §3's stated layout is wrong in detail.** It documents `Method/MethodData.xml` and `Runs/Run1.xml`; the archive actually has `Methods/MethodData1.xml` (plural, numbered) and `Runs/RunInfo1.xml`. Task 13 fixes the doc.
3. **`Analysis.xml` (6 MB) is the highest-value entry and is undocumented.** If it carries ChromLab's own integration results, it is independent ground truth for both the UV scale (Box 1) and our integration math — a far stronger check than reading a unit string. Task 4 investigates this first because a positive result makes Task 5 trivial and also validates work already shipped.

## File Structure

- `elusive-core/examples/sanitize_ngc.rs` — **exists, uncommitted, user-authored.** Redacts identity fields from a real archive without touching trace payloads. Task 1 reviews and commits it.
- `testdata/` — **new.** Committed sanitized fixtures. Must contain nothing traceable to a real sample.
- `docs/format-findings.md` — **new.** The evidence log. Every Phase B task appends; Phase C cites it. This is the artifact that makes the work reviewable by someone who was not present.
- `elusive-core/tests/real_archive.rs` — **new.** Fixture tests against the sanitized archive, separate from the existing synthetic `ngc_archive.rs` so a fixture failure is instantly distinguishable from a logic failure.
- `elusive-core/src/parse/ngc.rs` — modified per box. Relevant sites: `DEFAULT_WAVELENGTHS` (~line 27), `wavelength_for` (~line 364), `display_scale_for` (~line 407), fraction reconciliation (~line 865), rack/pattern warnings (~lines 906–920).
- `design.md` — §3 corrections and §15 checkbox updates.

## Out of scope — separate plans

ÄKTA/UNICORN decoding (PR #14, blocked on an export), font vendoring and the `install_fonts` working-directory bug, `cargo-dist` packaging and signing, and the small defects listed in the prior review. Each is independent and none gates this work.

---

### Task 1: Commit the fixture sanitizer

**Files:**
- Modify: `elusive-core/examples/sanitize_ngc.rs` (currently uncommitted in the working tree)
- Modify: `.gitignore` (the uncommitted `/test/` line)

**Interfaces:**
- Produces: a runnable `cargo run -p elusive-core --example sanitize_ngc -- IN OUT` that later tasks depend on.

- [ ] **Step 0: Branch from current `main`**

```bash
git fetch origin && git checkout main && git pull
git checkout -b feat/verify-ngc-format
```

The working tree holds uncommitted user-authored work (`elusive-core/examples/sanitize_ngc.rs` and a `.gitignore` edit). It belongs on this branch — Task 1 commits it deliberately. **Throughout this plan use `git add -u` or explicit paths, never `git add -A`**, so nothing else in the tree is swept in by accident.

- [ ] **Step 1: Read the existing tool and confirm what it redacts**

Run: `cat elusive-core/examples/sanitize_ngc.rs`

It declares a `SENSITIVE_TAGS` list. Write down which tags it covers. Its own doc comment warns it "cannot determine whether numerical data or an unrecognised XML field is sensitive" — that caveat must survive.

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p elusive-core --example sanitize_ngc`
Expected: compiles clean.

- [ ] **Step 3: Confirm the gitignore keeps real exports out**

Run: `git check-ignore -v test/test_data.ngcAnalysis`
Expected: a line naming the `/test/` rule. If it does not print, the real export is at risk of being committed — stop and fix `.gitignore` before continuing.

- [ ] **Step 4: Commit**

```bash
git add elusive-core/examples/sanitize_ngc.rs .gitignore
git commit -m "Add a fixture sanitizer for real NGC archives

Real exports are not redistributable, which is why testdata/ has always been
empty and every parser test runs against a synthetic archive. This redacts the
free-text identity fields while leaving trace payloads byte-identical, so a real
run can become a committed fixture."
```

---

### Task 2: Produce and commit the sanitized fixture

**Files:**
- Create: `testdata/sec-run.ngcAnalysis`
- Create: `testdata/README.md`

**Interfaces:**
- Produces: `testdata/sec-run.ngcAnalysis`, the fixture every later task loads.

- [ ] **Step 1: Run the sanitizer**

```bash
cargo run -p elusive-core --example sanitize_ngc -- \
  test/test_data.ngcAnalysis testdata/sec-run.ngcAnalysis
```

- [ ] **Step 2: Inspect the output before trusting it**

```bash
python3 - <<'EOF'
import zipfile, re
z = zipfile.ZipFile("testdata/sec-run.ngcAnalysis")
for n in z.namelist():
    body = z.read(n).decode("utf-8", "replace")
    for pat in (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+", r"(?i)\b(operator|user|owner|author)\b"):
        for m in re.findall(pat, body)[:3]:
            print(f"{n}: {m}")
EOF
```

Expected: no output. Any hit means `SENSITIVE_TAGS` missed a field — extend the tool and re-run Step 1. **This is a human gate: a person must eyeball the fixture before it is committed.** Once committed to a repo it is effectively permanent.

- [ ] **Step 3: Confirm the sanitized archive still parses**

```bash
cargo run -p elusive-app --bin elusive -- testdata/sec-run.ngcAnalysis
```

Expected: the app opens the run. If the sanitizer corrupted an XML entry this fails here rather than inside a confusing test.

- [ ] **Step 4: Write `testdata/README.md`**

```markdown
# Test fixtures

`sec-run.ngcAnalysis` — a real Bio-Rad NGC SEC run, passed through
`cargo run -p elusive-core --example sanitize_ngc` to redact identity fields.
Trace payloads are byte-identical to the instrument output; only free-text
metadata was replaced.

Do not add a raw export here. `/test/` and `/testdata/private/` are gitignored
for exactly that reason.
```

- [ ] **Step 5: Commit**

```bash
git add testdata/
git commit -m "Add a sanitized real SEC run as a test fixture

Every parser test so far has run against an archive the tests build themselves,
which proves internal consistency and nothing about whether we read the
instrument correctly."
```

---

### Task 3: Fixture smoke test

**Files:**
- Create: `elusive-core/tests/real_archive.rs`

**Interfaces:**
- Produces: `fn fixture() -> Run`, used by every later fixture test.

- [ ] **Step 1: Write the test**

```rust
//! Tests against a real (sanitized) instrument export.
//!
//! Separate from `ngc_archive.rs`, which builds a synthetic archive: when one of
//! these fails it means the real format diverged from what we believe, not that
//! the logic is wrong. Keeping them apart makes that distinction immediate.

// Import only what this file uses today: clippy runs with `-D warnings`, so an
// unused import fails the build. Task 5 adds `ChannelKind` when it needs it.
use elusive_core::model::Run;

fn fixture() -> Run {
    elusive_core::parse::open("../testdata/sec-run.ngcAnalysis")
        .expect("the committed fixture must parse")
}

#[test]
fn the_fixture_carries_the_channels_the_archive_listing_showed() {
    let run = fixture();
    for id in ["MWave0", "MWave1", "MWave2", "MWave3", "MD_Conductivity", "ModulePH"] {
        assert!(
            run.channels.iter().any(|c| c.id.as_str() == id && !c.is_empty()),
            "{id} missing or empty; channels present: {:?}",
            run.channels.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn channels_have_independent_sample_counts() {
    // design.md §3 warns pH was sampled at ~2x the UV rate, and the entry sizes
    // agree (728837 vs 364693 bytes). A shared-index assumption would silently
    // truncate or misalign; this is the fixture that would catch it.
    let run = fixture();
    let uv = run.channel(&"MWave2".into()).expect("MWave2").samples.len();
    let ph = run.channel(&"ModulePH".into()).expect("ModulePH").samples.len();
    assert!(ph > uv, "expected pH oversampled relative to UV: ph={ph} uv={uv}");
}

#[test]
fn the_run_collected_fractions() {
    let run = fixture();
    assert!(!run.fractions.is_empty(), "the archive has two fraction traces");
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p elusive-core --test real_archive`
Expected: PASS. If `the_fixture_carries_the_channels...` fails, the sanitizer damaged trace payloads — go back to Task 2.

- [ ] **Step 3: Commit**

```bash
git add elusive-core/tests/real_archive.rs
git commit -m "Add fixture smoke tests against the real archive"
```

---

### Task 4: Investigate `Analysis.xml` for ChromLab's own numbers

**Files:**
- Create: `docs/format-findings.md`

**Interfaces:**
- Produces: a recorded answer to "does the archive contain ChromLab's own peak areas?", consumed by Tasks 5 and 6.

This runs first among the investigations because a positive result gives independent ground truth for the UV scale, which is Box 1, the highest-priority box.

- [ ] **Step 1: Survey the element names**

```bash
python3 - <<'EOF'
import zipfile, collections, re
body = zipfile.ZipFile("testdata/sec-run.ngcAnalysis").read("Analysis.xml").decode("utf-8","replace")
tags = collections.Counter(re.findall(r"<([A-Za-z_][\w.:-]*)", body))
for t, n in tags.most_common(60):
    print(f"{n:>7}  {t}")
EOF
```

- [ ] **Step 2: Look specifically for integration results**

```bash
python3 - <<'EOF'
import zipfile, re
body = zipfile.ZipFile("testdata/sec-run.ngcAnalysis").read("Analysis.xml").decode("utf-8","replace")
for kw in ("Peak","Area","Height","Baseline","Retention","Integrat","Unit","mAU","Wavelength"):
    hits = [m.start() for m in re.finditer(kw, body)][:2]
    print(f"--- {kw}: {len(re.findall(kw, body))} occurrences")
    for h in hits:
        print("   ", body[max(0,h-120):h+200].replace("\n"," ")[:300])
EOF
```

- [ ] **Step 3: Record findings**

Create `docs/format-findings.md` with a `## Analysis.xml` section stating: whether peak records exist, the element and attribute names carrying area/height/retention, and **the units those values are expressed in**. Quote the raw XML for at least one peak. If no peak records exist, say so explicitly — a negative result is a finding and stops Task 5 from waiting on it.

- [ ] **Step 4: Commit**

```bash
git add docs/format-findings.md
git commit -m "Record what Analysis.xml contains"
```

---

### Task 5: Box 1 — settle the UV value scale (AU vs mAU)

**Files:**
- Modify: `docs/format-findings.md`
- Modify: `elusive-core/src/parse/ngc.rs` (`display_scale_for`, ~line 407)
- Modify: `elusive-core/tests/real_archive.rs`

**Interfaces:**
- Consumes: the units finding from Task 4.
- Produces: a verified `display_scale` policy; removes the magnitude-heuristic warning when the unit is declared.

**Why first:** `display_scale_for` currently falls back to a magnitude heuristic — "prep-scale UV in AU peaks below ~20." A heuristic that picks the plausible answer is the definition of confidently wrong. It multiplies every UV value by 1 or 1000, feeding peak height, absolute area, the plate heatmap, the CSV export, and the A280 concentration someone pipettes against. Area-% survives, being a ratio; nothing else does.

- [ ] **Step 1: Find the declared unit for each UV trace**

```bash
python3 - <<'EOF'
import zipfile, re
z = zipfile.ZipFile("testdata/sec-run.ngcAnalysis")
for n in sorted(x for x in z.namelist() if "MWave" in x):
    head = z.read(n).decode("utf-8","replace")[:4000]
    head = re.sub(r"<TraceData>.*", "<TraceData>[...]", head, flags=re.S)
    print(f"===== {n}\n{head}\n")
EOF
```

Also check `Runs/AnalysisRunViewSettings1.xml` (8.6 KB, undocumented) — a "view settings" entry is a plausible home for the display scale ChromLab actually applies:

```bash
python3 -c "
import zipfile
print(zipfile.ZipFile('testdata/sec-run.ngcAnalysis').read('Runs/AnalysisRunViewSettings1.xml').decode('utf-8','replace'))
"
```

- [ ] **Step 2: Compute what our parser currently produces**

Add a reporting test to `real_archive.rs`. It asserts nothing — it exists to print what the parser currently believes, so Step 3 has both sides to compare. Add `ChannelKind` to the import line here.

```rust
/// Prints, asserts nothing. Kept in the tree: when a future run disagrees with
/// the policy fixed in this task, this is the first thing to re-run.
#[test]
fn report_uv_scale() {
    let run = fixture();
    for c in run.channels.iter().filter(|c| c.kind == ChannelKind::Uv) {
        let (lo, hi) = c.display_value_range().unwrap_or((f32::NAN, f32::NAN));
        eprintln!(
            "{} unit={:?} scale={} display_range={lo}..{hi}",
            c.id.as_str(),
            c.display_unit,
            c.display_scale
        );
    }
}
```

Run: `cargo test -p elusive-core --test real_archive report_uv_scale -- --nocapture`
Expected: four lines, one per UV trace. Record them in the findings.

- [ ] **Step 3: Decide against ground truth**

Compare the range from Step 2 with either (a) ChromLab's own peak heights from Task 4, or (b) a value the user reads off ChromLab for this run. **If neither is available, stop and ask.** Do not resolve this box from the magnitude heuristic — that is the thing being replaced.

- [ ] **Step 4: Record the finding**

Append a `## Box 1 — UV scale` section to `docs/format-findings.md`: the declared unit per UV trace, where it was declared, the observed value range, the ground truth compared against, and the resulting policy.

- [ ] **Step 5: Write the failing test**

```rust
#[test]
fn uv_traces_are_displayed_on_the_scale_the_archive_declares() {
    // Box 1 (design.md §15). Replaces the magnitude heuristic in
    // `display_scale_for`, which guessed from peak amplitude and therefore
    // produced a plausible wrong answer whenever a run was unusually dilute or
    // unusually concentrated.
    let run = fixture();
    let uv = run.channel(&"MWave2".into()).expect("MWave2");
    assert_eq!(uv.display_unit, "<UNIT FROM STEP 1>");
    assert_eq!(uv.display_scale, /* 1.0 or 1000.0 from Step 3 */ 1000.0);

    let (_, hi) = uv.display_value_range().expect("a range");
    assert!(
        (/* lower bound from Step 3 */ 1.0..=/* upper */ 3000.0).contains(&hi),
        "displayed peak {hi} is outside the range ChromLab reports for this run"
    );
}
```

- [ ] **Step 6: Run it to see it fail**

Run: `cargo test -p elusive-core --test real_archive uv_traces_are_displayed -- --nocapture`
Expected: FAIL if the current heuristic disagrees with the declared unit. If it PASSES immediately, the heuristic happened to be right for this run — say so in the findings, keep the test, and still do Step 7, because "right by luck on one run" is not a policy.

- [ ] **Step 7: Make the declared unit authoritative**

In `display_scale_for`, ensure a declared unit always wins and the magnitude fallback is reached only when no unit is declared. Keep the fallback and keep its warning — but reword it to say the archive declared nothing, rather than implying uncertainty about a unit that was in fact stated.

- [ ] **Step 8: Verify**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 9: Check the box and commit**

Tick the UV-scale line in `design.md` §15 and move the confirmed policy into §3.1, replacing its "verify in implementation" watch-out.

```bash
git add -u
git commit -m "Verify the UV display scale against a real archive

design.md §15 box 2. The declared unit is now authoritative; the magnitude
heuristic survives only for traces that declare nothing, and its warning says
so rather than implying doubt about a stated unit."
```

**Note:** use `git add -u` and named paths, never `git add -A` — there is uncommitted user work in this tree.

---

### Task 6: Box 2 — settle the `MWave0..3` wavelength mapping

**Files:**
- Modify: `docs/format-findings.md`, `elusive-core/src/parse/ngc.rs` (`wavelength_for` ~line 364, `DEFAULT_WAVELENGTHS` ~line 27), `elusive-core/tests/real_archive.rs`

**Interfaces:**
- Consumes: `fixture()` from Task 3.
- Produces: a verified mapping; the "assumed order" warning fires only when the method genuinely omits it.

**Why second, and in the same sitting as Task 5:** together these answer one question — is the concentration number real? This decides which trace is 280 nm. If `MWave2` is actually 260, concentration is computed from the wrong trace *and* with an ε meant for a different wavelength, so the two errors compound.

- [ ] **Step 1: Search the method and run metadata**

```bash
python3 - <<'EOF'
import zipfile, re
z = zipfile.ZipFile("testdata/sec-run.ngcAnalysis")
for n in ("Methods/MethodData1.xml", "Runs/RunInfo1.xml", "Runs/Run1.xml"):
    body = z.read(n).decode("utf-8","replace")
    print(f"===== {n}")
    for m in re.finditer(r"(?i)(wave|wavelength|nm\b|MWave)", body):
        s = max(0, m.start()-150)
        print("   ", body[s:m.start()+250].replace("\n"," ")[:380])
        print("   ---")
EOF
```

- [ ] **Step 2: Record the mapping**

Append `## Box 2 — MWave wavelength mapping` to `docs/format-findings.md`: the exact element path where each wavelength is declared, the mapping for this run, and whether the declaration is positional or explicitly keyed to `MWave<i>`. **That last distinction is the whole point** — a mapping that happens to be in order proves nothing about a run configured differently.

- [ ] **Step 3: Write the failing test**

```rust
#[test]
fn uv_wavelengths_come_from_the_method_not_from_trace_order() {
    // Box 1 (design.md §15). The fallback assigns 215/255/280/495 by position.
    // This asserts the method-derived values, so a regression to positional
    // order is caught even when position happens to agree.
    let run = fixture();
    for (id, nm) in [
        ("MWave0", /* from Step 1 */ 215u16),
        ("MWave1", 255),
        ("MWave2", 280),
        ("MWave3", 495),
    ] {
        let c = run.channel(&id.into()).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(c.wavelength_nm, Some(nm), "{id}");
    }
    assert!(
        !run.warnings.iter().any(|w| w.message.contains("fall back to the default order")),
        "the method declares the mapping, so the fallback warning must not fire"
    );
}
```

- [ ] **Step 4: Run it**

Run: `cargo test -p elusive-core --test real_archive uv_wavelengths -- --nocapture`
Expected: FAIL on the warning assertion if `wavelength_for` is not reading the element found in Step 1.

- [ ] **Step 5: Point `wavelength_for` at the real element**

Adjust it to read the path recorded in Step 2. Keep `DEFAULT_WAVELENGTHS` and its warning for archives that declare nothing.

- [ ] **Step 6: Verify and commit**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git add -u
git commit -m "Read UV wavelengths from the method rather than trace order

design.md §15 box 1."
```

---

### Task 7: Box 3 — reconcile the two `Trace_Fractions_*` entries

**Files:**
- Modify: `docs/format-findings.md`, `elusive-core/src/parse/ngc.rs` (~line 865), `elusive-core/tests/real_archive.rs`

**Why third:** wrong reconciliation means wrong fraction windows, wrong wells in the Results *Fractions* column, and someone pooling the wrong tubes — a wet-lab consequence, not a display bug. v0.3.0 raised the stakes by surfacing well lists prominently.

- [ ] **Step 1: Compare the two entries**

```bash
python3 - <<'EOF'
import zipfile, base64, re
z = zipfile.ZipFile("testdata/sec-run.ngcAnalysis")
for n in ("Runs/Run1/Trace_Fractions_1.xml", "Runs/Run1/Trace_Fractions_19.xml"):
    raw = z.read(n).decode("utf-8","replace")
    m = re.search(r"<TraceData>(.*?)</TraceData>", raw, re.S)
    inner = base64.b64decode(m.group(1)).decode("utf-8","replace") if m else ""
    events = re.findall(r"<Event>([^<]+)</Event>", inner)
    tubes  = re.findall(r"<TubeNumber>([^<]+)</TubeNumber>", inner)
    print(f"===== {n}: {len(inner)} bytes inner")
    print(f"  events: {len(events)}  distinct: {sorted(set(events))}")
    print(f"  tubes:  {len(tubes)}  range: {min(tubes, default='-')}..{max(tubes, default='-')}")
    print(f"  first record:\n{inner[:700]}\n")
EOF
```

- [ ] **Step 2: Record which is authoritative and why**

Append `## Box 3 — duplicate fraction traces`. State which entry is the full stream and which the summary, whether the summary is a strict subset, whether `FractionDone` events appear in both, and therefore which one should win. Note whether the loser adds anything the winner lacks — if the summary carries a field the stream omits, reconciliation is a merge, not a choice.

- [ ] **Step 3: Write the failing test**

```rust
#[test]
fn fraction_windows_come_from_the_authoritative_trace() {
    // Box 3 (design.md §15). The archive carries two Trace_Fractions entries —
    // 350 KB and 885 bytes. Picking the wrong one shifts every fraction
    // boundary, which shifts the wells reported for a peak.
    let run = fixture();
    assert_eq!(run.fractions.len(), /* count from Step 1 */ 0);

    // Windows must tile forward without overlapping.
    let mut sorted = run.fractions.clone();
    sorted.sort_by(|a, b| a.vol_start_ml.total_cmp(&b.vol_start_ml));
    for pair in sorted.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(a.vol_end_ml <= b.vol_start_ml + 1e-4, "overlap: {a:?} then {b:?}");
    }

    // `end_estimated` must reflect reality, not convenience: a measured
    // FractionDone must not be reported as inferred, nor the reverse.
    let inferred = run.fractions.iter().filter(|f| f.end_estimated).count();
    assert_eq!(inferred, /* from Step 1 */ 0, "inferred-boundary count");
}
```

- [ ] **Step 4: Run, then implement deterministic reconciliation**

Run: `cargo test -p elusive-core --test real_archive fraction_windows -- --nocapture`

Implement the rule from Step 2 in `ngc.rs`. It must be deterministic and commented with *why* that entry wins. Keep a warning when both are present and disagree — agreement is not guaranteed on other instruments.

- [ ] **Step 5: Verify and commit**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git add -u
git commit -m "Reconcile the duplicate fraction traces deterministically

design.md §15 box 3."
```

---

### Task 8: Box 4 — confirm rack geometry and collection pattern

**Files:**
- Modify: `docs/format-findings.md`, `elusive-core/tests/real_archive.rs`, possibly `elusive-core/src/wells.rs`

**Why fourth:** HEP96 serpentine is already marked confirmed in `wells.rs`, degradation is graceful and loud, and the tube number stays correct even when the label does not. This is scope confirmation more than bug-fixing — but the *Fractions* column made well labels user-visible, so it is no longer cosmetic.

- [ ] **Step 1: Read the rack fields from the fixture**

```bash
python3 - <<'EOF'
import zipfile, base64, re, collections
z = zipfile.ZipFile("testdata/sec-run.ngcAnalysis")
raw = z.read("Runs/Run1/Trace_Fractions_1.xml").decode("utf-8","replace")
inner = base64.b64decode(re.search(r"<TraceData>(.*?)</TraceData>", raw, re.S).group(1)).decode("utf-8","replace")
for tag in ("RackType","CollectionPattern","FractionCollectorType","RackNumber","FractionSize"):
    print(tag, collections.Counter(re.findall(rf"<{tag}>([^<]*)</{tag}>", inner)))
EOF
```

- [ ] **Step 2: Ask the user which collectors they actually use**

This box has two halves. The fixture answers "what does this run use". Only the user answers "do other rack types appear in your workflows". Record both.

- [ ] **Step 3: Write the test**

```rust
#[test]
fn every_fraction_resolves_to_a_plate_position() {
    // Box 6 (design.md §15). An unresolved well is not a crash — the tube number
    // is still right — but the Results Fractions column shows well labels, so an
    // unresolved rack silently degrades a user-visible answer.
    let run = fixture();
    let unresolved: Vec<u32> = run
        .fractions
        .iter()
        .filter(|f| f.well.is_none())
        .map(|f| f.tube)
        .collect();
    assert!(unresolved.is_empty(), "tubes without a plate position: {unresolved:?}");

    assert!(
        !run.warnings.iter().any(|w| w.message.contains("assumed serpentine")),
        "the archive declares a pattern, so it must not be assumed"
    );
}
```

- [ ] **Step 4: Run, extend `wells.rs` only if the fixture demands it, verify, commit**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git add -u
git commit -m "Confirm HEP96 geometry and collection pattern against a real run

design.md §15 box 6."
```

---

### Task 9: Box 5 — V0/Vt availability

**Files:**
- Modify: `docs/format-findings.md`, `elusive-core/tests/real_archive.rs`, possibly `elusive-core/src/parse/ngc.rs`

**Why fifth:** three layers of honesty already sit between a wrong answer and a wrong decision — it falls back to a volume-based fit, it is user-enterable, and the output is labelled an estimate.

- [ ] **Step 1: Search the method for column geometry**

```bash
python3 - <<'EOF'
import zipfile, re
body = zipfile.ZipFile("testdata/sec-run.ngcAnalysis").read("Methods/MethodData1.xml").decode("utf-8","replace")
for kw in ("Column","Void","V0","Vt","TotalVolume","BedVolume","Diameter","Length","PathLength","CV"):
    for m in list(re.finditer(kw, body))[:3]:
        s = max(0, m.start()-140)
        print(f"[{kw}] {body[s:m.start()+220]}".replace("\n"," ")[:340]); print("  ---")
EOF
```

- [ ] **Step 2: Record, then write whichever test the finding supports**

If V0/Vt are present, assert `run.meta.v0_ml` / `vt_ml` are `Some` with the recorded values. If absent, assert they are `None` **and** that the Kav path correctly falls back — a negative result still deserves a test pinning the fallback.

```rust
#[test]
fn column_volumes_are_read_when_the_method_declares_them() {
    // Box 4 (design.md §15).
    let run = fixture();
    assert_eq!(run.meta.v0_ml, /* Some(x) or None from Step 1 */ None);
    assert_eq!(run.meta.vt_ml, None);
}
```

- [ ] **Step 3: Verify and commit**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git add -u
git commit -m "Pin V0/Vt availability from the method file

design.md §15 box 4."
```

---

### Task 10: Box 6 — extinction coefficient and path length source

**Files:**
- Modify: `docs/format-findings.md`, `elusive-core/tests/real_archive.rs`, possibly `elusive-core/src/parse/ngc.rs`

**Why last:** already honest by construction. `e_mg_per_ml` defaults to `1.0` specifically so the arithmetic is a pass-through and the user can see it is not yet a concentration. The only open question is whether the method carries these — convenience, not correctness.

- [ ] **Step 1: Search for flow-cell geometry**

```bash
python3 - <<'EOF'
import zipfile, re
z = zipfile.ZipFile("testdata/sec-run.ngcAnalysis")
for n in ("Methods/MethodData1.xml", "Runs/RunInfo1.xml"):
    body = z.read(n).decode("utf-8","replace")
    for kw in ("PathLength","FlowCell","Cell","Extinction","Epsilon","mm\\b","cm\\b"):
        for m in list(re.finditer(kw, body))[:2]:
            s = max(0, m.start()-130)
            print(f"[{n} {kw}] {body[s:m.start()+200]}".replace("\n"," ")[:320]); print("  ---")
EOF
```

- [ ] **Step 2: Record and test**

```rust
#[test]
fn flow_cell_path_length_is_read_when_declared() {
    // Box 5 (design.md §15). ε is a property of the molecule, never of the run,
    // so it stays manual by design; only the path length could come from here.
    let run = fixture();
    assert_eq!(run.meta.path_length_cm, /* Some(0.2) or None */ None);
}
```

- [ ] **Step 3: Verify and commit**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git add -u
git commit -m "Pin the path-length source from the method file

design.md §15 box 5."
```

---

### Task 11: Correct `design.md` §3 and close out §15

**Files:**
- Modify: `design.md`, `CLAUDE.md`, `docs/format-findings.md`

- [ ] **Step 1: Fix the documented archive layout**

§3 currently shows `Method/MethodData.xml` and omits `AnalysisRunViewSettings1.xml`. The real listing is `Methods/MethodData1.xml` (plural, numbered), `Methods/MethodInfo1.xml`, `Runs/RunInfo1.xml`, `Runs/AnalysisRunViewSettings1.xml`, and `Runs/Run1/Trace_<Name>_<idx>.xml`. Replace the layout block with the verified one and note the trailing index in trace names.

- [ ] **Step 2: Tick every box a test now proves**

Only those with a passing fixture test. Any box the fixture could not answer stays unchecked with a one-line note saying what evidence is still missing — an unchecked box with a reason is more useful than a checked one with a hedge.

- [ ] **Step 3: Update `CLAUDE.md`**

Its "Immediate implementation direction" says confirmation comes before construction and lists opening real files as step 1. Rewrite that section to reflect what is now verified, and update the "Known unresolved format questions" list to only what remains.

- [ ] **Step 4: Verify and commit**

```bash
cargo test --workspace
git add -u
git commit -m "Record the verified NGC layout and close the settled §15 boxes"
```

---

### Task 12: Open the pull request

- [ ] **Step 1: Push and open**

```bash
git push -u origin feat/verify-ngc-format
gh pr create --base main --title "Verify the NGC format against a real export" --body "..."
```

The body should state which boxes closed, which did not and why, and link `docs/format-findings.md` as the evidence. Note explicitly that `testdata/sec-run.ngcAnalysis` is sanitized and was eyeballed by a human before commit.

- [ ] **Step 2: Confirm CI is green on all four jobs**

Run: `gh pr checks --watch`

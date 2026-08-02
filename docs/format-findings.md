# NGC format findings

Evidence log for the `design.md` §15 verification. Every claim here was read out
of a real Bio-Rad NGC export (Superdex 200 10/300 GL SEC run, 16 channels, 75
fractions, 37.91 mL, 22759 UV samples), sanitized and committed as
`testdata/sec-run.ngcAnalysis`.

Ordering follows the priority agreed with the user: widest blast radius first.

## Corrections to `design.md` §3

The documented layout is wrong in detail. Verified listing:

| Documented | Actual |
|---|---|
| `Method/MethodData.xml` | `Methods/MethodData1.xml` |
| `Method/MethodInfo.xml` | `Methods/MethodInfo1.xml` |
| `Runs/Run1.xml`, `RunInfo1.xml` | `Runs/Run1.xml`, `Runs/RunInfo1.xml` |
| *(absent)* | `Runs/AnalysisRunViewSettings1.xml` |

Trace entries carry a **trailing index that is not the wavelength index**:
`Trace_MWave0_8.xml`, `Trace_MWave1_2.xml`, `Trace_MWave2_14.xml`,
`Trace_MWave3_4.xml`. Nothing may be inferred from that suffix.

## Analysis.xml — undocumented, and the most valuable entry

6.1 MB, and it holds **ChromLab's own integration results**: 58 `<Peak>` records
across four `<AlgorithmParameters>` blocks. Per peak: `Area`, `RelativeArea`,
`Height`, `Center`, `FWHM`, `BaseStartY`, `BaseEndY`, `Asymmetry`,
`StartIndex`/`EndIndex`/`CenterIndex`, `Manual`, `Exclude`, `PathLength`,
`ExtinctionCoefficient`, `MolecularWeight`.

**Peak-to-channel association is by GUID**: each block's `<RunDataId>` matches a
trace's `<OriginalRunDataId>`. Verified mapping in this run:

| Trace | Peaks |
|---|---|
| MWave3 | 31 |
| MWave0 | 12 |
| MWave2 | 8 |
| MWave1 | 7 |

**Every value has a `Raw` twin** (`Area`/`AreaRaw`, `Height`/`HeightRaw`, …).
This is *not* a unit conversion — ratios sit at 0.90–1.07, not 1000. The index
pairs give it away: `StartIndex` 5 versus `StartIndexRaw` 84, `EndIndex` 23
versus `EndIndexRaw` 376, a consistent ~16× factor. The non-`Raw` series is
decimated or smoothed; `Raw` indexes the full-rate trace. Both resolve to nearly
the same volume, so they describe one peak in two sampling spaces.

## Box 2 — UV value scale: **AU stored, mAU displayed (×1000)**. Confirmed.

Two independent measurements agree, so this needed no hand integration.

- Raw `TraceData` payload, before any scaling: MWave0 spans −0.06083 … **+0.22661**.
- ChromLab's own `Height` for the largest peak: **0.227303**.

They match, with ChromLab's height slightly the larger because it is measured
from a mildly negative baseline (`BaseStartY` ≈ −0.0152). ChromLab therefore
stores the same units as the payload, and a **227 mAU** peak is the plausible
reading of a prep SEC run — 0.227 mAU would be indistinguishable from noise.

**Consequence for the parser:** the trace XML header carries `Version`,
`OriginalRunDataId`, `TraceVersion`, `DeviceUID`, `TraceType`, `TraceData` — and
**no unit field at all**. So `display_scale_for`'s "honour the declared unit"
branch can never fire for an NGC trace; only the magnitude heuristic runs. The
fix is to make AU→mAU the *known convention* for NGC UV traces rather than a
guess inferred from amplitude, and to keep the heuristic only for formats that
genuinely declare nothing (CSV import).

## Box 1 — MWave→wavelength mapping: declared in the method. Confirmed.

`Methods/MethodData1.xml`:

```xml
<Wavelength1>215</Wavelength1>
<Wavelength2>255</Wavelength2>
<Wavelength3>280</Wavelength3>
<Wavelength4>495</Wavelength4>
<UV1_WaveLength>280</UV1_WaveLength>
```

**The indices are off by one against the trace names.** The method numbers from
1 (`Wavelength1..4`); traces number from 0 (`MWave0..3`). So `MWave0` → 215 nm,
`MWave1` → 255, `MWave2` → 280, `MWave3` → 495.

This run's mapping coincidentally equals `DEFAULT_WAVELENGTHS`, so the fallback
is right *here* and proves nothing — precisely why it must be read, not assumed.

Cross-check that the mapping is not reversed: MWave0 (215 nm, peptide bond) has
the largest signal at 0.227 AU while MWave2 (280 nm) reaches 0.0105 AU. A ~20×
ratio in favour of 215 nm is what protein absorbance looks like. Had the mapping
been inverted the physics would be wrong.

`UV1_WaveLength` = 280 is a separate field naming the *primary monitor*
wavelength; it is not the mapping and must not be used as one.

## Box 3 — duplicate `Trace_Fractions_*`: one is simply empty. Confirmed.

Simpler than `design.md` feared — it is not "full stream versus summary".

| Entry | Inner payload | Content |
|---|---|---|
| `Trace_Fractions_1.xml` | 350 069 B | 150 records: 75 `FractionStart` + 75 `FractionDone`, tubes 1–75 |
| `Trace_Fractions_19.xml` | 256 B | `<RootNodeOfCFCData><Node /></RootNodeOfCFCData>` — **no records** |

Reconciliation rule: **prefer the entry that contains records.** Warn only if
both are non-empty and disagree, which did not occur here.

**Every fraction has a measured `FractionDone`**, so all 75 boundaries are real
and `Fraction::end_estimated` must be `false` for all of them in this fixture.

## Box 6 — rack geometry and collection pattern: fully declared. Confirmed.

From the 150 fraction records, unanimous across every one:

| Field | Value |
|---|---|
| `RackType` | `HEP96` |
| `CollectionPattern` | `Serpentine` |
| `FractionCollectorType` | **`Hawkeye`** |
| `RackNumber` | `1` |
| `FractionSize` | `0.4` mL |
| `TubeNumber` | 1 … 75 |

HEP96 8×12 serpentine is confirmed against a real run. The remaining half of
this box — whether *other* rack types appear in the user's workflows — is a
question about lab practice, not about the format, and stays open.

## Box 4 — V0/Vt: **not present.** Confirmed absent.

No `Void`, `V0`, `Vt`, `BedVolume`, or `TotalVolume` element exists anywhere in
`Methods/MethodData1.xml`. Kav-based fitting cannot be automatic; the volume-based
fallback is correct and stays.

But the column *identity* is recorded:

```xml
<ColumnType>Superdex 200 10/300 GL</ColumnType>
<ColumnPosition>C2 Port 5</ColumnPosition>
```

`RunMeta.column` already exists and should be populated from this. A named
column has published V0/Vt (a Superdex 200 10/300 GL is ≈24 mL bed, ≈8 mL void),
so a small lookup table could *offer* values for the user to confirm — offered
and labelled, never silently assumed.

## Box 5 — path length and ε: path length yes, ε no. Confirmed.

Every one of the 58 peaks carries `<PathLength>0.5</PathLength>` — unanimous.
**`ConcentrationInputs::default()` uses 0.2 cm, which is wrong for this
instrument.**

`<ExtinctionCoefficient xsi:nil="true" />` in all 58, and `<MolecularWeight
xsi:nil="true" />` likewise. ε is genuinely absent, confirming the existing
design decision to keep it manual — it is a property of the molecule, not the
run.

Caveat: path length was found in `Analysis.xml`, which exists only in
`.ngcAnalysis` exports. A `.ngcMethodruns` archive may not carry it. Needs a
second fixture to settle.

## Sanitizer gap found and fixed

The first sanitized fixture still leaked. `Analysis.xml` embeds the run name a
second time as `<AnalysisName>Analysis of &lt;run&gt;</AnalysisName>`, which the
tag list missed while correctly redacting `RunName` in `Run1.xml` and
`RunInfo1.xml`. `AnalysisName` was added to `SENSITIVE_TAGS`; the regenerated
fixture makes 22 redactions and greps clean for sample codes, run numbers and
e-mail addresses.

`ColumnType` and `ColumnPosition` are deliberately **not** redacted — they are
instrument configuration, not sample identity, and Box 4 needs them.

## Opportunity noted, not acted on

Because `Analysis.xml` carries ChromLab's peaks with a GUID link to each trace,
EluSive could **import ChromLab's own integrations** as a starting point, or
cross-check its own integration maths against them. That is a feature, not a
verification task, and belongs in its own plan.

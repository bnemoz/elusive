# EluSive Design System

**Version 1.1.0**  
**Principle:** *Precise analysis. Invisible by design.*

EluSive is a visual system for scientific chromatography software. The interface should recede so the chromatogram, peaks, integrations, and analytical decisions remain visually dominant.

## 1. Brand model

- **Precise:** geometry, alignment, numbers, and controls are exact.
- **Reliable:** state changes are explicit; destructive actions are never ambiguous.
- **Pure:** minimal decoration, clean surfaces, no unnecessary gradients.
- **Invisible:** brand color supports the data; it never competes with it.

The logo mark is a three-peak chromatogram on a quiet baseline. The largest central peak creates an instantly recognizable silhouette without adding a separate emblem.

## 2. Typography

| Role | Font | Size | Weight | Use |
|---|---|---:|---:|---|
| Display | Inter | 32 px | 500 | Splash screen and empty states |
| H1 | Inter | 24 px | 600 | Window or workspace title |
| H2 | Inter | 20 px | 600 | Major panel headings |
| H3 | Inter | 16 px | 600 | Cards and sections |
| Body | Inter | 14 px | 400 | Controls and explanatory text |
| Small | Inter | 12 px | 400/500 | Metadata and table headers |
| Micro | Inter | 11 px | 500 | Axis labels and compact badges |
| Code/data | JetBrains Mono | 13 px | 400 | File paths, identifiers, exact values |

Fallbacks: UI — `Segoe UI`, `Noto Sans`, `Arial`, `sans-serif`; code — `Cascadia Mono`, `SFMono-Regular`, `Consolas`, `monospace`.

Use tabular numerals for retention time, area, height, width, and concentration values. Right-align numeric table columns.

## 3. Core color tokens

| Token | Hex | Role |
|---|---|---|
| `INK_950` | `#07111F` | Dark app shell, splash background |
| `INK_900` | `#0D1B2A` | Dark panels and navigation |
| `INK_800` | `#14283B` | Elevated dark surfaces |
| `BLUE_700` | `#245A9A` | Primary action on light surfaces |
| `BLUE_600` | `#3274BD` | Hover/selection stroke |
| `BLUE_500` | `#4C8FD8` | Primary accent on dark surfaces |
| `BLUE_300` | `#9FC7EE` | Secondary text on dark surfaces |
| `ICE_100` | `#EAF4FC` | Selected rows, chart selections |
| `MIST_50` | `#F7FAFD` | Light application background |
| `SLATE_700` | `#40566C` | Secondary text |
| `SLATE_500` | `#6E8193` | Axes and muted labels |
| `SLATE_200` | `#D7E1EA` | Borders and grid lines |
| `SUCCESS_600` | `#267B70` | Validated / passing result |
| `WARNING_600` | `#A96B19` | Review required |
| `DANGER_600` | `#B44555` | Error / excluded region |

Accessibility anchors: white on `INK_950` ≈ 18.9:1; `INK_950` on `MIST_50` ≈ 18.1:1; `BLUE_700` on white ≈ 7.0:1; `BLUE_600` on white ≈ 4.8:1.

## 4. Chart and peak language

**Default trace:** `#2F6FB3`, 1.5 px.  
**Selected trace:** 2.25 px.  
**Grid:** `SLATE_200`, 1 px, no minor grid unless zoomed.  
**Baseline:** `SLATE_500`; use dashed styling only for calculated or extrapolated baselines.  
**Integrated area:** `#9FC7EE66`; keep transparency so the raw signal remains readable.  
**Excluded region:** `#B4455526` plus a boundary stroke or hatch.

Categorical overlay sequence:

`#2F6FB3 · #56A8D8 · #6B70C8 · #2E9599 · #8A6FB8 · #C4773D · #3F8B63 · #B44D68`

Reserve hue for distinguishing samples, peak families, or states. Never encode pass/fail or quality by hue alone: pair color with an icon, label, stroke pattern, or table status.

Peak labels use `P1`, `P2`, … in 11 px semibold. Retention time remains a separate numeric field. Avoid placing long sample names directly on the plot.

## 5. Layout tokens

- Base spacing unit: **4 px**.
- Control height: **32 px** compact, **36 px** standard.
- Panel padding: **16 px**; dense tables may use **12 px**.
- Card radius: **8 px**; large modal/splash surfaces: **12 px**.
- Border: **1 px** `SLATE_200`; focus ring: **2 px** `BLUE_500`.
- Use shadows sparingly. Prefer a border plus surface contrast.
- **Measure:** a label/value form caps at **800 px** of content and centres in the
  space left over; below **480 px** it takes the full width instead of shrinking
  further. Data surfaces — chromatogram, plate, wide numeric tables — are exempt
  and still use the whole viewport. A form that fills a 4K window separates a
  label from its value by the width of the screen, and the row stops reading as
  a pair.
- Label/value rows put the label in a **fixed 168 px column** with the value
  immediately beside it, never justified to the opposite edge. The fixed column
  aligns every field in the app on one x and cannot jitter between frames.

## 6. Component rules

### Navigation
Dark `INK_900` rail with white primary text and `BLUE_300` secondary labels. The active item uses an `INK_800` fill, a 2 px `BLUE_500` indicator, and no glow.

### Buttons
Primary button: `BLUE_700` fill, white label. Secondary button: white fill, `SLATE_200` border, `INK_950` label. Destructive actions stay neutral until confirmation, then use `DANGER_600`.

### Data tables
White surface, 12 px headers, 14 px values, 40 px rows. Right-align numeric data. Use `ICE_100` for selection, not zebra striping by default. Fixed-width digits are recommended for dense analytical tables.

### Forms
Labels appear above controls. Units belong in the label or suffix, never only in placeholder text. Validation messages should state the problem and the corrective action.

### Empty/loading states
Use the chromatogram mark at low contrast. Prefer deterministic text such as “Import a chromatogram to begin” and “Analyzing 4 of 12 traces” over generic spinners.

## 7. Writing and naming

Use concise scientific language: “Integrate peak,” “Set baseline,” “Exclude region,” “Export report.” Avoid playful microcopy in analytical or regulated workflows. Preserve instrument terminology and units exactly.

Naming convention in code:

- Rust constants: `SCREAMING_SNAKE_CASE`
- Token groups/modules: singular lowercase (`color`, `spacing`, `chart`)
- Component names: `UpperCamelCase`
- Files: `snake_case.rs`

## 8. Rust implementation

Use `src/theme.rs` as the dependency-free equivalent of CSS variables. UI toolkits should adapt these tokens rather than redefining them. `src/egui_adapter.rs` demonstrates an optional `egui` mapping.

```rust
mod theme;
use theme::{chart, color, LIGHT};

let workspace = LIGHT.app_bg;
let trace = chart::PRIMARY_TRACE;
let selected_row = color::ICE_100;
```

For runtime customization, parse `elusive-theme.toml` with `serde` + `toml`, validate every token, and fall back to the compiled constants if parsing fails.

## 9. Non-negotiable rules

1. The data is always more saturated than the application chrome.
2. The raw trace remains visible beneath integrations and annotations.
3. Every status uses text or shape in addition to color.
4. Numeric columns align by decimal meaning, not visually by chance.
5. Animation never delays an analytical action.
6. Dark mode changes surfaces, not semantic meaning.
7. New colors require a named token and documented purpose.

## 10. EluSive domain extensions (prep SEC / NGC)

*Added in 1.1.0.* Sections 1–9 are domain-generic (HPLC-style peaks, retention time).
EluSive is a **preparative SEC / Bio-Rad NGC** tool, so it adds volume-axis, fraction,
and 96-well-plate language. These extensions **compose existing tokens** wherever
possible; the one new token group (`plate`) is named and purpose-documented per rule #9.

### 10.1 Axis and terminology
- The chromatogram x-axis is **elution volume (mL)**, not retention time. Where §4
  says "retention time," read **elution volume (Ve)**; keep time available as a
  secondary readout. Numeric fields still use tabular numerals, right-aligned (§2).
- Peak labels remain `P1, P2, …` (§4). SEC size results add an **estimated MW** field
  derived from the calibration curve; concentration is a separate A280 field.

### 10.2 Fraction bands (on the trace)
- Fraction windows: full-height zones filled at very low alpha (≈18/255, with
  alternating fractions a few steps apart so adjacent windows are separable), plus
  a hairline boundary stroke.

  *Revised in 0.2.0.* This previously specified 1 px ticks at the baseline, on the
  grounds that a full-height band would compete with the trace. In use the ticks
  turned out to be worse: they sit at the baseline, so they leave the viewport as
  soon as the user pans or zooms vertically, and the fraction windows — the thing
  the plate view is keyed to — become invisible exactly when someone is inspecting
  a peak closely. A zone faint enough to read *through* satisfies rule #2 while
  staying anchored to the data rather than to the viewport.
- Collected span fill (when a fraction is highlighted): `ICE_100` at low alpha on
  light, `INK_800` on dark. Hovered/selected fraction boundary: `BLUE_600` stroke.
- Fraction id badges use the Small/Micro type ramp; never place long labels on-plot.
- Zones are drawn from the **data's** y-extent, never from the plot's current
  bounds. An overlay sized from the current bounds re-enters the next frame's
  auto-bounds and inflates the scale on every repaint (see `chromatogram.rs`).

### 10.3 96-well plate heatmap
Wells are colored by a live-selected channel + metric (integrated area / max / mean /
value-at-center). Because the plate **is data**, it may be fully saturated (rule #1),
but it must still obey rule #3 — every well shows its numeric value and well label
(`A1…H12`); color never carries meaning alone.

- **`plate` token group** (sequential, single-hue → luminance-ordered, so it is
  colorblind-safe). Stops interpolate in linear RGB from low to high:
  `MIST_50 → BLUE_300 → BLUE_500 → BLUE_700`. Empty/uncollected well = `panel_bg`
  with a `SLATE_200` (light) / `INK_700` (dark) border.
- Show a vertical scale legend with min/max and the active channel + metric.
- Accessibility: offer an optional perceptually-uniform ramp (e.g. viridis) behind a
  toggle for users who prefer it; the default stays on-brand blue.
- Linking (see product design.md §9): hovering a well highlights its volume span on
  the trace using the 10.2 selected-fraction styling, and vice versa.

### 10.4 Channel overlays beyond eight
`chart::SERIES` holds 8 colors, but a run can carry 10+ channels (UV1–4, conductivity,
%B, pH, several pressures, temperature, flow). To distinguish without relying on hue
alone (rule #3):
- Channel *i* uses `SERIES[i % 8]` with a **stroke dash pattern** from
  `{ solid, dashed (6,4), dotted (2,4) }` selected by `i / 8`. First eight solid,
  next eight dashed, etc.
- A channel's ChromLab legend color may override `SERIES` **only if** it meets the
  contrast anchors (§3) on the current surface; otherwise fall back to `SERIES`.
- The hero UV trace uses `chart::PRIMARY_TRACE` regardless of position.
- A color the **user** picks from the legend swatch outranks all three, including
  the contrast gate above. That gate exists to reject a color the *instrument*
  happened to record; substituting a different one for a color the user typed
  would make the hex field lie about what is on screen. When their pick falls
  below `chart::MIN_TRACE_CONTRAST`, the legend shows a warning glyph plus a
  tooltip beside the swatch — rule #3, never color alone — and draws it anyway.
  Overrides are always opaque and are stored per channel in the sidecar.

### 10.5 Well and region states (status by shape + color, rule #3)
- Empty vs collected vs selected wells differ by **fill + border**, not hue alone.
- Integrated-peak regions use `color::INTEGRATED_AREA`; excluded regions use
  `color::EXCLUDED_REGION` plus a boundary stroke or hatch (§4).
- A fraction flagged out-of-spec pairs `WARNING_600`/`DANGER_600` with an icon and a
  table status column — never color only.

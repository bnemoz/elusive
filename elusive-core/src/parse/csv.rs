//! Secondary import path: ChromLab's "Analysis" and "Traces" CSV exports.
//!
//! Two quirks from `design.md` §3.3 drive the whole implementation:
//!
//! - The file is **ISO-8859-1 with mixed CRLF/LF**. Decoding as UTF-8 mangles the
//!   degree sign in `°C` and can fail outright.
//! - Channels are **independent (volume, value) pairs of columns**, not rows of a
//!   table. Column pairs run out at different lengths and the trailing cells are
//!   empty. Reading it as a rectangular table silently invents zero samples.
//!
//! Analysis CSV carries no fractions, so a CSV-only run has an empty plate. That
//! is communicated by [`crate::model::SourceFormat::supports_fractions`] rather
//! than by showing an empty grid.

use crate::error::{Error, Result};
use crate::model::{Channel, ChannelKind, Color, Run, RunMeta, Sample, SourceFormat, Warning};
use std::path::Path;

/// Open an Analysis CSV, optionally merging a Traces CSV legend from beside it.
pub fn open(path: impl AsRef<Path>) -> Result<Run> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let text = decode_latin1(&bytes);
    let mut run = parse_analysis_csv(&text, path)?;

    // A sibling `*_Traces.csv` (or `* Traces.csv`) holds the channel legend.
    if let Some(legend_path) = find_sibling_legend(path) {
        match std::fs::read(&legend_path) {
            Ok(b) => {
                let legend = parse_traces_csv(&decode_latin1(&b))?;
                let merged = merge_legend(&mut run, &legend);
                run.warnings.push(Warning::new(
                    "legend",
                    format!(
                        "merged {merged} channel colour/unit entries from {}",
                        legend_path.display()
                    ),
                ));
            }
            Err(e) => run.warnings.push(Warning::new(
                "legend",
                format!("could not read {}: {e}", legend_path.display()),
            )),
        }
    }

    Ok(run)
}

/// ChromLab writes ISO-8859-1. `encoding_rs`'s WINDOWS_1252 is a superset that
/// maps every byte, so decoding never fails and never loses a character.
pub fn decode_latin1(bytes: &[u8]) -> String {
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
    text.into_owned()
}

fn find_sibling_legend(path: &Path) -> Option<std::path::PathBuf> {
    let dir = path.parent()?;
    let stem = path.file_stem()?.to_str()?;
    for candidate in [
        format!("{stem}_Traces.csv"),
        format!("{stem} Traces.csv"),
        format!("{stem}Traces.csv"),
        "Traces.csv".to_string(),
    ] {
        let p = dir.join(&candidate);
        if p.is_file() && p != path {
            return Some(p);
        }
    }
    None
}

/// One channel's legend row from the Traces CSV.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LegendEntry {
    pub name: String,
    pub color: Option<Color>,
    pub unit: String,
    pub technique: String,
    pub show: Option<bool>,
}

/// Parse the Analysis CSV: run name row, paired header row, then data.
pub fn parse_analysis_csv(text: &str, path: &Path) -> Result<Run> {
    let mut lines = text.lines().enumerate();

    let (_, name_row) = lines.next().ok_or(Error::Csv {
        line: 1,
        detail: "file is empty".into(),
    })?;
    let (header_idx, header_row) = lines.next().ok_or(Error::Csv {
        line: 2,
        detail: "missing the paired header row".into(),
    })?;

    let run_name = split_csv_line(name_row)
        .into_iter()
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled run")
                .to_string()
        });

    let headers = split_csv_line(header_row);
    let columns = pair_columns(&headers).ok_or(Error::Csv {
        line: header_idx + 1,
        detail: "header row is not made of `<channel>_volume`,`<channel>_<unit>` pairs".into(),
    })?;
    if columns.is_empty() {
        return Err(Error::Csv {
            line: header_idx + 1,
            detail: "no channel column pairs found".into(),
        });
    }

    let mut channels: Vec<Channel> = columns
        .iter()
        .map(|c| {
            let kind = if is_baseline_column(&c.name) {
                // ChromLab's own baseline is reference data, not a measurement:
                // keeping it off the UV axis stops it being picked as the hero
                // trace or integrated by accident (`design.md` §3.3).
                ChannelKind::Other
            } else {
                ChannelKind::from_trace_name(&c.name)
            };
            let mut ch = Channel::new(c.name.clone(), c.name.clone(), kind);
            ch.unit = c.unit.clone();
            ch.display_unit = c.unit.clone();
            ch.wavelength_nm = wavelength_in_text(&c.name);
            ch
        })
        .collect();

    let mut warnings = Vec::new();
    let mut ragged = 0usize;

    for (idx, line) in lines {
        if line.trim().is_empty() {
            continue;
        }
        let cells = split_csv_line(line);
        for (ci, col) in columns.iter().enumerate() {
            let v = cells.get(col.volume_idx).map(|s| s.trim()).unwrap_or("");
            let y = cells.get(col.value_idx).map(|s| s.trim()).unwrap_or("");
            // Trailing empties mean this channel simply ended earlier than others.
            if v.is_empty() || y.is_empty() {
                continue;
            }
            match (parse_number(v), parse_number(y)) {
                (Some(volume_ml), Some(value)) => {
                    channels[ci]
                        .samples
                        .push(Sample::new(f32::NAN, volume_ml, value));
                }
                _ => ragged += 1,
            }
        }
        let _ = idx;
    }

    if ragged > 0 {
        warnings.push(Warning::new(
            "csv",
            format!("{ragged} cell pair(s) could not be parsed as numbers and were skipped"),
        ));
    }

    channels.retain(|c| !c.samples.is_empty());
    if channels.is_empty() {
        return Err(Error::Csv {
            line: header_idx + 1,
            detail: "no channel produced any samples".into(),
        });
    }

    // The CSV has no time column, so time is reconstructed as unknown rather than
    // faked; volume is the axis everything downstream uses anyway.
    warnings.push(Warning::new(
        "csv",
        "Analysis CSV carries no fraction records, so the plate view is unavailable for this run"
            .to_string(),
    ));

    Ok(Run {
        meta: RunMeta {
            run_name,
            ..RunMeta::default()
        },
        source_format: SourceFormat::AnalysisCsv,
        source_path: path.to_path_buf(),
        channels,
        fractions: Vec::new(),
        events: Vec::new(),
        warnings,
    })
}

/// Parse the Traces CSV legend: `Show, Type, Color (#AARRGGBB), Min Y, Max Y,
/// Units, Method, Start Time, End Time, Technique`, keyed by header name so
/// column order changes do not break the import.
pub fn parse_traces_csv(text: &str) -> Result<Vec<LegendEntry>> {
    let mut lines = text.lines();
    let header = lines.next().ok_or(Error::Csv {
        line: 1,
        detail: "traces legend is empty".into(),
    })?;
    let headers: Vec<String> = split_csv_line(header)
        .into_iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect();

    let find = |needle: &str| headers.iter().position(|h| h.contains(needle));
    let color_idx = find("color").or_else(|| find("colour"));
    let unit_idx = find("unit");
    let technique_idx = find("technique");
    let show_idx = find("show");
    // The trace name is either an explicit column or the unlabelled first one.
    let name_idx = find("name").or_else(|| find("trace")).unwrap_or(0);

    let mut out = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let cells = split_csv_line(line);
        let cell = |i: Option<usize>| {
            i.and_then(|i| cells.get(i))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        let name = cells
            .get(name_idx)
            .map(|s| s.trim())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        out.push(LegendEntry {
            name,
            color: Color::parse_argb(&cell(color_idx)),
            unit: cell(unit_idx),
            technique: cell(technique_idx),
            show: match cell(show_idx).to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => Some(true),
                "false" | "no" | "0" => Some(false),
                _ => None,
            },
        });
    }

    Ok(out)
}

/// Copy colours and units from a legend onto matching channels.
/// Returns how many channels were updated.
pub fn merge_legend(run: &mut Run, legend: &[LegendEntry]) -> usize {
    let mut merged = 0;
    for ch in run.channels.iter_mut() {
        let Some(entry) = legend
            .iter()
            .find(|e| names_match(&e.name, &ch.name) || names_match(&e.name, ch.id.as_str()))
        else {
            continue;
        };
        if ch.color.is_none() {
            ch.color = entry.color;
        }
        if ch.unit.is_empty() && !entry.unit.is_empty() {
            ch.unit = entry.unit.clone();
            ch.display_unit = entry.unit.clone();
        }
        merged += 1;
    }
    merged
}

/// Loose comparison: ignore case, spaces, and underscores, so `UV 1_280` from the
/// legend lines up with `UV1_280` from the data header.
fn names_match(a: &str, b: &str) -> bool {
    let squash = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let (a, b) = (squash(a), squash(b));
    !a.is_empty() && a == b
}

fn is_baseline_column(name: &str) -> bool {
    name.trim().to_ascii_lowercase().starts_with("baseline")
}

/// A `<channel>_volume` / `<channel>_<unit>` column pair.
#[derive(Clone, Debug, PartialEq)]
struct ColumnPair {
    name: String,
    unit: String,
    volume_idx: usize,
    value_idx: usize,
}

/// Match `_volume` headers to the value header that follows them.
///
/// The value column's suffix after the shared channel prefix is the unit, e.g.
/// `UV 1_280_mAU` paired with `UV 1_280_volume` yields unit `mAU`.
fn pair_columns(headers: &[String]) -> Option<Vec<ColumnPair>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < headers.len() {
        let h = headers[i].trim();
        let lower = h.to_ascii_lowercase();
        if let Some(stem) = lower
            .strip_suffix("_volume")
            .or_else(|| lower.strip_suffix("_ml"))
        {
            let name = h[..stem.len()].to_string();
            let value_idx = i + 1;
            let unit = headers
                .get(value_idx)
                .map(|v| {
                    let v = v.trim();
                    v.strip_prefix(&format!("{name}_"))
                        .or_else(|| v.strip_prefix(&name))
                        .map(|s| s.trim_start_matches('_').to_string())
                        .unwrap_or_else(|| v.to_string())
                })
                .unwrap_or_default();
            if value_idx < headers.len() {
                out.push(ColumnPair {
                    name,
                    unit,
                    volume_idx: i,
                    value_idx,
                });
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    Some(out)
}

/// Split one CSV line, honouring double quotes and `""` escapes.
pub fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => out.push(std::mem::take(&mut cur)),
            '\r' if !in_quotes => {}
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn parse_number(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f32>()
        .ok()
        .or_else(|| t.replace(',', ".").parse::<f32>().ok())
        .filter(|v| v.is_finite())
}

fn wavelength_in_text(text: &str) -> Option<u16> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i - start == 3 {
                if let Ok(n) = text[start..i].parse::<u16>() {
                    if (190..=900).contains(&n) {
                        return Some(n);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn quoted_fields_and_embedded_commas_survive_splitting() {
        let cells = split_csv_line(r#"a,"b,c","say ""hi""",,d"#);
        assert_eq!(cells, vec!["a", "b,c", r#"say "hi""#, "", "d"]);
    }

    #[test]
    fn carriage_returns_do_not_leak_into_cells() {
        assert_eq!(split_csv_line("a,b\r"), vec!["a", "b"]);
    }

    #[test]
    fn latin1_bytes_decode_without_mangling() {
        // 0xB0 is the degree sign in ISO-8859-1; it is invalid UTF-8 on its own.
        let bytes = b"Temperature_\xB0C";
        assert_eq!(decode_latin1(bytes), "Temperature_°C");
    }

    #[test]
    fn channels_keep_independent_lengths() {
        // UV has three points, conductivity two: the third conductivity cell pair
        // is empty and must not become a sample.
        let csv = "\
Run A,Run A,Run A,Run A
UV 1_280_volume,UV 1_280_mAU,Cond_volume,Cond_mS/cm
0.0,10,0.0,5
1.0,20,1.0,6
2.0,30,,
";
        let run = parse_analysis_csv(csv, &PathBuf::from("run.csv")).unwrap();
        assert_eq!(run.channels.len(), 2);
        let uv = run
            .channels
            .iter()
            .find(|c| c.id.as_str().contains("280"))
            .unwrap();
        let cond = run
            .channels
            .iter()
            .find(|c| c.id.as_str().contains("Cond"))
            .unwrap();
        assert_eq!(uv.samples.len(), 3);
        assert_eq!(cond.samples.len(), 2);
    }

    #[test]
    fn units_and_wavelength_come_out_of_the_header() {
        let csv = "\
Run A,Run A
UV 1_280_volume,UV 1_280_mAU
0.0,10
";
        let run = parse_analysis_csv(csv, &PathBuf::from("run.csv")).unwrap();
        assert_eq!(run.channels[0].unit, "mAU");
        assert_eq!(run.channels[0].wavelength_nm, Some(280));
        assert_eq!(run.channels[0].kind, ChannelKind::Uv);
    }

    #[test]
    fn csv_runs_are_flagged_as_having_no_fractions() {
        let csv = "Run A,Run A\nUV 1_280_volume,UV 1_280_mAU\n0.0,10\n";
        let run = parse_analysis_csv(csv, &PathBuf::from("run.csv")).unwrap();
        assert!(run.fractions.is_empty());
        assert!(!run.source_format.supports_fractions());
        assert!(run
            .warnings
            .iter()
            .any(|w| w.message.contains("plate view")));
    }

    #[test]
    fn chromlab_baseline_columns_stay_off_the_uv_axis() {
        let csv = "\
Run A,Run A,Run A,Run A
UV 1_280_volume,UV 1_280_mAU,Baseline of UV 1_280_volume,Baseline of UV 1_280_mAU
0.0,10,0.0,1
";
        let run = parse_analysis_csv(csv, &PathBuf::from("run.csv")).unwrap();
        let baseline = run
            .channels
            .iter()
            .find(|c| c.id.as_str().starts_with("Baseline"))
            .expect("baseline column imported");
        assert_eq!(baseline.kind, ChannelKind::Other);
        // The hero trace must be the measurement, not ChromLab's baseline.
        assert_eq!(run.hero_channel().unwrap().wavelength_nm, Some(280));
    }

    #[test]
    fn a_missing_header_row_is_an_error_with_a_line_number() {
        let err = parse_analysis_csv("only one line\n", &PathBuf::from("x.csv")).unwrap_err();
        assert!(matches!(err, Error::Csv { line: 2, .. }));
    }

    #[test]
    fn headers_that_are_not_paired_are_rejected() {
        let csv = "Run A,Run A\nsomething,else\n1,2\n";
        assert!(parse_analysis_csv(csv, &PathBuf::from("x.csv")).is_err());
    }

    #[test]
    fn traces_legend_is_read_by_header_name() {
        let csv = "\
Name,Show,Type,Color,Min Y,Max Y,Units,Method,Start Time,End Time,Technique
UV 1_280,True,UV,#FF2F6FB3,0,1000,mAU,SEC,0,60,SEC
";
        let legend = parse_traces_csv(csv).unwrap();
        assert_eq!(legend.len(), 1);
        assert_eq!(legend[0].name, "UV 1_280");
        assert_eq!(legend[0].unit, "mAU");
        assert_eq!(legend[0].show, Some(true));
        assert_eq!(legend[0].color, Some(Color::new(0x2F, 0x6F, 0xB3, 0xFF)));
    }

    #[test]
    fn legend_merges_onto_channels_despite_spacing_differences() {
        let csv = "Run A,Run A\nUV1_280_volume,UV1_280_mAU\n0.0,10\n";
        let mut run = parse_analysis_csv(csv, &PathBuf::from("x.csv")).unwrap();
        let legend = vec![LegendEntry {
            name: "UV 1_280".into(),
            color: Some(Color::new(1, 2, 3, 255)),
            unit: "mAU".into(),
            technique: "SEC".into(),
            show: Some(true),
        }];
        assert_eq!(merge_legend(&mut run, &legend), 1);
        assert_eq!(run.channels[0].color, Some(Color::new(1, 2, 3, 255)));
    }

    #[test]
    fn argb_colours_parse_with_and_without_alpha() {
        assert_eq!(
            Color::parse_argb("#FF2F6FB3"),
            Some(Color::new(0x2F, 0x6F, 0xB3, 0xFF))
        );
        assert_eq!(
            Color::parse_argb("2F6FB3"),
            Some(Color::new(0x2F, 0x6F, 0xB3, 0xFF))
        );
        assert_eq!(Color::parse_argb("not a colour"), None);
    }
}

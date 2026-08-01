//! Parser for the native `.ngcAnalysis` / `.ngcMethodruns` archives.
//!
//! Both are ZIP files laid out as (`design.md` §3):
//!
//! ```text
//! Version.txt                              (methodruns only)
//! Method/MethodData.xml, MethodInfo.xml
//! Runs/Run1.xml, RunInfo1.xml
//! Runs/Run1/Trace_<Name>_<idx>.xml
//! Analysis.xml                             (analysis export only)
//! ```
//!
//! Entries are located by *pattern*, not by exact path, so an extra directory
//! level or a `Run2` does not break the import.

use crate::error::{Error, Result};
use crate::model::{
    Channel, ChannelKind, Color, Fraction, LogEvent, Run, RunMeta, Sample, SourceFormat, Warning,
};
use crate::parse::xml::{self, Leaf};
use crate::wells::{self, CollectionPattern, RackGeometry};
use base64::Engine as _;
use std::collections::BTreeMap;
use std::io::{Read, Seek};
use std::path::Path;

/// Fallback UV wavelengths for `MWave0..3` when the method XML does not name them.
/// Using these unconditionally would be a silent lie, so a warning always
/// accompanies them (`IMPLEMENTATION_PLAN.md` Phase 1).
pub const DEFAULT_WAVELENGTHS: [u16; 4] = [215, 255, 280, 495];

/// Version word expected at the head of a signal-trace payload.
const TRACE_FORMAT_VERSION: u32 = 1;

/// Bytes per record: three little-endian f32s.
const RECORD_BYTES: usize = 12;

/// Open an NGC archive from disk.
pub fn open(path: impl AsRef<Path>) -> Result<Run> {
    let path = path.as_ref();
    let format = detect_format(path)?;
    let file = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let reader = std::io::BufReader::new(file);
    from_reader(reader, path, format)
}

fn detect_format(path: &Path) -> Result<SourceFormat> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "ngcanalysis" => Ok(SourceFormat::NgcAnalysis),
        "ngcmethodruns" => Ok(SourceFormat::NgcMethodruns),
        other => Err(Error::unsupported(format!(
            "'{other}' is not an NGC archive extension (expected .ngcAnalysis or .ngcMethodruns)"
        ))),
    }
}

/// Parse an already-open archive. Split out from [`open`] so tests can feed a
/// synthetic archive from memory.
pub fn from_reader<R: Read + Seek>(reader: R, path: &Path, format: SourceFormat) -> Result<Run> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| Error::Zip {
        path: path.to_path_buf(),
        source: e,
    })?;

    let entries: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
    let mut warnings: Vec<Warning> = Vec::new();

    // --- Metadata -----------------------------------------------------------
    let mut meta = RunMeta::default();
    let mut method_leaves: Vec<Leaf> = Vec::new();

    for entry in entries.iter().filter(|e| is_method_entry(e)) {
        match read_entry_text(&mut archive, entry, path) {
            Ok(text) => method_leaves.extend(xml::leaves(entry, &text)?),
            Err(e) => warnings.push(Warning::new(entry.clone(), e.to_string())),
        }
    }

    let mut run_leaves: Vec<Leaf> = Vec::new();
    for entry in entries.iter().filter(|e| is_run_info_entry(e)) {
        match read_entry_text(&mut archive, entry, path) {
            Ok(text) => run_leaves.extend(xml::leaves(entry, &text)?),
            Err(e) => warnings.push(Warning::new(entry.clone(), e.to_string())),
        }
    }

    fill_meta(&mut meta, &run_leaves, &method_leaves);
    if meta.run_name.is_empty() {
        meta.run_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled run")
            .to_string();
    }

    let wavelengths = resolve_wavelengths(&method_leaves, &run_leaves, &mut warnings);

    // --- Traces -------------------------------------------------------------
    let mut channels: Vec<Channel> = Vec::new();
    let mut fraction_sources: Vec<(String, Vec<Fraction>)> = Vec::new();
    let mut events: Vec<LogEvent> = Vec::new();

    let trace_entries: Vec<String> = entries
        .iter()
        .filter(|e| is_trace_entry(e))
        .cloned()
        .collect();

    for entry in &trace_entries {
        let text = match read_entry_text(&mut archive, entry, path) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(Warning::new(entry.clone(), e.to_string()));
                continue;
            }
        };
        let leaves = xml::leaves(entry, &text)?;
        let Some(payload) = xml::first_text_any(&leaves, &["TraceData", "Data", "Points"]) else {
            warnings.push(Warning::new(
                entry.clone(),
                "no <TraceData> payload; entry skipped".to_string(),
            ));
            continue;
        };
        let blob = decode_base64(entry, &payload)?;
        let trace_name = trace_name_from_entry(entry).unwrap_or_else(|| entry.clone());

        if is_fraction_trace(&trace_name) {
            match parse_fraction_payload(entry, &blob) {
                Ok(f) => fraction_sources.push((entry.clone(), f)),
                Err(e) => warnings.push(Warning::new(entry.clone(), e.to_string())),
            }
        } else if is_event_trace(&trace_name) {
            match parse_event_payload(entry, &blob) {
                Ok(mut e) => events.append(&mut e),
                Err(e) => warnings.push(Warning::new(entry.clone(), e.to_string())),
            }
        } else {
            match build_channel(
                entry,
                &trace_name,
                &leaves,
                &blob,
                &wavelengths,
                &mut warnings,
            ) {
                Ok(c) => channels.push(c),
                Err(e) => warnings.push(Warning::new(entry.clone(), e.to_string())),
            }
        }
    }

    if channels.is_empty() && fraction_sources.is_empty() {
        return Err(Error::NoRunData {
            path: path.to_path_buf(),
        });
    }

    // Stable, human-sensible channel order: UV first (by wavelength), then the
    // other kinds in enum order, then alphabetically inside a kind.
    channels.sort_by(|a, b| {
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then(a.wavelength_nm.cmp(&b.wavelength_nm))
            .then(a.name.cmp(&b.name))
    });

    let mut fractions = reconcile_fraction_sources(fraction_sources, &mut warnings);
    assign_wells(&mut fractions, &mut warnings);
    fractions.sort_by_key(|f| f.tube);

    events.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));

    Ok(Run {
        meta,
        source_format: format,
        source_path: path.to_path_buf(),
        channels,
        fractions,
        events,
        warnings,
    })
}

fn kind_rank(kind: ChannelKind) -> u8 {
    match kind {
        ChannelKind::Uv => 0,
        ChannelKind::Conductivity => 1,
        ChannelKind::PercentB => 2,
        ChannelKind::Ph => 3,
        ChannelKind::Pressure => 4,
        ChannelKind::Flow => 5,
        ChannelKind::Temperature => 6,
        ChannelKind::Other => 7,
    }
}

// --- Entry classification ---------------------------------------------------

fn normalise(entry: &str) -> String {
    entry.replace('\\', "/").to_ascii_lowercase()
}

fn is_method_entry(entry: &str) -> bool {
    let e = normalise(entry);
    e.starts_with("method/") && e.ends_with(".xml")
}

fn is_run_info_entry(entry: &str) -> bool {
    let e = normalise(entry);
    if !e.ends_with(".xml") {
        return false;
    }
    let file = e.rsplit('/').next().unwrap_or(&e);
    // `Runs/Run1.xml` and `Runs/RunInfo1.xml`, but not `Runs/Run1/Trace_*.xml`.
    (file.starts_with("run") && !file.starts_with("runs")) || file == "analysis.xml"
}

fn is_trace_entry(entry: &str) -> bool {
    let e = normalise(entry);
    let file = e.rsplit('/').next().unwrap_or(&e);
    file.starts_with("trace_") && file.ends_with(".xml")
}

/// `Runs/Run1/Trace_MD_Conductivity_7.xml` → `MD_Conductivity`.
///
/// The trailing `_<idx>` is a per-archive ordinal, not part of the channel
/// identity, so it is stripped — but only when it really is all digits.
pub fn trace_name_from_entry(entry: &str) -> Option<String> {
    let file = entry.replace('\\', "/");
    let file = file.rsplit('/').next()?;
    let stem = file
        .strip_suffix(".xml")
        .or_else(|| file.strip_suffix(".XML"))?;
    let rest = stem
        .strip_prefix("Trace_")
        .or_else(|| stem.strip_prefix("trace_"))?;
    match rest.rsplit_once('_') {
        Some((name, idx)) if !idx.is_empty() && idx.chars().all(|c| c.is_ascii_digit()) => {
            Some(name.to_string())
        }
        _ => Some(rest.to_string()),
    }
}

fn is_fraction_trace(name: &str) -> bool {
    name.to_ascii_lowercase().contains("fraction")
}

fn is_event_trace(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("event") || n.contains("annotation") || n.contains("logbook")
}

fn read_entry_text<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    entry: &str,
    path: &Path,
) -> Result<String> {
    let mut file = archive.by_name(entry).map_err(|e| Error::Zip {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| Error::io(entry, e))?;
    // ChromLab writes UTF-8 with a BOM on some entries and latin-1 on others;
    // lossy decoding keeps a stray byte from failing an otherwise good run.
    let text = String::from_utf8_lossy(strip_bom(&bytes)).into_owned();
    Ok(text)
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn decode_base64(entry: &str, payload: &str) -> Result<Vec<u8>> {
    // The payload is pretty-printed inside the XML, so whitespace must go first.
    let compact: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .map_err(|source| Error::Base64 {
            entry: entry.to_string(),
            source,
        })
}

// --- Signal traces ----------------------------------------------------------

/// Decode the binary signal payload: a `u32` version followed by `N` records of
/// three little-endian `f32`s — `[time_s, value, volume_mL]` (`design.md` §3.1).
pub fn decode_signal_blob(entry: &str, blob: &[u8]) -> Result<(Vec<Sample>, Option<Warning>)> {
    if blob.len() < 4 {
        return Err(Error::trace(
            entry,
            format!("payload is {} bytes, too short for a header", blob.len()),
        ));
    }
    let version = u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]);
    let body = &blob[4..];
    if body.len() % RECORD_BYTES != 0 {
        return Err(Error::trace(
            entry,
            format!(
                "payload body is {} bytes, not a multiple of the {RECORD_BYTES}-byte record size",
                body.len()
            ),
        ));
    }

    let warning = (version != TRACE_FORMAT_VERSION).then(|| {
        Warning::new(
            entry,
            format!(
                "trace format version {version} differs from the verified version \
                 {TRACE_FORMAT_VERSION}; values may be misread"
            ),
        )
    });

    let samples = body
        .chunks_exact(RECORD_BYTES)
        .map(|c| {
            let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
            Sample::new(f(0), f(8), f(4))
        })
        .collect();

    Ok((samples, warning))
}

fn build_channel(
    entry: &str,
    trace_name: &str,
    leaves: &[Leaf],
    blob: &[u8],
    wavelengths: &BTreeMap<usize, u16>,
    warnings: &mut Vec<Warning>,
) -> Result<Channel> {
    let (samples, version_warning) = decode_signal_blob(entry, blob)?;
    if let Some(w) = version_warning {
        warnings.push(w);
    }

    let kind = ChannelKind::from_trace_name(trace_name);
    let declared_name = xml::first_text_any(leaves, &["Name", "TraceName", "DisplayName", "Title"]);
    let unit =
        xml::first_text_any(leaves, &["Unit", "Units", "YUnit", "UnitName"]).unwrap_or_default();
    let color = xml::first_text_any(leaves, &["Color", "Colour", "TraceColor"])
        .and_then(|s| Color::parse_argb(&s));

    let mut channel = Channel::new(trace_name, String::new(), kind);
    channel.samples = samples;
    channel.unit = unit.clone();
    channel.color = color;

    if kind == ChannelKind::Uv {
        channel.wavelength_nm = wavelength_for(trace_name, leaves, wavelengths);
    }

    channel.name = match (&declared_name, channel.wavelength_nm) {
        (_, Some(nm)) => format!("UV {nm} nm"),
        (Some(n), None) if !n.trim().is_empty() => n.trim().to_string(),
        _ => pretty_trace_name(trace_name),
    };

    let (scale, display_unit, scale_warning) = display_scale_for(entry, kind, &unit, &channel);
    channel.display_scale = scale;
    channel.display_unit = display_unit;
    if let Some(w) = scale_warning {
        warnings.push(w);
    }

    Ok(channel)
}

/// `MD_Conductivity` → `Conductivity`; `SamplePumpFlowRate` → `Sample Pump Flow Rate`.
fn pretty_trace_name(raw: &str) -> String {
    let trimmed = raw.strip_prefix("MD_").unwrap_or(raw);
    let mut out = String::with_capacity(trimmed.len() + 4);
    for (i, ch) in trimmed.chars().enumerate() {
        if i > 0 && ch.is_ascii_uppercase() {
            let prev = trimmed.chars().nth(i - 1).unwrap_or(' ');
            if prev.is_ascii_lowercase() || prev.is_ascii_digit() {
                out.push(' ');
            }
        }
        out.push(if ch == '_' { ' ' } else { ch });
    }
    out
}

/// Storage-vs-display scale policy (`design.md` §15, open question 2).
///
/// Raw values are always stored untouched. UV is the only channel where ChromLab's
/// displayed unit differs from the stored one: the archive holds AU, the software
/// shows mAU. When the entry declares its unit we honour it. When it does not we
/// fall back to a magnitude test — prep-scale UV in AU peaks below ~20, in mAU it
/// runs into the hundreds — and always warn, because a wrong guess here is a
/// factor of 1000 on every reported area.
fn display_scale_for(
    entry: &str,
    kind: ChannelKind,
    unit: &str,
    channel: &Channel,
) -> (f32, String, Option<Warning>) {
    if kind != ChannelKind::Uv {
        let display_unit = if unit.is_empty() {
            default_unit_for(kind).to_string()
        } else {
            unit.to_string()
        };
        return (1.0, display_unit, None);
    }

    let u = unit.trim().to_ascii_lowercase();
    if u == "mau" {
        return (1.0, "mAU".to_string(), None);
    }
    if u == "au" {
        return (1000.0, "mAU".to_string(), None);
    }

    let peak = channel
        .samples
        .iter()
        .filter(|s| s.is_finite())
        .map(|s| s.value.abs())
        .fold(0.0f32, f32::max);

    if peak > 20.0 {
        (
            1.0,
            "mAU".to_string(),
            Some(Warning::new(
                entry,
                format!(
                    "UV unit not declared; peak value {peak:.1} suggests the trace is already \
                     in mAU, so no scaling was applied"
                ),
            )),
        )
    } else {
        (
            1000.0,
            "mAU".to_string(),
            Some(Warning::new(
                entry,
                format!(
                    "UV unit not declared; peak value {peak:.4} suggests AU, so values are \
                     displayed as mAU (x1000)"
                ),
            )),
        )
    }
}

fn default_unit_for(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Uv => "mAU",
        ChannelKind::Conductivity => "mS/cm",
        ChannelKind::PercentB => "%",
        ChannelKind::Ph => "pH",
        ChannelKind::Pressure => "psi",
        ChannelKind::Flow => "mL/min",
        ChannelKind::Temperature => "°C",
        ChannelKind::Other => "",
    }
}

// --- Wavelength mapping -----------------------------------------------------

/// Resolve `MWave<i>` → nm from the method/run XML.
///
/// Three layers, most trustworthy first: an explicit `Wavelength<i>` field in the
/// method, a wavelength embedded in the trace's own declared name, and finally the
/// documented default order — which always raises a warning because
/// `design.md` §3.1 is explicit that the order must not be assumed.
fn resolve_wavelengths(
    method_leaves: &[Leaf],
    run_leaves: &[Leaf],
    warnings: &mut Vec<Warning>,
) -> BTreeMap<usize, u16> {
    let mut map = BTreeMap::new();

    for leaf in method_leaves.iter().chain(run_leaves.iter()) {
        let lname = leaf.name.to_ascii_lowercase();
        if !lname.contains("wavelength") && !lname.contains("wave_length") {
            continue;
        }
        // Index may be on the element name (`Wavelength1`) or in an attribute.
        let idx = trailing_index(&leaf.name).or_else(|| {
            leaf.attrs
                .iter()
                .find(|(k, _)| {
                    let k = k.to_ascii_lowercase();
                    k == "index" || k == "id" || k == "number"
                })
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        });
        let nm = xml::parse_f32(&leaf.text)
            .or_else(|| {
                leaf.attrs
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("value") || k.eq_ignore_ascii_case("nm"))
                    .and_then(|(_, v)| xml::parse_f32(v))
            })
            .filter(|v| (150.0..=1000.0).contains(v));

        if let (Some(idx), Some(nm)) = (idx, nm) {
            // ChromLab numbers detectors from 1, trace entries from 0.
            let zero_based = idx.saturating_sub(usize::from(idx > 0));
            map.insert(zero_based, nm as u16);
        }
    }

    if map.is_empty() {
        warnings.push(Warning::new(
            "wavelengths",
            format!(
                "no wavelength mapping found in the method XML; assuming MWave0..3 = {} nm \
                 in order. Verify against the method before reporting UV numbers.",
                DEFAULT_WAVELENGTHS
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join("/")
            ),
        ));
    } else if map.len() < DEFAULT_WAVELENGTHS.len() {
        warnings.push(Warning::new(
            "wavelengths",
            format!(
                "method XML named {} of 4 UV wavelengths; the rest fall back to the default order",
                map.len()
            ),
        ));
    }

    map
}

/// Trailing digits of an identifier: `MWave2` → 2, `Wavelength` → None.
fn trailing_index(name: &str) -> Option<usize> {
    let digits: String = name
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}

fn wavelength_for(
    trace_name: &str,
    leaves: &[Leaf],
    resolved: &BTreeMap<usize, u16>,
) -> Option<u16> {
    // 1. An explicit wavelength on the trace entry itself wins.
    if let Some(nm) = xml::first_text_any(leaves, &["Wavelength", "WaveLength", "Nanometers"])
        .and_then(|s| xml::parse_f32(&s))
        .filter(|v| (150.0..=1000.0).contains(v))
    {
        return Some(nm as u16);
    }

    let idx = trailing_index(trace_name);

    // 2. The method-derived map.
    if let Some(idx) = idx {
        if let Some(nm) = resolved.get(&idx) {
            return Some(*nm);
        }
    }

    // 3. A wavelength spelled out in the declared name, e.g. "UV 1_280".
    if let Some(name) = xml::first_text_any(leaves, &["Name", "TraceName", "DisplayName"]) {
        if let Some(nm) = wavelength_in_text(&name) {
            return Some(nm);
        }
    }
    if let Some(nm) = wavelength_in_text(trace_name) {
        return Some(nm);
    }

    // 4. Documented fallback order.
    idx.and_then(|i| DEFAULT_WAVELENGTHS.get(i).copied())
}

/// Find a plausible detector wavelength written into a free-text name.
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

// --- Metadata ---------------------------------------------------------------

fn fill_meta(meta: &mut RunMeta, run_leaves: &[Leaf], method_leaves: &[Leaf]) {
    let pick = |leaves: &[Leaf], names: &[&str]| xml::first_text_any(leaves, names);

    meta.run_name = pick(run_leaves, &["RunName", "Name", "Title"]).unwrap_or_default();
    meta.method_name = pick(method_leaves, &["MethodName", "Name", "Title"])
        .or_else(|| pick(run_leaves, &["MethodName"]))
        .unwrap_or_default();
    meta.technique = pick(method_leaves, &["Technique", "TechniqueName"])
        .or_else(|| pick(run_leaves, &["Technique"]))
        .unwrap_or_default();
    meta.started = pick(
        run_leaves,
        &["StartTime", "StartDateTime", "Started", "RunDate"],
    );
    meta.ended = pick(run_leaves, &["EndTime", "EndDateTime", "Ended"]);
    meta.column = pick(method_leaves, &["Column", "ColumnName", "ColumnType"])
        .or_else(|| pick(run_leaves, &["Column", "ColumnName"]));

    meta.v0_ml = pick(
        method_leaves,
        &["VoidVolume", "V0", "ColumnVoidVolume", "VoidVolumeMl"],
    )
    .as_deref()
    .and_then(xml::parse_f32)
    .filter(|v| *v > 0.0);

    meta.vt_ml = pick(
        method_leaves,
        &["ColumnVolume", "Vt", "TotalVolume", "BedVolume"],
    )
    .as_deref()
    .and_then(xml::parse_f32)
    .filter(|v| *v > 0.0);

    meta.path_length_cm = pick(
        method_leaves,
        &[
            "PathLength",
            "PathLengthCm",
            "CellPathLength",
            "FlowCellPathLength",
        ],
    )
    .as_deref()
    .and_then(xml::parse_f32)
    .filter(|v| *v > 0.0);
}

// --- Fractions --------------------------------------------------------------

/// The fraction payload is base64 of an *inner* XML document
/// (`RootNodeOfCFCData`) holding `<CFCData>` records (`design.md` §3.2).
fn parse_fraction_payload(entry: &str, blob: &[u8]) -> Result<Vec<Fraction>> {
    let text = std::str::from_utf8(strip_bom(blob))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| String::from_utf8_lossy(strip_bom(blob)).into_owned());

    if !text.contains('<') {
        return Err(Error::fractions(
            entry,
            "payload is not XML; expected a base64-encoded RootNodeOfCFCData document",
        ));
    }

    let records = xml::records(entry, &text, "CFCData")?;
    if records.is_empty() {
        return Err(Error::fractions(entry, "no <CFCData> records found"));
    }

    // A tube may appear twice, once as FractionStart and once as FractionDone;
    // merge them so each tube yields a single window.
    let mut by_tube: BTreeMap<u32, Fraction> = BTreeMap::new();

    for rec in &records {
        let tube =
            xml::field_any(rec, &["TubeNumberNotMinusOne", "TubeNumber"]).and_then(xml::parse_u32);
        let Some(tube) = tube.filter(|t| *t > 0) else {
            continue;
        };

        let event = xml::field(rec, "Event").unwrap_or_default().to_string();
        let rack = xml::field(rec, "RackNumber")
            .and_then(xml::parse_u32)
            .unwrap_or(1);
        // `VolumeStartSec` / `VolumeEndSec` are millilitres despite the name.
        let v_start = xml::field_any(rec, &["VolumeStartSec", "VolumeStart", "StartVolume"])
            .and_then(xml::parse_f32);
        let v_end = xml::field_any(rec, &["VolumeEndSec", "VolumeEnd", "EndVolume"])
            .and_then(xml::parse_f32);
        let t_start = xml::field_any(rec, &["TimeStartSec", "TimeStart", "StartTime"])
            .and_then(xml::parse_f32);
        let t_end =
            xml::field_any(rec, &["TimeEndSec", "TimeEnd", "EndTime"]).and_then(xml::parse_f32);
        let size = xml::field_any(rec, &["FractionSize", "NominalSize"]).and_then(xml::parse_f32);
        let rack_type = xml::field_any(rec, &["RackType", "RackName"])
            .unwrap_or_default()
            .to_string();
        let pattern = xml::field_any(rec, &["CollectionPattern", "Pattern"])
            .unwrap_or_default()
            .to_string();

        let slot = by_tube.entry(tube).or_insert_with(|| Fraction {
            tube,
            rack,
            well: None,
            vol_start_ml: f32::NAN,
            vol_end_ml: f32::NAN,
            time_start_s: f32::NAN,
            time_end_s: f32::NAN,
            nominal_size_ml: None,
            end_estimated: false,
            rack_type: String::new(),
            pattern: String::new(),
        });

        // Merge field-by-field: whichever record carries a value wins, and a
        // `FractionDone` record's end values override a placeholder from the start.
        if let Some(v) = v_start.filter(|v| v.is_finite()) {
            if !slot.vol_start_ml.is_finite() || event.eq_ignore_ascii_case("FractionStart") {
                slot.vol_start_ml = v;
            }
        }
        if let Some(v) = v_end.filter(|v| v.is_finite() && *v > 0.0) {
            if !slot.vol_end_ml.is_finite() || event.eq_ignore_ascii_case("FractionDone") {
                slot.vol_end_ml = v;
            }
        }
        if let Some(v) = t_start.filter(|v| v.is_finite()) {
            if !slot.time_start_s.is_finite() || event.eq_ignore_ascii_case("FractionStart") {
                slot.time_start_s = v;
            }
        }
        if let Some(v) = t_end.filter(|v| v.is_finite() && *v > 0.0) {
            if !slot.time_end_s.is_finite() || event.eq_ignore_ascii_case("FractionDone") {
                slot.time_end_s = v;
            }
        }
        if slot.nominal_size_ml.is_none() {
            slot.nominal_size_ml = size.filter(|v| v.is_finite() && *v > 0.0);
        }
        if slot.rack_type.is_empty() && !rack_type.is_empty() {
            slot.rack_type = rack_type;
        }
        if slot.pattern.is_empty() && !pattern.is_empty() {
            slot.pattern = pattern;
        }
        if rack > 0 {
            slot.rack = rack;
        }
    }

    // A fraction with a start but no recorded end (the run was stopped mid-fraction)
    // gets its nominal size as a fallback window rather than being dropped.
    for f in by_tube.values_mut() {
        if f.vol_start_ml.is_finite() && !f.vol_end_ml.is_finite() {
            if let Some(size) = f.nominal_size_ml {
                f.vol_end_ml = f.vol_start_ml + size;
                f.end_estimated = true;
            }
        }
    }

    Ok(by_tube.into_values().collect())
}

/// Deterministic reconciliation of the two `Trace_Fractions_*` entries.
///
/// `design.md` §6 notes one entry is the full stream and the other a summary.
/// Rather than trusting the file order, each candidate is scored by how many tubes
/// carry a *complete* volume window; the best-scoring source becomes the base and
/// the others only contribute tubes or end-volumes it is missing
/// (`IMPLEMENTATION_PLAN.md` Phase 1).
fn reconcile_fraction_sources(
    mut sources: Vec<(String, Vec<Fraction>)>,
    warnings: &mut Vec<Warning>,
) -> Vec<Fraction> {
    sources.retain(|(_, f)| !f.is_empty());
    if sources.is_empty() {
        return Vec::new();
    }
    if sources.len() == 1 {
        return sources.remove(0).1;
    }

    // Score on *measured* windows only. A summary trace whose ends were filled in
    // from the nominal fraction size looks complete but is not, and must not
    // outrank the stream that carries real `FractionDone` records.
    let score = |fractions: &[Fraction]| -> (usize, usize) {
        let measured = fractions
            .iter()
            .filter(|f| f.has_usable_window() && !f.end_estimated)
            .count();
        (measured, fractions.len())
    };

    sources.sort_by_key(|(name, f)| {
        let (complete, total) = score(f);
        // Descending on completeness then count; name breaks ties reproducibly.
        (
            std::cmp::Reverse(complete),
            std::cmp::Reverse(total),
            name.clone(),
        )
    });

    let (base_name, base) = sources.remove(0);
    let mut merged: BTreeMap<u32, Fraction> = base.into_iter().map(|f| (f.tube, f)).collect();

    let mut added = 0usize;
    let mut completed = 0usize;
    for (_, other) in sources {
        for f in other {
            match merged.get_mut(&f.tube) {
                None => {
                    merged.insert(f.tube, f);
                    added += 1;
                }
                Some(existing) => {
                    // A measured end always replaces a missing or estimated one.
                    let existing_is_weak =
                        !existing.vol_end_ml.is_finite() || existing.end_estimated;
                    if existing_is_weak && f.vol_end_ml.is_finite() && !f.end_estimated {
                        existing.vol_end_ml = f.vol_end_ml;
                        existing.end_estimated = false;
                        completed += 1;
                    }
                    if !existing.vol_start_ml.is_finite() && f.vol_start_ml.is_finite() {
                        existing.vol_start_ml = f.vol_start_ml;
                    }
                    if existing.rack_type.is_empty() {
                        existing.rack_type = f.rack_type.clone();
                    }
                    if existing.pattern.is_empty() {
                        existing.pattern = f.pattern.clone();
                    }
                }
            }
        }
    }

    if added > 0 || completed > 0 {
        warnings.push(Warning::new(
            "fractions",
            format!(
                "reconciled duplicate fraction traces against {base_name}: \
                 {added} tube(s) added, {completed} window(s) completed"
            ),
        ));
    }

    merged.into_values().collect()
}

fn assign_wells(fractions: &mut [Fraction], warnings: &mut Vec<Warning>) {
    let mut unknown_rack: Option<String> = None;
    let mut unknown_pattern: Option<String> = None;
    let mut out_of_range = 0usize;

    for f in fractions.iter_mut() {
        let Some(geometry) = RackGeometry::from_rack_type(&f.rack_type) else {
            if unknown_rack.is_none() && !f.rack_type.is_empty() {
                unknown_rack = Some(f.rack_type.clone());
            }
            continue;
        };
        let pattern = match CollectionPattern::from_str_lenient(&f.pattern) {
            Some(p) => p,
            None => {
                if unknown_pattern.is_none() && !f.pattern.is_empty() {
                    unknown_pattern = Some(f.pattern.clone());
                }
                // HEP96's confirmed default; recorded as a warning below.
                CollectionPattern::Serpentine
            }
        };
        f.well = wells::well_for_tube(f.tube, geometry, pattern);
        if f.well.is_none() {
            out_of_range += 1;
        }
    }

    if let Some(rack) = unknown_rack {
        warnings.push(Warning::new(
            "plate",
            format!(
                "rack type '{rack}' has no known geometry; those fractions have no well position"
            ),
        ));
    }
    if let Some(pattern) = unknown_pattern {
        warnings.push(Warning::new(
            "plate",
            format!("collection pattern '{pattern}' is unknown; assumed serpentine"),
        ));
    }
    if out_of_range > 0 {
        warnings.push(Warning::new(
            "plate",
            format!("{out_of_range} fraction(s) have a tube number past the end of the rack"),
        ));
    }
}

// --- Events -----------------------------------------------------------------

fn parse_event_payload(entry: &str, blob: &[u8]) -> Result<Vec<LogEvent>> {
    let text = String::from_utf8_lossy(strip_bom(blob)).into_owned();
    if !text.contains('<') {
        // Not fatal: the logbook is a nicety, not analysis data.
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for record_name in ["NextGenEvent", "LogBookEntry", "Event", "Annotation"] {
        let records = xml::records(entry, &text, record_name)?;
        for rec in records {
            let time_s =
                xml::field_any(&rec, &["TimeSec", "Time", "TimeStartSec"]).and_then(xml::parse_f32);
            let Some(time_s) = time_s else { continue };
            out.push(LogEvent {
                time_s,
                volume_ml: xml::field_any(&rec, &["Volume", "VolumeMl", "VolumeStartSec"])
                    .and_then(xml::parse_f32),
                kind: xml::field_any(&rec, &["Type", "EventType", "Category"])
                    .unwrap_or(record_name)
                    .to_string(),
                text: xml::field_any(&rec, &["Text", "Message", "Description", "Name"])
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        if !out.is_empty() {
            break;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_names_drop_the_trailing_ordinal_only_when_numeric() {
        assert_eq!(
            trace_name_from_entry("Runs/Run1/Trace_MD_Conductivity_7.xml").as_deref(),
            Some("MD_Conductivity")
        );
        assert_eq!(
            trace_name_from_entry("Runs/Run1/Trace_MWave0_1.xml").as_deref(),
            Some("MWave0")
        );
        assert_eq!(
            trace_name_from_entry("Runs/Run1/Trace_Fractions_2.xml").as_deref(),
            Some("Fractions")
        );
        // No numeric ordinal: keep the whole name.
        assert_eq!(
            trace_name_from_entry("Runs/Run1/Trace_PercentB.xml").as_deref(),
            Some("PercentB")
        );
        assert_eq!(trace_name_from_entry("Runs/Run1.xml"), None);
    }

    #[test]
    fn entry_classification_separates_traces_from_run_metadata() {
        assert!(is_trace_entry("Runs/Run1/Trace_MWave0_1.xml"));
        assert!(!is_run_info_entry("Runs/Run1/Trace_MWave0_1.xml"));
        assert!(is_run_info_entry("Runs/Run1.xml"));
        assert!(is_run_info_entry("Runs/RunInfo1.xml"));
        assert!(is_method_entry("Method/MethodData.xml"));
        assert!(!is_method_entry("Runs/Run1.xml"));
    }

    #[test]
    fn signal_blob_decodes_the_documented_triplet_order() {
        // One record: time = 1.5 s, value = 42.0, volume = 0.25 mL.
        let mut blob = 1u32.to_le_bytes().to_vec();
        blob.extend(1.5f32.to_le_bytes());
        blob.extend(42.0f32.to_le_bytes());
        blob.extend(0.25f32.to_le_bytes());

        let (samples, warning) = decode_signal_blob("t.xml", &blob).unwrap();
        assert!(warning.is_none());
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].time_s, 1.5);
        assert_eq!(samples[0].value, 42.0);
        assert_eq!(samples[0].volume_ml, 0.25);
    }

    #[test]
    fn signal_blob_record_count_matches_the_documented_formula() {
        let n = 37;
        let mut blob = 1u32.to_le_bytes().to_vec();
        for i in 0..n {
            blob.extend((i as f32).to_le_bytes());
            blob.extend((i as f32 * 2.0).to_le_bytes());
            blob.extend((i as f32 * 0.1).to_le_bytes());
        }
        assert_eq!((blob.len() - 4) / RECORD_BYTES, n);
        let (samples, _) = decode_signal_blob("t.xml", &blob).unwrap();
        assert_eq!(samples.len(), n);
    }

    #[test]
    fn truncated_signal_blob_is_an_error_not_a_panic() {
        let mut blob = 1u32.to_le_bytes().to_vec();
        blob.extend([0u8; 7]); // not a multiple of 12
        let err = decode_signal_blob("t.xml", &blob).unwrap_err();
        assert!(matches!(err, Error::MalformedTrace { .. }));

        assert!(decode_signal_blob("t.xml", &[0u8; 2]).is_err());
    }

    #[test]
    fn unexpected_trace_version_warns_but_still_decodes() {
        let mut blob = 9u32.to_le_bytes().to_vec();
        blob.extend(0.0f32.to_le_bytes());
        blob.extend(1.0f32.to_le_bytes());
        blob.extend(2.0f32.to_le_bytes());
        let (samples, warning) = decode_signal_blob("t.xml", &blob).unwrap();
        assert_eq!(samples.len(), 1);
        assert!(warning.is_some());
    }

    #[test]
    fn wavelength_is_read_out_of_a_free_text_name() {
        assert_eq!(wavelength_in_text("UV 1_280"), Some(280));
        assert_eq!(wavelength_in_text("MWave3 495nm"), Some(495));
        // Two-digit and four-digit numbers are not wavelengths.
        assert_eq!(wavelength_in_text("Pump 12"), None);
        assert_eq!(wavelength_in_text("Run 2024"), None);
    }

    #[test]
    fn trailing_index_only_reads_digits() {
        assert_eq!(trailing_index("MWave2"), Some(2));
        assert_eq!(trailing_index("Wavelength"), None);
        assert_eq!(trailing_index("Wavelength10"), Some(10));
    }

    #[test]
    fn pretty_names_split_camel_case_and_drop_the_module_prefix() {
        assert_eq!(pretty_trace_name("MD_Conductivity"), "Conductivity");
        assert_eq!(
            pretty_trace_name("SamplePumpFlowRate"),
            "Sample Pump Flow Rate"
        );
        assert_eq!(pretty_trace_name("PercentB"), "Percent B");
    }

    #[test]
    fn uv_in_au_is_scaled_to_mau_and_other_kinds_are_not() {
        let mut uv = Channel::new("MWave2", "UV", ChannelKind::Uv);
        uv.samples = vec![Sample::new(0.0, 0.0, 0.5)];
        let (scale, unit, warn) = display_scale_for("e", ChannelKind::Uv, "AU", &uv);
        assert_eq!(scale, 1000.0);
        assert_eq!(unit, "mAU");
        assert!(warn.is_none());

        let (scale, unit, _) = display_scale_for("e", ChannelKind::Uv, "mAU", &uv);
        assert_eq!(scale, 1.0);
        assert_eq!(unit, "mAU");

        let cond = Channel::new("MD_Conductivity", "Conductivity", ChannelKind::Conductivity);
        let (scale, unit, warn) = display_scale_for("e", ChannelKind::Conductivity, "mS/cm", &cond);
        assert_eq!(scale, 1.0);
        assert_eq!(unit, "mS/cm");
        assert!(warn.is_none());
    }

    #[test]
    fn undeclared_uv_unit_is_guessed_from_magnitude_and_always_warns() {
        let mut small = Channel::new("MWave2", "UV", ChannelKind::Uv);
        small.samples = vec![Sample::new(0.0, 0.0, 1.2)];
        let (scale, _, warn) = display_scale_for("e", ChannelKind::Uv, "", &small);
        assert_eq!(scale, 1000.0);
        assert!(warn.is_some(), "an assumed scale must be surfaced");

        let mut large = Channel::new("MWave2", "UV", ChannelKind::Uv);
        large.samples = vec![Sample::new(0.0, 0.0, 850.0)];
        let (scale, _, warn) = display_scale_for("e", ChannelKind::Uv, "", &large);
        assert_eq!(scale, 1.0);
        assert!(warn.is_some());
    }

    #[test]
    fn fraction_start_and_done_records_merge_into_one_window() {
        let xml = r#"<RootNodeOfCFCData>
          <CFCData><Event>FractionStart</Event><TubeNumber>1</TubeNumber>
            <VolumeStartSec>10.0</VolumeStartSec><TimeStartSec>600</TimeStartSec>
            <FractionSize>0.4</FractionSize><RackType>HEP96</RackType>
            <CollectionPattern>Serpentine</CollectionPattern><RackNumber>1</RackNumber></CFCData>
          <CFCData><Event>FractionDone</Event><TubeNumber>1</TubeNumber>
            <VolumeEndSec>10.4</VolumeEndSec><TimeEndSec>624</TimeEndSec></CFCData>
        </RootNodeOfCFCData>"#;

        let f = parse_fraction_payload("Trace_Fractions_1.xml", xml.as_bytes()).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].tube, 1);
        assert_eq!(f[0].vol_start_ml, 10.0);
        assert_eq!(f[0].vol_end_ml, 10.4);
        assert_eq!(f[0].rack_type, "HEP96");
        assert!(f[0].has_usable_window());
    }

    #[test]
    fn a_fraction_missing_its_end_falls_back_to_the_nominal_size() {
        let xml = r#"<RootNodeOfCFCData>
          <CFCData><Event>FractionStart</Event><TubeNumber>5</TubeNumber>
            <VolumeStartSec>20.0</VolumeStartSec><FractionSize>0.5</FractionSize></CFCData>
        </RootNodeOfCFCData>"#;
        let f = parse_fraction_payload("Trace_Fractions_1.xml", xml.as_bytes()).unwrap();
        assert_eq!(f[0].vol_end_ml, 20.5);
    }

    #[test]
    fn reconciliation_prefers_the_source_with_complete_windows() {
        let mk = |tube: u32, start: f32, end: f32| Fraction {
            tube,
            rack: 1,
            well: None,
            vol_start_ml: start,
            vol_end_ml: end,
            time_start_s: 0.0,
            time_end_s: 0.0,
            nominal_size_ml: None,
            end_estimated: false,
            rack_type: "HEP96".into(),
            pattern: "Serpentine".into(),
        };

        // A summary source listing three tubes with no end volumes, and a full
        // stream with two complete windows. The complete one must win.
        let summary = vec![
            mk(1, 1.0, f32::NAN),
            mk(2, 2.0, f32::NAN),
            mk(3, 3.0, f32::NAN),
        ];
        let full = vec![mk(1, 1.0, 1.4), mk(2, 2.0, 2.4)];

        let mut warnings = Vec::new();
        let merged = reconcile_fraction_sources(
            vec![("summary.xml".into(), summary), ("full.xml".into(), full)],
            &mut warnings,
        );

        assert_eq!(merged.len(), 3, "tube 3 from the summary should be kept");
        let t1 = merged.iter().find(|f| f.tube == 1).unwrap();
        assert_eq!(t1.vol_end_ml, 1.4, "the complete window must survive");
        assert!(!warnings.is_empty(), "a merge should be reported");
    }

    #[test]
    fn reconciliation_is_order_independent() {
        let mk = |tube: u32, end: f32| Fraction {
            tube,
            rack: 1,
            well: None,
            vol_start_ml: tube as f32,
            vol_end_ml: end,
            time_start_s: 0.0,
            time_end_s: 0.0,
            nominal_size_ml: None,
            end_estimated: false,
            rack_type: "HEP96".into(),
            pattern: "Serpentine".into(),
        };
        let a = vec![mk(1, f32::NAN), mk(2, f32::NAN)];
        let b = vec![mk(1, 1.4), mk(2, 2.4)];

        let mut w1 = Vec::new();
        let mut w2 = Vec::new();
        let forward = reconcile_fraction_sources(
            vec![("a".into(), a.clone()), ("b".into(), b.clone())],
            &mut w1,
        );
        let backward = reconcile_fraction_sources(vec![("b".into(), b), ("a".into(), a)], &mut w2);

        let ends = |v: &[Fraction]| v.iter().map(|f| f.vol_end_ml).collect::<Vec<_>>();
        assert_eq!(ends(&forward), ends(&backward));
    }

    #[test]
    fn wells_are_assigned_from_the_rack_type_and_pattern() {
        let mut fractions = vec![Fraction {
            tube: 13,
            rack: 1,
            well: None,
            vol_start_ml: 1.0,
            vol_end_ml: 1.4,
            time_start_s: 0.0,
            time_end_s: 0.0,
            nominal_size_ml: None,
            end_estimated: false,
            rack_type: "HEP96".into(),
            pattern: "Serpentine".into(),
        }];
        let mut warnings = Vec::new();
        assign_wells(&mut fractions, &mut warnings);
        assert_eq!(fractions[0].well.unwrap().label(), "B12");
        assert!(warnings.is_empty());
    }

    #[test]
    fn an_unknown_rack_type_warns_rather_than_guessing_a_well() {
        let mut fractions = vec![Fraction {
            tube: 1,
            rack: 1,
            well: None,
            vol_start_ml: 1.0,
            vol_end_ml: 1.4,
            time_start_s: 0.0,
            time_end_s: 0.0,
            nominal_size_ml: None,
            end_estimated: false,
            rack_type: "TUBES18".into(),
            pattern: "Serpentine".into(),
        }];
        let mut warnings = Vec::new();
        assign_wells(&mut fractions, &mut warnings);
        assert!(fractions[0].well.is_none());
        assert!(warnings.iter().any(|w| w.message.contains("TUBES18")));
    }
}

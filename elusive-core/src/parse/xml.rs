//! A deliberately shallow XML reader.
//!
//! ChromLab's archive entries are .NET `XmlSerializer` output, and the exact
//! wrapper hierarchy differs between entry kinds, ChromLab versions, and whether
//! the file came from an analysis or a method-run export. `design.md` §3 pins down
//! the *payloads* precisely (`<TraceData>` holds base64; `<CFCData>` records hold
//! the fraction fields) but not the elements around them.
//!
//! So rather than encode a hierarchy we cannot verify, this module flattens a
//! document into named leaves and named records. Locating `TraceData` by name
//! survives an extra wrapper element; a path-based lookup would not.

use crate::error::{Error, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

/// One leaf element: its local name, its text content, and its attributes.
#[derive(Clone, Debug, Default)]
pub struct Leaf {
    pub name: String,
    pub text: String,
    pub attrs: Vec<(String, String)>,
}

fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    }
}

fn xml_err(entry: &str, source: quick_xml::Error) -> Error {
    Error::Xml {
        entry: entry.to_string(),
        source,
    }
}

fn make_reader(xml: &str) -> Reader<&[u8]> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    // ChromLab exports are machine-written but not always strictly well-formed
    // about entity escaping in free-text fields; recovering beats refusing to
    // open a run because a sample name contained a stray ampersand.
    reader.config_mut().check_end_names = false;
    reader
}

/// Every element in the document that carries text or attributes, in document order.
pub fn leaves(entry: &str, xml: &str) -> Result<Vec<Leaf>> {
    let mut reader = make_reader(xml);
    let mut buf = Vec::new();
    let mut out: Vec<Leaf> = Vec::new();
    let mut stack: Vec<Leaf> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                stack.push(Leaf {
                    name: local_name(e.name().as_ref()),
                    text: String::new(),
                    attrs: read_attrs(entry, &e)?,
                });
            }
            Ok(Event::Empty(e)) => {
                out.push(Leaf {
                    name: local_name(e.name().as_ref()),
                    text: String::new(),
                    attrs: read_attrs(entry, &e)?,
                });
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = stack.last_mut() {
                    let decoded = t
                        .decode()
                        .map_err(|e| xml_err(entry, quick_xml::Error::Encoding(e)))?;
                    top.text.push_str(decoded.as_ref());
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Ok(Event::End(_)) => {
                if let Some(done) = stack.pop() {
                    if !done.text.is_empty() || !done.attrs.is_empty() {
                        out.push(done);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(xml_err(entry, e)),
        }
        buf.clear();
    }

    Ok(out)
}

fn read_attrs(entry: &str, e: &quick_xml::events::BytesStart<'_>) -> Result<Vec<(String, String)>> {
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|err| Error::Xml {
            entry: entry.to_string(),
            source: quick_xml::Error::InvalidAttr(err),
        })?;
        let key = local_name(a.key.as_ref());
        let value = a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|err| xml_err(entry, err))?
            .to_string();
        attrs.push((key, value));
    }
    Ok(attrs)
}

/// Text of the first element with the given local name.
///
/// Case-insensitive: the same logical field appears as `TraceData` and
/// `Tracedata` across ChromLab versions.
pub fn first_text(leaves: &[Leaf], name: &str) -> Option<String> {
    leaves
        .iter()
        .find(|l| l.name.eq_ignore_ascii_case(name))
        .map(|l| l.text.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Text of the first element whose local name matches any of `names`, tried in order.
pub fn first_text_any(leaves: &[Leaf], names: &[&str]) -> Option<String> {
    names.iter().find_map(|n| first_text(leaves, n))
}

/// Records grouped under repeated occurrences of `record_name`.
///
/// Each record maps child local name → text. Attributes on the record element
/// itself and on its children are folded in, because ChromLab writes some fields
/// as attributes and some as child elements depending on the type.
pub fn records(entry: &str, xml: &str, record_name: &str) -> Result<Vec<BTreeMap<String, String>>> {
    let mut reader = make_reader(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();

    // `depth` is the nesting level relative to the record element, so nested
    // elements inside a record do not terminate it early.
    let mut current: Option<BTreeMap<String, String>> = None;
    let mut depth = 0usize;
    let mut field: Option<String> = None;
    let mut field_text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                if current.is_none() && name.eq_ignore_ascii_case(record_name) {
                    let mut map = BTreeMap::new();
                    for (k, v) in read_attrs(entry, &e)? {
                        map.insert(k, v);
                    }
                    current = Some(map);
                    depth = 0;
                    continue;
                }
                if let Some(map) = current.as_mut() {
                    depth += 1;
                    for (k, v) in read_attrs(entry, &e)? {
                        map.insert(k, v);
                    }
                    field = Some(name);
                    field_text.clear();
                }
            }
            // A self-closing element has no `End`, so it must not touch `depth`
            // or leave a dangling pending field.
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                let attrs = read_attrs(entry, &e)?;
                if current.is_none() && name.eq_ignore_ascii_case(record_name) {
                    out.push(attrs.into_iter().collect());
                    continue;
                }
                if let Some(map) = current.as_mut() {
                    for (k, v) in attrs {
                        map.insert(k, v);
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if current.is_some() && field.is_some() {
                    let decoded = t
                        .decode()
                        .map_err(|e| xml_err(entry, quick_xml::Error::Encoding(e)))?;
                    field_text.push_str(decoded.as_ref());
                }
            }
            Ok(Event::CData(t)) => {
                if current.is_some() && field.is_some() {
                    field_text.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref());
                if current.is_some() && depth == 0 && name.eq_ignore_ascii_case(record_name) {
                    if let Some(map) = current.take() {
                        out.push(map);
                    }
                    field = None;
                    field_text.clear();
                    continue;
                }
                if let Some(map) = current.as_mut() {
                    if let Some(f) = field.take() {
                        let text = field_text.trim();
                        if !text.is_empty() {
                            map.insert(f, text.to_string());
                        }
                    }
                    field_text.clear();
                    depth = depth.saturating_sub(1);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(xml_err(entry, e)),
        }
        buf.clear();
    }

    Ok(out)
}

/// Case-insensitive lookup across a record's keys.
pub fn field<'a>(record: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    record
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// First matching field from a list of candidate names.
pub fn field_any<'a>(record: &'a BTreeMap<String, String>, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|n| field(record, n))
}

/// Parse a field as `f32`, tolerating the comma decimal separator that a
/// German- or French-locale ChromLab install writes.
pub fn parse_f32(s: &str) -> Option<f32> {
    let t = s.trim();
    t.parse::<f32>()
        .ok()
        .or_else(|| t.replace(',', ".").parse::<f32>().ok())
        .filter(|v| v.is_finite())
}

pub fn parse_u32(s: &str) -> Option<u32> {
    s.trim()
        .parse::<u32>()
        .ok()
        .or_else(|| parse_f32(s).map(|v| v.max(0.0) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_named_leaf_regardless_of_nesting_depth() {
        let xml = r#"<Root><Wrapper><Inner><TraceData>QUJD</TraceData></Inner></Wrapper></Root>"#;
        let l = leaves("t.xml", xml).unwrap();
        assert_eq!(first_text(&l, "TraceData").as_deref(), Some("QUJD"));
    }

    #[test]
    fn leaf_lookup_is_case_insensitive() {
        let xml = r#"<Root><tracedata>QUJD</tracedata></Root>"#;
        let l = leaves("t.xml", xml).unwrap();
        assert_eq!(first_text(&l, "TraceData").as_deref(), Some("QUJD"));
    }

    #[test]
    fn namespaced_elements_match_on_local_name() {
        let xml = r#"<n:Root xmlns:n="urn:x"><n:TraceData>QUJD</n:TraceData></n:Root>"#;
        let l = leaves("t.xml", xml).unwrap();
        assert_eq!(first_text(&l, "TraceData").as_deref(), Some("QUJD"));
    }

    #[test]
    fn records_collect_child_elements_per_occurrence() {
        let xml = r#"<RootNodeOfCFCData>
            <CFCData><Event>FractionStart</Event><TubeNumber>1</TubeNumber></CFCData>
            <CFCData><Event>FractionDone</Event><TubeNumber>2</TubeNumber></CFCData>
        </RootNodeOfCFCData>"#;
        let recs = records("f.xml", xml, "CFCData").unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(field(&recs[0], "Event"), Some("FractionStart"));
        assert_eq!(field(&recs[1], "TubeNumber"), Some("2"));
    }

    #[test]
    fn records_fold_in_attributes() {
        let xml = r#"<Root><CFCData Event="FractionStart" TubeNumber="7"/></Root>"#;
        let recs = records("f.xml", xml, "CFCData").unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(field(&recs[0], "tubenumber"), Some("7"));
    }

    #[test]
    fn nested_elements_do_not_terminate_a_record_early() {
        let xml = r#"<Root><CFCData><Nested><Deep>x</Deep></Nested><TubeNumber>3</TubeNumber></CFCData></Root>"#;
        let recs = records("f.xml", xml, "CFCData").unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(field(&recs[0], "TubeNumber"), Some("3"));
    }

    #[test]
    fn decimal_comma_is_accepted() {
        assert_eq!(parse_f32("12,5"), Some(12.5));
        assert_eq!(parse_f32("12.5"), Some(12.5));
        assert_eq!(parse_f32("nonsense"), None);
        assert_eq!(parse_f32("NaN"), None);
    }
}

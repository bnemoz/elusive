//! Cytiva / GE ÄKTA support: UNICORN 6 and 7 result containers.
//!
//! **This module does not decode curves yet, and that is deliberate.** UNICORN's
//! result format has never been published by the vendor. Everything known about
//! it publicly comes from reverse engineering, and the one reverse-engineered
//! implementation in wide use (PyCORN) is GPL-licensed, so its *code* cannot be
//! borrowed into an MIT/Apache crate even though the format *facts* it documents
//! are free to use.
//!
//! Guessing here would be worse than not supporting the format at all. A
//! chromatogram parser that invents a wavelength mapping or a value scale
//! produces numbers that look plausible and are wrong, which is the failure mode
//! `CLAUDE.md` singles out. So what this module does instead is *report what it
//! finds*: it verifies the container really is a UNICORN 6/7 zip, enumerates the
//! entries, and surfaces the inventory through the same `Warning` channel the UI
//! already renders in **Overview → Review required**.
//!
//! That turns the first real ÄKTA export into evidence rather than a crash: open
//! it, read the panel, and the entry names and sizes say what the layout is. The
//! curve decoding lands in a follow-up, written against that evidence.
//!
//! UNICORN 5.x and earlier wrote a flat proprietary binary under the same `.res`
//! extension. That vintage is explicitly out of scope, so the container check
//! below tells such a user *why* their file was rejected instead of failing with
//! a confusing zip error.

use crate::error::{Error, Result};
use crate::model::{Run, RunMeta, SourceFormat, Warning};
use std::io::Read;
use std::path::Path;

/// Local-file-header magic for a non-empty ZIP archive (`PK\x03\x04`).
///
/// The container is identified by content, not by extension, because `.res` is
/// ambiguous: UNICORN 5.x used it for a flat binary and 6/7 reuse it for a zip.
/// Trusting the extension would mean handing a legacy binary to the zip reader
/// and reporting its complaint as the user's problem.
const ZIP_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];

/// How many entries the inventory names individually before summarising.
///
/// A UNICORN result can hold hundreds of chunks; the panel needs enough to
/// recognise the layout, not the whole listing.
const INVENTORY_LIMIT: usize = 40;

/// Open a UNICORN 6/7 result container.
///
/// Succeeds with a channel-less [`Run`] whose warnings describe the archive.
/// Returns [`Error::UnsupportedFormat`] for a legacy 5.x binary.
pub fn open(path: impl AsRef<Path>) -> Result<Run> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    open_bytes(&bytes, path)
}

/// The container-level work, split out so tests can build an archive in memory
/// rather than needing a proprietary file on disk.
pub fn open_bytes(bytes: &[u8], path: &Path) -> Result<Run> {
    if !is_zip_container(bytes) {
        return Err(Error::UnsupportedFormat {
            detail: format!(
                "{} is not a UNICORN 6/7 result container. UNICORN 5.x and earlier wrote a \
                 flat proprietary binary under the same '.res' extension, which EluSive does \
                 not read. Re-export the run from UNICORN 6 or 7, or export it as ASCII/CSV.",
                path.display()
            ),
        });
    }

    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|source| Error::Zip {
        path: path.to_path_buf(),
        source,
    })?;

    let entries = inventory(&mut archive);
    if entries.is_empty() {
        return Err(Error::UnsupportedFormat {
            detail: format!("{} is an empty ZIP archive", path.display()),
        });
    }

    Ok(Run {
        meta: RunMeta {
            run_name: run_name_from(path),
            technique: "UNICORN result (ÄKTA)".to_string(),
            ..RunMeta::default()
        },
        source_format: SourceFormat::UnicornResult,
        source_path: path.to_path_buf(),
        channels: Vec::new(),
        fractions: Vec::new(),
        events: Vec::new(),
        warnings: describe(&entries),
    })
}

/// Whether the bytes begin with a ZIP local file header.
pub fn is_zip_container(bytes: &[u8]) -> bool {
    bytes.starts_with(&ZIP_MAGIC)
}

/// One archive member, reduced to what the inventory needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
}

fn inventory<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> Vec<Entry> {
    // A plain loop, not an iterator chain: `by_index` hands back a value that
    // borrows the archive, so the borrow cannot outlive each step.
    let mut entries = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let Ok(file) = archive.by_index(i) else {
            continue;
        };
        if file.name().ends_with('/') {
            continue;
        }
        entries.push(Entry {
            name: file.name().to_string(),
            size: file.size(),
        });
    }
    // Largest first: in a curve container the big members are the sample data,
    // and those are the ones whose layout has to be worked out.
    entries.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    entries
}

/// Turn the inventory into the warnings the Review-required panel shows.
///
/// Split from [`open_bytes`] so the wording is testable without building an
/// archive, and so the shape of the report is easy to change once real files
/// show what is actually worth reporting.
pub fn describe(entries: &[Entry]) -> Vec<Warning> {
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let mut warnings = vec![Warning::new(
        "unicorn",
        format!(
            "ÄKTA/UNICORN import is not implemented yet: this run was opened as a container \
             only, so it carries no traces or fractions. The archive holds {} entries \
             totalling {} bytes; the inventory below is here so the layout can be confirmed \
             against a real export instead of guessed.",
            entries.len(),
            total
        ),
    )];

    for entry in entries.iter().take(INVENTORY_LIMIT) {
        warnings.push(Warning::new(
            "unicorn/entry",
            format!("{} — {} bytes", entry.name, entry.size),
        ));
    }
    if let Some(hidden) = entries
        .len()
        .checked_sub(INVENTORY_LIMIT)
        .filter(|n| *n > 0)
    {
        warnings.push(Warning::new(
            "unicorn/entry",
            format!("… and {hidden} smaller entr{}", plural(hidden)),
        ));
    }
    warnings
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        "y"
    } else {
        "ies"
    }
}

fn run_name_from(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "UNICORN run".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a ZIP in memory so the tests need no proprietary fixture.
    fn zip_with(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, body) in members {
                w.start_file(*name, opts).expect("start_file");
                w.write_all(body).expect("write");
            }
            w.finish().expect("finish");
        }
        buf
    }

    #[test]
    fn a_legacy_binary_res_is_rejected_with_an_explanation() {
        // UNICORN 5.x wrote a flat binary under the same extension. The message
        // has to say which vintage is unsupported, or the user just sees "not a
        // zip" and has no idea what to do next.
        let legacy = b"\x00\x01\x02\x03 not a zip at all".to_vec();
        let err = open_bytes(&legacy, Path::new("old.res")).expect_err("must reject");
        let text = err.to_string();
        assert!(text.contains("UNICORN 5"), "message was: {text}");
        assert!(text.contains("ASCII/CSV"), "must suggest a way out: {text}");
    }

    #[test]
    fn the_container_is_identified_by_magic_not_extension() {
        assert!(is_zip_container(&zip_with(&[("a.xml", b"<x/>")])));
        assert!(!is_zip_container(b"PK not really"));
        assert!(!is_zip_container(b""));
    }

    #[test]
    fn a_unicorn_container_opens_and_reports_its_inventory() {
        let bytes = zip_with(&[
            ("Chrom.1.Xml", b"<Chromatogram/>" as &[u8]),
            ("Chrom.1_1_True", &[0u8; 512]),
            ("Manifest.xml", b"<Manifest/>"),
        ]);
        let run = open_bytes(&bytes, Path::new("Run 42.res")).expect("opens");

        assert_eq!(run.source_format, SourceFormat::UnicornResult);
        assert_eq!(run.meta.run_name, "Run 42");
        assert!(
            run.channels.is_empty(),
            "no curve is decoded yet; inventing one is the thing this module refuses to do"
        );

        let report = run
            .warnings
            .iter()
            .map(|w| w.message.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(report.contains("not implemented yet"));
        for name in ["Chrom.1.Xml", "Chrom.1_1_True", "Manifest.xml"] {
            assert!(report.contains(name), "{name} missing from:\n{report}");
        }
    }

    #[test]
    fn the_inventory_leads_with_the_largest_entry() {
        // The big members are the sample arrays, and those are what a follow-up
        // has to decode, so they should not be buried under boilerplate XML.
        let bytes = zip_with(&[("tiny.xml", b"<a/>" as &[u8]), ("big.bin", &[7u8; 4096])]);
        let run = open_bytes(&bytes, Path::new("r.res")).expect("opens");
        let first_entry = run
            .warnings
            .iter()
            .find(|w| w.scope == "unicorn/entry")
            .expect("an entry warning");
        assert!(first_entry.message.starts_with("big.bin"));
    }

    #[test]
    fn a_long_inventory_is_summarised_rather_than_dumped() {
        let names: Vec<String> = (0..INVENTORY_LIMIT + 5)
            .map(|i| format!("e{i}.bin"))
            .collect();
        let members: Vec<(&str, &[u8])> =
            names.iter().map(|n| (n.as_str(), b"x" as &[u8])).collect();
        let run = open_bytes(&zip_with(&members), Path::new("many.res")).expect("opens");

        let listed = run
            .warnings
            .iter()
            .filter(|w| w.scope == "unicorn/entry")
            .count();
        assert_eq!(
            listed,
            INVENTORY_LIMIT + 1,
            "40 named plus one summary line"
        );
        assert!(run
            .warnings
            .last()
            .expect("last")
            .message
            .contains("and 5 smaller entries"));
    }

    #[test]
    fn an_empty_archive_is_an_error_not_an_empty_run() {
        let err = open_bytes(&zip_with(&[]), Path::new("empty.res")).expect_err("must reject");
        assert!(err.to_string().contains("empty"));
    }
}

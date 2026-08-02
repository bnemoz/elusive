//! Create a shareable NGC archive fixture without changing its trace payloads.
//!
//! Usage:
//!
//! ```text
//! cargo run -p elusive-core --example sanitize_ngc -- INPUT.ngcAnalysis OUTPUT.ngcAnalysis
//! ```
//!
//! Inspect the generated archive before committing it. This utility redacts the
//! common free-text identity fields, but cannot determine whether numerical data
//! or an unrecognised XML field is sensitive in a particular experiment.

use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const SENSITIVE_TAGS: &[&str] = &[
    "RunName",
    "SampleName",
    "UserName",
    "User",
    "Operator",
    "Project",
    "Customer",
    "Company",
    "Email",
    "Comment",
    "Description",
    "Notes",
    "Path",
];

fn redact_tag(mut text: String, tag: &str) -> (String, usize) {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let replacement = format!("{open}[redacted]{close}");
    let mut count = 0;
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find(&open) {
        let start = cursor + start;
        let value_start = start + open.len();
        let Some(end) = text[value_start..].find(&close) else {
            break;
        };
        let end = value_start + end + close.len();
        text.replace_range(start..end, &replacement);
        cursor = start + replacement.len();
        count += 1;
    }
    (text, count)
}

fn redact_xml(bytes: &[u8]) -> (Vec<u8>, usize) {
    // NGC XML is text; leave non-XML trace payloads byte-for-byte untouched.
    if !bytes.starts_with(b"<") {
        return (bytes.to_vec(), 0);
    }
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    let mut replacements = 0;
    for tag in SENSITIVE_TAGS {
        let (redacted, count) = redact_tag(text, tag);
        text = redacted;
        replacements += count;
    }
    (text.into_bytes(), replacements)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os();
    let program = args.next().unwrap_or_default();
    let (Some(input), Some(output), None) = (args.next(), args.next(), args.next()) else {
        return Err(format!(
            "usage: {} INPUT.ngcAnalysis OUTPUT.ngcAnalysis",
            Path::new(&program).display()
        )
        .into());
    };
    if Path::new(&input) == Path::new(&output) {
        return Err("input and output must be different files".into());
    }
    if Path::new(&output).exists() {
        return Err(format!("refusing to overwrite {}", Path::new(&output).display()).into());
    }

    let mut source = ZipArchive::new(File::open(&input)?)?;
    let target = File::create(&output)?;
    let mut target = ZipWriter::new(target);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut total = 0;

    for index in 0..source.len() {
        let mut entry = source.by_index(index)?;
        let name = entry.name().to_owned();
        if entry.is_dir() {
            target.add_directory(name, options)?;
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        let (bytes, replacements) = redact_xml(&bytes);
        total += replacements;
        target.start_file(name, options)?;
        target.write_all(&bytes)?;
    }
    target.finish()?;
    let run = elusive_core::parse::open(&output)?;
    eprintln!(
        "wrote {} with {total} redacted XML field(s); parser reopened {} channels and {} fractions",
        Path::new(&output).display(),
        run.channels.len(),
        run.fractions.len(),
    );
    Ok(())
}

//! Format detection and the entry points that turn a file into a [`Run`].
//!
//! Native NGC archives come first: they are the only export that carries
//! fractions, and they are a single file you can copy off the instrument PC
//! (`design.md` §2). CSV is the fallback for runs where that is all there is.

pub mod csv;
pub mod ngc;
pub mod xml;

use crate::error::{Error, Result};
use crate::model::Run;
use std::path::Path;

/// File extensions EluSive knows how to open, for a file dialog filter.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["ngcAnalysis", "ngcMethodruns", "csv"];

/// Open any supported run file, dispatching on extension.
pub fn open(path: impl AsRef<Path>) -> Result<Run> {
    let path = path.as_ref();
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "ngcanalysis" | "ngcmethodruns" => ngc::open(path),
        "csv" => csv::open(path),
        "" => Err(Error::unsupported(format!(
            "{} has no extension; expected one of {}",
            path.display(),
            SUPPORTED_EXTENSIONS.join(", ")
        ))),
        other => Err(Error::unsupported(format!(
            "'.{other}' is not a supported run format (expected one of {})",
            SUPPORTED_EXTENSIONS.join(", ")
        ))),
    }
}

//! Error type for `elusive-core`.
//!
//! Every variant carries enough context to tell the user *which* part of a run
//! failed, because a chromatography archive is a bag of a dozen loosely-related
//! XML files and "parse error" alone is useless when one trace out of fourteen is
//! malformed.

use std::path::PathBuf;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path} is not a readable ZIP archive: {source}")]
    Zip {
        path: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },

    #[error("malformed XML in {entry}: {source}")]
    Xml {
        entry: String,
        #[source]
        source: quick_xml::Error,
    },

    #[error("invalid base64 payload in {entry}: {source}")]
    Base64 {
        entry: String,
        #[source]
        source: base64::DecodeError,
    },

    #[error("malformed signal trace {entry}: {detail}")]
    MalformedTrace { entry: String, detail: String },

    #[error("malformed fraction record in {entry}: {detail}")]
    MalformedFractions { entry: String, detail: String },

    #[error("unsupported format: {detail}")]
    UnsupportedFormat { detail: String },

    #[error("{path}: no run data found (expected Runs/Run*.xml and Trace_*.xml entries)")]
    NoRunData { path: PathBuf },

    #[error("CSV parse error at line {line}: {detail}")]
    Csv { line: usize, detail: String },

    #[error("sidecar error: {detail}")]
    Sidecar { detail: String },

    /// A sidecar written by a newer EluSive. We refuse to guess at its meaning
    /// rather than silently dropping fields the user cares about.
    #[error(
        "sidecar schema version {found} is not supported (this build understands up to {supported})"
    )]
    SidecarVersion { found: u32, supported: u32 },

    #[error("cannot integrate: {detail}")]
    Integration { detail: String },

    #[error("calibration error: {detail}")]
    Calibration { detail: String },
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn trace(entry: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::MalformedTrace {
            entry: entry.into(),
            detail: detail.into(),
        }
    }

    pub(crate) fn fractions(entry: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::MalformedFractions {
            entry: entry.into(),
            detail: detail.into(),
        }
    }

    pub(crate) fn unsupported(detail: impl Into<String>) -> Self {
        Error::UnsupportedFormat {
            detail: detail.into(),
        }
    }
}

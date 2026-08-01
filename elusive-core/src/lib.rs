//! # elusive-core
//!
//! Parsing, model, integration and calibration for Bio-Rad NGC / ChromLab runs.
//!
//! This crate is **UI-free**: it must never depend on egui, eframe, or any other
//! toolkit. All format risk and all analysis math live here so they can be tested
//! headless, and so the same core can later back a CLI or batch pipeline
//! (`design.md` §4).
//!
//! ```no_run
//! # fn main() -> Result<(), elusive_core::Error> {
//! let run = elusive_core::parse::open("run.ngcAnalysis")?;
//! println!("{} — {} channels, {} fractions",
//!          run.meta.run_name, run.channels.len(), run.fractions.len());
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod calibration;
pub mod error;
pub mod integrate;
pub mod model;
pub mod parse;
pub mod sidecar;
pub mod wells;

pub use error::{Error, Result};
pub use model::Run;

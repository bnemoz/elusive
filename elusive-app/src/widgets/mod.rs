//! Reusable UI pieces. Each takes `&Run` and `&mut View` so the loaded run stays
//! read-only and all mutation is funnelled through one place.

pub mod chromatogram;
pub mod overview;
pub mod panels;
pub mod plate;

//! EluSive — desktop viewer for Bio-Rad NGC / ChromLab runs.
//!
//! `anyhow` is used here and only here: the application boundary is the one place
//! where a bare error string is the right output, because there is nobody left to
//! handle it programmatically (`IMPLEMENTATION_PLAN.md` Phase 0).

// Do not pop a console window alongside the GUI on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod egui_adapter;
mod theme;
mod view;
mod widgets;

use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    // `elusive <run>` opens a file directly, so the binary can be associated with
    // .ngcAnalysis in the OS and double-clicking a run just works.
    let initial_run = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_file());

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("EluSive")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([1024.0, 640.0]);
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "EluSive",
        options,
        Box::new(move |cc| {
            let mut app = app::EluSiveApp::new(cc);
            if let Some(path) = initial_run {
                app.open_at_startup(&path);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
    .context("failed to start the EluSive window")
}

/// Load the window icon if a rasterised one is present.
///
/// The brand assets are SVG; rasterising one would pull in a rendering crate for
/// a single 64x64 image, so a missing icon is not an error.
fn load_icon() -> Option<egui::IconData> {
    let bytes = std::fs::read("assets/app-icon.rgba").ok()?;
    let pixels = bytes.len() / 4;
    let side = (pixels as f64).sqrt() as u32;
    if side == 0 || (side as usize * side as usize * 4) != bytes.len() {
        return None;
    }
    Some(egui::IconData {
        rgba: bytes,
        width: side,
        height: side,
    })
}

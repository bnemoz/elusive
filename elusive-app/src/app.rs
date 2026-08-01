//! EluSive application shell and state.
//!
//! Layout follows the brand mockup adapted to prep SEC (`design.md` §11): a dark
//! `INK_900` navigation rail, a light content area of cards, and a right-hand
//! detail rail. The Chromatograms section is the "single linked pane" — trace on
//! top, HEP96 plate below, hover in either direction.

use crate::egui_adapter::{self as adapt, c, Mode};
use crate::theme::{color, spacing, Theme};
use crate::view::{BaselineChoice, Interaction, Section, View};
use crate::widgets::{chromatogram, panels, plate};
use elusive_core::integrate::{integrate_peak, PlateMetric};
use elusive_core::model::Run;
use elusive_core::sidecar;

/// How long a status message stays on screen, in frames-independent seconds.
const STATUS_TTL_SECS: f64 = 8.0;

pub struct EluSiveApp {
    run: Option<Run>,
    view: View,
    mode: Mode,
    theme: Theme,
    status: Vec<(String, f64)>,
    error: Option<String>,
    /// Fonts the design system asked for that were not found on disk.
    missing_fonts: Vec<String>,
    styled: bool,
    /// A run named on the command line, opened on the first frame.
    pending_open: Option<std::path::PathBuf>,
}

impl EluSiveApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let missing_fonts = adapt::install_fonts(&cc.egui_ctx);
        let mode = Mode::System;
        let theme = mode.resolve(&cc.egui_ctx);
        adapt::apply(&cc.egui_ctx, theme);

        Self {
            run: None,
            view: View::default(),
            mode,
            theme,
            status: Vec::new(),
            error: None,
            missing_fonts,
            styled: true,
            pending_open: None,
        }
    }

    /// Open a run given on the command line, before the first frame.
    ///
    /// Status messages are timestamped against the egui clock, which has not
    /// started yet, so this defers its message to the first update instead.
    pub fn open_at_startup(&mut self, path: &std::path::Path) {
        self.pending_open = Some(path.to_path_buf());
    }

    fn note(&mut self, ctx: &egui::Context, message: impl Into<String>) {
        self.status
            .push((message.into(), ctx.input(|i| i.time) + STATUS_TTL_SECS));
    }

    // --- file handling ------------------------------------------------------

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("NGC run", &["ngcAnalysis", "ngcMethodruns"])
            .add_filter("ChromLab CSV export", &["csv"])
            .add_filter("All files", &["*"])
            .pick_file();
        if let Some(path) = picked {
            self.open_path(ctx, &path);
        }
    }

    fn open_path(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        match elusive_core::parse::open(path) {
            Ok(run) => {
                let channels = run.channels.len();
                let fractions = run.fractions.len();
                self.view.adopt_run(&run);
                let name = run.meta.run_name.clone();

                // A sidecar next to the run is loaded automatically: the user's
                // annotations should come back with the file, not on request.
                let sidecar_path = run.sidecar_path();
                if sidecar_path.is_file() {
                    match sidecar::load(&sidecar_path) {
                        Ok(s) => {
                            if s.matches(&run) {
                                let notes = self.view.apply_sidecar(&s, &run);
                                for n in notes {
                                    self.note(ctx, n);
                                }
                                self.mode = match s.view.dark_mode {
                                    Some(true) => Mode::Dark,
                                    Some(false) => Mode::Light,
                                    None => Mode::System,
                                };
                                self.styled = false;
                                self.note(
                                    ctx,
                                    format!(
                                        "Restored {} saved integration(s) from {}",
                                        self.view.peaks.len(),
                                        sidecar_path
                                            .file_name()
                                            .map(|s| s.to_string_lossy().into_owned())
                                            .unwrap_or_default()
                                    ),
                                );
                            } else {
                                self.note(
                                    ctx,
                                    format!(
                                        "Skipped {} because it does not match this run",
                                        sidecar_path
                                            .file_name()
                                            .map(|s| s.to_string_lossy().into_owned())
                                            .unwrap_or_default()
                                    ),
                                );
                            }
                        }
                        Err(e) => self.note(ctx, format!("Could not read the sidecar: {e}")),
                    }
                }

                self.run = Some(run);
                self.error = None;
                self.view.section = Section::Chromatograms;
                self.note(
                    ctx,
                    format!("Opened {name} — {channels} channels, {fractions} fractions"),
                );
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.note(ctx, format!("Could not open {}: {e}", path.display()));
            }
        }
    }

    fn save_sidecar(&mut self, ctx: &egui::Context) {
        let Some(run) = &self.run else { return };
        let path = run.sidecar_path();
        let mut payload = self.view.to_sidecar(run);
        payload.view.dark_mode = match self.mode {
            Mode::Dark => Some(true),
            Mode::Light => Some(false),
            Mode::System => None,
        };
        match sidecar::save(&path, &payload) {
            Ok(()) => {
                self.view.dirty = false;
                self.note(ctx, format!("Saved analysis to {}", path.display()));
            }
            Err(e) => self.note(ctx, format!("Could not save: {e}")),
        }
    }

    fn export(&mut self, ctx: &egui::Context, kind: ExportKind) {
        let Some(run) = &self.run else { return };
        let (default_name, contents) = match kind {
            ExportKind::Peaks => (
                format!("{}-peaks.csv", stem(run)),
                sidecar::peaks_to_csv(run, &self.view.peaks),
            ),
            ExportKind::Wells => (
                format!("{}-wells.csv", stem(run)),
                sidecar::wells_to_csv(&plate::export_rows(run, &self.view)),
            ),
        };

        let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("CSV", &["csv"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, contents) {
            Ok(()) => self.note(ctx, format!("Exported {}", path.display())),
            Err(e) => self.note(ctx, format!("Export failed: {e}")),
        }
    }

    // --- interactions -------------------------------------------------------

    fn handle(&mut self, ctx: &egui::Context, interaction: Interaction) {
        match interaction {
            Interaction::IntegrateRange(v0, v1) => {
                let Some(run) = &self.run else { return };
                let Some(channel_id) = self
                    .view
                    .selected_channel
                    .clone()
                    .or_else(|| self.view.hero_channel_id.clone())
                else {
                    self.note(ctx, "Select a channel to integrate first.");
                    return;
                };
                let Some(channel) = run.channel(&channel_id) else {
                    return;
                };
                let id = self.view.allocate_peak_id();
                let baseline = self.view.baseline_choice.resolve(v0, v1);
                match integrate_peak(id, channel, v0, v1, baseline) {
                    Ok(mut peak) => {
                        peak.estimated_mw_kda = self
                            .view
                            .calibration
                            .as_ref()
                            .and_then(|cal| estimated_mw(cal, channel, peak.apex_volume_ml));
                        let summary = format!(
                            "Integrated {} on {}: area {:.2} {}·mL",
                            peak.id, channel.name, peak.area, channel.display_unit
                        );
                        self.view.add_peak(peak);
                        self.note(ctx, summary);
                    }
                    Err(e) => self.note(ctx, e.to_string()),
                }
            }
        }
    }

    // --- layout -------------------------------------------------------------

    fn nav(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        egui::Panel::left("nav")
            .exact_size(200.0)
            .resizable(false)
            .frame(adapt::nav_frame(t))
            .show(ui, |ui| {
                ui.add_space(spacing::SM);
                ui.label(
                    egui::RichText::new("EluSive")
                        .font(adapt::font_h1())
                        .color(c(color::WHITE))
                        .strong(),
                );
                ui.label(
                    egui::RichText::new("Precise analysis.\nInvisible by design.")
                        .font(adapt::font_micro())
                        .color(c(color::BLUE_300)),
                );
                ui.add_space(spacing::LG);

                for section in Section::ALL {
                    let active = self.view.section == section;
                    // Active item: INK_800 fill plus a 2 px BLUE_500 indicator (§6).
                    let response = ui.add_sized(
                        [ui.available_width(), crate::theme::control::HEIGHT_COMPACT],
                        egui::Button::selectable(
                            active,
                            egui::RichText::new(section.label()).color(c(if active {
                                color::WHITE
                            } else {
                                color::BLUE_300
                            })),
                        ),
                    );
                    if active {
                        let rect = response.rect;
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                rect.left_top(),
                                egui::vec2(2.0, rect.height()),
                            ),
                            0,
                            c(color::BLUE_500),
                        );
                    }
                    if response.clicked() {
                        self.view.section = section;
                    }
                }

                // Settings and help pinned to the bottom (§11).
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(spacing::SM);
                    if !self.missing_fonts.is_empty() {
                        ui.label(
                            egui::RichText::new(format!(
                                "Using fallback fonts ({})",
                                self.missing_fonts.join(", ")
                            ))
                            .font(adapt::font_micro())
                            .color(c(color::BLUE_300)),
                        );
                    }
                    ui.label(
                        egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                            .font(adapt::font_micro())
                            .color(c(color::BLUE_300)),
                    );
                });
            });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let t = self.theme;
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(c(t.panel_bg))
                    .inner_margin(spacing::SM as i8),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Open run…").clicked() {
                        self.open_dialog(ctx);
                    }

                    let has_run = self.run.is_some();
                    ui.add_enabled_ui(has_run, |ui| {
                        let label = if self.view.dirty {
                            "Save analysis •"
                        } else {
                            "Save analysis"
                        };
                        if ui.button(label).clicked() {
                            self.save_sidecar(ctx);
                        }
                    });

                    ui.separator();

                    ui.add_enabled_ui(has_run, |ui| {
                        let mut integrating = self.view.integrate_mode;
                        if ui.toggle_value(&mut integrating, "Integrate").changed() {
                            self.view.integrate_mode = integrating;
                            self.view.pending_selection = None;
                        }
                        egui::ComboBox::from_id_salt("baseline")
                            .selected_text(self.view.baseline_choice.label())
                            .show_ui(ui, |ui| {
                                for choice in BaselineChoice::ALL {
                                    ui.selectable_value(
                                        &mut self.view.baseline_choice,
                                        choice,
                                        choice.label(),
                                    );
                                }
                            });
                        let mut show_fractions = self.view.show_fractions;
                        if ui.checkbox(&mut show_fractions, "Fraction zones").changed() {
                            self.view.set_show_fractions(show_fractions);
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::ComboBox::from_id_salt("theme-mode")
                            .selected_text(self.mode.label())
                            .show_ui(ui, |ui| {
                                for mode in [Mode::System, Mode::Dark, Mode::Light] {
                                    if ui
                                        .selectable_value(&mut self.mode, mode, mode.label())
                                        .clicked()
                                    {
                                        self.styled = false;
                                        self.view.dirty = true;
                                    }
                                }
                            });
                        if self.view.integrate_mode {
                            ui.label(
                                egui::RichText::new("Drag across a peak to integrate")
                                    .font(adapt::font_micro())
                                    .color(c(t.text_secondary)),
                            );
                        }
                    });
                });
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.status.retain(|(_, expiry)| *expiry > now);
        if self.status.is_empty() {
            return;
        }
        let t = self.theme;
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(c(t.panel_elevated))
                    .inner_margin(spacing::SM as i8),
            )
            .show(ui, |ui| {
                for (message, _) in self.status.iter().rev().take(3) {
                    ui.label(
                        egui::RichText::new(message)
                            .font(adapt::font_micro())
                            .color(c(t.text_primary)),
                    );
                }
            });
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                // The chromatogram mark at low contrast, not a spinner (§6).
                mark(ui, t);
                ui.add_space(spacing::LG);
                ui.label(
                    egui::RichText::new("Import a chromatogram to begin")
                        .font(adapt::font_display())
                        .color(c(t.text_primary)),
                );
                ui.label(
                    egui::RichText::new(
                        "Open a .ngcAnalysis or .ngcMethodruns archive — a CSV export works too, \
                         but carries no fractions.",
                    )
                    .color(c(t.text_secondary)),
                );
                ui.add_space(spacing::LG);
                if ui.button("Open run…").clicked() {
                    let ctx = ui.ctx().clone();
                    self.open_dialog(&ctx);
                }
                if let Some(err) = &self.error {
                    ui.add_space(spacing::LG);
                    ui.label(egui::RichText::new(err).color(c(color::DANGER_600)));
                }
            });
        });
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(c(t.app_bg))
                    .inner_margin(spacing::LG as i8),
            )
            .show(ui, |ui| {
                if self.run.is_none() {
                    self.empty_state(ui);
                    return;
                }

                // Split borrows: the run stays immutable, the view is mutable.
                let EluSiveApp { run, view, .. } = self;
                let run = run.as_ref().expect("checked above");

                match view.section {
                    Section::Overview => overview(ui, run, view, t),
                    Section::Chromatograms | Section::Peaks | Section::Integration => {
                        if let Some(action) = linked_pane(ui, run, view, t) {
                            let ctx = ui.ctx().clone();
                            self.handle(&ctx, action);
                        }
                    }
                    Section::Calibration => {
                        adapt::card(t).show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("calibration-scroll")
                                .show(ui, |ui| {
                                    panels::calibration_panel(ui, run, view, t);
                                });
                        });
                    }
                    Section::Results => {
                        adapt::card(t).show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("results-scroll")
                                .show(ui, |ui| {
                                    panels::results_table(ui, run, view, t);
                                });
                        });
                    }
                    Section::Reports => reports(ui, run, view, t),
                }
            });
    }
}

/// Estimated molecular weight for a peak, or `None` when the question does not
/// apply.
///
/// A SEC curve maps the elution volume of an *absorbance* peak to a mass, so
/// stamping an MW onto a conductivity or pressure peak would be a number with no
/// meaning behind it. Gating here keeps that rule in one place, shared by
/// freshly-integrated peaks and by a re-fit that restamps them all.
pub fn estimated_mw(
    calibration: &elusive_core::calibration::Calibration,
    channel: &elusive_core::model::Channel,
    apex_volume_ml: f32,
) -> Option<f64> {
    (channel.kind == elusive_core::model::ChannelKind::Uv)
        .then(|| calibration.mw_for_volume(apex_volume_ml))
        .flatten()
}

fn stem(run: &Run) -> String {
    run.source_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "run".to_string())
}

#[derive(Clone, Copy)]
enum ExportKind {
    Peaks,
    Wells,
}

/// The low-contrast three-peak mark used for empty and loading states.
fn mark(ui: &mut egui::Ui, t: Theme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(180.0, 60.0), egui::Sense::hover());
    let painter = ui.painter();
    let base = rect.bottom() - 6.0;
    let stroke = egui::Stroke::new(2.0, adapt::c_alpha(t.text_secondary, 90));

    let mut points = vec![egui::pos2(rect.left(), base)];
    // Three peaks with the centre one tallest — the logo silhouette.
    for (offset, height) in [(0.22, 0.45), (0.5, 1.0), (0.78, 0.35)] {
        let cx = rect.left() + rect.width() * offset;
        let h = (rect.height() - 12.0) * height;
        points.push(egui::pos2(cx - 16.0, base));
        points.push(egui::pos2(cx, base - h));
        points.push(egui::pos2(cx + 16.0, base));
    }
    points.push(egui::pos2(rect.right(), base));
    painter.add(egui::Shape::line(points, stroke));
}

fn overview(ui: &mut egui::Ui, run: &Run, view: &mut View, t: Theme) {
    egui::ScrollArea::vertical()
        .id_salt("overview-scroll")
        .show(ui, |ui| {
            adapt::card(t).show(ui, |ui| {
                panels::run_summary(ui, run, t);
            });
            ui.add_space(spacing::LG);

            if !run.warnings.is_empty() {
                adapt::card(t).show(ui, |ui| {
                    panels::warnings(ui, run, t);
                });
                ui.add_space(spacing::LG);
            }

            adapt::card(t).show(ui, |ui| {
                panels::channel_table(ui, run, t);
            });
            ui.add_space(spacing::LG);

            adapt::card(t).show(ui, |ui| {
                panels::heading(ui, t, "Fractions");
                panels::fraction_table(ui, run, view, t);
            });
        });
}

/// The single linked pane: chromatogram above, plate below, detail rail right.
fn linked_pane(ui: &mut egui::Ui, run: &Run, view: &mut View, t: Theme) -> Option<Interaction> {
    let mut outcome = chromatogram::ChartOutcome::default();
    let mut plate_hover = None;

    egui::Panel::right("detail-rail")
        .resizable(true)
        // egui 0.35 merged SidePanel and TopBottomPanel into one `Panel`, so the
        // sizing builders are axis-neutral: `default_size`/`size_range`, not
        // `default_width`/`min_width`/`max_width`.
        .default_size(340.0)
        .size_range(egui::Rangef::new(260.0, 640.0))
        .frame(adapt::card(t))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("detail-scroll")
                .show(ui, |ui| {
                    panels::peak_detail(ui, run, view, t);
                    ui.add_space(spacing::LG);
                    ui.separator();
                    ui.add_space(spacing::SM);
                    panels::heading(ui, t, "Channels");
                    chromatogram::legend(ui, run, view, t);
                });
        });

    egui::Panel::bottom(plate::PANE_ID)
        .resizable(true)
        // The plate reports the height it needs; the pane's only job is to keep
        // that inside bounds which leave the chromatogram the larger share.
        .default_size(plate::natural_pane_height(ui.available_width()))
        // A belt-and-braces cap. The content no longer sizes itself from the
        // available height, but a panel that can grow without limit is one bug
        // away from hiding the chromatogram entirely.
        .size_range(egui::Rangef::new(
            plate::MIN_PANE_HEIGHT,
            plate::MAX_PANE_HEIGHT,
        ))
        .frame(adapt::card(t))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                panels::heading(ui, t, "HEP96 plate");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut uniform_ramp = view.plate_uniform_ramp;
                    if ui.checkbox(&mut uniform_ramp, "Uniform ramp").changed() {
                        view.set_plate_uniform_ramp(uniform_ramp);
                    }

                    egui::ComboBox::from_id_salt("plate-metric")
                        .selected_text(view.plate_metric.label())
                        .show_ui(ui, |ui| {
                            for metric in PlateMetric::ALL {
                                let selected = view.plate_metric == metric;
                                if ui.selectable_label(selected, metric.label()).clicked() {
                                    view.set_plate_metric(metric);
                                }
                            }
                        });

                    let current = view
                        .plate_channel
                        .as_ref()
                        .and_then(|id| run.channel(id))
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| "Channel".to_string());
                    egui::ComboBox::from_id_salt("plate-channel")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for channel in run.channels.iter().filter(|c| !c.is_empty()) {
                                let selected = view.plate_channel.as_ref() == Some(&channel.id);
                                if ui.selectable_label(selected, &channel.name).clicked() {
                                    view.set_plate_channel(Some(channel.id.clone()));
                                }
                            }
                        });
                });
            });

            // The plate only *reports* what it hovered; the shared state is
            // written once, below, after both panes have had their say.
            plate_hover = plate::show(ui, run, view, t);
        });

    // Peak table under the chart when the user is working on peaks.
    if matches!(view.section, Section::Peaks | Section::Integration) {
        egui::Panel::bottom("peak-pane")
            .resizable(true)
            .default_size(180.0)
            .size_range(egui::Rangef::new(90.0, 360.0))
            .frame(adapt::card(t))
            .show(ui, |ui| {
                panels::heading(ui, t, "Integrations");
                panels::peak_table(ui, run, view, t);
            });
    }

    egui::CentralPanel::default()
        .frame(adapt::card(t))
        .show(ui, |ui| {
            outcome = chromatogram::show(ui, run, view, t);
        });

    resolve_hover(ui.ctx(), run, view, plate_hover, outcome.hovered_volume);
    outcome.interaction
}

/// Decide the frame's hover state from what each pane reported.
///
/// Single-writer on purpose. Previously the plate and every stacked plot each
/// assigned `view.hovered_*` as they drew, so whichever ran last won and the
/// highlight flickered or never appeared. The plate takes precedence because the
/// pointer can only be over one of the two panes, and the plate is the more
/// specific target.
fn resolve_hover(
    ctx: &egui::Context,
    run: &Run,
    view: &mut View,
    plate_hover: Option<elusive_core::model::Well>,
    hovered_volume: Option<f32>,
) {
    let (well, range) = match (plate_hover, hovered_volume) {
        (Some(well), _) => (
            Some(well),
            run.fractions
                .iter()
                .find(|f| f.well == Some(well))
                .map(|f| f.volume_window()),
        ),
        (None, Some(v)) => {
            let fraction = run.fractions.iter().find(|f| {
                let (a, b) = f.volume_window();
                v >= a && v <= b
            });
            (
                fraction.and_then(|f| f.well),
                fraction.map(|f| f.volume_window()),
            )
        }
        (None, None) => (None, None),
    };

    // The panes drew using last frame's answer, so a change needs one more frame
    // to appear. Ask for it rather than waiting for the next unrelated event.
    if view.hovered_well != well || view.hovered_vol_range != range {
        ctx.request_repaint();
    }
    view.hovered_well = well;
    view.hovered_vol_range = range;
    view.hovered_volume = hovered_volume;
}

fn reports(ui: &mut egui::Ui, run: &Run, view: &mut View, t: Theme) {
    adapt::card(t).show(ui, |ui| {
        panels::heading(ui, t, "Export");
        ui.label(
            egui::RichText::new(
                "Exports are plain CSV with a fixed column order, so they drop straight into a \
                 notebook or a script.",
            )
            .color(c(t.text_secondary)),
        );
        ui.add_space(spacing::MD);

        ui.horizontal(|ui| {
            if ui.button("Peak table (CSV)").clicked() {
                ui.data_mut(|d| d.insert_temp(egui::Id::new("export"), 0u8));
            }
            if ui.button("Plate metrics (CSV)").clicked() {
                ui.data_mut(|d| d.insert_temp(egui::Id::new("export"), 1u8));
            }
        });

        ui.add_space(spacing::LG);
        panels::heading(ui, t, "Peak table preview");
        let csv = sidecar::peaks_to_csv(run, &view.peaks);
        egui::ScrollArea::both()
            .id_salt("csv-preview")
            .max_height(200.0)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(if csv.lines().count() > 1 {
                        csv
                    } else {
                        "No integrations to export yet.".to_string()
                    })
                    .font(adapt::font_code())
                    .color(c(t.text_secondary)),
                );
            });

        ui.add_space(spacing::LG);
        panels::heading(ui, t, "Sidecar");
        ui.label(
            egui::RichText::new(format!(
                "Analysis is stored beside the run as {}",
                run.sidecar_path()
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ))
            .font(adapt::font_code())
            .color(c(t.text_secondary)),
        );
        ui.label(
            egui::RichText::new("The source archive is never modified.").color(c(t.text_secondary)),
        );
    });
}

impl eframe::App for EluSiveApp {
    /// Do not carry egui's memory across runs.
    ///
    /// eframe persists panel sizes by default, which means a layout saved by an
    /// older build is restored on top of a newer one — a user who ran a version
    /// with an oversized plate pane would keep it even after upgrading. Starting
    /// from the declared defaults every launch makes the layout a property of the
    /// build, not of whatever state happens to be on disk. View preferences that
    /// are worth keeping live in the run's sidecar, deliberately.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        if !self.styled {
            self.theme = self.mode.resolve(ctx);
            adapt::apply(ctx, self.theme);
            self.styled = true;
        }

        if let Some(path) = self.pending_open.take() {
            self.open_path(ctx, &path);
        }

        // Accept a run dropped onto the window — the fastest path from USB stick
        // to plot.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(path) = dropped.first() {
            self.open_path(ctx, path);
        }

        self.nav(ui);
        self.toolbar(ui);
        self.status_bar(ui);
        self.content(ui);

        // Export requests are raised inside the Reports panel, where `self` is
        // already borrowed; they are picked up here.
        let pending: Option<u8> = ctx.data_mut(|d| d.remove_temp(egui::Id::new("export")));
        match pending {
            Some(0) => self.export(ctx, ExportKind::Peaks),
            Some(1) => self.export(ctx, ExportKind::Wells),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_has_a_label() {
        for section in Section::ALL {
            assert!(!section.label().is_empty());
        }
    }

    #[test]
    fn a_channel_id_round_trips_through_the_view() {
        use elusive_core::model::ChannelId;
        let mut view = View::default();
        let id = ChannelId::from("MWave2");
        assert!(view.is_channel_visible(&id));
        view.set_channel_visible(&id, false);
        assert!(!view.is_channel_visible(&id));
        view.set_channel_visible(&id, true);
        assert!(view.is_channel_visible(&id));
    }
}

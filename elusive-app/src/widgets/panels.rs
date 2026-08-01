//! Cards, tables and the right-hand detail rail.
//!
//! Every numeric column is right-aligned monospace with fixed decimals, so digits
//! line up by place value rather than by chance (`DESIGN_SYSTEM.md` rule #4).

use crate::egui_adapter::{c, c_alpha, font_code, font_h3, font_micro, num};
use crate::theme::{color, spacing, Theme};
use crate::view::View;
use egui::Ui;
use elusive_core::calibration::{self, FitBasis, BIORAD_GFS, LOW_CONFIDENCE_R2};
use elusive_core::model::{Channel, PeakId, PeakResult, Run};

/// Section heading inside a card.
pub fn heading(ui: &mut Ui, t: Theme, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(font_h3())
            .color(c(t.text_primary)),
    );
    ui.add_space(spacing::SM);
}

/// Label above a value, per §6 (labels above controls, units in the label).
pub fn field(ui: &mut Ui, t: Theme, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .font(font_micro())
                .color(c(t.text_secondary)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(value)
                    .font(font_code())
                    .color(c(t.text_primary)),
            );
        });
    });
}

/// The header row of a data grid: 11 px secondary labels, one cell per column.
///
/// Shared so that every table in the app agrees on what a header looks like;
/// callers still own the `Grid` itself, because column counts and interactivity
/// differ from table to table.
pub fn table_header_row(ui: &mut Ui, t: Theme, headers: &[&str]) {
    for header in headers {
        ui.label(
            egui::RichText::new(*header)
                .font(font_micro())
                .color(c(t.text_secondary)),
        );
    }
    ui.end_row();
}

/// A numeric grid cell, right-aligned inside a fixed-width box.
///
/// A grid cell sizes itself to its content, so `1234.5` and `9.0` start at the
/// same left edge and the ones place drifts. Reserving a known width and laying
/// out right-to-left inside it puts place value under place value (rule #4).
fn num_cell(ui: &mut Ui, t: Theme, width: f32, text: &str) {
    let height = ui.text_style_height(&egui::TextStyle::Monospace);
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.label(
                egui::RichText::new(text)
                    .font(font_code())
                    .color(c(t.text_primary)),
            );
        },
    );
}

/// Run identity and provenance.
pub fn run_summary(ui: &mut Ui, run: &Run, t: Theme) {
    heading(ui, t, "Run");
    field(ui, t, "Name", &run.meta.run_name);
    if !run.meta.method_name.is_empty() {
        field(ui, t, "Method", &run.meta.method_name);
    }
    if !run.meta.technique.is_empty() {
        field(ui, t, "Technique", &run.meta.technique);
    }
    if let Some(col) = &run.meta.column {
        field(ui, t, "Column", col);
    }
    if let Some(started) = &run.meta.started {
        field(ui, t, "Started", started);
    }
    field(ui, t, "Source", run.source_format.label());
    field(
        ui,
        t,
        "File",
        &run.source_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );

    if let Some((lo, hi)) = run.volume_range() {
        field(ui, t, "Volume (mL)", &format!("{lo:.2} – {hi:.2}"));
    }
    field(ui, t, "Channels", &run.channels.len().to_string());
    field(
        ui,
        t,
        "Fractions",
        &if run.source_format.supports_fractions() {
            run.fractions.len().to_string()
        } else {
            "not in CSV".to_string()
        },
    );
}

/// Parser diagnostics. These change how the numbers should be read, so they get a
/// visible card rather than a log line — and each carries an icon plus text, never
/// colour alone (rule #3).
pub fn warnings(ui: &mut Ui, run: &Run, t: Theme) {
    if run.warnings.is_empty() {
        return;
    }
    heading(ui, t, &format!("Review required ({})", run.warnings.len()));
    for w in &run.warnings {
        ui.horizontal_top(|ui| {
            ui.label(
                egui::RichText::new("!")
                    .font(font_code())
                    .strong()
                    .color(c(color::WARNING_600)),
            );
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(&w.scope)
                        .font(font_micro())
                        .color(c(t.text_secondary)),
                );
                ui.label(egui::RichText::new(&w.message).color(c(t.text_primary)));
            });
        });
        ui.add_space(spacing::XS);
    }
}

/// Channel inventory: what loaded, in what unit, with how many points.
pub fn channel_table(ui: &mut Ui, run: &Run, t: Theme) {
    heading(ui, t, "Channels");
    egui::Grid::new("channel-table")
        .num_columns(5)
        .striped(false)
        .spacing([spacing::LG, spacing::XS])
        .show(ui, |ui| {
            for header in ["Channel", "Kind", "Unit", "Points", "Range"] {
                ui.label(
                    egui::RichText::new(header)
                        .font(font_micro())
                        .color(c(t.text_secondary)),
                );
            }
            ui.end_row();

            for channel in &run.channels {
                ui.label(egui::RichText::new(&channel.name).color(c(t.text_primary)));
                ui.label(
                    egui::RichText::new(channel.kind.label())
                        .font(font_micro())
                        .color(c(t.text_secondary)),
                );
                ui.label(
                    egui::RichText::new(&channel.display_unit)
                        .font(font_code())
                        .color(c(t.text_secondary)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(channel.samples.len().to_string())
                            .font(font_code())
                            .color(c(t.text_primary)),
                    );
                });
                let range = channel
                    .display_value_range()
                    .map(|(lo, hi)| format!("{lo:.3} – {hi:.3}"))
                    .unwrap_or_else(|| "—".to_string());
                ui.label(
                    egui::RichText::new(range)
                        .font(font_code())
                        .color(c(t.text_secondary)),
                );
                ui.end_row();
            }
        });
}

/// Fraction table with selection linked to the plot and plate.
pub fn fraction_table(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) {
    if run.fractions.is_empty() {
        ui.label(
            egui::RichText::new(if run.source_format.supports_fractions() {
                "This run collected no fractions."
            } else {
                "Fractions are not recorded in a CSV export."
            })
            .color(c(t.text_secondary)),
        );
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("fraction-table")
        .show(ui, |ui| {
            egui::Grid::new("fractions")
                .num_columns(5)
                .spacing([spacing::LG, spacing::XS])
                .show(ui, |ui| {
                    for header in ["Tube", "Well", "Start (mL)", "End (mL)", "Window"] {
                        ui.label(
                            egui::RichText::new(header)
                                .font(font_micro())
                                .color(c(t.text_secondary)),
                        );
                    }
                    ui.end_row();

                    for f in &run.fractions {
                        let (a, b) = f.volume_window();
                        let selected = view.hovered_vol_range == Some((a, b));
                        let label = ui.selectable_label(
                            selected,
                            egui::RichText::new(f.tube.to_string()).font(font_code()),
                        );
                        if label.hovered() {
                            view.hovered_vol_range = Some((a, b));
                            view.hovered_well = f.well;
                        }

                        ui.label(
                            egui::RichText::new(
                                f.well.map(|w| w.label()).unwrap_or_else(|| "—".into()),
                            )
                            .font(font_code()),
                        );
                        ui.label(egui::RichText::new(num(a as f64, 3)).font(font_code()));
                        ui.label(egui::RichText::new(num(b as f64, 3)).font(font_code()));
                        // Status by text, not colour: an inferred end says so.
                        ui.label(
                            egui::RichText::new(if f.end_estimated {
                                "end inferred"
                            } else {
                                "measured"
                            })
                            .font(font_micro())
                            .color(c(if f.end_estimated {
                                color::WARNING_600
                            } else {
                                t.text_secondary
                            })),
                        );
                        ui.end_row();
                    }
                });
        });
}

/// The peak results table. Rows are selectable and deletable.
pub fn peak_table(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) {
    if view.peaks.is_empty() {
        ui.label(
            egui::RichText::new(
                "No integrations yet. Turn on Integrate and drag across a peak on the chromatogram.",
            )
            .color(c(t.text_secondary)),
        );
        return;
    }

    let mut to_delete: Option<PeakId> = None;

    egui::ScrollArea::vertical()
        .id_salt("peak-table")
        .show(ui, |ui| {
            egui::Grid::new("peaks")
                .num_columns(9)
                .spacing([spacing::LG, spacing::XS])
                .show(ui, |ui| {
                    table_header_row(
                        ui,
                        t,
                        &[
                            "Peak",
                            "Channel",
                            "Ve (mL)",
                            "Window (mL)",
                            "Area",
                            "Area %",
                            "Height",
                            "FWHM (mL)",
                            "",
                        ],
                    );

                    let peaks = view.peaks.clone();
                    for peak in &peaks {
                        let total = view.total_area_on(&peak.channel_id);
                        let selected = view.selected_peak == Some(peak.id);

                        if ui
                            .selectable_label(
                                selected,
                                egui::RichText::new(peak.id.to_string()).font(font_code()),
                            )
                            .clicked()
                        {
                            view.selected_peak = Some(peak.id);
                        }

                        let channel_name = run
                            .channel(&peak.channel_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|| peak.channel_id.0.clone());
                        ui.label(egui::RichText::new(channel_name).color(c(t.text_primary)));

                        ui.label(
                            egui::RichText::new(num(peak.apex_volume_ml as f64, 3))
                                .font(font_code()),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "{} – {}",
                                num(peak.v_start_ml as f64, 2),
                                num(peak.v_end_ml as f64, 2)
                            ))
                            .font(font_code()),
                        );
                        ui.label(egui::RichText::new(num(peak.area, 2)).font(font_code()));
                        ui.label(
                            egui::RichText::new(if total > 0.0 {
                                num(peak.area.abs() / total * 100.0, 1)
                            } else {
                                "—".to_string()
                            })
                            .font(font_code()),
                        );
                        ui.label(egui::RichText::new(num(peak.height, 2)).font(font_code()));
                        ui.label(
                            egui::RichText::new(
                                peak.fwhm_ml
                                    .map(|v| num(v as f64, 3))
                                    .unwrap_or_else(|| "—".to_string()),
                            )
                            .font(font_code()),
                        );

                        if ui.small_button("Delete").clicked() {
                            to_delete = Some(peak.id);
                        }
                        ui.end_row();
                    }
                });
        });

    if let Some(id) = to_delete {
        view.remove_peak(id);
    }
}

/// Columns of the peak export, in the order `sidecar::peaks_to_csv` writes them.
const EXPORT_COLUMNS: [&str; 9] = [
    "Peak",
    "Channel",
    "Start (mL)",
    "End (mL)",
    "Ve (mL)",
    "Area",
    "Height",
    "FWHM (mL)",
    "Est. MW (kDa)",
];

/// Reserved width per export column; `0.0` means "natural width, left aligned",
/// which is what an identifier or a channel name wants.
const EXPORT_WIDTHS: [f32; 9] = [0.0, 0.0, 60.0, 60.0, 60.0, 92.0, 92.0, 64.0, 80.0];

/// The exported cells for one peak, at the precision the export writes.
///
/// Split out from the drawing code so the preview's agreement with
/// `sidecar::peaks_to_csv` is a unit test rather than a promise. A value the run
/// does not carry shows an em dash, never a zero that would read as a result.
fn export_row(peak: &PeakResult) -> [String; EXPORT_COLUMNS.len()] {
    [
        peak.id.to_string(),
        peak.channel_id.0.clone(),
        num(peak.v_start_ml as f64, 4),
        num(peak.v_end_ml as f64, 4),
        num(peak.apex_volume_ml as f64, 4),
        num(peak.area, 6),
        num(peak.height, 6),
        peak.fwhm_ml
            .map(|v| num(v as f64, 4))
            .unwrap_or_else(|| "—".to_string()),
        peak.estimated_mw_kda
            .map(|v| num(v, 3))
            .unwrap_or_else(|| "—".to_string()),
    ]
}

/// How tall the Reports preview may grow before it scrolls.
///
/// Bounded on purpose: a run with thirty peaks must not push the Sidecar section
/// below the fold.
const EXPORT_PREVIEW_MAX_HEIGHT: f32 = 200.0;

/// A read-only preview of the peak export.
///
/// Same columns and same decimals as the CSV and Markdown writers, so the user is
/// reading the file rather than a paraphrase of it. No zebra striping: §"Data
/// tables" reserves row fill for selection, and nothing here is selectable.
pub fn peak_export_preview(ui: &mut Ui, view: &View, t: Theme) {
    if view.peaks.is_empty() {
        ui.label(egui::RichText::new("No integrations to export yet.").color(c(t.text_secondary)));
        return;
    }

    egui::ScrollArea::both()
        .id_salt("peak-export-preview")
        .max_height(EXPORT_PREVIEW_MAX_HEIGHT)
        .show(ui, |ui| {
            egui::Grid::new("peak-export")
                .num_columns(EXPORT_COLUMNS.len())
                .spacing([spacing::MD, spacing::XS])
                .show(ui, |ui| {
                    table_header_row(ui, t, &EXPORT_COLUMNS);
                    for peak in &view.peaks {
                        for (cell, width) in export_row(peak).iter().zip(EXPORT_WIDTHS) {
                            if width > 0.0 {
                                num_cell(ui, t, width, cell);
                            } else {
                                ui.label(
                                    egui::RichText::new(cell)
                                        .font(font_code())
                                        .color(c(t.text_primary)),
                                );
                            }
                        }
                        ui.end_row();
                    }
                });
        });
}

/// Right-rail detail card for the selected peak, plus its shape mini-view.
pub fn peak_detail(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) {
    let Some(peak) = view.selected_peak().cloned() else {
        heading(ui, t, "Peak detail");
        ui.label(
            egui::RichText::new("Select a peak to see its numbers.").color(c(t.text_secondary)),
        );
        return;
    };

    let channel = run.channel(&peak.channel_id);
    let unit = channel.map(|c| c.display_unit.clone()).unwrap_or_default();

    heading(ui, t, &format!("Peak {}", peak.id.0));
    field(
        ui,
        t,
        "Channel",
        &channel.map(|c| c.name.clone()).unwrap_or_default(),
    );
    field(ui, t, "Baseline", peak.baseline.label());
    field(
        ui,
        t,
        "Elution volume (mL)",
        &num(peak.apex_volume_ml as f64, 3),
    );
    field(ui, t, &format!("Area ({unit}·mL)"), &num(peak.area, 3));
    let total = view.total_area_on(&peak.channel_id);
    field(
        ui,
        t,
        "Area %",
        &if total > 0.0 {
            num(peak.area.abs() / total * 100.0, 2)
        } else {
            "—".to_string()
        },
    );
    field(ui, t, &format!("Height ({unit})"), &num(peak.height, 3));
    field(
        ui,
        t,
        "FWHM (mL)",
        &peak
            .fwhm_ml
            .map(|v| num(v as f64, 3))
            .unwrap_or_else(|| "not resolved".to_string()),
    );
    if let Some(mw) = estimated_mw_for_peak(run, &peak, view.calibration.as_ref()) {
        field(ui, t, "Estimated MW (kDa)", &num(mw, 1));
    }

    ui.add_space(spacing::MD);
    if let Some(channel) = channel {
        peak_shape(ui, channel, &peak, t);
    }
}

/// A small profile of the peak, drawn directly rather than through egui_plot so
/// it stays compact and non-interactive.
fn peak_shape(
    ui: &mut Ui,
    channel: &elusive_core::model::Channel,
    peak: &elusive_core::model::PeakResult,
    t: Theme,
) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 90.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4, c(t.panel_elevated));

    // Pad the window so the flanks are visible either side of the integration.
    let pad = (peak.v_end_ml - peak.v_start_ml) * 0.4;
    let (v0, v1) = (peak.v_start_ml - pad, peak.v_end_ml + pad);
    let samples = channel.samples_in_volume(v0, v1);
    if samples.len() < 2 {
        return;
    }

    let scale = channel.display_scale;
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for s in samples.iter().filter(|s| s.is_finite()) {
        lo = lo.min(s.value * scale);
        hi = hi.max(s.value * scale);
    }
    // A flat or non-finite span has no shape to draw.
    if !hi.is_finite() || !lo.is_finite() || hi <= lo {
        return;
    }

    let to_screen = |v: f32, y: f32| -> egui::Pos2 {
        let x = rect.left() + rect.width() * ((v - v0) / (v1 - v0)).clamp(0.0, 1.0);
        let yy =
            rect.bottom() - (rect.height() - 8.0) * ((y - lo) / (hi - lo)).clamp(0.0, 1.0) - 4.0;
        egui::pos2(x, yy)
    };

    // The integrated window, translucent so the trace stays readable (rule #2).
    let a = to_screen(peak.v_start_ml, lo).x;
    let b = to_screen(peak.v_end_ml, lo).x;
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(a, rect.top()), egui::pos2(b, rect.bottom())),
        0,
        c_alpha(
            crate::theme::Rgb::new(
                color::INTEGRATED_AREA.r,
                color::INTEGRATED_AREA.g,
                color::INTEGRATED_AREA.b,
            ),
            color::INTEGRATED_AREA.a,
        ),
    );

    let points: Vec<egui::Pos2> = samples
        .iter()
        .filter(|s| s.is_finite())
        .map(|s| to_screen(s.volume_ml, s.value * scale))
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(
            crate::theme::stroke::TRACE,
            c(crate::theme::chart::PRIMARY_TRACE),
        ),
    ));
}

/// SEC calibration: assign standards, choose a basis, fit, and read the result.
pub fn calibration_panel(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) {
    heading(ui, t, "SEC molecular-weight calibration");

    ui.label(
        egui::RichText::new(
            "Integrate the standard's peaks, then assign each apex to a marker. \
             Elution order is largest first.",
        )
        .color(c(t.text_secondary)),
    );
    ui.add_space(spacing::SM);

    ui.horizontal(|ui| {
        if ui.button("Pre-fill from integrated peaks").clicked() {
            let apexes: Vec<f32> = view.peaks.iter().map(|p| p.apex_volume_ml).collect();
            view.cal_points = calibration::suggest_assignment(&apexes);
            view.dirty = true;
        }
        if ui.button("Clear assignment").clicked() {
            view.cal_points.clear();
            view.calibration = None;
            clear_peak_mw(view);
            view.dirty = true;
        }
    });

    ui.add_space(spacing::SM);
    egui::Grid::new("cal-points")
        .num_columns(3)
        .spacing([spacing::LG, spacing::XS])
        .show(ui, |ui| {
            for header in ["Standard", "MW (kDa)", "Ve (mL)"] {
                ui.label(
                    egui::RichText::new(header)
                        .font(font_micro())
                        .color(c(t.text_secondary)),
                );
            }
            ui.end_row();

            for (i, standard) in BIORAD_GFS.iter().enumerate() {
                ui.label(
                    egui::RichText::new(format!("{} · {}", standard.letter, standard.name))
                        .color(c(t.text_primary)),
                );
                ui.label(egui::RichText::new(num(standard.mw_kda, 2)).font(font_code()));

                let mut ve = view
                    .cal_points
                    .iter()
                    .find(|p| (p.mw_kda - standard.mw_kda).abs() < f64::EPSILON)
                    .map(|p| p.ve_ml)
                    .unwrap_or(0.0);
                if ui
                    .add(
                        egui::DragValue::new(&mut ve)
                            .speed(0.01)
                            .range(0.0..=1000.0),
                    )
                    .changed()
                {
                    set_cal_point(view, standard.mw_kda, ve);
                }
                let _ = i;
                ui.end_row();
            }
        });

    ui.add_space(spacing::MD);
    ui.horizontal(|ui| {
        if ui.checkbox(&mut view.use_kav, "Fit against Kav").changed() {
            view.dirty = true;
        }
        if view.use_kav {
            ui.label(
                egui::RichText::new("V0 (mL)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            if ui
                .add(egui::DragValue::new(&mut view.v0_ml).speed(0.01))
                .changed()
            {
                view.dirty = true;
            }
            ui.label(
                egui::RichText::new("Vt (mL)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            if ui
                .add(egui::DragValue::new(&mut view.vt_ml).speed(0.01))
                .changed()
            {
                view.dirty = true;
            }
        }
    });
    if view.use_kav && run.meta.v0_ml.is_none() {
        ui.label(
            egui::RichText::new(
                "The method did not record V0/Vt — enter the column's values to use Kav.",
            )
            .font(font_micro())
            .color(c(color::WARNING_600)),
        );
    }

    ui.add_space(spacing::SM);
    if ui.button("Fit calibration").clicked() {
        let basis = if view.use_kav {
            FitBasis::Kav {
                v0_ml: view.v0_ml,
                vt_ml: view.vt_ml,
            }
        } else {
            FitBasis::ElutionVolume
        };
        let usable: Vec<_> = view
            .cal_points
            .iter()
            .filter(|p| p.ve_ml > 0.0)
            .copied()
            .collect();
        match calibration::fit(&usable, basis) {
            Ok(cal) => {
                apply_calibration_to_peaks(run, view, &cal);
                view.calibration = Some(cal);
                view.dirty = true;
            }
            Err(e) => {
                view.calibration = None;
                clear_peak_mw(view);
                ui.label(egui::RichText::new(e.to_string()).color(c(color::DANGER_600)));
            }
        }
    }

    if let Some(cal) = view.calibration.clone() {
        ui.add_space(spacing::MD);
        heading(ui, t, "Fit");
        field(ui, t, "Basis", cal.basis.label());
        field(ui, t, "log10(MW) slope", &num(cal.slope, 4));
        field(ui, t, "Intercept", &num(cal.intercept, 4));
        field(ui, t, "R²", &num(cal.r_squared, 4));
        field(ui, t, "Standards used", &cal.points.len().to_string());

        // A weak fit is stated in words, not signalled by colour alone (rule #3).
        if cal.is_low_confidence() {
            ui.label(
                egui::RichText::new(format!(
                    "Low confidence: R² is below {LOW_CONFIDENCE_R2}. Check the peak assignment \
                     before quoting these molecular weights."
                ))
                .color(c(color::WARNING_600)),
            );
        }
    }

    ui.add_space(spacing::XL);
    concentration_panel(ui, run, view, t);
}

fn set_cal_point(view: &mut View, mw_kda: f64, ve_ml: f32) {
    if let Some(existing) = view
        .cal_points
        .iter_mut()
        .find(|p| (p.mw_kda - mw_kda).abs() < f64::EPSILON)
    {
        existing.ve_ml = ve_ml;
    } else {
        view.cal_points
            .push(calibration::CalibrationPoint { mw_kda, ve_ml });
    }
    view.cal_points.retain(|p| p.ve_ml > 0.0);
    view.dirty = true;
}

/// Stamp estimated MWs onto every peak the curve can speak to.
pub fn apply_calibration_to_peaks(run: &Run, view: &mut View, cal: &calibration::Calibration) {
    for peak in view.peaks.iter_mut() {
        peak.estimated_mw_kda = if peak_supports_mw(run, peak) {
            cal.mw_for_volume(peak.apex_volume_ml)
        } else {
            None
        };
    }
}

fn clear_peak_mw(view: &mut View) {
    for peak in &mut view.peaks {
        peak.estimated_mw_kda = None;
    }
}

/// Beer–Lambert concentration from a peak's A280 apex — deliberately a separate
/// card from size, because they answer different questions (`design.md` §10).
fn concentration_panel(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) {
    heading(ui, t, "Concentration from A280");

    ui.horizontal(|ui| {
        if ui
            .selectable_value(&mut view.concentration.use_molar, false, "A(1%) / mg·mL⁻¹")
            .changed()
        {
            view.dirty = true;
        }
        if ui
            .selectable_value(&mut view.concentration.use_molar, true, "Molar ε")
            .changed()
        {
            view.dirty = true;
        }
    });

    if view.concentration.use_molar {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("ε (M⁻¹cm⁻¹)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            if ui
                .add(egui::DragValue::new(&mut view.concentration.epsilon_molar).speed(10.0))
                .changed()
            {
                view.dirty = true;
            }
            ui.label(
                egui::RichText::new("MW (Da)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            if ui
                .add(egui::DragValue::new(&mut view.concentration.mw_da).speed(100.0))
                .changed()
            {
                view.dirty = true;
            }
        });
    } else {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("ε ((mg/mL)⁻¹cm⁻¹)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            if ui
                .add(
                    egui::DragValue::new(&mut view.concentration.e_mg_per_ml)
                        .speed(0.01)
                        .range(0.0001..=1000.0),
                )
                .changed()
            {
                view.dirty = true;
            }
        });
    }

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Path length (cm)")
                .font(font_micro())
                .color(c(t.text_secondary)),
        );
        if ui
            .add(
                egui::DragValue::new(&mut view.concentration.path_length_cm)
                    .speed(0.01)
                    .range(0.0001..=10.0),
            )
            .changed()
        {
            view.dirty = true;
        }
    });

    ui.add_space(spacing::SM);
    match view.selected_peak() {
        None => {
            ui.label(
                egui::RichText::new("Select a peak to estimate its concentration.")
                    .color(c(t.text_secondary)),
            );
        }
        Some(peak) => {
            let Some(channel) = run.channel(&peak.channel_id) else {
                ui.label(
                    egui::RichText::new("This peak's source channel is unavailable.")
                        .color(c(color::WARNING_600)),
                );
                return;
            };
            if !peak_supports_a280(channel) {
                ui.label(
                    egui::RichText::new(
                        "Concentration is only available for peaks integrated on UV 280 nm.",
                    )
                    .color(c(color::WARNING_600)),
                );
                return;
            }
            // The peak height is in mAU; Beer–Lambert wants AU.
            let absorbance = calibration::au_from_mau(peak.height);
            field(ui, t, "Peak", &peak.id.to_string());
            field(ui, t, "A280 (AU)", &num(absorbance, 4));
            match calibration::concentration_mg_per_ml(
                absorbance,
                view.concentration.extinction(),
                view.concentration.path_length_cm,
            ) {
                Ok(conc) => field(ui, t, "Concentration (mg/mL)", &num(conc, 4)),
                Err(e) => {
                    ui.label(egui::RichText::new(e.to_string()).color(c(color::DANGER_600)));
                }
            }
        }
    }
}

/// Combined results: peak size and concentration side by side.
pub fn results_table(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) {
    heading(ui, t, "Results");
    if view.peaks.is_empty() {
        ui.label(egui::RichText::new("Integrate a peak first.").color(c(t.text_secondary)));
        return;
    }

    let extinction = view.concentration.extinction();
    let path = view.concentration.path_length_cm;
    let cal = view.calibration.clone();

    egui::Grid::new("results")
        .num_columns(6)
        .spacing([spacing::LG, spacing::XS])
        .show(ui, |ui| {
            table_header_row(
                ui,
                t,
                &[
                    "Peak",
                    "Channel",
                    "Ve (mL)",
                    "Area",
                    "Est. MW (kDa)",
                    "Conc. (mg/mL)",
                ],
            );

            for peak in &view.peaks {
                ui.label(egui::RichText::new(peak.id.to_string()).font(font_code()));
                ui.label(
                    egui::RichText::new(
                        run.channel(&peak.channel_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|| peak.channel_id.0.clone()),
                    )
                    .color(c(t.text_primary)),
                );
                ui.label(egui::RichText::new(num(peak.apex_volume_ml as f64, 3)).font(font_code()));
                ui.label(egui::RichText::new(num(peak.area, 2)).font(font_code()));

                // An extrapolated size is labelled, because it is unsupported by
                // the standards rather than merely imprecise.
                let mw_text = match (&cal, estimated_mw_for_peak(run, peak, cal.as_ref())) {
                    (Some(cal), Some(mw)) if cal.is_extrapolated(peak.apex_volume_ml) => {
                        format!("{} (extrapolated)", num(mw, 1))
                    }
                    (_, Some(mw)) => num(mw, 1),
                    _ => "—".to_string(),
                };
                ui.label(egui::RichText::new(mw_text).font(font_code()));

                let conc = if peak_supports_a280_for_run(run, peak) {
                    calibration::concentration_mg_per_ml(
                        calibration::au_from_mau(peak.height),
                        extinction,
                        path,
                    )
                    .map(|v| num(v, 4))
                    .unwrap_or_else(|_| "—".to_string())
                } else {
                    "—".to_string()
                };
                ui.label(egui::RichText::new(conc).font(font_code()));
                ui.end_row();
            }
        });
}

fn peak_supports_mw(run: &Run, peak: &PeakResult) -> bool {
    run.channel(&peak.channel_id)
        .map(|channel| channel.kind == elusive_core::model::ChannelKind::Uv)
        .unwrap_or(false)
}

fn peak_supports_a280(channel: &Channel) -> bool {
    channel.kind == elusive_core::model::ChannelKind::Uv && channel.wavelength_nm == Some(280)
}

fn peak_supports_a280_for_run(run: &Run, peak: &PeakResult) -> bool {
    run.channel(&peak.channel_id)
        .map(peak_supports_a280)
        .unwrap_or(false)
}

fn estimated_mw_for_peak(
    run: &Run,
    peak: &PeakResult,
    calibration: Option<&calibration::Calibration>,
) -> Option<f64> {
    if !peak_supports_mw(run, peak) {
        return None;
    }
    calibration.and_then(|cal| cal.mw_for_volume(peak.apex_volume_ml))
}

#[cfg(test)]
mod tests {
    use super::*;
    use elusive_core::model::{BaselineMode, ChannelId, PeakId};
    use elusive_core::sidecar;

    fn sample_peak() -> PeakResult {
        PeakResult {
            id: PeakId(1),
            channel_id: ChannelId::from("MWave2"),
            v_start_ml: 12.0,
            v_end_ml: 14.0,
            baseline: BaselineMode::LinearEndpoints,
            area: 1234.5,
            height: 890.0,
            apex_volume_ml: 13.0,
            fwhm_ml: Some(0.8),
            estimated_mw_kda: None,
        }
    }

    #[test]
    fn every_export_column_has_a_cell_and_a_width() {
        assert_eq!(EXPORT_COLUMNS.len(), EXPORT_WIDTHS.len());
        assert_eq!(export_row(&sample_peak()).len(), EXPORT_COLUMNS.len());
    }

    #[test]
    fn the_preview_shows_the_same_columns_the_csv_writes() {
        // The point of the preview is that it previews. If the export schema
        // gains a column, this fails rather than quietly showing a stale table.
        let header = sidecar::peaks_to_csv(&[]);
        assert_eq!(
            header.trim_end().split(',').count(),
            EXPORT_COLUMNS.len(),
            "csv header = {header:?}"
        );
    }

    #[test]
    fn preview_cells_match_the_markdown_export_value_for_value() {
        let peak = sample_peak();
        let cells = export_row(&peak);
        let md = sidecar::peaks_to_markdown(std::slice::from_ref(&peak));
        let row: Vec<&str> = md
            .lines()
            .nth(2)
            .expect("one peak produces one body row")
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        assert_eq!(row, cells.iter().map(String::as_str).collect::<Vec<_>>());
    }

    #[test]
    fn an_unmeasured_value_shows_a_dash_not_a_zero() {
        let mut peak = sample_peak();
        peak.fwhm_ml = None;
        let cells = export_row(&peak);
        assert_eq!(cells[7], "—");
        assert_eq!(cells[8], "—");
    }
}

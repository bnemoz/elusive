//! Cards, tables and the right-hand detail rail.
//!
//! Every numeric column is right-aligned monospace with fixed decimals, so digits
//! line up by place value rather than by chance (`DESIGN_SYSTEM.md` rule #4).

use crate::egui_adapter::{c, c_alpha, font_code, font_h3, font_micro, num};
use crate::theme::{color, spacing, Theme};
use crate::view::View;
use egui::Ui;
use elusive_core::calibration::{self, FitBasis, BIORAD_GFS, LOW_CONFIDENCE_R2};
use elusive_core::model::{PeakId, Run};

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
                    for header in [
                        "Peak",
                        "Channel",
                        "Ve (mL)",
                        "Window (mL)",
                        "Area",
                        "Area %",
                        "Height",
                        "FWHM (mL)",
                        "",
                    ] {
                        ui.label(
                            egui::RichText::new(header)
                                .font(font_micro())
                                .color(c(t.text_secondary)),
                        );
                    }
                    ui.end_row();

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
    if let Some(mw) = peak.estimated_mw_kda {
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
        ui.checkbox(&mut view.use_kav, "Fit against Kav");
        if view.use_kav {
            ui.label(
                egui::RichText::new("V0 (mL)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            ui.add(egui::DragValue::new(&mut view.v0_ml).speed(0.01));
            ui.label(
                egui::RichText::new("Vt (mL)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            ui.add(egui::DragValue::new(&mut view.vt_ml).speed(0.01));
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
                apply_calibration_to_peaks(view, &cal);
                view.calibration = Some(cal);
                view.dirty = true;
            }
            Err(e) => {
                view.calibration = None;
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
    concentration_panel(ui, view, t);
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
pub fn apply_calibration_to_peaks(view: &mut View, cal: &calibration::Calibration) {
    for peak in view.peaks.iter_mut() {
        peak.estimated_mw_kda = cal.mw_for_volume(peak.apex_volume_ml);
    }
}

/// Beer–Lambert concentration from a peak's A280 apex — deliberately a separate
/// card from size, because they answer different questions (`design.md` §10).
fn concentration_panel(ui: &mut Ui, view: &mut View, t: Theme) {
    heading(ui, t, "Concentration from A280");

    ui.horizontal(|ui| {
        ui.selectable_value(&mut view.concentration.use_molar, false, "A(1%) / mg·mL⁻¹");
        ui.selectable_value(&mut view.concentration.use_molar, true, "Molar ε");
    });

    if view.concentration.use_molar {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("ε (M⁻¹cm⁻¹)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            ui.add(egui::DragValue::new(&mut view.concentration.epsilon_molar).speed(10.0));
            ui.label(
                egui::RichText::new("MW (Da)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            ui.add(egui::DragValue::new(&mut view.concentration.mw_da).speed(100.0));
        });
    } else {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("ε ((mg/mL)⁻¹cm⁻¹)")
                    .font(font_micro())
                    .color(c(t.text_secondary)),
            );
            ui.add(
                egui::DragValue::new(&mut view.concentration.e_mg_per_ml)
                    .speed(0.01)
                    .range(0.0001..=1000.0),
            );
        });
    }

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Path length (cm)")
                .font(font_micro())
                .color(c(t.text_secondary)),
        );
        ui.add(
            egui::DragValue::new(&mut view.concentration.path_length_cm)
                .speed(0.01)
                .range(0.0001..=10.0),
        );
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
            for header in [
                "Peak",
                "Channel",
                "Ve (mL)",
                "Area",
                "Est. MW (kDa)",
                "Conc. (mg/mL)",
            ] {
                ui.label(
                    egui::RichText::new(header)
                        .font(font_micro())
                        .color(c(t.text_secondary)),
                );
            }
            ui.end_row();

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
                let mw_text = match (&cal, peak.estimated_mw_kda) {
                    (Some(cal), Some(mw)) if cal.is_extrapolated(peak.apex_volume_ml) => {
                        format!("{} (extrapolated)", num(mw, 1))
                    }
                    (_, Some(mw)) => num(mw, 1),
                    _ => "—".to_string(),
                };
                ui.label(egui::RichText::new(mw_text).font(font_code()));

                let conc = calibration::concentration_mg_per_ml(
                    calibration::au_from_mau(peak.height),
                    extinction,
                    path,
                )
                .map(|v| num(v, 4))
                .unwrap_or_else(|_| "—".to_string());
                ui.label(egui::RichText::new(conc).font(font_code()));
                ui.end_row();
            }
        });
}

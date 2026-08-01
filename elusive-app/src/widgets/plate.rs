//! The 96-well plate heatmap.
//!
//! The plate *is* data, so it may be fully saturated (rule #1) — but every well
//! still shows its label and value, because colour never carries meaning alone
//! (rule #3). Three states are distinguished by fill **and** border, not hue:
//! uncollected, collected, and hovered/selected (§10.5).

use crate::egui_adapter::{c, c_alpha};
use crate::theme::{color, plate, radius, spacing, stroke, Rgb, Theme};
use crate::view::View;
use egui::Ui;
use elusive_core::integrate::{metric_over_window, PlateMetric};
use elusive_core::model::{Fraction, Run, Well};
use elusive_core::wells::RackGeometry;

/// One rendered well.
#[derive(Clone, Debug)]
pub struct WellCell {
    pub well: Well,
    pub tube: u32,
    pub value: Option<f64>,
    pub volume_window: (f32, f32),
    pub end_estimated: bool,
}

/// Compute the plate contents for the current channel + metric selection.
pub fn compute(run: &Run, view: &View) -> (Vec<WellCell>, Option<(f64, f64)>) {
    let Some(channel_id) = view.plate_channel.as_ref() else {
        return (Vec::new(), None);
    };
    let Some(channel) = run.channel(channel_id) else {
        return (Vec::new(), None);
    };

    let cells: Vec<WellCell> = run
        .fractions
        .iter()
        .filter_map(|f: &Fraction| {
            let well = f.well?;
            let (a, b) = f.volume_window();
            let value = f
                .has_usable_window()
                .then(|| metric_over_window(channel, a, b, view.plate_metric))
                .flatten();
            Some(WellCell {
                well,
                tube: f.tube,
                value,
                volume_window: (a, b),
                end_estimated: f.end_estimated,
            })
        })
        .collect();

    // One shared scale across all wells, so two wells are comparable by eye.
    let range = cells
        .iter()
        .filter_map(|c| c.value)
        .fold(None, |acc: Option<(f64, f64)>, v| match acc {
            None => Some((v, v)),
            Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
        });

    (cells, range)
}

/// Draw the plate. Returns the well the pointer is over, if any.
pub fn show(ui: &mut Ui, run: &Run, view: &mut View, t: Theme) -> Option<Well> {
    if !run.source_format.supports_fractions() {
        return unavailable(
            ui,
            t,
            "Fractions are not recorded in a CSV export.",
            "Open the .ngcAnalysis or .ngcMethodruns archive to see the plate.",
        );
    }
    if run.fractions.is_empty() {
        return unavailable(
            ui,
            t,
            "This run collected no fractions.",
            "Nothing was dispensed to a rack during the run.",
        );
    }

    let (cells, range) = compute(run, view);
    if cells.is_empty() {
        return unavailable(
            ui,
            t,
            "Fractions have no known well positions.",
            "The rack type in this run is not one EluSive can lay out.",
        );
    }

    let geometry = RackGeometry::HEP96;
    let ramp: &[Rgb] = if view.plate_uniform_ramp {
        &plate::VIRIDIS
    } else {
        &plate::RAMP
    };

    let mut hovered = None;

    // Size cells to fill the available area while staying square-ish.
    let avail = ui.available_size();
    let label_gutter = 22.0;
    let cell_w = ((avail.x - label_gutter) / geometry.cols as f32 - spacing::XS).max(18.0);
    let cell_h = ((avail.y - label_gutter) / geometry.rows as f32 - spacing::XS).max(16.0);
    let cell = egui::vec2(cell_w, cell_h);

    ui.horizontal(|ui| {
        ui.add_space(label_gutter);
        for col in 0..geometry.cols {
            ui.allocate_ui(egui::vec2(cell.x, label_gutter), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new((col + 1).to_string())
                            .font(crate::egui_adapter::font_micro())
                            .color(c(t.text_secondary)),
                    );
                });
            });
            ui.add_space(spacing::XS);
        }
    });

    for row in 0..geometry.rows {
        ui.horizontal(|ui| {
            ui.allocate_ui(egui::vec2(label_gutter, cell.y), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(((b'A' + row) as char).to_string())
                            .font(crate::egui_adapter::font_micro())
                            .color(c(t.text_secondary)),
                    );
                });
            });

            for col in 0..geometry.cols {
                let well = Well::new(row, col);
                let entry = cells.iter().find(|c| c.well == well);
                if draw_well(ui, cell, well, entry, range, ramp, view, t) {
                    hovered = Some(well);
                }
                ui.add_space(spacing::XS);
            }
        });
        ui.add_space(spacing::XS);
    }

    if let Some((lo, hi)) = range {
        legend(ui, run, view, t, lo, hi, ramp);
    }

    hovered
}

#[allow(clippy::too_many_arguments)]
fn draw_well(
    ui: &mut Ui,
    size: egui::Vec2,
    well: Well,
    entry: Option<&WellCell>,
    range: Option<(f64, f64)>,
    ramp: &[Rgb],
    view: &View,
    t: Theme,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    let corner = egui::CornerRadius::same(radius::SM as u8);

    let is_hovered = response.hovered() || view.hovered_well == Some(well);

    match entry {
        // Uncollected: panel fill plus a border. Distinguished from a *zero*
        // measurement by having no value text at all.
        None => {
            painter.rect_filled(rect, corner, c(t.panel_bg));
            painter.rect_stroke(
                rect,
                corner,
                egui::Stroke::new(stroke::HAIRLINE, c(t.border)),
                egui::StrokeKind::Inside,
            );
        }
        Some(cell) => {
            let fraction = match (cell.value, range) {
                (Some(v), Some((lo, hi))) if hi > lo => ((v - lo) / (hi - lo)) as f32,
                (Some(_), _) => 1.0,
                (None, _) => 0.0,
            };
            let fill = if cell.value.is_some() {
                plate::sample(ramp, fraction)
            } else {
                t.panel_elevated
            };
            painter.rect_filled(rect, corner, c(fill));

            // Collected wells carry a solid border; hovered wells a thicker accent
            // one. Shape carries the state, colour only reinforces it.
            let (width, stroke_color) = if is_hovered {
                (stroke::FOCUS, t.accent)
            } else {
                (stroke::HAIRLINE, t.border)
            };
            painter.rect_stroke(
                rect,
                corner,
                egui::Stroke::new(width, c(stroke_color)),
                egui::StrokeKind::Inside,
            );

            let label_style = if cell.value.is_some() {
                plate::label_on(fill)
            } else {
                plate::LabelStyle {
                    text: t.text_secondary,
                    halo: None,
                }
            };

            // Label always; value too when the cell is tall enough to hold both.
            let label = well.label();
            let value_text = cell.value.map(compact_number).unwrap_or_default();

            haloed_text(
                painter,
                rect.left_top() + egui::vec2(3.0, 2.0),
                egui::Align2::LEFT_TOP,
                &label,
                label_style,
            );
            if rect.height() >= 30.0 && !value_text.is_empty() {
                haloed_text(
                    painter,
                    rect.right_bottom() - egui::vec2(3.0, 2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    &value_text,
                    label_style,
                );
            }

            // An inferred fraction end is marked with a corner tick, so a
            // provisional window is never mistaken for a measured one.
            if cell.end_estimated {
                painter.line_segment(
                    [
                        rect.right_top() + egui::vec2(-6.0, 1.0),
                        rect.right_top() + egui::vec2(-1.0, 6.0),
                    ],
                    egui::Stroke::new(stroke::HAIRLINE, c(color::WARNING_600)),
                );
            }

            let value_str = cell
                .value
                .map(|v| format!("{v:.4}"))
                .unwrap_or_else(|| "no data".to_string());
            response.clone().on_hover_text(format!(
                "{} · tube {}\n{:.3}–{:.3} mL{}\n{} = {}",
                label,
                cell.tube,
                cell.volume_window.0,
                cell.volume_window.1,
                if cell.end_estimated {
                    " (end inferred)"
                } else {
                    ""
                },
                view.plate_metric.label(),
                value_str
            ));
        }
    }

    response.hovered()
}

/// Draw text with an optional 1 px outline, so a label stays legible where the
/// ramp's mid-tones defeat both plain black and plain white (`theme::plate`).
fn haloed_text(
    painter: &egui::Painter,
    pos: egui::Pos2,
    align: egui::Align2,
    text: &str,
    style: plate::LabelStyle,
) {
    let font = crate::egui_adapter::font_micro();
    if let Some(halo) = style.halo {
        for offset in [
            egui::vec2(-1.0, 0.0),
            egui::vec2(1.0, 0.0),
            egui::vec2(0.0, -1.0),
            egui::vec2(0.0, 1.0),
        ] {
            painter.text(pos + offset, align, text, font.clone(), c(halo));
        }
    }
    painter.text(pos, align, text, font, c(style.text));
}

/// Vertical scale legend with min/max and the active channel + metric (§10.3).
#[allow(clippy::too_many_arguments)]
fn legend(ui: &mut Ui, run: &Run, view: &View, t: Theme, lo: f64, hi: f64, ramp: &[Rgb]) {
    ui.add_space(spacing::SM);
    ui.horizontal(|ui| {
        let channel_name = view
            .plate_channel
            .as_ref()
            .and_then(|id| run.channel(id))
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let unit = view
            .plate_channel
            .as_ref()
            .and_then(|id| run.channel(id))
            .map(|c| view.plate_metric.unit_suffix(&c.display_unit))
            .unwrap_or_default();

        ui.label(
            egui::RichText::new(format!(
                "{channel_name} · {}  ({unit})",
                view.plate_metric.label()
            ))
            .font(crate::egui_adapter::font_micro())
            .color(c(t.text_secondary)),
        );

        ui.label(
            egui::RichText::new(format!("{lo:.3}"))
                .font(crate::egui_adapter::font_code())
                .color(c(t.text_secondary)),
        );

        let (rect, _) = ui.allocate_exact_size(egui::vec2(140.0, 10.0), egui::Sense::hover());
        let steps = 40;
        for i in 0..steps {
            let x0 = rect.left() + rect.width() * i as f32 / steps as f32;
            let x1 = rect.left() + rect.width() * (i + 1) as f32 / steps as f32;
            let fill = plate::sample(ramp, i as f32 / (steps - 1) as f32);
            ui.painter().rect_filled(
                egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom())),
                0,
                c(fill),
            );
        }
        ui.painter().rect_stroke(
            rect,
            0,
            egui::Stroke::new(stroke::HAIRLINE, c(t.border)),
            egui::StrokeKind::Inside,
        );

        ui.label(
            egui::RichText::new(format!("{hi:.3}"))
                .font(crate::egui_adapter::font_code())
                .color(c(t.text_secondary)),
        );
    });
}

fn unavailable(ui: &mut Ui, t: Theme, headline: &str, detail: &str) -> Option<Well> {
    // Deterministic text, not an empty grid: showing 96 blank wells would imply
    // the run collected nothing, which is a different claim (§6, empty states).
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(headline)
                    .font(crate::egui_adapter::font_h3())
                    .color(c(t.text_primary)),
            );
            ui.label(egui::RichText::new(detail).color(c_alpha(t.text_secondary, 220)));
        });
    });
    None
}

/// Compact numeric label for a well: enough significant digits to compare
/// neighbours without overflowing a 40 px cell.
fn compact_number(v: f64) -> String {
    let a = v.abs();
    if a >= 1000.0 {
        format!("{:.0}", v)
    } else if a >= 10.0 {
        format!("{:.1}", v)
    } else if a >= 1.0 {
        format!("{:.2}", v)
    } else {
        format!("{:.3}", v)
    }
}

/// Rows for the well CSV export.
pub fn export_rows(run: &Run, view: &View) -> Vec<(Well, String, PlateMetric, Option<f64>)> {
    let channel_id = match view.plate_channel.as_ref() {
        Some(id) => id.0.clone(),
        None => String::new(),
    };
    let (cells, _) = compute(run, view);
    cells
        .into_iter()
        .map(|c| (c.well, channel_id.clone(), view.plate_metric, c.value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use elusive_core::model::{Channel, ChannelKind, RunMeta, Sample, SourceFormat};

    fn run_with_fractions() -> Run {
        let mut uv = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        uv.samples = (0..=100)
            .map(|i| {
                let v = i as f32 / 10.0;
                Sample::new(
                    v * 60.0,
                    v,
                    if (4.0..6.0).contains(&v) { 10.0 } else { 0.0 },
                )
            })
            .collect();

        let fractions = (1..=4)
            .map(|tube| Fraction {
                tube,
                rack: 1,
                well: elusive_core::wells::well_for_tube(
                    tube,
                    RackGeometry::HEP96,
                    elusive_core::wells::CollectionPattern::Serpentine,
                ),
                vol_start_ml: 3.0 + (tube - 1) as f32,
                vol_end_ml: 4.0 + (tube - 1) as f32,
                time_start_s: 0.0,
                time_end_s: 0.0,
                nominal_size_ml: Some(1.0),
                end_estimated: false,
                rack_type: "HEP96".into(),
                pattern: "Serpentine".into(),
            })
            .collect();

        Run {
            meta: RunMeta::default(),
            source_format: SourceFormat::NgcAnalysis,
            source_path: std::path::PathBuf::from("t.ngcAnalysis"),
            channels: vec![uv],
            fractions,
            events: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn wells_are_populated_and_share_one_scale() {
        let run = run_with_fractions();
        let mut view = View::default();
        view.adopt_run(&run);

        let (cells, range) = compute(run_ref(&run), &view);
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[0].well.label(), "A1");
        let (lo, hi) = range.expect("a shared min/max across wells");
        assert!(hi > lo, "the peak fraction must outrank the empty ones");
    }

    fn run_ref(run: &Run) -> &Run {
        run
    }

    #[test]
    fn the_well_under_the_peak_has_the_largest_metric() {
        let run = run_with_fractions();
        let mut view = View::default();
        view.adopt_run(&run);
        let (cells, _) = compute(&run, &view);

        let best = cells
            .iter()
            .max_by(|a, b| a.value.unwrap().total_cmp(&b.value.unwrap()))
            .unwrap();
        // The signal is high over 4..6 mL; tube 2 covers 4..5 mL.
        assert_eq!(best.tube, 2);
    }

    #[test]
    fn every_metric_produces_a_value_for_a_collected_well() {
        let run = run_with_fractions();
        let mut view = View::default();
        view.adopt_run(&run);
        for metric in PlateMetric::ALL {
            view.plate_metric = metric;
            let (cells, _) = compute(&run, &view);
            assert!(
                cells.iter().all(|c| c.value.is_some()),
                "{metric:?} left a collected well empty"
            );
        }
    }

    #[test]
    fn export_rows_carry_the_active_channel_and_metric() {
        let run = run_with_fractions();
        let mut view = View::default();
        view.adopt_run(&run);
        view.plate_metric = PlateMetric::MaxValue;
        let rows = export_rows(&run, &view);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].1, "MWave2");
        assert_eq!(rows[0].2, PlateMetric::MaxValue);
    }

    #[test]
    fn compact_labels_keep_a_readable_number_of_digits() {
        // Rust rounds halves to even, so 1234.5 formats as 1234 — asserted here
        // so the label width, not the tie-breaking rule, is what is under test.
        assert_eq!(compact_number(1234.5), "1234");
        assert_eq!(compact_number(1234.6), "1235");
        assert_eq!(compact_number(12.345), "12.3");
        assert_eq!(compact_number(1.2345), "1.23");
        assert_eq!(compact_number(0.12345), "0.123");
    }
}

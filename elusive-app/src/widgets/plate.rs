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

    let label_gutter = LABEL_GUTTER;
    let cell = cell_size(ui.available_width(), geometry);

    // Scroll only when the user has dragged the pane below the plate's natural
    // height. The cap comes from the pane's stored size rather than
    // `available_height`, which reports the panel's *maximum*. Note it feeds
    // scrolling only, never `cell` — that independence is what keeps the pane
    // from creeping open a few pixels per frame.
    let used_above = (ui.cursor().top() - ui.max_rect().top()).max(0.0);
    let cap =
        (pane_height(ui) - CARD_MARGINS - used_above - LEGEND_HEIGHT).max(2.0 * MIN_CELL_HEIGHT);

    egui::ScrollArea::vertical()
        .id_salt("plate-grid")
        .max_height(cap)
        .show(ui, |ui| {
            hovered = plate_grid(
                ui,
                &cells,
                range,
                ramp,
                cell,
                label_gutter,
                geometry,
                view,
                t,
            );
        });

    if let Some((lo, hi)) = range {
        legend(ui, run, view, t, lo, hi, ramp);
    }

    hovered
}

/// Width reserved for the row letters / column numbers.
const LABEL_GUTTER: f32 = 22.0;
/// Smallest cell that still fits a two-character well label.
const MIN_CELL_WIDTH: f32 = 20.0;
/// Beyond this the plate stops looking like a plate and starts wasting space.
const MAX_CELL_WIDTH: f32 = 56.0;
const MIN_CELL_HEIGHT: f32 = 16.0;
/// Tall enough to show a label and a value, small enough that eight rows fit
/// a pane that still leaves the chromatogram the larger share.
const MAX_CELL_HEIGHT: f32 = 30.0;
/// Wells render slightly wider than tall, matching a real HEP96 footprint.
const CELL_ASPECT: f32 = 0.72;
/// Cell height at which a well can show its value as well as its label.
const VALUE_TEXT_MIN_HEIGHT: f32 = 26.0;
/// Smallest useful pane: two rows of wells plus the legend.
pub const MIN_PANE_HEIGHT: f32 = 220.0;
/// The plate must never be able to crowd out the chromatogram (design.md §11).
pub const MAX_PANE_HEIGHT: f32 = 470.0;
/// Id of the pane, shared with `app::linked_pane` so its size can be read back.
pub const PANE_ID: &str = "plate-pane";
/// Height the legend row adds below the grid.
const LEGEND_HEIGHT: f32 = 50.0;
/// Room the heading row (title plus the channel/metric pickers) takes.
const HEADING_HEIGHT: f32 = 48.0;
/// The card frame's inner margin, top and bottom.
const CARD_MARGINS: f32 = 2.0 * spacing::LG;

/// Height the well grid needs at a given cell size, excluding the legend.
///
/// Exact, not approximate: `plate_grid` zeroes the ambient item spacing and adds
/// its own, so this model and the rendered result agree to the pixel. They have
/// to — the pane height is chosen from this number.
fn grid_height(cell_height: f32, geometry: RackGeometry) -> f32 {
    LABEL_GUTTER + geometry.rows as f32 * (cell_height + spacing::XS)
}

/// The plate's natural height: what it asks the pane for.
///
/// Intrinsic on purpose. Earlier versions derived the cell height from the pane
/// and let the pane derive its height from the content, which is circular — the
/// pane crept a few pixels open every frame until it reached its ceiling and
/// squeezed the chromatogram out. Making the content's height independent of the
/// pane removes the cycle rather than trying to balance it.
pub fn natural_height(available_width: f32, geometry: RackGeometry) -> f32 {
    let cell = cell_size(available_width, geometry);
    CARD_MARGINS + HEADING_HEIGHT + grid_height(cell.y, geometry) + LEGEND_HEIGHT
}

/// Height the pane should open at, given the width it will have.
///
/// `app::linked_pane` asks for this rather than passing a constant, so the pane
/// opens at exactly the size the plate needs and there is no number to keep in
/// sync by hand.
pub fn natural_pane_height(available_width: f32) -> f32 {
    natural_height(available_width, RackGeometry::HEP96).clamp(MIN_PANE_HEIGHT, MAX_PANE_HEIGHT)
}

/// The pane's height as of last frame, clamped to what the panel permits.
///
/// Reading the stored size is safe where reading `ui.available_height()` was not:
/// the content below is built to occupy exactly this height, so the value is a
/// fixed point — it only changes when the user drags the splitter. Available
/// height, by contrast, reports the panel's *maximum*, so sizing from it made the
/// plate expand to its ceiling every frame.
fn pane_height(ui: &Ui) -> f32 {
    egui::PanelState::load(ui.ctx(), egui::Id::new(PANE_ID))
        .map(|state| state.size().y)
        .unwrap_or(MAX_PANE_HEIGHT)
        .clamp(MIN_PANE_HEIGHT, MAX_PANE_HEIGHT)
}

/// Cell size for a given pane width.
///
/// Width in, size out. The height comes from the *constant* pane budget, never
/// from `ui.available_height()` — inside an `egui::Panel` that reports space up
/// to `size_range.max`, so sizing from it makes the plate expand to its own
/// ceiling and squeeze the chromatogram out. Constants cannot feed back.
fn cell_size(available_width: f32, geometry: RackGeometry) -> egui::Vec2 {
    let cols = geometry.cols.max(1) as f32;
    let w = ((available_width - LABEL_GUTTER) / cols - spacing::XS)
        .clamp(MIN_CELL_WIDTH, MAX_CELL_WIDTH);
    let h = (w * CELL_ASPECT).clamp(MIN_CELL_HEIGHT, MAX_CELL_HEIGHT);
    egui::vec2(w, h)
}

#[allow(clippy::too_many_arguments)]
fn plate_grid(
    ui: &mut Ui,
    cells: &[WellCell],
    range: Option<(f64, f64)>,
    ramp: &[Rgb],
    cell: egui::Vec2,
    label_gutter: f32,
    geometry: RackGeometry,
    view: &View,
    t: Theme,
) -> Option<Well> {
    let mut hovered = None;

    // Own the spacing outright: the ambient 8 px between rows would put the
    // rendered grid ~64 px past what `grid_height` predicts, and the pane height
    // is derived from that prediction.
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

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
            if rect.height() >= VALUE_TEXT_MIN_HEIGHT && !value_text.is_empty() {
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
    fn cell_size_never_grows_without_bound() {
        // The runaway this guards against: a plate that keeps getting bigger
        // until it hides the chromatogram.
        for width in [0.0f32, 120.0, 640.0, 1920.0, 10_000.0] {
            let cell = cell_size(width, RackGeometry::HEP96);
            assert!(
                (MIN_CELL_WIDTH..=MAX_CELL_WIDTH).contains(&cell.x),
                "width {width} gave cell width {}",
                cell.x
            );
            assert!(
                (MIN_CELL_HEIGHT..=MAX_CELL_HEIGHT).contains(&cell.y),
                "width {width} gave cell height {}",
                cell.y
            );
        }
    }

    #[test]
    fn the_plate_asks_for_a_height_the_pane_can_actually_give() {
        // The v0.1 bug was the plate growing until it hid the chromatogram. The
        // plate's height is now intrinsic, so the only thing to check is that
        // what it asks for fits inside the ceiling the panel enforces.
        for width in [640.0f32, 1000.0, 1440.0, 1920.0, 3840.0] {
            let natural = natural_height(width, RackGeometry::HEP96);
            assert!(
                natural <= MAX_PANE_HEIGHT,
                "width {width}: plate wants {natural} px, ceiling is {MAX_PANE_HEIGHT}"
            );
            assert!(
                natural >= MIN_PANE_HEIGHT,
                "width {width}: {natural} px is too short"
            );
        }
    }

    #[test]
    fn the_plate_height_does_not_depend_on_the_space_offered() {
        // There is no pane argument to depend on — that is the point. This test
        // exists so a future refactor cannot quietly reintroduce one.
        let a = natural_height(1440.0, RackGeometry::HEP96);
        let b = natural_height(1440.0, RackGeometry::HEP96);
        assert_eq!(a, b);
    }

    #[test]
    fn cell_size_is_a_pure_function_of_width() {
        // Same width, same answer — the plate's height can never influence it,
        // which is what breaks the panel/content feedback loop.
        assert_eq!(
            cell_size(900.0, RackGeometry::HEP96),
            cell_size(900.0, RackGeometry::HEP96)
        );
        assert!(cell_size(1400.0, RackGeometry::HEP96).x > cell_size(400.0, RackGeometry::HEP96).x);
    }

    #[test]
    fn the_plate_leaves_the_chromatogram_the_larger_share() {
        // design.md §11: chromatogram on top with most of the height. The plate
        // taking the window was the v0.1 bug; this pins the intent numerically.
        let typical_content_height = 900.0f32;
        for width in [700.0f32, 1440.0, 2560.0] {
            let pane = natural_pane_height(width);
            assert!(
                pane < typical_content_height - pane,
                "width {width}: plate pane {pane} would out-size the chart pane"
            );
        }
    }

    #[test]
    fn a_full_plate_fits_the_pane_it_opens_at() {
        // Scrolling is a safety net for a user who drags the splitter small, not
        // the normal way to read a plate: at any realistic width the pane must
        // open large enough for all 96 wells.
        for width in [700.0f32, 1000.0, 1440.0, 2560.0, 3840.0] {
            let natural = natural_height(width, RackGeometry::HEP96);
            let pane = natural_pane_height(width);
            assert!(
                natural <= pane + 0.5,
                "width {width}: plate needs {natural} px, pane opens at {pane}"
            );
        }
    }

    #[test]
    fn wells_are_tall_enough_to_show_their_value() {
        // Rule #3: colour never carries meaning alone, so the number has to fit.
        // This is what pins MAX_CELL_HEIGHT — shrink it and the values go.
        for width in [1000.0f32, 1280.0, 1920.0] {
            let cell = cell_size(width, RackGeometry::HEP96);
            assert!(
                cell.y >= VALUE_TEXT_MIN_HEIGHT,
                "width {width}: cell height {} hides the value",
                cell.y
            );
        }
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

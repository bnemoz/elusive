//! The Overview section: a responsive card flow the user can rearrange.
//!
//! Two behaviours live here that a plain top-to-bottom stack did not need.
//!
//! **Columns.** On a 27" screen a single column of cards leaves two thirds of the
//! window empty, so the card count per row follows the available width.
//!
//! **Reordering.** Each card carries a drag handle in its top-right corner;
//! dropping one card on another takes that slot. The handle is the *only* drag
//! target, because making the whole card draggable would eat the clicks that the
//! fraction table needs and make text unselectable.
//!
//! ## Why every measurement here is a width
//!
//! Layout decisions are taken from `available_width()` and from static per-panel
//! hints, never from how tall a card turned out to be. A parent that sizes itself
//! from content that sized itself from the parent oscillates forever, and this
//! repository has already paid for that twice — see the doc comment on
//! [`crate::widgets::chromatogram::data_y_range`] and on
//! `EluSiveApp::persist_egui_memory`. Width inside a vertical `ScrollArea` is
//! fixed by the window; height is an output, so it is never an input.

use crate::egui_adapter::{self as adapt, c, font_micro};
use crate::theme::{radius, spacing, stroke, Theme};
use crate::view::{PanelId, View};
use crate::widgets::panels;
use elusive_core::model::Run;

/// Widths at or above which the Overview uses two and three columns.
///
/// **Chosen, not measured.** `DESIGN_SYSTEM.md` fixes the spacing scale and card
/// padding but names no responsive breakpoints, so these are a starting point:
/// roughly the width at which the channel table stops wrapping its headers, and
/// twice that. They are safe to retune — nothing else depends on their values.
pub const TWO_COLUMN_MIN_WIDTH: f32 = 900.0;
pub const THREE_COLUMN_MIN_WIDTH: f32 = 1500.0;

/// The drag handle glyph.
///
/// U+2630 TRIGRAM FOR HEAVEN, and not one of the prettier braille "grip" dots
/// (U+28FF, U+283F): the Inter and JetBrains Mono files are not vendored in this
/// repository, so the app usually runs on egui's bundled faces, and those cover
/// U+2630 but not the braille block. A missing glyph would draw nothing at all —
/// egui's fallback stack has no replacement character either. See the test at the
/// bottom of this file, which fails if that ever stops being true.
const HANDLE_GLYPH: &str = "\u{2630}";

/// Side of the square drag-handle hit area, in points. Comfortably above the
/// ~24 pt minimum touch/pointer target while still reading as chrome.
const HANDLE_SIZE: f32 = 20.0;

/// How many columns fit in `available_width`.
pub fn column_count(available_width: f32) -> usize {
    // Written as ascending comparisons so a NaN width (which compares false
    // against everything) falls through to the safe single-column answer.
    if available_width >= THREE_COLUMN_MIN_WIDTH {
        3
    } else if available_width >= TWO_COLUMN_MIN_WIDTH {
        2
    } else {
        1
    }
}

/// A *hint* at how much vertical room a card tends to want, in arbitrary units.
///
/// Deliberately static and deliberately approximate. Packing by real height would
/// mean measuring a laid-out card and feeding last frame's measurement into this
/// frame's column choice — which flips as soon as a card changes column and so
/// changes width and height. A hint that is sometimes wrong gives a slightly
/// uneven but *stable* layout, and that is the better failure.
pub fn nominal_weight(panel: PanelId) -> f32 {
    match panel {
        // A handful of label/value rows.
        PanelId::RunSummary => 2.0,
        // Usually a few lines; occasionally none at all, in which case the card
        // is not in the layout to begin with.
        PanelId::Warnings => 1.5,
        // Tables. One row per channel and one per fraction — up to 96 of those.
        PanelId::Channels => 4.0,
        PanelId::Fractions => 5.0,
    }
}

/// Deal `panels` into `columns`, keeping the user's order and putting each card
/// in the column that currently looks shortest by [`nominal_weight`].
///
/// Ties go to the leftmost column, so a single-panel Overview stays on the left
/// rather than wandering.
pub fn distribute(panels: &[PanelId], columns: usize) -> Vec<Vec<PanelId>> {
    let columns = columns.max(1);
    let mut out: Vec<Vec<PanelId>> = vec![Vec::new(); columns];
    let mut load = vec![0.0f32; columns];
    for &panel in panels {
        // `min_by` keeps the first of equal minima, which is the leftmost column.
        let target = (0..columns)
            .min_by(|&a, &b| load[a].total_cmp(&load[b]))
            .unwrap_or(0);
        load[target] += nominal_weight(panel);
        out[target].push(panel);
    }
    out
}

/// Is this card worth a slot in the layout at all?
///
/// The warnings card is the only conditional one. It is skipped entirely rather
/// than drawn empty — a hole reserved for a card with nothing in it reads as a
/// rendering bug — and because the order is stored over *all* panels, its absence
/// does not disturb where the others sit when it comes back.
fn is_visible(panel: PanelId, run: &Run) -> bool {
    match panel {
        PanelId::Warnings => !run.warnings.is_empty(),
        _ => true,
    }
}

/// Draw the whole Overview section.
pub fn show(ui: &mut egui::Ui, run: &Run, view: &mut View, t: Theme) {
    // Read the width *outside* the scroll area. Inside, the width shrinks when a
    // vertical scrollbar appears — and a card count that changed with scrollbar
    // visibility could flip the layout, change the height, and hide the scrollbar
    // again, one frame at a time. Deciding out here breaks that circle.
    let columns = column_count(ui.available_width());

    egui::ScrollArea::vertical()
        .id_salt("overview-scroll")
        .show(ui, |ui| {
            toolbar(ui, view, t, columns);
            ui.add_space(spacing::SM);

            let visible: Vec<PanelId> = view
                .overview_order
                .iter()
                .copied()
                .filter(|p| is_visible(*p, run))
                .collect();

            // Resolved after the columns close: reordering mid-layout would move
            // a card the user is still looking at, and the borrow of `view` is
            // already spoken for inside the closure.
            let mut drop: Option<(PanelId, PanelId)> = None;
            ui.columns(columns, |cols| {
                for (col, panels) in cols.iter_mut().zip(distribute(&visible, columns)) {
                    for panel in panels {
                        if let Some(dragged) = card(col, run, view, t, panel) {
                            drop = Some((dragged, panel));
                        }
                        col.add_space(spacing::LG);
                    }
                }
            });

            if let Some((dragged, target)) = drop {
                view.move_overview_panel(dragged, target);
            }
        });
}

/// The one row of section chrome: how to rearrange, and how to undo it.
fn toolbar(ui: &mut egui::Ui, view: &mut View, t: Theme, columns: usize) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{columns} column{} — drag {HANDLE_GLYPH} to rearrange",
                if columns == 1 { "" } else { "s" }
            ))
            .font(font_micro())
            .color(c(t.text_secondary)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Disabled rather than hidden, so the escape hatch is somewhere the
            // user can find it *before* they need it.
            let reset = ui.add_enabled(
                !view.overview_order_is_default(),
                egui::Button::new(egui::RichText::new("Reset layout").font(font_micro())),
            );
            if reset.clicked() {
                view.reset_overview_order();
            }
        });
    });
}

/// One card. Returns the panel that was dropped onto it, if any.
fn card(
    ui: &mut egui::Ui,
    run: &Run,
    view: &mut View,
    t: Theme,
    panel: PanelId,
) -> Option<PanelId> {
    let response = adapt::card(t)
        .show(ui, |ui| {
            // Pin the content to the column so the handle lands on the card's
            // right edge rather than on the right edge of whatever the panel drew,
            // and so cards in a column share one width. Width only: a card's
            // height stays whatever its content needs.
            ui.set_min_width(ui.available_width());
            body(ui, run, view, t, panel);
            handle(ui, t, panel);
        })
        .response;

    let dragged = response.dnd_hover_payload::<PanelId>();
    if let Some(dragged) = dragged.as_deref() {
        if *dragged != panel {
            // Feedback by outline, and by the "moves here" label the handle draws
            // — never colour alone (`DESIGN_SYSTEM.md` rule #3).
            ui.painter().rect_stroke(
                response.rect,
                radius::MD,
                egui::Stroke::new(stroke::FOCUS, c(t.focus_ring)),
                egui::StrokeKind::Inside,
            );
        }
    }

    response
        .dnd_release_payload::<PanelId>()
        .map(|dragged| *dragged)
        .filter(|dragged| *dragged != panel)
}

/// The card's contents. Each panel still draws its own heading, which is why the
/// handle is positioned over the corner instead of sharing the heading's row.
fn body(ui: &mut egui::Ui, run: &Run, view: &mut View, t: Theme, panel: PanelId) {
    match panel {
        PanelId::RunSummary => panels::run_summary(ui, run, t),
        PanelId::Warnings => panels::warnings(ui, run, t),
        PanelId::Channels => panels::channel_table(ui, run, t),
        PanelId::Fractions => {
            panels::heading(ui, t, "Fractions");
            panels::fraction_table(ui, run, view, t);
        }
    }
}

/// `dnd_drag_source` documents that its id must be globally unique, and one
/// derived from the panel is — a panel is drawn at most once per frame. Being
/// addressable from outside the layout also lets a headless test find the handle
/// with `Context::read_response` and drive a real drag.
fn handle_id(panel: PanelId) -> egui::Id {
    egui::Id::new(("overview-handle", panel.as_str()))
}

/// The drag handle, placed over the top-right corner of the card's content.
///
/// Placed rather than laid out: the corner is inside the content the card has
/// already drawn, so the handle costs no vertical space and cannot push the card
/// taller. It is added last for the same reason — it needs the content's rect.
fn handle(ui: &mut egui::Ui, t: Theme, panel: PanelId) {
    let content = ui.min_rect();
    let rect = egui::Rect::from_min_size(
        egui::pos2(content.right() - HANDLE_SIZE, content.top()),
        egui::vec2(HANDLE_SIZE, HANDLE_SIZE),
    );

    let dragging_something_else =
        egui::DragAndDrop::payload::<PanelId>(ui.ctx()).is_some_and(|dragged| *dragged != panel);

    let builder =
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::centered_and_justified(
                egui::Direction::TopDown,
            ));
    ui.scope_builder(builder, |ui| {
        let response = ui
            .dnd_drag_source(handle_id(panel), panel, |ui| {
                ui.label(
                    egui::RichText::new(HANDLE_GLYPH)
                        .font(font_micro())
                        .color(c(t.text_secondary)),
                );
            })
            .response;

        // The gesture is not discoverable from a glyph alone, so say it in words
        // — and say what a drop would do while one is in flight.
        response.on_hover_text(if dragging_something_else {
            "Drop to move the dragged card here"
        } else {
            "Drag to move this card"
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_count_changes_exactly_at_the_breakpoints() {
        assert_eq!(column_count(0.0), 1);
        assert_eq!(column_count(TWO_COLUMN_MIN_WIDTH - 0.1), 1);
        assert_eq!(column_count(TWO_COLUMN_MIN_WIDTH), 2);
        assert_eq!(column_count(THREE_COLUMN_MIN_WIDTH - 0.1), 2);
        assert_eq!(column_count(THREE_COLUMN_MIN_WIDTH), 3);
        assert_eq!(column_count(10_000.0), 3);
    }

    #[test]
    fn a_nonsense_width_still_lays_out_one_column() {
        assert_eq!(column_count(f32::NAN), 1);
        assert_eq!(column_count(-1.0), 1);
    }

    #[test]
    fn one_column_keeps_the_users_order_exactly() {
        let order = vec![PanelId::Fractions, PanelId::RunSummary, PanelId::Channels];
        assert_eq!(distribute(&order, 1), vec![order]);
    }

    #[test]
    fn every_panel_is_dealt_out_exactly_once() {
        for columns in 1..=4 {
            let dealt: Vec<PanelId> = distribute(&PanelId::ALL, columns)
                .into_iter()
                .flatten()
                .collect();
            let mut sorted = dealt.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), PanelId::ALL.len(), "{columns} columns");
        }
    }

    #[test]
    fn a_tall_card_pushes_the_next_one_into_the_emptier_column() {
        // Fractions (5.0) then RunSummary (2.0): the short card must not stack
        // under the tall one while another column is empty.
        let columns = distribute(&[PanelId::Fractions, PanelId::RunSummary], 2);
        assert_eq!(columns[0], vec![PanelId::Fractions]);
        assert_eq!(columns[1], vec![PanelId::RunSummary]);
    }

    #[test]
    fn more_columns_than_cards_leaves_empty_columns_rather_than_panicking() {
        let columns = distribute(&[PanelId::RunSummary], 3);
        assert_eq!(columns.len(), 3);
        assert_eq!(columns[0], vec![PanelId::RunSummary]);
        assert!(columns[1].is_empty() && columns[2].is_empty());
    }

    #[test]
    fn zero_columns_is_treated_as_one_rather_than_dropping_every_card() {
        assert_eq!(distribute(&PanelId::ALL, 0).len(), 1);
    }

    /// A run with something in every card, including a warning.
    fn test_run() -> Run {
        use elusive_core::model::{
            Channel, ChannelKind, Fraction, RunMeta, Sample, SourceFormat, Warning,
        };

        let mut uv = Channel::new("MWave2", "UV 280 nm", ChannelKind::Uv);
        uv.wavelength_nm = Some(280);
        uv.samples = vec![Sample::new(0.0, 0.0, 1.0), Sample::new(60.0, 1.0, 2.0)];

        Run {
            meta: RunMeta {
                run_name: "layout smoke test".into(),
                ..RunMeta::default()
            },
            source_format: SourceFormat::NgcAnalysis,
            source_path: std::path::PathBuf::from("smoke.ngcAnalysis"),
            channels: vec![uv],
            fractions: vec![Fraction {
                tube: 1,
                rack: 1,
                well: None,
                vol_start_ml: 0.0,
                vol_end_ml: 0.5,
                time_start_s: 0.0,
                time_end_s: 30.0,
                nominal_size_ml: Some(0.5),
                end_estimated: false,
                rack_type: "HEP96".into(),
                pattern: "Serpentine".into(),
            }],
            events: Vec::new(),
            warnings: vec![Warning::new("wavelengths", "assumed a default mapping")],
        }
    }

    /// Lay the section out for real, headlessly, at one width per breakpoint.
    ///
    /// The GUI cannot be driven here, but egui will still panic on a zero-width
    /// column split or an id clash, and this catches both. Returns the order
    /// afterwards so the caller can check that a frame with no pointer input did
    /// not somehow rearrange anything.
    fn lay_out_at(width: f32) -> Vec<PanelId> {
        let run = test_run();
        let mut view = View::default();
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 900.0),
            )),
            ..Default::default()
        };
        // Two passes: egui resolves some interaction state against the previous
        // frame, so a single pass would not exercise the same code.
        for _ in 0..2 {
            let _ = ctx.run_ui(input.clone(), |ui| {
                show(ui, &run, &mut view, crate::theme::LIGHT);
            });
        }
        view.overview_order
    }

    #[test]
    fn the_section_lays_out_at_every_breakpoint_without_panicking() {
        for width in [320.0, 899.0, 900.0, 1499.0, 1500.0, 2560.0] {
            assert_eq!(
                lay_out_at(width),
                PanelId::ALL.to_vec(),
                "an untouched frame at {width} px must not rearrange anything"
            );
        }
    }

    #[test]
    fn dragging_one_handle_onto_another_card_takes_its_slot() {
        let run = test_run();
        let mut view = View::default();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1200.0));

        let frame = |events: Vec<egui::Event>, view: &mut View| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| show(ui, &run, view, crate::theme::LIGHT));
        };

        // One frame to lay out, then ask egui where the two handles ended up.
        frame(Vec::new(), &mut view);
        let at = |panel: PanelId| {
            ctx.read_response(handle_id(panel))
                .map(|r| r.rect.center())
                .expect("the handle was drawn this frame")
        };
        let (from, to) = (at(PanelId::RunSummary), at(PanelId::Fractions));

        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };

        frame(
            vec![egui::Event::PointerMoved(from), press(from, true)],
            &mut view,
        );
        // Several moves: egui only calls it a drag once the pointer has travelled
        // past its threshold, and the payload is set on the frame after that.
        for step in 1..=4 {
            let pos = from + (to - from) * (step as f32 / 4.0);
            frame(vec![egui::Event::PointerMoved(pos)], &mut view);
        }
        frame(
            vec![egui::Event::PointerMoved(to), press(to, false)],
            &mut view,
        );

        assert_eq!(
            view.overview_order,
            vec![
                PanelId::Warnings,
                PanelId::Channels,
                PanelId::Fractions,
                PanelId::RunSummary,
            ],
            "the run summary should have taken the fractions card's slot"
        );
        assert!(view.dirty, "a reorder is unsaved analysis state");
    }

    #[test]
    fn the_drag_handle_glyph_exists_in_the_font_stack_the_app_actually_ships() {
        // The Inter/JetBrains Mono files are not vendored, so the shipped app
        // falls back to egui's bundled faces — which is what a bare Context uses.
        // A glyph they do not cover renders as nothing, leaving an invisible
        // control, so this is worth asserting rather than eyeballing.
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        for font in [font_micro(), egui::FontId::proportional(14.0)] {
            assert!(
                ctx.fonts_mut(|f| f.has_glyphs(&font, HANDLE_GLYPH)),
                "the fallback stack cannot draw {HANDLE_GLYPH:?} at {font:?}"
            );
        }
    }
}

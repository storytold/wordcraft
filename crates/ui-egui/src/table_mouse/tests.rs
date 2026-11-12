use super::*;
use crate::table_resize::tests::{H, button, cell, screen, steps, table_window, table_window_stepping};

/// On the dark paper the column and cell arrows are drawn light, so they stay visible; on
/// white paper they stay black, and the hollow row arrow is the same on both.
#[test]
fn the_selection_arrow_is_light_on_dark_paper() {
    let dark_paper = Color32::from_gray(wordcraft_render::DARK_PAPER);
    assert_eq!(arrow_colors(Spot::Column(0), false), (Color32::BLACK, Color32::WHITE));
    assert_eq!(arrow_colors(Spot::Cell(0, 0), false), (Color32::BLACK, Color32::WHITE));
    assert_eq!(arrow_colors(Spot::Column(0), true), (Color32::WHITE, dark_paper));
    assert_eq!(arrow_colors(Spot::Cell(0, 0), true), (Color32::WHITE, dark_paper));
    assert_eq!(arrow_colors(Spot::Row(0), false), (Color32::WHITE, Color32::BLACK));
    assert_eq!(arrow_colors(Spot::Row(0), true), (Color32::WHITE, Color32::BLACK));
}

fn click(h: &mut H, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.step();
    button(h, at, true);
    h.step();
    button(h, at, false);
    steps(h, 2);
}

/// Above a column the pointer is the drawn selection arrow; a click selects the column's
/// cells (no undo step), and a click in a cell's text still places the caret.
#[test]
fn a_click_above_a_column_selects_it() {
    let mut h = table_window();
    let b = cell(&mut h, 0, 1);
    let depth = h.state().session.undo_depth();
    let at = screen(&mut h, b.x + b.w / 2.0, b.y - 3.0);
    h.event(egui::Event::PointerMoved(at));
    steps(&mut h, 2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "the drawn selection arrow");
    click(&mut h, at);
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (1, 1))));
    assert_eq!(h.state().session.undo_depth(), depth);
    assert!(!crate::mini_toolbar::visible(h.state()), "no mini toolbar over the table's selection areas");
    let c = cell(&mut h, 1, 2);
    let text = screen(&mut h, c.x + c.w / 2.0, c.y + c.h / 2.0);
    click(&mut h, text);
    assert!(h.state().session.cell_range().is_none());
    assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((1, 2)));
}

/// A drag down the row selection areas selects every row it passes, as it goes.
#[test]
fn dragging_down_the_row_areas_selects_rows() {
    let mut h = table_window();
    let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 1, 0));
    let start = screen(&mut h, a.x - 10.0, a.y + a.h / 2.0);
    let end = screen(&mut h, a.x - 10.0, b.y + b.h / 2.0);
    h.event(egui::Event::PointerMoved(start));
    h.step();
    button(&h, start, true);
    h.step();
    assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((0, 0)));
    h.event(egui::Event::PointerMoved(end));
    h.step();
    assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((0, 1)), "the selection follows the pointer");
    button(&h, end, false);
    steps(&mut h, 2);
    assert!(h.state().canvas.table_drag.is_none());
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 2))));
}

/// The selection bar beside a table (the page margin, well past the row areas) selects the
/// row; a drag down it selects the rows it passes; Shift+click extends by rows.
#[test]
fn the_margin_beside_a_table_selects_rows() {
    let mut h = table_window();
    let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 1, 0));
    let (ya, yb) = (a.y + a.h / 2.0, b.y + b.h / 2.0);
    let far = 20.0;
    assert!(a.x - far > 30.0, "well past the row areas");
    let at_a = screen(&mut h, far, ya);
    h.event(egui::Event::PointerMoved(at_a));
    steps(&mut h, 2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "the drawn selection arrow");
    click(&mut h, at_a);
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 0), (0, 2))), "one click, the row");
    // Drag from row 0 to row 1.
    let end = screen(&mut h, far, yb);
    h.event(egui::Event::PointerMoved(at_a));
    h.step();
    button(&h, at_a, true);
    h.step();
    h.event(egui::Event::PointerMoved(end));
    h.step();
    assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((0, 1)), "the selection follows the pointer");
    button(&h, end, false);
    steps(&mut h, 2);
    // Click row 0 again, then Shift+click row 1.
    click(&mut h, at_a);
    assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((0, 0)));
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    h.step();
    h.event(egui::Event::PointerMoved(end));
    h.step();
    button(&h, end, true);
    h.step();
    button(&h, end, false);
    steps(&mut h, 2);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::default()));
    h.step();
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 2))), "extended by rows");
}

/// Shift+click beside an upper row with the last row selected takes the rows between, and
/// the pointer moving a little before the release keeps them: the anchor row is the one at
/// the press, not re-read from the rows the press selected.
#[test]
fn a_shift_click_up_the_margin_keeps_its_anchor_while_held() {
    let mut h = table_window();
    let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 1, 0));
    let far = 20.0;
    let at_b = screen(&mut h, far, b.y + b.h / 2.0);
    click(&mut h, at_b);
    assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((1, 1)));
    let at = screen(&mut h, far, a.y + a.h / 2.0);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    h.step();
    h.event(egui::Event::PointerMoved(at));
    h.step();
    button(&h, at, true);
    h.step();
    assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((0, 1)), "extended up");
    for k in 1..=3 {
        h.event(egui::Event::PointerMoved(at + vec2(k as f32, 1.0)));
        h.step();
    }
    button(&h, at + vec2(3.0, 1.0), false);
    steps(&mut h, 2);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::default()));
    h.step();
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 2))), "the jitter kept the rows");
}

/// Word: a triple-click, or Ctrl/Cmd+click, in the margin beside a table row selects the whole
/// document; a single click (and a drag) keeps selecting rows.
#[test]
fn triple_click_or_ctrl_click_beside_a_row_selects_the_document() {
    let mut h = table_window_stepping(1.0 / 60.0);
    let b = cell(&mut h, 1, 0);
    let at = screen(&mut h, 20.0, b.y + b.h / 2.0);
    let whole = |h: &mut H| {
        let s = &h.state().session;
        // From the first cell's start to the end of the paragraph after the table.
        let (a, e) = s.sel.ordered();
        a.path == wordcraft_doc::Path(vec![0, 0, 0, 0]) && a.off == 0 && e == wordcraft_doc::Pos::body(1, 0)
    };
    // One click: the row.
    click(&mut h, at);
    assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((1, 1)), "a click selects the row");
    // Three quick clicks: everything.
    steps(&mut h, 40);
    for _ in 0..3 {
        h.event(egui::Event::PointerMoved(at));
        h.step();
        button(&h, at, true);
        h.step();
        button(&h, at, false);
        h.step();
    }
    steps(&mut h, 2);
    assert!(whole(&mut h), "triple-click: {:?}", h.state().session.sel);
    // Ctrl+click: everything too.
    steps(&mut h, 40);
    click(&mut h, at);
    assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((1, 1)));
    steps(&mut h, 40);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
    h.step();
    click(&mut h, at);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::default()));
    h.step();
    assert!(whole(&mut h), "ctrl+click: {:?}", h.state().session.sel);
}

/// Shift+click in the bar beside a row, with the selection started above the table, extends
/// that selection (the ordinary Shift+click into a table), instead of selecting only the row.
#[test]
fn shift_click_beside_a_row_extends_a_selection_from_above() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Above"})).unwrap();
    a.run("insert.table", json!({"rows": 3, "cols": 2})).unwrap();
    a.run("caret.set", json!({"pos": serde_json::to_value(wordcraft_doc::Pos::body(0, 0)).unwrap()})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    let b = cell(&mut h, 1, 0);
    let at = screen(&mut h, 20.0, b.y + b.h / 2.0);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    h.step();
    click(&mut h, at);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::default()));
    h.step();
    let sel = h.state().session.sel.clone();
    assert_eq!(sel.anchor, wordcraft_doc::Pos::body(0, 0), "the selection still starts above the table");
    assert!(!sel.is_collapsed());
    assert!(sel.focus.path.cell().is_some_and(|(_, r, _)| r == 1), "and runs into the clicked row: {:?}", sel.focus);
}

/// An ordinary drag from one cell's text into another's selects the rectangle of whole cells
/// between them as the pointer goes: no undo step, no mini toolbar over the cells.
#[test]
fn dragging_across_cells_selects_their_rectangle() {
    let mut h = table_window();
    let (a, d) = (cell(&mut h, 0, 1), cell(&mut h, 1, 2));
    let depth = h.state().session.undo_depth();
    let start = screen(&mut h, a.x + a.w / 2.0, a.y + a.h / 2.0);
    let end = screen(&mut h, d.x + d.w / 2.0, d.y + d.h / 2.0);
    h.event(egui::Event::PointerMoved(start));
    h.step();
    button(&h, start, true);
    h.step();
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 4.0)));
        h.step();
    }
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (1, 2))), "the selection follows the pointer");
    button(&h, end, false);
    steps(&mut h, 2);
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (1, 2))));
    assert_eq!(h.state().session.undo_depth(), depth);
    assert!(!crate::mini_toolbar::visible(h.state()));
}

/// The "+" left of a row border adds a row there and selects it, in one undo step; over it
/// the pointer is the plain arrow.
#[test]
fn the_plus_button_adds_a_row() {
    let mut h = table_window();
    let a = cell(&mut h, 0, 0);
    let depth = h.state().session.undo_depth();
    let at = screen(&mut h, a.x - 10.0, a.bottom());
    h.event(egui::Event::PointerMoved(at));
    steps(&mut h, 2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
    click(&mut h, at);
    let rows = h.state().session.doc.body.iter().find_map(|b| b.as_table()).map(|t| t.rows.len());
    assert_eq!(rows, Some(3));
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (0, 2))), "the new row is selected");
    assert_eq!(h.state().session.undo_depth(), depth + 1);
}

/// A double-click along a cell's left edge selects its row, as in Word.
#[test]
fn a_double_click_by_a_cells_edge_selects_its_row() {
    let mut h = table_window_stepping(1.0 / 60.0);
    let c = cell(&mut h, 1, 1);
    let at = screen(&mut h, c.x + 4.5, c.y + c.h / 2.0);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    button(&h, at, true);
    h.step();
    button(&h, at, false);
    h.step();
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (1, 1))), "a click selects the cell");
    button(&h, at, true);
    h.step();
    button(&h, at, false);
    steps(&mut h, 2);
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (0, 2))), "a double-click its row");
}

/// Dragging the move handle: the move arrows over it, nothing changes while dragging, then
/// the table floats where it's dropped, in one undo step.
#[test]
fn dragging_the_move_handle_moves_the_table() {
    let mut h = table_window();
    let a = cell(&mut h, 0, 0);
    let depth = h.state().session.undo_depth();
    // The handle's centre, past the top-left corner.
    let (hx, hy) = (a.x - 6.0, a.y - 6.0);
    let start = screen(&mut h, hx, hy);
    h.event(egui::Event::PointerMoved(start));
    steps(&mut h, 2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Move);
    button(&h, start, true);
    h.step();
    let end = screen(&mut h, hx + 40.0, hy + 30.0);
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
        h.step();
    }
    assert!(h.state().canvas.table_drag.is_some(), "a move is in progress");
    assert_eq!(cell(&mut h, 0, 0), a, "the table waits for the release");
    button(&h, end, false);
    steps(&mut h, 3);
    let moved = cell(&mut h, 0, 0);
    assert!((moved.x - (a.x + 40.0)).abs() < 1.0 && (moved.y - (a.y + 30.0)).abs() < 1.0, "{a:?} → {moved:?}");
    assert_eq!(h.state().session.undo_depth(), depth + 1);
}

/// #1738: dropping the move handle leaves the view where it is.
#[test]
fn dropping_the_move_handle_keeps_the_view_where_it_is() {
    let mut h = table_window();
    let a = cell(&mut h, 0, 0);
    let (hx, hy) = (a.x - 6.0, a.y - 6.0);
    let start = screen(&mut h, hx, hy);
    h.event(egui::Event::PointerMoved(start));
    steps(&mut h, 2);
    button(&h, start, true);
    h.step();
    let end = screen(&mut h, hx + 40.0, hy + 30.0);
    h.event(egui::Event::PointerMoved(end));
    steps(&mut h, 2);
    h.state_mut().canvas.scroll_to_caret = false;
    let depth = h.state().session.undo_depth();
    button(&h, end, false);
    h.step();
    assert_eq!(h.state().session.undo_depth(), depth + 1, "the table moved");
    assert!(!h.state().canvas.scroll_to_caret, "the move scrolled to the caret");
}

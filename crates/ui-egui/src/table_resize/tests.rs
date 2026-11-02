use super::*;

/// Screen ↔ page conversion survives a page list that doesn't match the layout.
#[test]
fn conversions_survive_missing_pages() {
    let layout = DocLayout::default();
    assert!(page_scale(&[], &layout, 1.0, 0).is_none());
    assert!(to_screen(&[], &layout, 1.0, 3, 1.0, 1.0).is_none());
    let rects = [Rect::from_min_size(pos2(10.0, 20.0), egui::vec2(100.0, 200.0))];
    assert_eq!(page_scale(&rects, &layout, 2.0, 0), Some(2.0));
    assert_eq!(to_screen(&rects, &layout, 2.0, 0, 5.0, 5.0), Some(pos2(20.0, 30.0)));
    assert!(page_scale(&rects, &layout, 0.0, 0).is_none());
}

pub(crate) type H = egui_kittest::Harness<'static, WordApp>;

/// The whole window with a 2×3 table whose first cell's text is selected.
pub(crate) fn table_window() -> H {
    table_window_stepping(0.25)
}

/// The same, each step `dt` seconds apart (short enough for a double-click: 1/60).
pub(crate) fn table_window_stepping(dt: f32) -> H {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("insert.table", json!({"rows": 2, "cols": 3})).unwrap();
    a.run("text.insert", json!({"text": "Product"})).unwrap();
    a.run("select.text", json!({"text": "Product"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(dt).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    h
}

/// Cell (`row`, `cell`)'s rect in page points.
pub(crate) fn cell(h: &mut H, row: usize, cell: usize) -> wordcraft_geom::Rect {
    h.state_mut().session.layout().pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            wordcraft_layout::Placed::Cell { rect, row: r, cell: c, .. } if *r == row && *c == cell => Some(*rect),
            _ => None,
        })
        .unwrap()
}

pub(crate) fn screen(h: &mut H, x: f32, y: f32) -> Pos2 {
    crate::canvas::page_to_screen(h.state_mut(), 0, x, y).unwrap()
}

pub(crate) fn button(h: &H, at: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
}

pub(crate) fn steps(h: &mut H, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

/// A real mouse drag of a column border: the resize arrow on hover, a preview and no change
/// while dragging, then one undo step on release, the selection untouched.
#[test]
fn dragging_a_column_border_with_the_mouse_is_one_undo_step() {
    let mut h = table_window();
    let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
    let sel = h.state().session.sel.clone();
    let depth = h.state().session.undo_depth();
    let start = screen(&mut h, a.right(), a.y + a.h / 2.0);
    h.event(egui::Event::PointerMoved(start));
    steps(&mut h, 2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::ResizeColumn, "the resize arrow over the border");
    button(&h, start, true);
    h.step();
    // 30 page points to the right, in a few moves.
    let end = screen(&mut h, a.right() + 30.0, a.y + a.h / 2.0);
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
        h.step();
    }
    assert!(h.state().canvas.table_resize.is_some(), "a drag is in progress");
    assert_eq!(h.state().session.undo_depth(), depth, "nothing is written while dragging");
    assert_eq!(cell(&mut h, 0, 0), a, "the table waits for the release");
    button(&h, end, false);
    steps(&mut h, 3);
    assert!(h.state().canvas.table_resize.is_none());
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the drag");
    let (a1, b1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
    assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
    assert!((b1.w - (b.w - 30.0)).abs() < 1.0, "{b:?} → {b1:?}");
    assert_eq!(h.state().session.sel, sel, "the selection is kept");
    assert!(h.state_mut().session.undo());
    assert_eq!(cell(&mut h, 0, 0), a);
}

/// #1738: releasing a column-border drag leaves the view where it is, not scrolled back to
/// the caret; typing afterwards still follows the caret.
#[test]
fn dragging_a_column_border_keeps_the_view_where_it_is() {
    let mut h = table_window();
    let a = cell(&mut h, 0, 0);
    let start = screen(&mut h, a.right(), a.y + a.h / 2.0);
    let end = screen(&mut h, a.right() + 30.0, a.y + a.h / 2.0);
    h.event(egui::Event::PointerMoved(start));
    steps(&mut h, 2);
    button(&h, start, true);
    h.step();
    h.event(egui::Event::PointerMoved(end));
    steps(&mut h, 2);
    h.state_mut().canvas.scroll_to_caret = false;
    let depth = h.state().session.undo_depth();
    button(&h, end, false);
    h.step();
    assert_eq!(h.state().session.undo_depth(), depth + 1, "the drag was applied");
    assert!(!h.state().canvas.scroll_to_caret, "the resize scrolled to the caret");
    h.state_mut().run("text.insert", json!({"text": "x"})).unwrap();
    assert!(h.state().canvas.scroll_to_caret, "typing still brings the caret into view");
}

/// The zone of the ruler marker named `label` nearest to screen point `near`.
fn ruler_marker(h: &H, label: &str, near: Pos2) -> Rect {
    use egui_kittest::kittest::Queryable;
    h.query_all_by_label(label).map(|n| n.rect()).min_by(|p, q| p.center().distance(near).total_cmp(&q.center().distance(near))).expect(label)
}

/// A real mouse drag from `from` to `to` (screen points), in a few moves.
fn drag(h: &mut H, from: Pos2, to: Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.step();
    button(h, from, true);
    h.step();
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 5.0)));
        h.step();
    }
    button(h, to, false);
    steps(h, 3);
}

/// #49: with the caret in the table, a column marker on the horizontal ruler drags its border,
/// in one undo step; only the column left of it changes and the columns right of it move
/// along, as on Word's ruler. The marker follows the drag.
#[test]
fn a_column_marker_on_the_ruler_moves_the_columns_right_of_it() {
    let mut h = table_window();
    let (a, b, c) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
    let depth = h.state().session.undo_depth();
    let near = screen(&mut h, a.right(), a.y);
    let start = ruler_marker(&h, "Table Column Width", near).center();
    assert!((start.x - near.x).abs() < 0.5, "a marker over the first column border");
    let end = start + egui::vec2(screen(&mut h, a.right() + 30.0, a.y).x - near.x, 0.0);
    h.event(egui::Event::PointerMoved(start));
    h.step();
    button(&h, start, true);
    h.step();
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
        h.step();
    }
    assert!((ruler_marker(&h, "Table Column Width", end).center().x - end.x).abs() < 1.0, "the marker follows the pointer");
    assert_eq!(h.state().session.undo_depth(), depth, "nothing is written while dragging");
    button(&h, end, false);
    steps(&mut h, 3);
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the drag");
    let (a1, b1, c1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
    assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
    assert!((b1.w - b.w).abs() < 0.5 && (c1.w - c.w).abs() < 0.5, "the columns right of it keep their widths");
    assert!((c1.right() - (c.right() + 30.0)).abs() < 1.0, "and move along, so the table is wider");
}

/// With Shift a column marker on the ruler works the other way round, as on Word's ruler: the
/// neighbour on the right makes up the difference, so the table keeps its width.
#[test]
fn a_shift_drag_on_the_ruler_trades_width_with_the_neighbour() {
    let mut h = table_window();
    let (a, b, c) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
    let near = screen(&mut h, a.right(), a.y);
    let start = ruler_marker(&h, "Table Column Width", near).center();
    let end = start + egui::vec2(screen(&mut h, a.right() + 30.0, a.y).x - near.x, 0.0);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    drag(&mut h, start, end);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    let (a1, b1, c1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
    assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
    assert!((b1.w - (b.w - 30.0)).abs() < 1.0, "{b:?} → {b1:?}");
    assert!((c1.w - c.w).abs() < 0.5 && (c1.right() - c.right()).abs() < 1.0, "the table keeps its width");
}

/// #49: a row marker on the vertical ruler drags that row's bottom border: the row gets taller,
/// in one undo step.
#[test]
fn a_row_marker_on_the_ruler_sets_the_row_height() {
    let mut h = table_window();
    let a = cell(&mut h, 0, 0);
    let depth = h.state().session.undo_depth();
    let near = screen(&mut h, a.x, a.bottom());
    let start = ruler_marker(&h, "Table Row Height", near).center();
    assert!((start.y - near.y).abs() < 0.5, "a marker at the first row's bottom");
    let end = start + egui::vec2(0.0, screen(&mut h, a.x, a.bottom() + 20.0).y - near.y);
    drag(&mut h, start, end);
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the drag");
    let a1 = cell(&mut h, 0, 0);
    assert!((a1.h - (a.h + 20.0)).abs() < 1.0, "{a:?} → {a1:?}");
}

/// #49: in a cell the ruler's indents are the cell's: the left-indent marker sits where the
/// cell's text starts (its box less its margins), and dragging it 18 points indents the
/// paragraph 18 points.
#[test]
fn in_a_cell_the_ruler_measures_indents_from_its_text() {
    let mut h = table_window();
    let b = cell(&mut h, 0, 1);
    // Into the second cell: its text starts far from the page's left margin.
    let inside = screen(&mut h, b.x + b.w / 2.0, b.y + b.h / 2.0);
    h.event(egui::Event::PointerMoved(inside));
    h.step();
    button(&h, inside, true);
    h.step();
    button(&h, inside, false);
    steps(&mut h, 3);
    assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, cl)| (r, cl)), Some((0, 1)));
    let layout = h.state_mut().session.layout();
    let (left, w) = crate::table_ruler::text_area(h.state(), &layout, 0).unwrap();
    assert!((left - (b.x + 5.4)).abs() < 0.01 && (w - (b.w - 10.8)).abs() < 0.01, "the cell less its margins: {left}, {w}");
    // The left-indent marker is at the text's start, just above the ruler's bottom edge.
    let near = screen(&mut h, left, b.y);
    let bar_bottom = ruler_marker(&h, "Table Column Width", near).bottom();
    let start = egui::pos2(near.x, bar_bottom - 3.0);
    let end = start + egui::vec2(screen(&mut h, left + 18.0, b.y).x - near.x, 0.0);
    drag(&mut h, start, end);
    let focus = h.state().session.sel.focus.clone();
    assert_eq!(h.state().session.doc.para_at(&focus).and_then(|p| p.props.indent_left), Some(18.0));
}

/// With Ctrl held, the change of a dragged column border is shared by the columns right of
/// it, so the table keeps its width.
#[test]
fn a_ctrl_drag_shares_the_change_among_the_columns_right_of_it() {
    let mut h = table_window();
    let (a, b, c) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
    let y = a.y + a.h / 2.0;
    let (start, end) = (screen(&mut h, a.right(), y), screen(&mut h, a.right() + 30.0, y));
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
    drag(&mut h, start, end);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    let (a1, b1, c1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
    assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
    assert!((b1.w - (b.w - 15.0)).abs() < 1.0 && (c1.w - (c.w - 15.0)).abs() < 1.0, "{b1:?} {c1:?}");
    assert!((c1.right() - c.right()).abs() < 1.0, "the table keeps its width");
}

/// The table's left edge, under the resize arrow just outside it, drags the table in; the
/// first column makes up the difference. The horizontal ruler has a marker on it too.
#[test]
fn the_left_edge_drags_the_table_in_and_has_a_ruler_marker() {
    let mut h = table_window();
    let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
    let y = a.y + a.h / 2.0;
    let (start, end) = (screen(&mut h, a.x - 1.5, y), screen(&mut h, a.x + 20.0, y));
    h.event(egui::Event::PointerMoved(start));
    steps(&mut h, 2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::ResizeColumn, "the resize arrow on the left edge");
    drag(&mut h, start, end);
    let (a1, b1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
    assert!((a1.x - (a.x + 21.5)).abs() < 1.0 && (a1.right() - a.right()).abs() < 0.5, "{a:?} → {a1:?}");
    assert!((b1.x - b.x).abs() < 0.5 && (b1.w - b.w).abs() < 0.5, "{b:?} → {b1:?}");
    let near = screen(&mut h, a1.x, a1.y);
    let marker = ruler_marker(&h, "Table Column Width", near).center();
    assert!((marker.x - near.x).abs() < 0.5, "a marker on the left edge");
    let back = marker - egui::vec2(near.x - screen(&mut h, a.x, a1.y).x, 0.0);
    drag(&mut h, marker, back);
    assert!((cell(&mut h, 0, 0).x - a.x).abs() < 1.0, "the marker drags it back out");
}

/// Alt while a column border is dragged shows each column's width on the ruler, as it is
/// going to be.
#[test]
fn alt_shows_the_column_widths_while_dragging() {
    let mut h = table_window();
    let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
    let y = a.y + a.h / 2.0;
    let (start, end) = (screen(&mut h, a.right(), y), screen(&mut h, a.right() + 18.0, y));
    h.event(egui::Event::PointerMoved(start));
    h.step();
    button(&h, start, true);
    h.step();
    h.event(egui::Event::PointerMoved(end));
    h.step();
    let widths = |h: &mut H| {
        let layout = h.state_mut().session.layout();
        crate::table_ruler::widths(h.state(), &h.ctx, &layout, 0)
    };
    assert!(widths(&mut h).is_empty(), "not without Alt");
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    h.step();
    let w = widths(&mut h);
    assert_eq!(w.len(), 3, "{w:?}");
    assert!((w[0].1 - (a.w + 18.0)).abs() < 0.5 && (w[1].1 - (b.w - 18.0)).abs() < 0.5, "{w:?}");
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    button(&h, end, false);
    steps(&mut h, 3);
    assert!(widths(&mut h).is_empty(), "gone with the drag");
}

/// A cell selected with a click along its left edge: dragging its right border with the
/// mouse moves it in that cell's row only (#962).
#[test]
fn a_selected_cells_border_drags_in_its_row_only() {
    let mut h = table_window();
    let (above, b, c) = (cell(&mut h, 0, 1), cell(&mut h, 1, 1), cell(&mut h, 1, 2));
    let y = b.y + b.h / 2.0;
    let at = screen(&mut h, b.x + 4.5, y);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    button(&h, at, true);
    h.step();
    button(&h, at, false);
    steps(&mut h, 2);
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (1, 1))), "the cell is selected");
    let (start, end) = (screen(&mut h, b.right(), y), screen(&mut h, b.right() + 30.0, y));
    drag(&mut h, start, end);
    let (b1, c1) = (cell(&mut h, 1, 1), cell(&mut h, 1, 2));
    assert!((b1.w - (b.w + 30.0)).abs() < 1.0 && (c1.w - (c.w - 30.0)).abs() < 1.0, "{b1:?} {c1:?}");
    assert!((cell(&mut h, 0, 1).right() - above.right()).abs() < 0.5, "the first row keeps its border");
}

/// Clicks inside a cell still place the caret (the border doesn't steal them), and Esc drops a
/// drag without changing anything.
#[test]
fn clicks_in_cells_still_move_the_caret_and_esc_cancels_a_drag() {
    let mut h = table_window();
    let c = cell(&mut h, 1, 1);
    let at = screen(&mut h, c.x + c.w / 2.0, c.y + c.h / 2.0);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    button(&h, at, true);
    h.step();
    button(&h, at, false);
    steps(&mut h, 3);
    let focus = &h.state().session.sel.focus;
    assert_eq!(focus.path.cell().map(|(_, r, cl)| (r, cl)), Some((1, 1)), "the caret went into the clicked cell");
    let table = h.state().session.doc.body.iter().find_map(|b| b.as_table()).cloned();
    let depth = h.state().session.undo_depth();
    let a = cell(&mut h, 0, 0);
    let start = screen(&mut h, a.x + a.w / 2.0, a.bottom());
    h.event(egui::Event::PointerMoved(start));
    h.step();
    button(&h, start, true);
    h.step();
    let end = start + egui::vec2(0.0, 40.0);
    h.event(egui::Event::PointerMoved(end));
    h.step();
    assert!(h.state().canvas.table_resize.is_some(), "the row border is being dragged");
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    h.step();
    button(&h, end, false);
    steps(&mut h, 3);
    assert_eq!(h.state().session.undo_depth(), depth, "Esc cancelled it");
    assert_eq!(h.state().session.doc.body.iter().find_map(|b| b.as_table()).cloned(), table);
}

/// A double-click on a column border fits the column left of it to its text, the next
/// column keeping its width and moving along (the table narrows, as in Word), in one undo
/// step.
#[test]
fn double_clicking_a_column_border_fits_the_column() {
    let mut h = table_window_stepping(1.0 / 60.0);
    let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
    let depth = h.state().session.undo_depth();
    let at = screen(&mut h, a.right(), a.y + a.h / 2.0);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for _ in 0..2 {
        button(&h, at, true);
        h.step();
        button(&h, at, false);
        h.step();
    }
    steps(&mut h, 2);
    let (a1, b1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
    assert!(a1.w < a.w - 40.0, "{a:?} → {a1:?}");
    assert!((b1.w - b.w).abs() < 1.0 && (b1.x - b.x - (a1.w - a.w)).abs() < 1.0, "the next column keeps its width and moves along");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step");
}

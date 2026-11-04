use super::*;
use serde_json::json;

fn first_table(s: &Session) -> Table {
    s.doc.body.iter().find_map(|b| b.as_table()).cloned().unwrap()
}

fn cell_rect(s: &mut Session, row: usize, cell: usize) -> Rect {
    s.layout().pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Cell { rect, row: r, cell: c, .. } if *r == row && *c == cell => Some(*rect),
            _ => None,
        })
        .unwrap()
}

fn table_session(rows: usize, cols: usize) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("insert.table", &json!({"rows": rows, "cols": cols})).unwrap();
    s
}

/// Dragging the border between two columns moves it; the right neighbour gives up the room,
/// so the table keeps its width; one undo puts it back.
#[test]
fn a_column_border_drag_trades_width_with_the_neighbour() {
    let mut s = table_session(2, 3);
    let start = first_table(&s);
    let a = cell_rect(&mut s, 0, 0);
    let (b0, total0) = (cell_rect(&mut s, 0, 1), cell_rect(&mut s, 0, 2).right() - a.x);
    let y = a.y + a.h / 2.0;
    s.run("table.resizeDrag", &json!({"page": 0, "x": a.right() + 1.0, "y": y, "toX": a.right() + 31.0, "toY": y})).unwrap();
    let (a1, b1) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
    assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
    assert!((b1.w - (b0.w - 30.0)).abs() < 1.0, "{b0:?} → {b1:?}");
    let total1 = cell_rect(&mut s, 0, 2).right() - a1.x;
    assert!((total1 - total0).abs() < 1.0, "the table keeps its width");
    // The second row follows: one border for the whole column.
    assert!((cell_rect(&mut s, 1, 0).w - a1.w).abs() < 0.5);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(first_table(&s), start, "one undo step");
}

/// With Shift only the left column changes and the table grows; the right edge does the same.
#[test]
fn shift_and_the_right_edge_change_the_table_width() {
    let mut s = table_session(1, 2);
    let (a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
    let y = a.y + a.h / 2.0;
    s.run("table.resizeDrag", &json!({"page": 0, "x": a.right(), "y": y, "toX": a.right() - 40.0, "toY": y, "shift": true})).unwrap();
    let (a1, b1) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
    assert!((a1.w - (a.w - 40.0)).abs() < 1.0 && (b1.w - b.w).abs() < 1.0, "{a1:?} {b1:?}");
    s.run("table.resizeDrag", &json!({"page": 0, "x": b1.right() - 1.0, "y": y, "toX": b1.right() - 51.0, "toY": y})).unwrap();
    let b2 = cell_rect(&mut s, 0, 1);
    assert!((b2.w - (b1.w - 50.0)).abs() < 1.0, "{b1:?} → {b2:?}");
}

/// No column gets narrower than the minimum, however far the drag goes.
#[test]
fn column_drags_stop_at_the_minimum_width() {
    let mut s = table_session(1, 2);
    let (a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
    let y = a.y + a.h / 2.0;
    s.run("table.resizeDrag", &json!({"page": 0, "x": a.right(), "y": y, "toX": a.right() + 5000.0, "toY": y})).unwrap();
    let t = first_table(&s);
    assert!((t.grid[1] - MIN_COL).abs() < 0.01, "{:?}", t.grid);
    assert!((t.grid[0] + t.grid[1] - (a.w + b.w)).abs() < 1.0);
}

/// Dragging a row's bottom border sets its height ("at least"), and the row grows.
#[test]
fn a_row_border_drag_sets_the_row_height() {
    let mut s = table_session(2, 2);
    let a = cell_rect(&mut s, 0, 0);
    let x = a.x + a.w / 2.0;
    s.run("table.resizeDrag", &json!({"page": 0, "x": x, "y": a.bottom() + 1.0, "toX": x, "toY": a.bottom() + 41.0})).unwrap();
    let t = first_table(&s);
    assert_eq!(t.rows[0].props.height_rule, HeightRule::AtLeast);
    // The stored height leaves out the margins and borders; the drawn row is what was dragged.
    assert!(t.rows[0].props.height.unwrap() < a.h + 40.0, "{:?}", t.rows[0].props);
    let h = cell_rect(&mut s, 0, 0).h;
    assert!((h - (a.h + 40.0)).abs() < 0.1, "{} → {h}", a.h);
    assert_eq!(t.rows[1].props.height, None, "the other row is untouched");
}

/// An exact row stays exact when its border is dragged: the stored height leaves out the
/// 4pt bottom margin the layout adds below it, and the drawn row is what was dragged.
#[test]
fn an_exact_row_border_drag_leaves_out_the_bottom_margin() {
    let mut s = table_session(2, 2);
    let t = s.doc.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
        wordcraft_doc::Block::Table(t) => Some(t),
        _ => None,
    });
    let t = t.unwrap();
    t.props.cell_margins = Some([0.0, 5.4, 4.0, 5.4]);
    t.rows[0].props.height = Some(30.0);
    t.rows[0].props.height_rule = HeightRule::Exact;
    s.touch();
    let a = cell_rect(&mut s, 0, 0);
    assert!((a.h - 34.0).abs() < 0.1, "{a:?}");
    let x = a.x + a.w / 2.0;
    s.run("table.resizeDrag", &json!({"page": 0, "x": x, "y": a.bottom(), "toX": x, "toY": a.bottom() + 20.0})).unwrap();
    let t = first_table(&s);
    assert_eq!(t.rows[0].props.height_rule, HeightRule::Exact);
    let stored = t.rows[0].props.height.unwrap();
    assert!((stored - 50.0).abs() < 0.1, "stored {stored}");
    let h = cell_rect(&mut s, 0, 0).h;
    assert!((h - 54.0).abs() < 0.1, "drawn {h}");
}

/// The handle past the bottom-right corner scales every column and row.
#[test]
fn the_corner_handle_scales_the_table() {
    let mut s = table_session(2, 2);
    let (a, d) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 1, 1));
    let (w, h) = (d.right() - a.x, d.bottom() - a.y);
    let (hx, hy) = (d.right() + 1.0 + HANDLE / 2.0, d.bottom() + 1.0 + HANDLE / 2.0);
    let l = s.layout();
    let tgt = resize_target(&l, &s.doc, 0, hx, hy, REACH).unwrap();
    assert_eq!(tgt.edge, Edge::Corner);
    // Half as wide, twice as tall: the handle moves by that much.
    s.run("table.resizeDrag", &json!({"page": 0, "x": hx, "y": hy, "toX": hx - w * 0.5, "toY": hy + h})).unwrap();
    let (a1, d1) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 1, 1));
    assert!(((d1.right() - a1.x) - w * 0.5).abs() < 1.5, "{w} → {}", d1.right() - a1.x);
    assert!(((d1.bottom() - a1.y) - h * 2.0).abs() < 0.5, "{h} → {}", d1.bottom() - a1.y);
}

/// A merged cell hides the border it covers: no column border to grab in its middle.
#[test]
fn merged_cells_have_no_border_inside() {
    let mut s = table_session(2, 2);
    let a = cell_rect(&mut s, 0, 0);
    s.run("table.eraser", &json!({"page": 0, "x": a.right(), "y": a.y + a.h / 2.0})).unwrap();
    let l = s.layout();
    let m = cell_rect(&mut s, 0, 0);
    let mid = resize_target(&l, &s.doc, 0, a.right(), m.y + m.h / 2.0, REACH);
    assert!(mid.as_ref().is_none_or(|t| t.edge != Edge::Column(1)), "{mid:?}");
    // The second row still has it.
    let b = cell_rect(&mut s, 1, 0);
    let t = resize_target(&l, &s.doc, 0, b.right(), b.y + b.h / 2.0, REACH).unwrap();
    assert_eq!(t.edge, Edge::Column(1));
}

/// A vertical merge lays out as one box as tall as its rows (#640); each row keeps its own
/// bottom border, and dragging one changes that row alone.
#[test]
fn a_vertical_merge_keeps_each_rows_border() {
    let mut s = table_session(3, 2);
    let (m, a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1), cell_rect(&mut s, 1, 1));
    s.run("table.eraser", &json!({"page": 0, "x": m.x + m.w / 2.0, "y": m.bottom() + 0.5})).unwrap();
    assert_eq!(first_table(&s).rows[1].cells[0].props.vmerge, wordcraft_doc::props::VMerge::Continue);
    let l = s.layout();
    let x = b.x + b.w / 2.0;
    let t = resize_target(&l, &s.doc, 0, x, a.bottom() + 1.0, REACH).unwrap();
    assert_eq!(t.edge, Edge::Row(0), "the border between the merged rows is still there");
    let t = resize_target(&l, &s.doc, 0, x, b.bottom() + 1.0, REACH).unwrap();
    assert_eq!(t.edge, Edge::Row(1), "the merge's bottom is the second row's border");
    s.run("table.resizeDrag", &json!({"page": 0, "x": x, "y": a.bottom() + 1.0, "toX": x, "toY": a.bottom() + 21.0})).unwrap();
    let t = first_table(&s);
    let h = cell_rect(&mut s, 0, 1).h;
    assert!((h - (a.h + 20.0)).abs() < 0.1, "{} → {h}", a.h);
    assert_eq!(t.rows[1].props.height, None, "the other rows are untouched");
    assert!((cell_rect(&mut s, 1, 1).h - b.h).abs() < 0.1);
}

/// Away from any border, or with hostile numbers: an error and no change, never a panic.
#[test]
fn misses_and_hostile_params_change_nothing() {
    let mut s = table_session(2, 2);
    let start = first_table(&s);
    let c = cell_rect(&mut s, 0, 0);
    let mid = json!({"page": 0, "x": c.x + c.w / 2.0, "y": c.y + c.h / 2.0, "toX": 10, "toY": 10});
    assert!(s.run("table.resizeDrag", &mid).is_err());
    assert!(s.run("table.resizeDrag", &json!({"page": 0, "x": 1e30, "y": -1e30, "toX": 1e30, "toY": 1e30})).is_err());
    assert!(s.run("table.resizeDrag", &json!({"page": 99, "x": 1, "y": 1, "toX": 1, "toY": 1})).is_err());
    assert!(s.run("table.resizeDrag", &json!({})).is_err());
    let y = c.y + c.h / 2.0;
    s.run("table.resizeDrag", &json!({"page": 0, "x": c.right(), "y": y, "toX": f32::MAX, "toY": y, "reach": 1e9})).unwrap();
    assert!(first_table(&s).grid.iter().all(|w| w.is_finite() && *w >= MIN_COL));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(first_table(&s), start);
}

/// With Ctrl the columns right of the border share the change in proportion to their
/// widths, so the table keeps its width (Ctrl wins over Shift); none gets narrower than the
/// minimum.
#[test]
fn ctrl_shares_a_border_drag_among_the_columns_right_of_it() {
    let mut s = table_session(1, 4);
    let (a, b, d) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1), cell_rect(&mut s, 0, 3));
    let y = a.y + a.h / 2.0;
    s.run("table.resizeDrag", &json!({"page": 0, "x": a.right(), "y": y, "toX": a.right() + 30.0, "toY": y, "ctrl": true, "shift": true})).unwrap();
    let a1 = cell_rect(&mut s, 0, 0);
    assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
    for c in 1..4 {
        let w = cell_rect(&mut s, 0, c).w;
        assert!((w - (b.w - 10.0)).abs() < 1.0, "column {c}: {} → {w}", b.w);
    }
    assert!((cell_rect(&mut s, 0, 3).right() - d.right()).abs() < 1.0, "the table keeps its width");
    // In proportion: 40 and 80 give up 10 and 20 of a 30-point move.
    let mut w = [100.0, 40.0, 80.0];
    assert_eq!(share_out(&mut w, 0, 30.0, Share::Proportional), Some(30.0));
    assert!((w[1] - 30.0).abs() < 0.01 && (w[2] - 60.0).abs() < 0.01, "{w:?}");
    // As far as the narrowest stays at the minimum.
    let mut w = [100.0, 12.0, 120.0];
    share_out(&mut w, 0, 500.0, Share::Proportional);
    assert!((w[1] - MIN_COL).abs() < 0.01 && (w.iter().sum::<f32>() - 232.0).abs() < 0.01, "{w:?}");
    assert_eq!(share_out(&mut w, 3, 1.0, Share::Proportional), None);
}

/// The table's left edge, grabbed from just outside it, moves the table in or out: its
/// indent changes and the first column makes up the difference, so the other borders stay.
/// A centred or right-aligned table keeps its alignment, as in Word: only the first column
/// changes, and the table stays centred or on its right edge.
#[test]
fn the_left_edge_moves_the_indent_and_the_first_column() {
    let mut s = table_session(2, 3);
    let start = first_table(&s);
    let (a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
    let y = a.y + a.h / 2.0;
    let l = s.layout();
    assert_eq!(resize_target(&l, &s.doc, 0, a.x - 2.0, y, REACH).map(|t| t.edge), Some(Edge::Column(0)));
    assert!(resize_target(&l, &s.doc, 0, a.x + 2.5, y, REACH).is_none(), "inside the first cell");
    s.run("table.resizeDrag", &json!({"page": 0, "x": a.x - 1.0, "y": y, "toX": a.x + 35.0, "toY": y})).unwrap();
    let (a1, b1) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
    assert!((a1.x - (a.x + 36.0)).abs() < 0.5 && (a1.right() - a.right()).abs() < 0.5, "{a:?} → {a1:?}");
    assert!((b1.x - b.x).abs() < 0.5 && (b1.w - b.w).abs() < 0.5, "the other columns stay: {b:?} → {b1:?}");
    assert!((cell_rect(&mut s, 1, 0).x - a1.x).abs() < 0.01, "the whole table moved in");
    assert!((first_table(&s).props.indent.unwrap() - 36.0).abs() < 0.5, "{:?}", first_table(&s).props.indent);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(first_table(&s), start, "one undo step");
    s.run("table.properties", &json!({"align": "center", "width": 300})).unwrap();
    let (c, e) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 2));
    let y = c.y + c.h / 2.0;
    s.run("table.resizeDrag", &json!({"page": 0, "x": c.x, "y": y, "toX": c.x - 20.0, "toY": y})).unwrap();
    let (c1, e1) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 2));
    assert!((c1.w - (c.w + 20.0)).abs() < 0.5, "the first column takes the drag: {c:?} → {c1:?}");
    assert!((e1.w - e.w).abs() < 0.5, "the other columns keep their widths");
    assert!(((c1.x + e1.right()) - (c.x + e.right())).abs() < 1.0, "the table stays centred: {c:?} {e:?} → {c1:?} {e1:?}");
    assert_eq!(first_table(&s).props.align, Some(Align::Center));
    s.run("edit.undo", &json!({})).unwrap();
    s.run("table.properties", &json!({"align": "right"})).unwrap();
    let (c, e) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 2));
    s.run("table.resizeDrag", &json!({"page": 0, "x": c.x, "y": y, "toX": c.x - 20.0, "toY": y})).unwrap();
    let (c1, e1) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 2));
    assert!((c1.x - (c.x - 20.0)).abs() < 0.5 && (e1.right() - e.right()).abs() < 0.5, "only the left edge moves: {c:?} → {c1:?}");
    assert_eq!(first_table(&s).props.align, Some(Align::Right));
}

/// With the selection across cells, a border of theirs moves only in their rows: the grid
/// gets a line for those rows' border and the other rows keep theirs. Text within one cell
/// isn't a cell selection; a border grabbed in a row outside the selection moves in the
/// whole column.
#[test]
fn with_cells_selected_their_border_moves_only_in_their_rows() {
    let mut s = table_session(3, 3);
    let (b, d) = (cell_rect(&mut s, 1, 1), cell_rect(&mut s, 2, 1));
    s.run("caret.set", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": b.y + b.h / 2.0})).unwrap();
    s.run("text.insert", &json!({"text": "Wide"})).unwrap();
    let start = first_table(&s);
    let (b, c) = (cell_rect(&mut s, 1, 1), cell_rect(&mut s, 1, 2));
    let y = b.y + b.h / 2.0;
    let drag = json!({"page": 0, "x": b.right(), "y": y, "toX": b.right() + 30.0, "toY": y});
    s.run("select.text", &json!({"text": "Wide"})).unwrap();
    s.run("table.resizeDrag", &drag).unwrap();
    assert!((cell_rect(&mut s, 0, 1).w - (b.w + 30.0)).abs() < 1.0, "text in one cell: every row");
    s.run("edit.undo", &json!({})).unwrap();
    // From the second row's cell down into the third's.
    s.run("caret.set", &json!({"page": 0, "x": d.x + d.w / 2.0, "y": d.y + d.h / 2.0, "extend": true})).unwrap();
    s.run("table.resizeDrag", &drag).unwrap();
    for r in [1, 2] {
        let (b1, c1) = (cell_rect(&mut s, r, 1), cell_rect(&mut s, r, 2));
        assert!((b1.w - (b.w + 30.0)).abs() < 1.0 && (c1.w - (c.w - 30.0)).abs() < 1.0, "row {r}: {b1:?} {c1:?}");
    }
    assert!((cell_rect(&mut s, 0, 1).right() - b.right()).abs() < 0.5, "the first row keeps its border");
    let t = first_table(&s);
    assert_eq!(t.grid.len(), 4, "{:?}", t.grid);
    assert_eq!(t.rows[0].cells.iter().map(Cell::span).collect::<Vec<_>>(), [1, 1, 2]);
    assert_eq!(t.rows[2].cells.iter().map(Cell::span).collect::<Vec<_>>(), [1, 2, 1]);
    let a = cell_rect(&mut s, 0, 0);
    let ya = a.y + a.h / 2.0;
    s.run("table.resizeDrag", &json!({"page": 0, "x": a.right(), "y": ya, "toX": a.right() - 10.0, "toY": ya})).unwrap();
    assert!((cell_rect(&mut s, 1, 0).w - (a.w - 10.0)).abs() < 1.0, "a row outside the selection: the whole column");
    s.run("edit.undo", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(first_table(&s), start, "one undo step each");
}

/// One selected cell (Select Cell, unlike its text) is a cell selection too: its border
/// moves in its row only (#962).
#[test]
fn a_selected_cells_border_moves_only_in_its_row() {
    let mut s = table_session(3, 3);
    let (b, c) = (cell_rect(&mut s, 1, 1), cell_rect(&mut s, 1, 2));
    let y = b.y + b.h / 2.0;
    s.run("caret.set", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": y})).unwrap();
    s.run("table.selectCell", &json!({})).unwrap();
    assert!(s.cell_range().is_some());
    let start = first_table(&s);
    s.run("table.resizeDrag", &json!({"page": 0, "x": b.right(), "y": y, "toX": b.right() + 30.0, "toY": y})).unwrap();
    let (b1, c1) = (cell_rect(&mut s, 1, 1), cell_rect(&mut s, 1, 2));
    assert!((b1.w - (b.w + 30.0)).abs() < 1.0 && (c1.w - (c.w - 30.0)).abs() < 1.0, "{b1:?} {c1:?}");
    for r in [0, 2] {
        assert!((cell_rect(&mut s, r, 1).right() - b.right()).abs() < 0.5, "row {r} keeps its border");
    }
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(first_table(&s), start, "one undo step");
}

/// A cell merged down past the selection keeps one width: a border of it moves in the whole
/// column.
#[test]
fn a_border_of_a_cell_merged_past_the_selection_moves_in_every_row() {
    let mut s = table_session(3, 3);
    let (a, b) = (cell_rect(&mut s, 0, 1), cell_rect(&mut s, 1, 1));
    s.run("caret.set", &json!({"page": 0, "x": a.x + a.w / 2.0, "y": a.y + a.h / 2.0})).unwrap();
    s.run("caret.set", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": b.y + b.h / 2.0, "extend": true})).unwrap();
    s.run("table.merge", &json!({})).unwrap();
    let (c, e) = (cell_rect(&mut s, 1, 2), cell_rect(&mut s, 2, 2));
    let y = c.y + c.h / 2.0;
    s.run("caret.set", &json!({"page": 0, "x": c.x + c.w / 2.0, "y": y})).unwrap();
    s.run("caret.set", &json!({"page": 0, "x": e.x + e.w / 2.0, "y": e.y + e.h / 2.0, "extend": true})).unwrap();
    s.run("table.resizeDrag", &json!({"page": 0, "x": c.x, "y": y, "toX": c.x + 20.0, "toY": y})).unwrap();
    assert_eq!(first_table(&s).grid.len(), 3, "no new grid line");
    assert!((cell_rect(&mut s, 0, 2).x - (c.x + 20.0)).abs() < 1.0, "the first row follows");
}

/// Selects the cell at (row, cell) one of the ways Word does: Select Cell, the left-edge
/// arrow, or a triple-click on its text.
fn select_one_cell(s: &mut Session, how: &str, r: usize, c: usize) {
    let b = cell_rect(s, r, c);
    let y = b.y + b.h / 2.0;
    match how {
        "arrow" => {
            s.run("table.selectAt", &json!({"page": 0, "x": b.x + 4.5, "y": y})).unwrap();
        }
        "triple" => {
            s.run("caret.set", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": y})).unwrap();
            s.run("select.paragraph", &json!({"cell": true})).unwrap();
        }
        _ => {
            s.run("caret.set", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": y})).unwrap();
            s.run("table.selectCell", &json!({})).unwrap();
        }
    }
}

/// Each way of selecting one cell makes its right and its left border move in its row
/// only; the neighbour on that side gives up or gains the room and the row keeps its width.
#[test]
fn a_single_selected_cells_borders_move_only_in_its_row() {
    for how in ["cell", "arrow", "triple"] {
        for left in [false, true] {
            let mut s = table_session(3, 3);
            let (a, b, c) = (cell_rect(&mut s, 1, 0), cell_rect(&mut s, 1, 1), cell_rect(&mut s, 1, 2));
            let y = b.y + b.h / 2.0;
            select_one_cell(&mut s, how, 1, 1);
            let start = first_table(&s);
            let x = if left { b.x } else { b.right() };
            s.run("table.resizeDrag", &json!({"page": 0, "x": x, "y": y, "toX": x + 30.0, "toY": y})).unwrap();
            let (a1, b1, c1) = (cell_rect(&mut s, 1, 0), cell_rect(&mut s, 1, 1), cell_rect(&mut s, 1, 2));
            if left {
                assert!((b1.w - (b.w - 30.0)).abs() < 1.0 && (a1.w - (a.w + 30.0)).abs() < 1.0, "{how} left: {a1:?} {b1:?}");
                assert!((c1.w - c.w).abs() < 0.5, "{how} left");
            } else {
                assert!((b1.w - (b.w + 30.0)).abs() < 1.0 && (c1.w - (c.w - 30.0)).abs() < 1.0, "{how} right: {b1:?} {c1:?}");
                assert!((a1.w - a.w).abs() < 0.5, "{how} right");
            }
            assert!((c1.right() - c.right()).abs() < 0.5, "{how}: the row keeps its width");
            for r in [0, 2] {
                assert!(
                    (cell_rect(&mut s, r, 1).x - b.x).abs() < 0.5 && (cell_rect(&mut s, r, 1).right() - b.right()).abs() < 0.5,
                    "{how}: row {r} unchanged"
                );
            }
            s.run("edit.undo", &json!({})).unwrap();
            assert_eq!(first_table(&s), start, "{how}: one undo step");
        }
    }
}

/// A caret, a part of a cell's text, or another cell's border with a cell selected: the
/// whole column moves.
#[test]
fn without_that_cell_selected_the_whole_column_moves() {
    let mut s = table_session(3, 3);
    let b = cell_rect(&mut s, 1, 1);
    let y = b.y + b.h / 2.0;
    s.run("caret.set", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": y})).unwrap();
    s.run("table.resizeDrag", &json!({"page": 0, "x": b.right(), "y": y, "toX": b.right() + 30.0, "toY": y})).unwrap();
    assert!((cell_rect(&mut s, 0, 1).w - (b.w + 30.0)).abs() < 1.0, "caret only");
    s.run("edit.undo", &json!({})).unwrap();
    select_one_cell(&mut s, "arrow", 2, 2);
    s.run("table.resizeDrag", &json!({"page": 0, "x": b.right(), "y": y, "toX": b.right() + 30.0, "toY": y})).unwrap();
    assert!((cell_rect(&mut s, 0, 1).w - (b.w + 30.0)).abs() < 1.0, "another cell selected");
    s.run("edit.undo", &json!({})).unwrap();
    // All of a cell's text, picked as a range (no cell mark): not a cell selection.
    s.run("caret.set", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": y})).unwrap();
    s.run("text.insert", &json!({"text": "Wide"})).unwrap();
    s.run("select.text", &json!({"text": "Wide"})).unwrap();
    let (anchor, focus) = (serde_json::to_value(&s.sel.anchor).unwrap(), serde_json::to_value(&s.sel.focus).unwrap());
    s.run("select.range", &json!({"anchor": anchor, "focus": focus})).unwrap();
    assert!(s.cell_range().is_none());
    let b = cell_rect(&mut s, 1, 1);
    s.run("table.resizeDrag", &json!({"page": 0, "x": b.right(), "y": y, "toX": b.right() + 30.0, "toY": y})).unwrap();
    assert!((cell_rect(&mut s, 0, 1).w - (b.w + 30.0)).abs() < 1.0, "all of a cell's text selected");
}

/// The jagged row a single cell's drag leaves is saved to .docx and read back the same.
#[test]
fn a_jagged_row_survives_a_docx_round_trip() {
    let mut s = table_session(3, 3);
    let b = cell_rect(&mut s, 1, 1);
    let y = b.y + b.h / 2.0;
    select_one_cell(&mut s, "arrow", 1, 1);
    s.run("table.resizeDrag", &json!({"page": 0, "x": b.right(), "y": y, "toX": b.right() + 30.0, "toY": y})).unwrap();
    let t = first_table(&s);
    let bytes = crate::io::save_bytes("jagged.docx", &s.doc).unwrap();
    let doc = crate::io::open_bytes("jagged.docx", &bytes).unwrap();
    let back = doc.body.iter().find_map(|b| b.as_table()).cloned().unwrap();
    let spans = |t: &Table| t.rows.iter().map(|r| r.cells.iter().map(Cell::span).collect::<Vec<_>>()).collect::<Vec<_>>();
    assert_eq!(spans(&back), spans(&t));
    let mut s2 = Session::new(doc);
    for c in 0..spans(&t)[1].len() {
        assert!((cell_rect(&mut s2, 1, c).w - cell_rect(&mut s, 1, c).w).abs() < 1.0, "cell {c}");
    }
}

/// In a cell of two paragraphs, a triple-click on the last one selects the cell (its border
/// then moves in its row only); on the first one it selects text, and the whole column moves.
#[test]
fn triple_click_selects_a_cell_only_on_its_last_paragraph() {
    for last in [true, false] {
        let mut s = table_session(3, 3);
        let b = cell_rect(&mut s, 1, 1);
        s.run("caret.set", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": b.y + b.h / 2.0})).unwrap();
        s.run("text.insert", &json!({"text": "One"})).unwrap();
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "Two"})).unwrap();
        s.run("select.text", &json!({"text": if last { "Two" } else { "One" }})).unwrap();
        s.run("select.paragraph", &json!({})).unwrap();
        assert!(s.cell_range().is_none(), "the ribbon's Select Paragraph never selects the cell");
        s.run("select.text", &json!({"text": if last { "Two" } else { "One" }})).unwrap();
        s.run("select.paragraph", &json!({"cell": true})).unwrap();
        assert_eq!(s.cell_range().is_some(), last);
        let b = cell_rect(&mut s, 1, 1);
        let y = b.y + b.h / 2.0;
        s.run("table.resizeDrag", &json!({"page": 0, "x": b.right(), "y": y, "toX": b.right() + 30.0, "toY": y})).unwrap();
        let moved = (cell_rect(&mut s, 0, 1).w - (b.w + 30.0)).abs() < 1.0;
        assert_eq!(moved, !last, "last paragraph {last}: the column moves only without a cell selection");
    }
}

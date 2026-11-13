use super::*;
use serde_json::json;
use wordcraft_doc::props::Align;

fn table_session(rows: usize, cols: usize) -> Session {
    let mut s = Session::new(Document::new());
    s.run("insert.table", &json!({"rows": rows, "cols": cols})).unwrap();
    s
}

fn table_path(s: &Session) -> Path {
    Path(vec![s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap() as u32])
}

fn first_table(s: &Session) -> Table {
    s.doc.body.iter().find_map(|b| b.as_table()).cloned().unwrap()
}

fn cell_pos(s: &Session, r: usize, c: usize) -> Pos {
    let mut path = table_path(s).0;
    path.extend([r as u32, c as u32, 0]);
    Pos { story: StoryRef::Body, path: Path(path), off: 0 }
}

/// Type "r{row}c{cell}" into every cell.
fn fill(s: &mut Session) {
    let t = first_table(s);
    for (r, row) in t.rows.iter().enumerate() {
        for c in 0..row.cells.len() {
            s.sel = Selection::caret(cell_pos(s, r, c));
            s.run("text.insert", &json!({"text": format!("r{r}c{c}")})).unwrap();
        }
    }
}

fn para(s: &Session, r: usize, c: usize) -> wordcraft_doc::Paragraph {
    s.doc.para(StoryRef::Body, &cell_pos(s, r, c).path).cloned().unwrap()
}

fn bold(s: &Session, r: usize, c: usize) -> bool {
    let p = para(s, r, c);
    if p.is_empty() { p.mark.bold == Some(true) } else { p.run_ranges().all(|(_, ch)| ch.bold == Some(true)) }
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

fn spot_at(s: &mut Session, x: f32, y: f32) -> Option<Spot> {
    let l = s.layout();
    target_at(&l, &s.doc, StoryRef::Body, 0, x, y, REACH).map(|t| t.spot)
}

fn mid_y(r: Rect) -> f32 {
    r.y + r.h / 2.0
}

/// The areas around and in a table: column, row and cell selection areas, the insert buttons
/// and the move handle; borders stay the resize module's, the text the caret's.
#[test]
fn the_areas_of_a_table_are_found() {
    let mut s = table_session(3, 3);
    let (a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
    let (left, top) = (a.x, a.y);
    assert_eq!(spot_at(&mut s, b.x + b.w / 2.0, top - 3.0), Some(Spot::Column(1)));
    assert_eq!(spot_at(&mut s, b.x + b.w / 2.0, top + 1.0), Some(Spot::Column(1)));
    let r1 = cell_rect(&mut s, 1, 0);
    assert_eq!(spot_at(&mut s, left - 10.0, mid_y(r1)), Some(Spot::Row(1)));
    let c = cell_rect(&mut s, 1, 2);
    assert_eq!(spot_at(&mut s, c.x + 4.5, mid_y(c)), Some(Spot::Cell(1, 2)));
    assert_eq!(spot_at(&mut s, r1.x + 2.0, mid_y(r1)), Some(Spot::Cell(1, 0)), "the first column's edge has no resize border");
    assert_eq!(spot_at(&mut s, c.x + 20.0, mid_y(c)), None, "the text is the caret's");
    assert_eq!(spot_at(&mut s, b.x, mid_y(a)), None, "a column border resizes");
    assert_eq!(spot_at(&mut s, b.x, top - 8.0), Some(Spot::InsertColumn(1)));
    assert_eq!(spot_at(&mut s, left - 10.0, a.bottom()), Some(Spot::InsertRow(1)));
    assert_eq!(spot_at(&mut s, left - 10.0, top), Some(Spot::InsertRow(0)));
    let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
    assert_eq!(spot_at(&mut s, h.x + h.w / 2.0, h.y + h.h / 2.0), Some(Spot::Handle));
    // Another story's tables aren't in reach.
    let l = s.layout();
    assert!(target_at(&l, &s.doc, StoryRef::Part(7), 0, b.x + b.w / 2.0, top - 3.0, REACH).is_none());
}

/// A selected column's text is its own cells', so the status bar and Word Count count only
/// them, not the cells between its ends in the text.
#[test]
fn a_selected_columns_text_is_its_cells() {
    let mut s = table_session(3, 3);
    fill(&mut s);
    let b = cell_rect(&mut s, 0, 1);
    s.run("table.selectAt", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": b.y - 3.0})).unwrap();
    assert_eq!(s.selected_text(), "r0c1\nr1c1\nr2c1");
    assert_eq!(s.run("review.wordCount", &json!({})).unwrap()["words"], 3);
}

/// A click above a column selects just its cells: formatting, alignment and Delete leave the
/// cells between its ends alone, each one undo step; table commands take the column.
#[test]
fn a_selected_column_is_edited_cell_by_cell() {
    let mut s = table_session(3, 3);
    fill(&mut s);
    let b = cell_rect(&mut s, 0, 1);
    s.run("table.selectAt", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": b.y - 3.0})).unwrap();
    let range = s.cell_range().cloned().unwrap();
    assert_eq!((range.rows, range.cols, range.stream), ((0, 2), (1, 1), false));
    assert_eq!(s.column_segments().unwrap().len(), 3);
    s.run("format.bold", &json!({})).unwrap();
    for r in 0..3 {
        assert!(bold(&s, r, 1), "row {r} of the column is bold");
        assert!(!bold(&s, r, 0) && !bold(&s, r, 2), "row {r}'s other cells aren't");
    }
    s.run("para.alignCenter", &json!({})).unwrap();
    assert_eq!(para(&s, 1, 1).props.align, Some(Align::Center));
    assert_eq!(para(&s, 0, 2).props.align, None, "the cells between the corners keep theirs");
    assert!(s.cell_range().is_some(), "formatting keeps the selection");
    s.run("table.shading", &json!({"color": "FF0000"})).unwrap();
    let t = first_table(&s);
    assert!(t.rows.iter().all(|row| row.cells[1].props.shading.is_some() && row.cells[0].props.shading.is_none()));
    let depth = s.undo_depth();
    s.run("text.delete", &json!({})).unwrap();
    assert_eq!(s.undo_depth(), depth + 1);
    for r in 0..3 {
        assert!(para(&s, r, 1).is_empty());
        assert_eq!(para(&s, r, 0).plain_text(), format!("r{r}c0"));
        assert_eq!(para(&s, r, 2).plain_text(), format!("r{r}c2"));
    }
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(para(&s, 2, 1).plain_text(), "r2c1");
    assert_eq!(s.cell_range().map(|r| r.cols), Some((1, 1)), "undo brings the column selection back");
}

/// Numbering a column numbers its cells as one list, like Word.
#[test]
fn numbering_a_column_makes_one_list() {
    let mut s = table_session(3, 2);
    fill(&mut s);
    s.sel = Selection::caret(cell_pos(&s, 1, 0));
    s.run("table.selectColumn", &json!({})).unwrap();
    s.run("para.numbering", &json!({})).unwrap();
    let nums: Vec<_> = (0..3).map(|r| para(&s, r, 0).props.numbering.map(|n| n.num)).collect();
    assert!(nums[0].is_some_and(|n| n != 0) && nums.iter().all(|n| *n == nums[0]), "{nums:?}");
    assert!((0..3).all(|r| para(&s, r, 1).props.numbering.is_none()));
}

/// An empty column's cells take character formatting in their marks; Tab moves on without
/// clearing a selected column; a click in place ends a selected empty cell.
#[test]
fn empty_cells_tab_and_clicks() {
    let mut s = table_session(2, 2);
    s.sel = Selection::caret(cell_pos(&s, 0, 0));
    s.run("table.selectColumn", &json!({})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    assert!(bold(&s, 0, 0) && bold(&s, 1, 0) && !bold(&s, 0, 1) && !bold(&s, 1, 1));
    fill(&mut s);
    s.sel = Selection::caret(cell_pos(&s, 0, 1));
    s.run("table.selectColumn", &json!({})).unwrap();
    s.run("text.tab", &json!({})).unwrap();
    assert_eq!(para(&s, 0, 1).plain_text(), "r0c1");
    assert_eq!(para(&s, 1, 1).plain_text(), "r1c1");
    // A selected empty cell's corners are one place; clicking there ends the selection.
    let mut s = table_session(2, 2);
    let c = cell_rect(&mut s, 1, 1);
    s.run("table.selectAt", &json!({"page": 0, "x": c.x + 4.5, "y": mid_y(c)})).unwrap();
    assert!(s.cell_range().is_some() && s.sel.is_collapsed());
    let at = serde_json::to_value(s.sel.focus.clone()).unwrap();
    s.run("caret.set", &json!({"pos": at})).unwrap();
    assert!(s.cell_range().is_none());
}

/// Rows, cells and drags: whole rows and cells of one row are an ordinary stream selection;
/// a drag over cells of several rows selects their rectangle only.
#[test]
fn rows_cells_and_drags_select_rectangles() {
    let mut s = table_session(3, 3);
    fill(&mut s);
    let r1 = cell_rect(&mut s, 1, 0);
    s.run("table.selectAt", &json!({"page": 0, "x": r1.x - 10.0, "y": mid_y(r1)})).unwrap();
    let range = s.cell_range().cloned().unwrap();
    assert_eq!((range.rows, range.cols, range.stream), ((1, 1), (0, 2), true));
    assert!(s.column_segments().is_none(), "a row is one run of the text");
    let r2 = cell_rect(&mut s, 2, 0);
    s.run("table.selectAt", &json!({"page": 0, "x": r1.x - 10.0, "y": mid_y(r1), "toX": r1.x - 10.0, "toY": mid_y(r2) + 50.0})).unwrap();
    assert_eq!(s.cell_range().map(|r| r.rows), Some((1, 2)));
    // From cell (0, 0) to cell (1, 1): four cells, not the row between.
    let (a, d) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 1, 1));
    s.run("table.selectAt", &json!({"page": 0, "x": a.x + 2.0, "y": mid_y(a), "toX": d.x + d.w / 2.0, "toY": mid_y(d)})).unwrap();
    let range = s.cell_range().cloned().unwrap();
    assert_eq!((range.rows, range.cols, range.stream), ((0, 1), (0, 1), false));
    s.run("format.italic", &json!({})).unwrap();
    let italic = |s: &Session, r, c| para(s, r, c).run_ranges().all(|(_, ch)| ch.italic == Some(true));
    assert!(italic(&s, 0, 0) && italic(&s, 1, 1) && !italic(&s, 0, 2) && !italic(&s, 1, 2));
    // Dragging across columns: from the first to the last is every column, all rows.
    let (c0, c2) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 2));
    s.run("table.selectAt", &json!({"page": 0, "x": c0.x + 20.0, "y": c0.y - 3.0, "toX": c2.right() + 30.0, "toY": c0.y})).unwrap();
    let range = s.cell_range().cloned().unwrap();
    assert_eq!((range.rows, range.cols, range.stream), ((0, 2), (0, 2), true));
    // The move handle selects the table.
    let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
    s.run("table.selectAt", &json!({"page": 0, "x": h.x + 4.0, "y": h.y + 4.0})).unwrap();
    assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 2), (0, 2))));
    // The ribbon's Select Row is the same whole-cell selection.
    s.sel = Selection::caret(cell_pos(&s, 2, 1));
    s.run("table.selectRow", &json!({})).unwrap();
    assert_eq!(s.cell_range().map(|r| (r.rows, r.cols, r.stream)), Some(((2, 2), (0, 2), true)));
}

/// The page margin beside a table is a row selection area too, as far as the selection bar
/// reaches (Word): left of a left-to-right table, right of a right-to-left one.
#[test]
fn the_margin_beside_a_row_selects_it() {
    let mut s = table_session(3, 3);
    let r1 = cell_rect(&mut s, 1, 0);
    let body_x = s.layout().pages[0].body.x;
    assert!(r1.x - 40.0 > 0.0 && r1.x - body_x < 24.0);
    // Far past the 24pt of the plain row area, still in the margin: the row.
    assert_eq!(spot_at(&mut s, 10.0, mid_y(r1)), Some(Spot::Row(1)));
    assert_eq!(spot_at(&mut s, body_x - 35.0, mid_y(r1)), Some(Spot::Row(1)));
    // The right margin is not a row bar of a left-to-right table.
    let right = s.layout().pages[0].body.right();
    assert_eq!(spot_at(&mut s, right + 20.0, mid_y(r1)), None);
    // A right-to-left table has its bars on the right.
    let tp = table_path(&s);
    s.doc.table_mut(StoryRef::Body, &tp).unwrap().props.bidi_visual = true;
    s.touch();
    let r1 = cell_rect(&mut s, 1, 0);
    let (page_w, right) = (s.layout().pages[0].w, s.layout().pages[0].body.right());
    assert_eq!(spot_at(&mut s, right + 30.0, mid_y(r1)), Some(Spot::Row(1)));
    assert_eq!(spot_at(&mut s, page_w - 5.0, mid_y(r1)), Some(Spot::Row(1)));
    assert_eq!(spot_at(&mut s, 10.0, mid_y(r1)), None);
    // Shift+click in the bar extends the selected rows from the anchor row.
    s.run("table.selectAt", &json!({"page": 0, "x": right + 30.0, "y": mid_y(r1)})).unwrap();
    let r2 = cell_rect(&mut s, 2, 0);
    s.run("table.selectAt", &json!({"page": 0, "x": right + 30.0, "y": mid_y(r2), "extend": true})).unwrap();
    assert_eq!(s.cell_range().map(|r| r.rows), Some((1, 2)));
    let r0 = cell_rect(&mut s, 0, 0);
    s.run("table.selectAt", &json!({"page": 0, "x": right + 30.0, "y": mid_y(r0), "extend": true})).unwrap();
    assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 2)), "the far edge of the old rows stays");
}

/// `anchorRow` fixes the row a Shift+click extends from, whatever is selected (a drag after
/// an upward Shift+click keeps its rows); out of range it's the last row.
#[test]
fn an_anchor_row_param_fixes_the_extend_anchor() {
    let mut s = table_session(3, 3);
    let (r0, r2) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 2, 0));
    let x = r0.x - 10.0;
    s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r2)})).unwrap();
    s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r0), "extend": true})).unwrap();
    assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 2)));
    // Without it, the next frame of the drag would re-read the anchor as row 0.
    s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r0), "toX": x, "toY": mid_y(r0) + 1.0, "extend": true, "anchorRow": 2})).unwrap();
    assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 2)), "the press's anchor stays");
    s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r0), "extend": true, "anchorRow": 99999999999u64})).unwrap();
    assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 2)), "clamped to the last row");
    s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r0), "extend": true, "anchorRow": 1})).unwrap();
    assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 1)));
}

/// The margin is the row bar only where the table is what's beside it: not at a column-2
/// table's height beside column 1's lines (the selection bar's lines), nor beside text
/// wrapped round a floating table.
#[test]
fn the_row_bar_needs_the_table_beside_the_margin() {
    // A two-column section: 36 short lines fill column 1 and start column 2, then a table.
    let mut s = Session::new(Document::new());
    s.run("layout.columns", &json!({"count": 2})).unwrap();
    let lines: Vec<String> = (0..36).map(|i| format!("Line {i}")).collect();
    s.run("text.insert", &json!({"text": format!("{}\r", lines.join("\r"))})).unwrap();
    s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    let r0 = cell_rect(&mut s, 0, 0);
    let body = s.layout().pages[0].body;
    assert!(r0.x > body.x + body.w / 2.0, "the table is in column 2: {r0:?}");
    assert_eq!(spot_at(&mut s, 10.0, mid_y(r0)), None, "column 1's line is the selection bar's");
    // A floating table with text wrapped to its left.
    let mut d = Document::from_text(&"Words that wrap round the floating table beside them. ".repeat(40));
    let mut t = Table::new(3, 1, 150.0);
    t.props.float = Some(wordcraft_doc::props::TableFloat {
        h_rel: wordcraft_doc::para::Anchor::Margin,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        x: 300.0,
        overlap: false,
        ..Default::default()
    });
    d.insert_block(StoryRef::Body, &Path(vec![0]), wordcraft_doc::Block::Table(t)).unwrap();
    let mut s = Session::new(d);
    let r0 = cell_rect(&mut s, 0, 0);
    assert!(r0.x > 300.0, "the table floats at the right: {r0:?}");
    assert_eq!(spot_at(&mut s, 10.0, mid_y(r0)), None, "the text wrapped beside it has the line bar");
}

/// A paragraph right above the table (no space after) doesn't hide the first row's bar.
#[test]
fn a_paragraph_flush_above_the_table_keeps_the_row_bar() {
    let mut d = Document::from_text("Above");
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.space_after = Some(0.0);
    d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(Table::new(2, 2, 400.0))).unwrap();
    let mut s = Session::new(d);
    let r0 = cell_rect(&mut s, 0, 0);
    assert_eq!(spot_at(&mut s, 10.0, mid_y(r0)), Some(Spot::Row(0)));
    assert_eq!(spot_at(&mut s, 10.0, r0.y + 0.3), Some(Spot::Row(0)), "even at the very top of the row");
}

/// The insert buttons add a row or column at their border, one undo step, and select it (as
/// Word does); typing then goes into its first cell.
#[test]
fn the_insert_buttons_add_rows_and_columns() {
    let mut s = table_session(2, 3);
    fill(&mut s);
    let a = cell_rect(&mut s, 0, 0);
    s.run("table.insertAt", &json!({"page": 0, "x": a.x - 10.0, "y": a.bottom()})).unwrap();
    let t = first_table(&s);
    assert_eq!(t.rows.len(), 3);
    assert!(para(&s, 1, 0).is_empty() && para(&s, 2, 0).plain_text() == "r1c0", "the new row is the second");
    assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (0, 2))), "it's selected");
    s.run("text.insert", &json!({"text": "x"})).unwrap();
    assert_eq!(para(&s, 1, 0).plain_text(), "x");
    assert_eq!((first_table(&s).rows.len(), para(&s, 2, 0).plain_text()), (3, "r1c0".to_string()));
    s.run("edit.undo", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(first_table(&s).rows.len(), 2);
    // Above the first row.
    s.run("table.insertAt", &json!({"page": 0, "x": a.x - 10.0, "y": a.y})).unwrap();
    assert!(para(&s, 0, 0).is_empty() && para(&s, 1, 0).plain_text() == "r0c0");
    assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 0)));
    s.run("edit.undo", &json!({})).unwrap();
    // A column at the border between the first two, the table keeping its width.
    let width: f32 = first_table(&s).grid.iter().sum();
    let b = cell_rect(&mut s, 0, 1);
    s.run("table.insertAt", &json!({"page": 0, "x": b.x, "y": a.y - 8.0})).unwrap();
    let t = first_table(&s);
    assert_eq!(t.cols(), 4);
    assert!(para(&s, 0, 1).is_empty() && para(&s, 0, 2).plain_text() == "r0c1");
    assert!((t.grid.iter().sum::<f32>() - width).abs() < 0.5);
    assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (1, 1))), "it's selected");
    // And at the right edge.
    let e = cell_rect(&mut s, 0, 3);
    s.run("table.insertAt", &json!({"page": 0, "x": e.right(), "y": a.y - 8.0})).unwrap();
    assert_eq!(first_table(&s).cols(), 5);
    assert!(para(&s, 0, 4).is_empty() && para(&s, 0, 3).plain_text() == "r0c2");
    assert_eq!(s.cell_range().map(|r| r.cols), Some((4, 4)));
}

/// Dragging the move handle floats the table where it's dropped; one undo puts it back; a
/// click on the handle moves nothing.
#[test]
fn dragging_the_handle_moves_the_table() {
    let mut s = table_session(2, 2);
    let a = cell_rect(&mut s, 0, 0);
    let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
    let (hx, hy) = (h.x + h.w / 2.0, h.y + h.h / 2.0);
    s.run("table.moveDrag", &json!({"page": 0, "x": hx, "y": hy, "toX": hx, "toY": hy})).unwrap();
    assert!(first_table(&s).props.float.is_none(), "a click doesn't move it");
    s.run("table.moveDrag", &json!({"page": 0, "x": hx, "y": hy, "toX": hx + 50.0, "toY": hy + 30.0})).unwrap();
    let f = first_table(&s).props.float.unwrap();
    assert_eq!((f.h_rel, f.v_rel, f.overlap), (Anchor::Column, Anchor::Paragraph, true));
    let moved = cell_rect(&mut s, 0, 0);
    assert!((moved.x - (a.x + 50.0)).abs() < 0.5 && (moved.y - (a.y + 30.0)).abs() < 0.5, "{a:?} → {moved:?}");
    // Moving it again starts from where it floats.
    let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
    let (hx, hy) = (h.x + h.w / 2.0, h.y + h.h / 2.0);
    s.run("table.moveDrag", &json!({"page": 0, "x": hx, "y": hy, "toX": hx - 20.0, "toY": hy + 10.0})).unwrap();
    let again = cell_rect(&mut s, 0, 0);
    assert!((again.x - (moved.x - 20.0)).abs() < 0.5 && (again.y - (moved.y + 10.0)).abs() < 0.5, "{moved:?} → {again:?}");
    s.run("edit.undo", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert!(first_table(&s).props.float.is_none());
    assert_eq!(cell_rect(&mut s, 0, 0), a);
    // Nested tables and tables outside the main text stay where they are.
    let mut nested = table_resize::tables_on(&s.layout(), &s.doc, 0)[0].clone();
    assert!(movable(&nested));
    nested.table = Path(vec![0, 0, 0, 1]);
    assert!(!movable(&nested));
}

/// A double-click on a column border fits the column left of it to its text; as in Word, the
/// columns right of it keep their widths and move along, so the table narrows. The right
/// edge's column fits the same way.
#[test]
fn a_double_click_on_a_border_fits_the_column() {
    let mut s = table_session(1, 3);
    s.sel = Selection::caret(cell_pos(&s, 0, 0));
    s.run("text.insert", &json!({"text": "ab"})).unwrap();
    let (a, b, c) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1), cell_rect(&mut s, 0, 2));
    let fit = wordcraft_layout::measure_table_columns(&s.doc, &first_table(&s));
    s.run("table.fitColumn", &json!({"page": 0, "x": a.right(), "y": mid_y(a)})).unwrap();
    let (a1, b1, c1) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1), cell_rect(&mut s, 0, 2));
    assert!((a1.w - fit[0].1).abs() < 0.5 && a1.w < a.w - 50.0, "{a:?} → {a1:?}, {fit:?}");
    assert!((b1.w - b.w).abs() < 0.5 && (c1.w - c.w).abs() < 0.5, "the columns right of it keep their widths");
    assert!((b1.x - b.x - (a1.w - a.w)).abs() < 0.5 && (c1.right() - c.right() - (a1.w - a.w)).abs() < 0.5, "and move along");
    s.run("table.fitColumn", &json!({"page": 0, "x": c1.right(), "y": mid_y(c1)})).unwrap();
    let c2 = cell_rect(&mut s, 0, 2);
    assert!((c2.w - fit[2].1).abs() < 0.5 && c2.w < c.w, "{c:?} → {c2:?}");
}

/// A nested table's column fits within the cell around the table: the table widens only to
/// the right edge of that cell's text, not to the page's right margin, and the table around
/// it keeps its columns.
#[test]
fn a_nested_tables_column_fits_within_its_cell() {
    let mut s = table_session(1, 2);
    s.sel = Selection::caret(cell_pos(&s, 0, 0));
    s.run("insert.table", &json!({"rows": 1, "cols": 2})).unwrap();
    let (inner, _, _) = s.sel.focus.path.cell().unwrap();
    let rect = |s: &mut Session, table: &Path, cell: usize| {
        s.layout().pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Cell { rect, table: t, row: 0, cell: c, .. } if t == table && *c == cell => Some(*rect),
                _ => None,
            })
            .unwrap()
    };
    // Its empty columns narrow first, leaving room right of it in its cell.
    for col in [1, 0] {
        let r = rect(&mut s, &inner, col);
        s.run("table.fitColumn", &json!({"page": 0, "x": r.right(), "y": mid_y(r)})).unwrap();
    }
    s.run("text.insert", &json!({"text": "word ".repeat(40)})).unwrap();
    let outer = table_path(&s);
    let host = rect(&mut s, &outer, 0);
    let (a, b) = (rect(&mut s, &inner, 0), rect(&mut s, &inner, 1));
    assert!(b.right() < host.right() - 40.0, "{b:?} in {host:?}");
    s.run("table.fitColumn", &json!({"page": 0, "x": a.right(), "y": mid_y(a)})).unwrap();
    let (a1, b1) = (rect(&mut s, &inner, 0), rect(&mut s, &inner, 1));
    let text_right = host.right() - wordcraft_layout::cell_margins(&s.doc, &first_table(&s), 0, 0)[3];
    assert!(a1.w > a.w + 40.0 && (b1.w - b.w).abs() < 0.5, "{a:?} {b:?} → {a1:?} {b1:?}");
    assert!((b1.right() - text_right).abs() < 0.5, "the table fills its cell's text: {b1:?}, {text_right}");
    let host1 = rect(&mut s, &outer, 0);
    assert_eq!((host1.x, host1.w), (host.x, host.w), "the table around it keeps its columns");
}

/// Pasting formatting onto a selected column leaves its text (it used to clear it).
#[test]
fn the_format_painter_keeps_a_selected_columns_text() {
    let mut s = table_session(2, 2);
    fill(&mut s);
    s.sel = Selection::caret(cell_pos(&s, 0, 0));
    s.run("edit.formatPainter", &json!({})).unwrap();
    s.sel = Selection::caret(cell_pos(&s, 0, 1));
    s.run("table.selectColumn", &json!({})).unwrap();
    s.run("edit.pasteFormat", &json!({})).unwrap();
    assert_eq!(para(&s, 0, 1).plain_text(), "r0c1");
    assert_eq!(para(&s, 1, 1).plain_text(), "r1c1");
}

/// Where the app extends the selection while dragging (`caret.set` with `extend`).
fn drag_to(s: &mut Session, to: Pos) {
    s.run("caret.set", &json!({"pos": to, "extend": true})).unwrap();
}

/// An ordinary drag from one cell into another selects the rectangle of whole cells between
/// them, so bold takes just those, as in Word; back in the first cell it's text again, from
/// where the drag started. Out of a nested table, the cells are the table's around it.
#[test]
fn a_drag_across_cells_selects_their_rectangle() {
    let mut s = table_session(3, 3);
    fill(&mut s);
    let start = Pos { off: 1, ..cell_pos(&s, 0, 1) };
    s.run("caret.set", &json!({"pos": start})).unwrap();
    let down = Pos { off: 2, ..cell_pos(&s, 2, 1) };
    drag_to(&mut s, down);
    let range = s.cell_range().cloned().unwrap();
    assert_eq!((range.rows, range.cols, range.stream), ((0, 2), (1, 1), false));
    s.run("format.bold", &json!({})).unwrap();
    for r in 0..3 {
        assert!(bold(&s, r, 1) && !bold(&s, r, 0) && !bold(&s, r, 2), "row {r}: just the dragged column");
    }
    // On to the left: the rectangle still runs from the cell the drag started in.
    let left = cell_pos(&s, 1, 0);
    drag_to(&mut s, left);
    assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 1))));
    // Back in that cell: its text, from where the drag started.
    let back = Pos { off: 3, ..cell_pos(&s, 0, 1) };
    drag_to(&mut s, back.clone());
    assert!(s.cell_range().is_none());
    assert_eq!(s.sel, Selection { anchor: start, focus: back });
    // From a nested table's cell into another cell of the table around it.
    let mut s = table_session(2, 2);
    s.sel = Selection::caret(cell_pos(&s, 0, 0));
    s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    assert_eq!(s.sel.focus.path.0.len(), 7, "the caret is in the nested table");
    let to = cell_pos(&s, 1, 1);
    drag_to(&mut s, to);
    let range = s.cell_range().cloned().unwrap();
    assert_eq!((range.table, range.rows, range.cols), (table_path(&s), (0, 1), (0, 1)));
}

/// Shift+Down from a cell takes it and the cell below, whole; on past the table, the anchor's
/// row and every row after it, then the text; Shift+Up comes back to the cells, then into the
/// first cell's text. Shift+click goes on from where the selection started, and from text
/// before the table a drag into a cell takes the rows down to it, whole.
#[test]
fn shift_arrows_and_clicks_select_cells_then_rows() {
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": "Intro"})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("insert.table", &json!({"rows": 3, "cols": 3})).unwrap();
    fill(&mut s);
    let after = table_path(&s).last() + 1;
    let shift = |s: &mut Session, id: &str| {
        s.run(id, &json!({"extend": true})).unwrap();
    };
    s.sel = Selection::caret(cell_pos(&s, 1, 1));
    shift(&mut s, "caret.down");
    assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 2), (1, 1))));
    shift(&mut s, "caret.down");
    assert!(s.cell_range().is_none());
    assert_eq!(s.sel.anchor, cell_pos(&s, 1, 0), "from the start of the anchor's row");
    assert_eq!(s.sel.focus.path, Path::top(after));
    s.run("format.italic", &json!({})).unwrap();
    let italic = |s: &Session, r, c| para(s, r, c).run_ranges().all(|(_, ch)| ch.italic == Some(true));
    assert!(italic(&s, 1, 0) && italic(&s, 2, 2) && !italic(&s, 0, 1), "whole rows from the anchor's");
    shift(&mut s, "caret.up");
    assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 2), (1, 1))));
    shift(&mut s, "caret.up");
    assert!(s.cell_range().is_none());
    assert_eq!(s.sel.anchor, cell_pos(&s, 1, 1));
    assert_eq!(s.sel.focus.path, cell_pos(&s, 1, 1).path);
    // Shift+click: the rectangle from the anchor's cell.
    let corner = cell_pos(&s, 0, 0);
    s.run("caret.set", &json!({"pos": corner, "extend": true})).unwrap();
    assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 1))));
    // From the text before the table into a cell: the rows down to that cell's, whole.
    s.sel = Selection::caret(Pos::body(0, 2));
    let into = cell_pos(&s, 1, 1);
    drag_to(&mut s, into);
    assert!(s.cell_range().is_none());
    assert_eq!(s.sel, Selection { anchor: Pos::body(0, 2), focus: Pos { off: 4, ..cell_pos(&s, 1, 2) } });
}

/// A drag from the text before a table into it takes whole rows, and they shade every cell
/// of them (an empty one too), not just their letters; the rows it doesn't take and a table
/// both ends are in shade their text.
#[test]
fn a_selection_into_a_table_shades_its_rows_whole_cells() {
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": "Intro"})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("insert.table", &json!({"rows": 3, "cols": 2})).unwrap();
    fill(&mut s);
    let emptied = cell_pos(&s, 0, 1);
    s.sel = Selection { anchor: emptied.clone(), focus: Pos { off: 4, ..emptied } };
    s.run("text.delete", &json!({})).unwrap();
    assert!(para(&s, 0, 1).is_empty());
    s.sel = Selection::caret(Pos::body(0, 2));
    let into = cell_pos(&s, 1, 0);
    drag_to(&mut s, into);
    let (a, b) = s.sel.ordered();
    let l = s.layout();
    let shaded = selection_rects(&l, &s.doc, &a, &b, 0);
    for r in 0..3 {
        for c in 0..2 {
            let cell = cell_rect(&mut s, r, c);
            assert_eq!(shaded.iter().filter(|(_, x)| *x == cell).count(), usize::from(r < 2), "cell {r},{c}");
        }
    }
    let first = cell_rect(&mut s, 0, 0);
    let on = |x: &Rect| first.contains(Point::new(x.x + x.w / 2.0, x.y + x.h / 2.0));
    assert_eq!(shaded.iter().filter(|(_, x)| on(x)).count(), 1, "the cell, its letters not again");
    assert!(shaded.iter().any(|(_, x)| x.bottom() <= first.y), "and the text before the table");
    // Both ends in the table (here a whole row of it): its text.
    let (start, end) = table::row_ends(&s.doc, StoryRef::Body, &table_path(&s), 2).unwrap();
    assert_eq!(selection_rects(&l, &s.doc, &start, &end, 0), l.selection_rects(&s.doc, &start, &end, 0));
}

/// Clicks away from the areas, and hostile numbers: an error and no change, never a panic.
#[test]
fn misses_and_hostile_params_change_nothing() {
    let mut s = table_session(2, 2);
    let start = first_table(&s);
    let c = cell_rect(&mut s, 1, 1);
    let mid = json!({"page": 0, "x": c.x + c.w / 2.0, "y": mid_y(c), "toX": 1, "toY": 1});
    for id in ["table.selectAt", "table.insertAt", "table.moveDrag", "table.fitColumn"] {
        assert!(s.run(id, &mid).is_err(), "{id}");
        assert!(s.run(id, &json!({"page": 99, "x": 1, "y": 1, "toX": 1, "toY": 1})).is_err(), "{id}");
        assert!(s.run(id, &json!({"page": 0, "x": 1e30, "y": -1e30, "toX": 1e30, "toY": 1e30, "reach": 1e30})).is_err(), "{id}");
        assert!(s.run(id, &json!({})).is_err(), "{id}");
    }
    // An insert button isn't a selection area, nor a selection area a button.
    let a = cell_rect(&mut s, 0, 0);
    assert!(s.run("table.selectAt", &json!({"page": 0, "x": a.x - 10.0, "y": a.bottom()})).is_err());
    assert!(s.run("table.insertAt", &json!({"page": 0, "x": a.x - 10.0, "y": mid_y(a)})).is_err());
    assert_eq!(first_table(&s), start);
    // A drag to the ends of the earth keeps the table on its page.
    let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
    s.run("table.moveDrag", &json!({"page": 0, "x": h.x + 4.0, "y": h.y + 4.0, "toX": 3e38, "toY": -3e38})).unwrap();
    let f = first_table(&s).props.float.unwrap();
    assert!(f.x.is_finite() && f.y.is_finite());
    let moved = cell_rect(&mut s, 0, 0);
    assert!(moved.x >= -0.5 && moved.right() <= 612.5 && moved.y >= -0.5, "{moved:?}");
}

/// #2038: a table dragged to the right floats and the empty paragraph after it moves up beside
/// it. As in Word, a click anywhere below the table then goes to that paragraph, not to a cell of the last row; a click in a cell still lands there.
#[test]
fn a_click_below_a_floating_table_goes_to_the_text_not_its_cells() {
    let mut s = table_session(3, 3);
    let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
    let (hx, hy) = (h.x + h.w / 2.0, h.y + h.h / 2.0);
    s.run("table.moveDrag", &json!({"page": 0, "x": hx, "y": hy, "toX": hx + 60.0, "toY": hy + 10.0})).unwrap();
    assert!(first_table(&s).props.float.is_some());
    let after = Path(vec![table_path(&s).0[0] + 1]);
    assert!(s.doc.block(StoryRef::Body, &after).is_some_and(|b| b.as_table().is_none()), "a paragraph follows the table");
    let (first, last) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 2, 2));
    let (mid, bottom, margin) = ((first.x + last.right()) / 2.0, last.bottom(), s.layout().pages[0].body.x + 5.0);
    let at = |s: &mut Session, x: f32, y: f32| {
        s.run("caret.set", &json!({"page": 0, "x": x, "y": y})).unwrap();
        s.sel.focus.path.clone()
    };
    for (name, x, y) in [
        ("just below the middle", mid, bottom + 12.0),
        ("far below the middle", mid, bottom + 200.0),
        ("below, at the margin", margin, bottom + 12.0),
    ] {
        assert_eq!(at(&mut s, x, y), after, "a click {name}");
    }
    let b2 = cell_rect(&mut s, 1, 1);
    assert_eq!(at(&mut s, b2.x + b2.w / 2.0, mid_y(b2)), cell_pos(&s, 1, 1).path, "in B2");
    assert_eq!(at(&mut s, last.x + last.w / 2.0, last.bottom() - 1.0), cell_pos(&s, 2, 2).path, "low in C3");
    s.run("caret.docEnd", &json!({})).unwrap();
    assert_eq!(s.sel.focus.path, after, "Ctrl+End");
}

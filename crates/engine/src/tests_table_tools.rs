//! Scenario tests for the table pen and eraser (#2543, #2544) on empty tables and tables full
//! of text: what Word does, and what a user hit (a border erased between two rows couldn't be
//! drawn back). Coordinates are page points from the layout, as the app sends them.

use serde_json::{Value, json};
use wordcraft_doc::props::VMerge;
use wordcraft_doc::{Block, Paragraph, Table, para_block};
use wordcraft_geom::Rect;
use wordcraft_layout::Placed;

use crate::Session;

const NAMES: [&str; 4] = ["A", "B", "C", "D"];

/// A `rows`×`cols` table; `filled`: each cell holds a sentence that starts with its name ("B2 …").
fn table_session(rows: usize, cols: usize, filled: bool) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("insert.table", &json!({"rows": rows, "cols": cols})).unwrap();
    if filled {
        let t = s.doc.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
            Block::Table(t) => Some(t),
            _ => None,
        });
        for (r, row) in t.unwrap().rows.iter_mut().enumerate() {
            for (c, cell) in row.cells.iter_mut().enumerate() {
                let text = format!("{}{} some words in this cell", NAMES[c], r + 1);
                cell.blocks = vec![para_block(Paragraph::with_text(&text, Default::default()))];
            }
        }
        s.touch();
    }
    s
}

fn first_table(s: &Session) -> Table {
    s.doc.body.iter().find_map(|b| b.as_table()).cloned().unwrap()
}

/// Each row's cell spans.
fn shape(t: &Table) -> Value {
    json!(t.rows.iter().map(|r| r.cells.iter().map(|c| c.span()).collect::<Vec<_>>()).collect::<Vec<_>>())
}

/// How many cells each row has.
fn cells_per_row(t: &Table) -> Vec<usize> {
    t.rows.iter().map(|r| r.cells.len()).collect()
}

fn vmerges(t: &Table, r: usize) -> Vec<VMerge> {
    t.rows[r].cells.iter().map(|c| c.props.vmerge).collect()
}

/// A cell's paragraphs' text.
fn texts(t: &Table, r: usize, c: usize) -> Vec<String> {
    t.rows[r].cells[c].blocks.iter().filter_map(|b| b.as_para().map(|q| q.plain_text())).filter(|x| !x.is_empty()).collect()
}

/// Every non-empty paragraph in the table, sorted (nothing lost, nothing doubled).
fn all_texts(t: &Table) -> Vec<String> {
    let mut v: Vec<String> =
        (0..t.rows.len()).flat_map(|r| (0..t.rows[r].cells.len()).map(move |c| (r, c))).flat_map(|(r, c)| texts(t, r, c)).collect();
    v.sort();
    v
}

/// Which of a cell's top, right, bottom and left borders are hidden.
fn hidden(t: &Table, r: usize, c: usize) -> [bool; 4] {
    let b = t.rows[r].cells[c].props.borders.unwrap_or_default();
    [b.top, b.right, b.bottom, b.left].map(|e| e.is_some_and(|e| !e.is_visible()))
}

/// Where cell (`row`, `cell`) of the body's first table is drawn on page 1.
fn cell_rect(s: &mut Session, row: usize, cell: usize) -> Rect {
    s.layout().pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Cell { rect, row: r, cell: c, table, .. } if *r == row && *c == cell && table.0.len() == 1 => Some(*rect),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no cell ({row}, {cell}) on page 1"))
}

fn pen(s: &mut Session, (x0, y0): (f32, f32), (x1, y1): (f32, f32)) -> Result<Value, crate::CmdError> {
    s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x1, "y1": y1}))
}

fn erase_at(s: &mut Session, x: f32, y: f32) {
    s.run("table.eraser", &json!({"page": 0, "x": x, "y": y})).unwrap();
}

fn reopen(s: &Session) -> Session {
    Session::new(wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap())
}

// ── Eraser ─────────────────────────────────────────────────────────────────────────────────

/// As in Word: a click on the line between A2 and B2 merges them, in that row only; with text the
/// merged cell keeps both, one paragraph each.
#[test]
fn eraser_merges_across_a_vertical_line_empty_and_filled() {
    for filled in [false, true] {
        let mut s = table_session(3, 3, filled);
        let a = cell_rect(&mut s, 1, 0);
        erase_at(&mut s, a.right(), a.y + a.h / 2.0);
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 1, 1], [2, 1], [1, 1, 1]]), "filled: {filled}");
        if filled {
            assert_eq!(texts(&t, 1, 0)[..2], ["A2 some words in this cell", "B2 some words in this cell"]);
        }
    }
}

/// As in Word: a click on the line between C3 and C4 merges them down; with text, both stay.
#[test]
fn eraser_merges_down_a_horizontal_line_empty_and_filled() {
    for filled in [false, true] {
        let mut s = table_session(4, 4, filled);
        let before = all_texts(&first_table(&s));
        let c = cell_rect(&mut s, 2, 2);
        erase_at(&mut s, c.x + c.w / 2.0, c.bottom());
        let t = first_table(&s);
        assert_eq!(vmerges(&t, 2)[2], VMerge::Restart, "filled: {filled}");
        assert_eq!(vmerges(&t, 3)[2], VMerge::Continue, "filled: {filled}");
        assert_eq!(all_texts(&t), before, "no text lost (filled: {filled})");
        if filled {
            assert_eq!(texts(&t, 2, 2), ["C3 some words in this cell", "C4 some words in this cell"], "Word: one paragraph each");
        }
    }
}

/// As in Word: an outside border only hides; undo brings everything back in one step.
#[test]
fn eraser_hides_an_outside_border_and_undoes_in_one_step() {
    let mut s = table_session(3, 3, true);
    let start = first_table(&s);
    let b = cell_rect(&mut s, 0, 1);
    erase_at(&mut s, b.x + b.w / 2.0, b.y);
    let t = first_table(&s);
    assert_eq!(shape(&t), shape(&start), "no merge");
    assert!(hidden(&t, 0, 1)[0], "B1's top hidden");
    assert!(!hidden(&t, 0, 0)[0] && !hidden(&t, 0, 2)[0], "only B1's piece");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(first_table(&s), start);
}

// ── Pen ────────────────────────────────────────────────────────────────────────────────────

/// As in Word: a vertical line through all rows splits every row at the same x.
#[test]
fn pen_line_through_all_rows_splits_them_in_line() {
    for filled in [false, true] {
        let mut s = table_session(3, 3, filled);
        let (a0, a2) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 2, 0));
        let x = a0.x + a0.w / 2.0;
        pen(&mut s, (x, a0.y + 2.0), (x, a2.bottom() - 2.0)).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 1, 1, 1], [1, 1, 1, 1], [1, 1, 1, 1]]), "filled: {filled}");
        let xs: Vec<f32> = (0..3).map(|r| cell_rect(&mut s, r, 1).x).collect();
        assert!(xs.iter().all(|v| (v - xs[0]).abs() < 0.5), "one straight line: {xs:?}");
        assert!((xs[0] - x).abs() < 1.5, "where it was drawn: {} vs {x}", xs[0]);
    }
}

/// As in Word: splitting a cell with text keeps all of it in the left cell; the right is empty.
#[test]
fn pen_split_keeps_the_text_in_the_left_cell() {
    let mut s = table_session(2, 2, true);
    let a = cell_rect(&mut s, 0, 0);
    let x = a.x + a.w / 2.0;
    pen(&mut s, (x, a.y + 1.0), (x, a.bottom() - 1.0)).unwrap();
    let t = first_table(&s);
    assert_eq!(shape(&t), json!([[1, 1, 1], [2, 1]]));
    assert_eq!(texts(&t, 0, 0), ["A1 some words in this cell"]);
    assert!(texts(&t, 0, 1).is_empty(), "the new cell is empty: {:?}", texts(&t, 0, 1));
}

/// As in Word: a vertical line 4 px beside a column border of another row lines up with
/// it (one straight line, no new grid column); further away it makes its own column.
#[test]
fn pen_line_beside_another_rows_border_lines_up_with_it() {
    for filled in [false, true] {
        let mut s = table_session(3, 3, filled);
        let a0 = cell_rect(&mut s, 0, 0);
        let x = a0.x + a0.w / 2.0;
        pen(&mut s, (x, a0.y + 1.0), (x, a0.bottom() - 1.0)).unwrap();
        let cols = first_table(&s).grid.len();
        let a1 = cell_rect(&mut s, 1, 0);
        pen(&mut s, (x + 2.0, a1.y + 1.0), (x + 2.0, a1.bottom() - 1.0)).unwrap();
        let t = first_table(&s);
        assert_eq!(t.grid.len(), cols, "lined up with row 1's line, no new grid column (filled: {filled})");
        assert!((cell_rect(&mut s, 1, 1).x - cell_rect(&mut s, 0, 1).x).abs() < 0.5, "same x (filled: {filled})");
        let a2 = cell_rect(&mut s, 2, 0);
        pen(&mut s, (x + 12.0, a2.y + 1.0), (x + 12.0, a2.bottom() - 1.0)).unwrap();
        assert_eq!(first_table(&s).grid.len(), cols + 1, "12 pt away: a column of its own (filled: {filled})");
    }
}

/// As in Word: a horizontal line inside a cell splits the row there; text isn't lost.
#[test]
fn pen_horizontal_line_splits_a_row_empty_and_filled() {
    for filled in [false, true] {
        let mut s = table_session(2, 3, filled);
        let before = all_texts(&first_table(&s));
        let b = cell_rect(&mut s, 0, 1);
        let y = b.y + b.h / 2.0;
        pen(&mut s, (b.x + 3.0, y), (b.right() - 3.0, y)).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows.len(), 3, "filled: {filled}");
        assert_eq!(vmerges(&t, 0), [VMerge::Restart, VMerge::None, VMerge::Restart], "A and C span both halves");
        assert_eq!(all_texts(&t), before, "no text lost (filled: {filled})");
    }
}

/// A short vertical line in one row that runs a little into the next row — at 100 % zoom a row
/// is ~20 px and the app needs a 23 px stroke, so it always does — splits only the row it is
/// mostly in; the next row's line beside it lines up with it. Not observed in Word yet (Word
/// previews such a line on one row's height).
#[test]
fn pen_short_line_that_grazes_the_next_row_splits_one_row() {
    let mut s = table_session(3, 3, true);
    let a0 = cell_rect(&mut s, 0, 0);
    let x = a0.x + a0.w / 2.0;
    // 24 px at 100 % zoom = 18 pt, from just under the row's top.
    pen(&mut s, (x, a0.y + 1.0), (x, a0.y + 19.0)).unwrap();
    let t = first_table(&s);
    assert_eq!(shape(&t), json!([[1, 1, 1, 1], [2, 1, 1], [2, 1, 1]]), "only row 1 split; row height {}", a0.h);
    let a1 = cell_rect(&mut s, 1, 0);
    pen(&mut s, (x + 2.0, a1.y + 1.5), (x + 2.0, a1.y + 19.5)).unwrap();
    let t = first_table(&s);
    assert_eq!(shape(&t), json!([[1, 1, 1, 1], [1, 1, 1, 1], [2, 1, 1]]), "row 2 too, in line with row 1; row 3 whole");
}

// ── Drawing back what the eraser took ───────────────────────────────────────────────────

/// Erase the line between B1 and B2 (they merge down), then draw it back with the pen at the
/// same place: B1 and B2 again, as before — empty and with text.
#[test]
fn pen_draws_back_an_erased_line_between_rows() {
    for filled in [false, true] {
        let mut s = table_session(3, 3, filled);
        let start = first_table(&s);
        let b = cell_rect(&mut s, 0, 1);
        let y = b.bottom();
        erase_at(&mut s, b.x + b.w / 2.0, y);
        assert_eq!(vmerges(&first_table(&s), 0)[1], VMerge::Restart, "erased (filled: {filled})");
        let r = pen(&mut s, (b.x + 3.0, y), (b.right() - 3.0, y));
        assert!(r.is_ok(), "the pen draws it back (filled: {filled}): {r:?}");
        let t = first_table(&s);
        assert_eq!(t.rows.len(), start.rows.len(), "no extra row (filled: {filled})");
        assert_eq!(vmerges(&t, 0), [VMerge::None; 3], "B1 and B2 apart again (filled: {filled})");
        assert_eq!(vmerges(&t, 1), [VMerge::None; 3], "filled: {filled}");
        assert_eq!(all_texts(&t), all_texts(&start), "no text lost (filled: {filled})");
    }
}

/// Erase the line between A1 and B1 (they merge across), then draw it back: two cells again on
/// the old grid line.
#[test]
fn pen_draws_back_an_erased_line_between_columns() {
    for filled in [false, true] {
        let mut s = table_session(3, 3, filled);
        let start = first_table(&s);
        let a = cell_rect(&mut s, 0, 0);
        let x = a.right();
        erase_at(&mut s, x, a.y + a.h / 2.0);
        assert_eq!(shape(&first_table(&s)), json!([[2, 1], [1, 1, 1], [1, 1, 1]]), "erased (filled: {filled})");
        let r = pen(&mut s, (x, a.y + 1.0), (x, a.bottom() - 1.0));
        assert!(r.is_ok(), "the pen draws it back (filled: {filled}): {r:?}");
        let t = first_table(&s);
        assert_eq!(shape(&t), shape(&start), "filled: {filled}");
        assert_eq!(t.grid.len(), start.grid.len(), "on the old grid line (filled: {filled})");
        assert_eq!(all_texts(&t), all_texts(&start), "no text lost (filled: {filled})");
    }
}

/// A whole horizontal line erased with a drag (every column merges down; the row under it only
/// continues the merges, so the layout draws it with no height and the two rows look like one),
/// then drawn back through the middle of that row: the rows are apart again, no row added.
#[test]
fn pen_draws_back_a_whole_erased_row_line() {
    for filled in [false, true] {
        let mut s = table_session(3, 3, filled);
        let (a, c) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 2));
        let y = a.bottom();
        s.run("table.eraser", &json!({"page": 0, "x0": a.x + 3.0, "y0": y, "x1": c.right() - 3.0, "y1": y + 0.5})).unwrap();
        let merged = cell_rect(&mut s, 0, 0);
        let y = merged.y + merged.h / 2.0;
        let r = pen(&mut s, (a.x + 3.0, y), (c.right() - 3.0, y));
        assert!(r.is_ok(), "the pen draws it back (filled: {filled}): {r:?}");
        let t = first_table(&s);
        assert_eq!(t.rows.len(), 3, "filled: {filled}");
        assert_eq!(vmerges(&t, 0), [VMerge::None; 3], "filled: {filled}");
    }
}

/// The same as `pen_draws_back_an_erased_line_between_rows` on a table saved to .docx and
/// opened again after the erase.
#[test]
fn pen_draws_back_an_erased_line_after_save_and_open() {
    let mut s = table_session(3, 3, true);
    let start = first_table(&s);
    let b = cell_rect(&mut s, 0, 1);
    let y = b.bottom();
    erase_at(&mut s, b.x + b.w / 2.0, y);
    let mut s = reopen(&s);
    let r = pen(&mut s, (b.x + 3.0, y), (b.right() - 3.0, y));
    assert!(r.is_ok(), "{r:?}");
    let t = first_table(&s);
    assert_eq!(vmerges(&t, 0), [VMerge::None; 3]);
    assert_eq!(all_texts(&t), all_texts(&start));
}

/// Erase then draw back, then undo twice: the table as it started.
#[test]
fn erase_draw_back_and_undo_twice_restores_the_table() {
    let mut s = table_session(3, 3, true);
    let start = first_table(&s);
    let a = cell_rect(&mut s, 0, 0);
    erase_at(&mut s, a.right(), a.y + a.h / 2.0);
    pen(&mut s, (a.right(), a.y + 1.0), (a.right(), a.bottom() - 1.0)).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(first_table(&s), start);
}

// ── Eraser, more cases, empty and filled ──────────────────────────────────────────────────

/// Where cell (`row`, `cell`) of the table nested in the body table's cell (`outer_row`, `outer_cell`) is drawn.
fn nested_rect(s: &mut Session, (outer_row, outer_cell): (usize, usize), row: usize, cell: usize) -> Rect {
    s.layout().pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Cell { rect, row: r, cell: c, table, .. }
                if *r == row
                    && *c == cell
                    && table.0.len() == 4
                    && table.0.get(1) == Some(&(outer_row as u32))
                    && table.0.get(2) == Some(&(outer_cell as u32)) =>
            {
                Some(*rect)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no nested cell ({row}, {cell})"))
}

fn nested_table(s: &Session, r: usize, c: usize) -> Table {
    first_table(s).rows[r].cells[c].blocks.iter().find_map(|b| b.as_table()).cloned().expect("a nested table")
}

/// As in Word: a drag across row 2's vertical lines makes the row one cell, the text
/// kept in order.
#[test]
fn eraser_drag_across_a_row_makes_it_one_cell() {
    for filled in [false, true] {
        let mut s = table_session(4, 4, filled);
        let (a, d) = (cell_rect(&mut s, 1, 0), cell_rect(&mut s, 1, 3));
        let y = a.y + a.h / 2.0;
        s.run("table.eraser", &json!({"page": 0, "x0": a.x + a.w / 2.0, "y0": y, "x1": d.x + d.w / 2.0, "y1": y + 1.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 1, 1, 1], [4], [1, 1, 1, 1], [1, 1, 1, 1]]), "filled: {filled}");
        if filled {
            let firsts: Vec<String> = texts(&t, 1, 0).iter().map(|x| x[..2].to_string()).collect();
            assert_eq!(firsts, ["A2", "B2", "C2", "D2"]);
        }
    }
}

/// As in Word: corner to corner from B1 to C3 makes one cell, 2 columns × 3 rows.
#[test]
fn eraser_diagonal_drag_makes_one_block() {
    for filled in [false, true] {
        let mut s = table_session(4, 4, filled);
        let before = all_texts(&first_table(&s));
        let (b, c) = (cell_rect(&mut s, 0, 1), cell_rect(&mut s, 2, 2));
        s.run("table.eraser", &json!({"page": 0, "x0": b.x + b.w / 2.0, "y0": b.y + b.h / 2.0, "x1": c.x + c.w / 2.0, "y1": c.y + c.h / 2.0}))
            .unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 2, 1], [1, 2, 1], [1, 2, 1], [1, 1, 1, 1]]), "filled: {filled}");
        assert_eq!((0..3).map(|r| t.rows[r].cells[1].props.vmerge).collect::<Vec<_>>(), [VMerge::Restart, VMerge::Continue, VMerge::Continue]);
        assert_eq!(all_texts(&t), before, "no text lost (filled: {filled})");
    }
}

/// As in Word: a drag from above the table down into D2 hides D1's top and merges
/// D1 with D2; one from C3 out under the table merges C3 with C4 and hides the bottom under C.
#[test]
fn eraser_drag_from_outside_hides_and_merges() {
    for filled in [false, true] {
        let mut s = table_session(4, 4, filled);
        let (d1, d2) = (cell_rect(&mut s, 0, 3), cell_rect(&mut s, 1, 3));
        let x = d1.x + d1.w / 2.0;
        s.run("table.eraser", &json!({"page": 0, "x0": x, "y0": d1.y - 10.0, "x1": x + 1.0, "y1": d2.y + d2.h / 2.0})).unwrap();
        let t = first_table(&s);
        assert!(hidden(&t, 0, 3)[0], "D1's top hidden (filled: {filled})");
        assert_eq!((vmerges(&t, 0)[3], vmerges(&t, 1)[3]), (VMerge::Restart, VMerge::Continue), "filled: {filled}");
        let (c3, c4) = (cell_rect(&mut s, 2, 2), cell_rect(&mut s, 3, 2));
        let x = c3.x + c3.w / 2.0;
        s.run("table.eraser", &json!({"page": 0, "x0": x, "y0": c3.y + c3.h / 2.0, "x1": x + 1.0, "y1": c4.bottom() + 10.0})).unwrap();
        let t = first_table(&s);
        assert_eq!((vmerges(&t, 2)[2], vmerges(&t, 3)[2]), (VMerge::Restart, VMerge::Continue), "filled: {filled}");
        assert!(hidden(&t, 2, 2)[2] || hidden(&t, 3, 2)[2], "the bottom under C hidden (filled: {filled})");
    }
}

/// As in Word: a drag down the table's left side hides it in every row, merging nothing.
#[test]
fn eraser_drag_along_the_left_side_hides_it_everywhere() {
    let mut s = table_session(4, 4, true);
    let start = first_table(&s);
    let (a1, a4) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 3, 0));
    s.run("table.eraser", &json!({"page": 0, "x0": a1.x, "y0": a1.y + 2.0, "x1": a1.x + 0.5, "y1": a4.bottom() - 2.0})).unwrap();
    let t = first_table(&s);
    assert_eq!(shape(&t), shape(&start));
    assert!((0..4).all(|r| hidden(&t, r, 0)[3]), "the left side hidden in every row");
    assert!((0..4).all(|r| t.rows[r].cells[0].props.vmerge == VMerge::None), "nothing merged");
}

/// As in Word: A1:A2 merged down beside B1 and B2 — a click between them merges
/// nothing and hides the line along the whole shared edge.
#[test]
fn eraser_between_cells_that_dont_make_a_rectangle_hides_the_line() {
    for filled in [false, true] {
        let mut s = table_session(3, 3, filled);
        let a = cell_rect(&mut s, 0, 0);
        erase_at(&mut s, a.x + a.w / 2.0, a.bottom());
        let shaped = shape(&first_table(&s));
        let a = cell_rect(&mut s, 0, 0);
        erase_at(&mut s, a.right(), a.y + 4.0);
        let t = first_table(&s);
        assert_eq!(shape(&t), shaped, "no merge (filled: {filled})");
        assert!(hidden(&t, 0, 0)[1] || hidden(&t, 0, 1)[3], "row 1 hidden (filled: {filled})");
        assert!(hidden(&t, 1, 0)[1] || hidden(&t, 1, 1)[3], "and row 2: the whole shared edge (filled: {filled})");
    }
}

/// As in Word: in a table nested in B2, a click on its vertical line merges only its cells;
/// the outer table stays.
#[test]
fn eraser_in_a_nested_table_merges_only_its_cells() {
    let mut s = table_session(3, 3, true);
    s.run("caret.set", &json!({"pos": {"story": "body", "path": [0, 1, 1, 0], "off": 0}})).unwrap();
    s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    let outer = shape(&first_table(&s));
    let n = nested_rect(&mut s, (1, 1), 0, 0);
    erase_at(&mut s, n.right(), n.y + n.h / 2.0);
    assert_eq!(shape(&nested_table(&s, 1, 1)), json!([[2], [1, 1]]), "the nested table's A1 and B1 merge");
    assert_eq!(shape(&first_table(&s)), outer, "the outer table stays");
}

/// As in Word: columns that don't line up (C3 narrower than C2) — a click on the line
/// between them merges nothing; only the shared piece hides.
#[test]
fn eraser_between_unaligned_cells_hides_only_the_shared_piece() {
    for filled in [false, true] {
        let mut s = table_session(4, 4, filled);
        let c3 = cell_rect(&mut s, 2, 2);
        let x = c3.x + c3.w / 2.0;
        pen(&mut s, (x, c3.y + 1.0), (x, c3.bottom() - 1.0)).unwrap();
        let shaped = shape(&first_table(&s));
        let c2 = cell_rect(&mut s, 1, 2);
        erase_at(&mut s, c2.x + 10.0, c2.bottom());
        let t = first_table(&s);
        assert_eq!(shape(&t), shaped, "no merge (filled: {filled})");
        assert!(t.rows.iter().all(|r| r.cells.iter().all(|c| c.props.vmerge == VMerge::None)), "filled: {filled}");
    }
}

/// As in Word: a cell holding a nested table merges with its neighbour; the nested table
/// stays in the merged cell.
#[test]
fn eraser_merges_a_cell_holding_a_nested_table() {
    let mut s = table_session(3, 3, true);
    s.run("caret.set", &json!({"pos": {"story": "body", "path": [0, 1, 1, 0], "off": 0}})).unwrap();
    s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    let b = cell_rect(&mut s, 1, 1);
    erase_at(&mut s, b.right(), b.y + 3.0);
    let t = first_table(&s);
    assert_eq!(shape(&t), json!([[1, 1, 1], [1, 2], [1, 1, 1]]));
    assert!(t.rows[1].cells[1].blocks.iter().any(|x| x.as_table().is_some()), "the nested table stays");
}

/// As in Word: the eraser works the same on a floating table.
#[test]
fn eraser_works_on_a_floating_table() {
    let mut s = table_session(3, 3, true);
    s.run("table.properties", &json!({"width": 300.0, "wrap": "around", "position": {"x": 100.0, "hRel": "page", "y": 300.0, "vRel": "page"}}))
        .unwrap();
    let c = cell_rect(&mut s, 0, 1);
    erase_at(&mut s, c.right(), c.y + c.h / 2.0);
    assert_eq!(shape(&first_table(&s)), json!([[1, 2], [1, 1, 1], [1, 1, 1]]));
}

/// Erasing every inner line of a 2×2 table leaves one cell with all four texts.
#[test]
fn eraser_every_inner_line_leaves_one_cell() {
    let mut s = table_session(2, 2, true);
    let before = all_texts(&first_table(&s));
    let a = cell_rect(&mut s, 0, 0);
    erase_at(&mut s, a.right(), a.y + a.h / 2.0);
    let a = cell_rect(&mut s, 1, 0);
    erase_at(&mut s, a.right(), a.y + a.h / 2.0);
    let a = cell_rect(&mut s, 0, 0);
    erase_at(&mut s, a.x + a.w / 2.0, a.bottom());
    let t = first_table(&s);
    assert_eq!(shape(&t), json!([[2], [2]]));
    assert_eq!((vmerges(&t, 0)[0], vmerges(&t, 1)[0]), (VMerge::Restart, VMerge::Continue));
    assert_eq!(all_texts(&t), before);
}

/// The click's reach: within it the nearest border goes; further away (the middle of a cell)
/// nothing happens and no undo step is made.
#[test]
fn eraser_click_reach() {
    let mut s = table_session(3, 3, true);
    let start = first_table(&s);
    let depth = s.undo_depth();
    let b = cell_rect(&mut s, 1, 1);
    assert!(s.run("table.eraser", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": b.y + b.h / 2.0})).is_err(), "the middle of a cell");
    assert_eq!((first_table(&s), s.undo_depth()), (start, depth), "nothing, no undo step");
    s.run("table.eraser", &json!({"page": 0, "x": b.right() - 3.0, "y": b.y + b.h / 2.0})).unwrap();
    assert_eq!(shape(&first_table(&s)), json!([[1, 1, 1], [1, 2], [1, 1, 1]]), "3 pt from the line: B2|C2");
}

// ── Eraser and pen together ─────────────────────────────────────────────────────────────────

/// Draw a line, erase it, draw it again, erase it again: each time the table matches.
#[test]
fn pen_and_eraser_back_and_forth_on_a_column_line() {
    for filled in [false, true] {
        let mut s = table_session(3, 3, filled);
        let start = first_table(&s);
        let a = cell_rect(&mut s, 0, 0);
        let x = a.x + a.w / 2.0;
        pen(&mut s, (x, a.y + 1.0), (x, a.bottom() - 1.0)).unwrap();
        let drawn = first_table(&s);
        for round in 0..2 {
            let a = cell_rect(&mut s, 0, 0);
            erase_at(&mut s, a.right(), a.y + a.h / 2.0);
            assert_eq!(cells_per_row(&first_table(&s)), cells_per_row(&start), "erased, round {round} (filled: {filled})");
            // The row may be lower now: its text has the whole width again.
            let m = cell_rect(&mut s, 0, 0);
            let r = pen(&mut s, (x, m.y + 1.0), (x, m.bottom() - 1.0));
            assert!(r.is_ok(), "drawn again, round {round} (filled: {filled}): {r:?}");
            assert_eq!(cells_per_row(&first_table(&s)), cells_per_row(&drawn), "round {round} (filled: {filled})");
        }
        assert_eq!(all_texts(&first_table(&s)), all_texts(&start), "no text lost (filled: {filled})");
    }
}

/// The same with a row line: split a row with the pen, erase the line, draw it again.
#[test]
fn pen_and_eraser_back_and_forth_on_a_row_line() {
    for filled in [false, true] {
        let mut s = table_session(2, 2, filled);
        let start = first_table(&s);
        let a = cell_rect(&mut s, 0, 0);
        let b = cell_rect(&mut s, 0, 1);
        let y = a.y + a.h / 2.0;
        pen(&mut s, (a.x + 3.0, y), (b.right() - 3.0, y)).unwrap();
        assert_eq!(first_table(&s).rows.len(), 3, "split (filled: {filled})");
        for round in 0..2 {
            let a = cell_rect(&mut s, 0, 0);
            let b = cell_rect(&mut s, 0, 1);
            let y = a.bottom();
            s.run("table.eraser", &json!({"page": 0, "x0": a.x + 3.0, "y0": y, "x1": b.right() - 3.0, "y1": y + 0.5})).unwrap();
            // Drawn where the merged cell is now: the two rows share its height.
            let m = cell_rect(&mut s, 0, 0);
            let y = m.y + m.h / 2.0;
            let r = pen(&mut s, (a.x + 3.0, y), (b.right() - 3.0, y));
            assert!(r.is_ok(), "drawn again, round {round} (filled: {filled}): {r:?}");
            let t = first_table(&s);
            assert_eq!(t.rows.len(), 3, "round {round} (filled: {filled})");
            assert_eq!(vmerges(&t, 0), [VMerge::None; 2], "round {round} (filled: {filled})");
        }
        assert_eq!(all_texts(&first_table(&s)), all_texts(&start), "no text lost (filled: {filled})");
    }
}

/// A line the pen splits a merged block with: erase B1:C2 into one cell, then draw the vertical
/// line back through both rows: B and C apart again in rows 1 and 2.
#[test]
fn pen_splits_a_block_the_eraser_made() {
    for filled in [false, true] {
        let mut s = table_session(3, 4, filled);
        let start = first_table(&s);
        let (b1, c2) = (cell_rect(&mut s, 0, 1), cell_rect(&mut s, 1, 2));
        s.run("table.eraser", &json!({"page": 0, "x0": b1.x + 8.0, "y0": b1.y + 8.0, "x1": c2.right() - 8.0, "y1": c2.bottom() - 8.0})).unwrap();
        assert_eq!(shape(&first_table(&s)), json!([[1, 2, 1], [1, 2, 1], [1, 1, 1, 1]]), "one block (filled: {filled})");
        let x = b1.right();
        let r = pen(&mut s, (x, b1.y + 1.0), (x, c2.bottom() - 1.0));
        assert!(r.is_ok(), "filled: {filled}: {r:?}");
        let t = first_table(&s);
        assert_eq!(shape(&t), shape(&start), "B and C apart again (filled: {filled})");
        assert_eq!(all_texts(&t), all_texts(&start), "no text lost (filled: {filled})");
    }
}

/// The pen undoes what the eraser does: every inside border of a 3×3 table with text, erased
/// and drawn back at the same place, leaves the table as it was (cells, merges and text).
#[test]
fn every_erased_inside_border_draws_back() {
    for vertical in [true, false] {
        for r in 0..3 {
            for c in 0..3 {
                if (vertical && c == 2) || (!vertical && r == 2) {
                    continue;
                }
                let mut s = table_session(3, 3, true);
                let start = first_table(&s);
                let a = cell_rect(&mut s, r, c);
                let (from, to) = if vertical {
                    ((a.right(), a.y + 1.0), (a.right(), a.bottom() - 1.0))
                } else {
                    ((a.x + 3.0, a.bottom()), (a.right() - 3.0, a.bottom()))
                };
                erase_at(&mut s, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
                assert_ne!(first_table(&s), start, "erased: row {r}, cell {c}, vertical {vertical}");
                let drawn = pen(&mut s, from, to);
                assert!(drawn.is_ok(), "drawn back: row {r}, cell {c}, vertical {vertical}: {drawn:?}");
                let t = first_table(&s);
                let what = format!("row {r}, cell {c}, vertical {vertical}");
                assert_eq!(cells_per_row(&t), cells_per_row(&start), "{what}");
                assert!((0..3).all(|k| vmerges(&t, k) == [VMerge::None; 3]), "no merge left: {what}");
                assert_eq!(all_texts(&t), all_texts(&start), "{what}");
            }
        }
    }
}

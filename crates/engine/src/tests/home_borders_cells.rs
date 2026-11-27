//! #7874: Home › Paragraph › Borders with whole table cells selected borders the cells as a
//! block, as Word does, instead of each paragraph in them.

use std::sync::Arc;

use serde_json::json;
use wordcraft_doc::props::{Border, Borders};
use wordcraft_doc::{Block, Cell, Document, Paragraph, Path, Pos, StoryRef, Table, para_block};

use crate::{Selection, Session};

/// A document of a 3 x 3 table with text in every cell, the caret in the first cell.
fn doc() -> Session {
    let mut t = Table::new(3, 3, 300.0);
    for (r, row) in t.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            cell.blocks = Cell::with_text(&format!("c{r}{c}")).blocks;
        }
    }
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(t)), para_block(Paragraph::new())];
    let mut s = Session::new(d);
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0]), 0));
    s
}

fn table(s: &Session) -> &Table {
    s.doc.body.iter().find_map(|b| b.as_table()).unwrap()
}

fn cell_borders(s: &Session, r: usize, c: usize) -> Borders {
    table(s).rows[r].cells[c].props.borders.unwrap_or_default()
}

/// Which sides of cell (`r`, `c`) show a line: top, left, bottom, right.
fn shown(s: &Session, r: usize, c: usize) -> [bool; 4] {
    let b = cell_borders(s, r, c);
    [b.top, b.left, b.bottom, b.right].map(|x| x.is_some_and(|x| x.is_visible()))
}

/// Whether any paragraph in the table has borders of its own.
fn para_borders(s: &Session) -> bool {
    table(s).rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.blocks).filter_map(|b| b.as_para()).any(|p| p.props.borders.is_some())
}

fn borders(s: &mut Session, kind: &str) {
    s.run("para.borders", &json!({"kind": kind})).unwrap();
}

/// Select whole cells: rows `rows` and grid columns `cols` of the table.
fn select(s: &mut Session, rows: (usize, usize), cols: (usize, usize)) {
    crate::cmd::table::select_range(s, StoryRef::Body, &Path(vec![0]), rows, cols).unwrap();
}

#[test]
fn all_borders_on_a_selected_table_borders_every_cell() {
    let mut s = doc();
    s.run("table.selectTable", &json!({})).unwrap();
    borders(&mut s, "all");
    for r in 0..3 {
        for c in 0..3 {
            assert_eq!(shown(&s, r, c), [true; 4], "cell ({r}, {c}) has a line on every side");
        }
    }
    assert!(!para_borders(&s), "the paragraphs in the cells get no borders of their own");
    // The line is the one asked for, with no paragraph spacing.
    let top = cell_borders(&s, 1, 1).top.unwrap();
    assert_eq!((top.width, top.space), (0.5, 0.0));
}

#[test]
fn outside_and_inside_borders_follow_the_selected_block() {
    let mut s = doc();
    // The top-left 2 x 2 cells.
    select(&mut s, (0, 1), (0, 1));
    borders(&mut s, "outside");
    assert_eq!(shown(&s, 0, 0), [true, true, false, false]);
    assert_eq!(shown(&s, 0, 1), [true, false, false, true]);
    assert_eq!(shown(&s, 1, 0), [false, true, true, false]);
    assert_eq!(shown(&s, 1, 1), [false, false, true, true]);
    assert_eq!(table(&s).rows[2].cells[2].props.borders, None, "a cell outside the selection is left alone");
    // The cells beyond the block share its edge lines: their facing sides get the same line.
    assert_eq!(shown(&s, 0, 2), [false, true, false, false]);
    assert_eq!(shown(&s, 2, 0), [true, false, false, false]);
    borders(&mut s, "inside");
    assert_eq!(shown(&s, 0, 0), [true; 4]);
    assert_eq!(shown(&s, 1, 1), [true; 4]);
    assert_eq!(shown(&s, 0, 2), [false, true, false, false], "Inside leaves the cells beyond the block alone");
    assert!(!para_borders(&s));
}

#[test]
fn no_border_on_a_row_clears_the_lines_it_shares_with_the_rows_around_it() {
    let mut s = doc();
    s.run("table.selectTable", &json!({})).unwrap();
    borders(&mut s, "all");
    select(&mut s, (1, 1), (0, 2));
    borders(&mut s, "none");
    for c in 0..3 {
        // Each cell draws its own sides, so the rows above and below must not keep the lines.
        assert_eq!(shown(&s, 0, c), [true, true, false, true], "the row above keeps its other lines, cell (0, {c})");
        assert_eq!(shown(&s, 1, c), [false; 4], "cell (1, {c})");
        assert_eq!(shown(&s, 2, c), [false, true, true, true], "the row below keeps its other lines, cell (2, {c})");
    }
}

#[test]
fn a_row_that_skips_grid_columns_has_its_first_cell_on_the_outside() {
    let mut s = doc();
    // The middle row starts at grid column 1 (`w:gridBefore`): two cells.
    let tb = s.doc.table_mut(StoryRef::Body, &Path(vec![0])).unwrap();
    tb.rows[1].cells.remove(0);
    tb.rows[1].props.grid_before = 1;
    s.run("table.selectTable", &json!({})).unwrap();
    borders(&mut s, "none");
    borders(&mut s, "outside");
    assert_eq!(shown(&s, 1, 0), [false, true, false, false], "the row's first cell is on the table's left edge");
    assert_eq!(shown(&s, 1, 1), [false, false, false, true]);
    borders(&mut s, "none");
    borders(&mut s, "inside");
    assert_eq!(shown(&s, 1, 0), [true, false, true, true], "Inside leaves the row's outer edge without a line");
}

#[test]
fn one_side_toggles_along_the_block_edge_and_no_border_clears_the_cells() {
    let mut s = doc();
    // The middle row.
    select(&mut s, (1, 1), (0, 2));
    borders(&mut s, "bottom");
    for c in 0..3 {
        assert_eq!(shown(&s, 1, c), [false, false, true, false], "cell (1, {c})");
    }
    borders(&mut s, "bottom");
    for c in 0..3 {
        assert_eq!(shown(&s, 1, c), [false; 4], "the second press takes the bottom line off cell (1, {c})");
    }
    borders(&mut s, "all");
    borders(&mut s, "none");
    let nil = |x: Option<Border>| x.is_some_and(|x| !x.is_visible());
    for c in 0..3 {
        let b = cell_borders(&s, 1, c);
        assert!(
            nil(b.top) && nil(b.left) && nil(b.bottom) && nil(b.right),
            "No Border writes no line on every side of cell (1, {c}), over the table's own borders"
        );
    }
}

#[test]
fn a_caret_in_a_cell_still_borders_its_paragraph() {
    let mut s = doc();
    borders(&mut s, "all");
    assert!(para_borders(&s), "without whole cells selected, the paragraph is bordered as before");
    assert_eq!(table(&s).rows[0].cells[0].props.borders, None);
}

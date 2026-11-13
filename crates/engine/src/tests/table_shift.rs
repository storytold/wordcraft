//! #6330: Insert Cells and Delete Cells with Word's four options each (shift right / down /
//! entire row / entire column; shift left / up / entire row / entire column), on the caret's
//! cell or the selected cells.

use serde_json::{Value, json};
use wordcraft_doc::{Path, StoryRef, Table};

use super::revision_checks::{caret_in, cells, cells_deleted, filled_table, run, table};
use crate::Session;

/// A 3×3 table A1 B1 C1 / A2 B2 C2 / A3 B3 C3 whose columns are 72, 144 and 216 pt wide, so a
/// cell that slides into another column shows whether it kept its own width.
fn grid3() -> Session {
    let mut s = filled_table(3, &["A1", "B1", "C1", "A2", "B2", "C2", "A3", "B3", "C3"]);
    let t = s.doc.table_mut(StoryRef::Body, &Path(vec![0])).unwrap();
    t.grid = vec![72.0, 144.0, 216.0];
    t.props.width = None;
    crate::cmd::table::sync_cell_widths(t);
    s
}

/// Selects the cells of rows `rows` and grid columns `cols` (both inclusive).
fn select(s: &mut Session, rows: (usize, usize), cols: (usize, usize)) {
    crate::cmd::table::select_range(s, StoryRef::Body, &Path(vec![0]), rows, cols).unwrap();
}

/// Each row's cell widths as the table's grid draws them.
fn widths(s: &Session) -> Vec<Vec<f32>> {
    let t = table(s).unwrap();
    t.rows
        .iter()
        .map(|row| {
            let mut g = row.first_grid();
            row.cells
                .iter()
                .map(|c| {
                    let w = t.grid[g..g + c.span()].iter().sum::<f32>();
                    g += c.span();
                    w
                })
                .collect()
        })
        .collect()
}

/// `id` with `params`, and the table it leaves (or none).
fn after(mut s: Session, id: &str, params: Value) -> (Session, Option<Table>) {
    run(&mut s, id, params);
    let t = table(&s).cloned();
    (s, t)
}

fn rows(t: &[&[&str]]) -> Vec<Vec<String>> {
    t.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect()
}

#[test]
fn shift_cells_right_moves_the_rest_of_the_row_along_with_their_widths() {
    let mut s = grid3();
    caret_in(&mut s, 1, 1, 0);
    let (s, _) = after(s, "table.insertCells", json!({"shift": "right"}));
    assert_eq!(cells(&s), rows(&[&["A1", "B1", "C1"], &["A2", "", "B2", "C2"], &["A3", "B3", "C3"]]));
    // Word adds no column: the row sticks out by the new cell, as wide as the cell it went before.
    assert_eq!(widths(&s), [vec![72.0, 144.0, 216.0], vec![72.0, 144.0, 144.0, 216.0], vec![72.0, 144.0, 216.0]]);
    assert_eq!(s.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((1, 1)), "the caret is in the new cell");

    // Several selected cells: as many new cells in each selected row, before the first of them.
    let mut s = grid3();
    select(&mut s, (0, 1), (1, 2));
    let (s, _) = after(s, "table.insertCells", json!({"shift": "right"}));
    assert_eq!(cells(&s), rows(&[&["A1", "", "", "B1", "C1"], &["A2", "", "", "B2", "C2"], &["A3", "B3", "C3"]]));
    assert_eq!(widths(&s)[0], [72.0, 144.0, 216.0, 144.0, 216.0]);
}

#[test]
fn shift_cells_down_moves_the_column_down_and_adds_rows_at_the_bottom() {
    for params in [json!({"shift": "down"}), json!({})] {
        let mut s = grid3();
        caret_in(&mut s, 1, 1, 0);
        let (s, _) = after(s, "table.insertCells", params.clone());
        assert_eq!(cells(&s), rows(&[&["A1", "B1", "C1"], &["A2", "", "C2"], &["A3", "B2", "C3"], &["", "B3", ""]]), "{params}");
        assert_eq!(widths(&s), vec![vec![72.0, 144.0, 216.0]; 4], "the grid stays");
    }
    // Two selected rows: the cells below move two rows, and two rows are added.
    let mut s = grid3();
    select(&mut s, (0, 1), (1, 1));
    let (s, _) = after(s, "table.insertCells", json!({"shift": "down"}));
    assert_eq!(cells(&s), rows(&[&["A1", "", "C1"], &["A2", "", "C2"], &["A3", "B1", "C3"], &["", "B2", ""], &["", "B3", ""]]));
}

#[test]
fn shift_cells_left_closes_the_gap_and_the_cells_keep_their_widths() {
    for params in [json!({"shift": "left"}), json!({})] {
        let mut s = grid3();
        caret_in(&mut s, 1, 1, 0);
        let (s, _) = after(s, "table.deleteCells", params.clone());
        assert_eq!(cells(&s), rows(&[&["A1", "B1", "C1"], &["A2", "C2"], &["A3", "B3", "C3"]]), "{params}");
        // C2 keeps its 216 pt: the row gets shorter rather than C2 taking B's column.
        assert_eq!(widths(&s), [vec![72.0, 144.0, 216.0], vec![72.0, 216.0], vec![72.0, 144.0, 216.0]], "{params}");
    }
    let mut s = grid3();
    select(&mut s, (1, 2), (0, 1));
    let (s, _) = after(s, "table.deleteCells", json!({"shift": "left"}));
    assert_eq!(cells(&s), rows(&[&["A1", "B1", "C1"], &["C2"], &["C3"]]));
    assert_eq!(widths(&s)[1..], [vec![216.0], vec![216.0]]);
}

#[test]
fn shift_cells_left_over_whole_rows_deletes_them_and_over_everything_the_table() {
    let mut s = grid3();
    select(&mut s, (1, 1), (0, 2));
    let (s, _) = after(s, "table.deleteCells", json!({"shift": "left"}));
    assert_eq!(cells(&s), rows(&[&["A1", "B1", "C1"], &["A3", "B3", "C3"]]));

    let mut s = grid3();
    select(&mut s, (0, 2), (0, 2));
    let (s, t) = after(s, "table.deleteCells", json!({"shift": "left"}));
    assert!(t.is_none(), "every cell went, so the table did");
    assert!(s.doc.body.iter().all(|b| b.as_table().is_none()));
}

#[test]
fn shift_cells_up_pulls_the_column_up_and_blanks_its_bottom() {
    let mut s = grid3();
    caret_in(&mut s, 0, 1, 0);
    let (s, _) = after(s, "table.deleteCells", json!({"shift": "up"}));
    assert_eq!(cells(&s), rows(&[&["A1", "B2", "C1"], &["A2", "B3", "C2"], &["A3", "", "C3"]]));
    let mut s = grid3();
    select(&mut s, (0, 1), (1, 2));
    let (s, _) = after(s, "table.deleteCells", json!({"shift": "up"}));
    assert_eq!(cells(&s), rows(&[&["A1", "B3", "C3"], &["A2", "", ""], &["A3", "", ""]]));
    assert_eq!(widths(&s), vec![vec![72.0, 144.0, 216.0]; 3], "the grid stays");
}

#[test]
fn entire_row_and_column_options_insert_and_delete_whole_rows_and_columns() {
    let mut s = grid3();
    caret_in(&mut s, 1, 1, 0);
    run(&mut s, "table.insertCells", json!({"shift": "row"}));
    assert_eq!(cells(&s), rows(&[&["A1", "B1", "C1"], &["", "", ""], &["A2", "B2", "C2"], &["A3", "B3", "C3"]]), "a row above");

    // As many columns as the selection covers, to its left.
    let mut s = grid3();
    select(&mut s, (0, 0), (1, 2));
    run(&mut s, "table.insertCells", json!({"shift": "column"}));
    assert_eq!(cells(&s)[0], ["A1", "", "", "B1", "C1"]);
    assert_eq!(table(&s).unwrap().grid.len(), 5);

    let mut s = grid3();
    caret_in(&mut s, 1, 1, 0);
    run(&mut s, "table.deleteCells", json!({"shift": "row"}));
    assert_eq!(cells(&s), rows(&[&["A1", "B1", "C1"], &["A3", "B3", "C3"]]));
    let mut s = grid3();
    caret_in(&mut s, 1, 1, 0);
    run(&mut s, "table.deleteCells", json!({"shift": "column"}));
    assert_eq!(cells(&s), rows(&[&["A1", "C1"], &["A2", "C2"], &["A3", "C3"]]));
}

#[test]
fn each_option_is_one_undo_step_and_redoes() {
    for (id, shift) in [
        ("table.insertCells", "right"),
        ("table.insertCells", "down"),
        ("table.insertCells", "row"),
        ("table.insertCells", "column"),
        ("table.deleteCells", "left"),
        ("table.deleteCells", "up"),
        ("table.deleteCells", "row"),
        ("table.deleteCells", "column"),
    ] {
        let mut s = grid3();
        caret_in(&mut s, 1, 1, 0);
        let before = table(&s).cloned();
        run(&mut s, id, json!({"shift": shift}));
        let done = table(&s).cloned();
        assert_ne!(done, before, "{id} {shift} changed nothing");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(table(&s).cloned(), before, "{id} {shift}: undo");
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(table(&s).cloned(), done, "{id} {shift}: redo");
    }
}

/// [`grid3`] with B1 and B2 merged into one cell.
fn merged_b() -> Session {
    let mut s = grid3();
    select(&mut s, (0, 1), (1, 1));
    run(&mut s, "table.merge", json!({}));
    s
}

#[test]
fn shifts_through_a_vertically_merged_cell_are_refused_and_change_nothing() {
    // Each would move part of the merged B1:B2 (or the cells of its rows) and break it.
    for (id, shift, r, c) in [
        ("table.insertCells", "right", 0, 0),
        ("table.insertCells", "down", 0, 1),
        ("table.deleteCells", "left", 1, 0),
        ("table.deleteCells", "up", 0, 1),
    ] {
        let mut s = merged_b();
        caret_in(&mut s, r, c, 0);
        let before = s.doc.clone();
        let depth = s.undo_depth();
        assert!(s.run(id, &json!({"shift": shift})).is_err(), "{id} {shift}");
        assert_eq!(s.doc, before, "{id} {shift} changed the document");
        assert_eq!(s.undo_depth(), depth, "{id} {shift} left an undo step");
        assert!(s.status.contains("vertically merged"), "{id} {shift}: {}", s.status);
    }
    // Cells clear of the merge still shift: C1 and C2 move down past it.
    let mut s = merged_b();
    caret_in(&mut s, 0, 2, 0);
    run(&mut s, "table.insertCells", json!({"shift": "down"}));
    assert_eq!(cells(&s).iter().map(|r| r.last().cloned().unwrap_or_default()).collect::<Vec<_>>(), ["", "C1", "C2", "C3"]);
}

#[test]
fn an_unknown_shift_is_refused() {
    for id in ["table.insertCells", "table.deleteCells"] {
        let mut s = grid3();
        let before = s.doc.clone();
        assert!(s.run(id, &json!({"shift": "sideways"})).is_err(), "{id}");
        assert_eq!(s.doc, before, "{id}");
    }
}

#[test]
fn tracked_shift_up_marks_the_text_deleted_and_keeps_the_cells() {
    let mut s = grid3();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    caret_in(&mut s, 0, 1, 0);
    run(&mut s, "table.deleteCells", json!({"shift": "up"}));
    // As the other table deletes under Track Changes: the structure stays, B1's text is marked.
    assert_eq!(cells(&s), rows(&[&["A1", "B1", "C1"], &["A2", "B2", "C2"], &["A3", "B3", "C3"]]));
    assert!(cells_deleted(&s, &[(0, 1)]));
    assert!(!cells_deleted(&s, &[(1, 1)]));
    run(&mut s, "review.rejectAll", json!({}));
    assert!(!cells_deleted(&s, &[(0, 1)]) && s.doc.revisions.is_empty());
}

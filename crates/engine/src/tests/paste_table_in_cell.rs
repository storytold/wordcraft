//! #6739: a copied table pasted with the caret in a table cell does what Word's Paste does (rows
//! inserted above, or cells overwritten from the caret's), and nests only when asked to.

use std::sync::Arc;

use serde_json::json;
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::{Block, Cell, Document, Paragraph, Path, Pos, StoryRef, Table, para_block};

use crate::cmd::table::cell_text;
use crate::{Selection, Session};

/// A table of `rows` x `cols` cells whose texts are `<prefix><row><col>` (from 1), 100 pt wide cells.
fn table(prefix: &str, rows: usize, cols: usize) -> Table {
    let mut t = Table::new(rows, cols, 100.0 * cols as f32);
    for (r, row) in t.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            cell.blocks = Cell::with_text(&format!("{prefix}{}{}", r + 1, c + 1)).blocks;
        }
    }
    t
}

/// A document of the 3 x 3 table d11..d33 and an empty paragraph, the caret in cell (`row`, `col`).
fn dest(row: usize, col: usize) -> Session {
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(table("d", 3, 3))), para_block(Paragraph::new())];
    let mut s = Session::new(d);
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, row as u32, col as u32, 0]), 0));
    s
}

fn copy(s: &mut Session, t: Table) {
    s.clipboard = Some(Fragment { blocks: vec![Block::Table(t)], ..Default::default() });
}

fn paste(s: &mut Session) {
    s.run("edit.paste", &json!({})).unwrap();
}

fn t0(s: &Session) -> &Table {
    s.doc.body[0].as_table().expect("the table")
}

/// The texts of the table's rows, a nested table shown as `[nested]`.
fn grid(t: &Table) -> Vec<Vec<String>> {
    t.rows
        .iter()
        .map(|r| r.cells.iter().map(|c| if c.blocks.iter().any(|b| b.as_table().is_some()) { "[nested]".into() } else { cell_text(c) }).collect())
        .collect()
}

fn rows(spec: &[&str]) -> Vec<Vec<String>> {
    spec.iter().map(|r| r.split(',').map(str::to_string).collect()).collect()
}

fn nested(t: &Table) -> bool {
    t.rows.iter().flat_map(|r| &r.cells).any(|c| c.blocks.iter().any(|b| b.as_table().is_some()))
}

#[test]
fn paste_all_keeps_tables_in_copy_order_inside_the_cell() {
    for col in [0, 1] {
        let mut s = dest(1, col);
        for prefix in ["first", "second"] {
            copy(&mut s, table(prefix, 1, 3));
            let frag = s.clipboard.as_ref().unwrap();
            s.clip_history.push(frag, &frag.clipboard_text());
        }
        s.run("edit.pasteAllClipboard", &json!({})).unwrap();
        let pasted: Vec<_> = t0(&s).rows[1].cells[col].blocks.iter().filter_map(|b| b.as_table()).map(|t| cell_text(&t.rows[0].cells[0])).collect();
        assert_eq!(pasted, ["first11", "second11"]);
    }
}

#[test]
fn a_copied_table_over_selected_cells_overwrites_from_the_top_left() {
    for (source_cols, expected) in
        [(2, rows(&["s11,s12,d13", "s21,s22,d23", "d31,d32,d33"])), (3, rows(&["s11,s12,s13", "s21,s22,s23", "d31,d32,d33"]))]
    {
        let mut s = dest(1, 1);
        crate::cmd::table::extend_selection(&mut s, Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0]), 0));
        let range = s.cell_range().unwrap();
        assert_eq!((range.rows, range.cols, range.stream), ((0, 1), (0, 1), false));
        copy(&mut s, table("s", 2, source_cols));
        paste(&mut s);
        assert_eq!(grid(t0(&s)), expected, "source width {source_cols}");
        assert_eq!(s.sel, Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0]), 0)));
        assert!(s.cell_range().is_none());

        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(grid(t0(&s)), grid(&table("d", 3, 3)));
        s.run("edit.redo", &json!({})).unwrap();
        assert_eq!(grid(t0(&s)), expected);
        let reopened = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
        assert_eq!(grid(reopened.body[0].as_table().unwrap()), expected);
    }
}

#[test]
fn plain_text_paste_still_replaces_a_cell_rectangle() {
    let mut s = dest(0, 0);
    crate::cmd::table::extend_selection(&mut s, Pos::new(StoryRef::Body, Path(vec![0, 1, 1, 0]), 0));
    s.run("edit.paste", &json!({"text": "replacement"})).unwrap();
    assert_eq!(grid(t0(&s)), rows(&["replacement,,d13", ",,d23", "d31,d32,d33"]));
}

#[test]
fn a_whole_table_with_the_caret_in_the_first_column_inserts_its_rows_above() {
    for (row, want) in [
        (1, rows(&["d11,d12,d13", "s11,s12,s13", "s21,s22,s23", "s31,s32,s33", "d21,d22,d23", "d31,d32,d33"])),
        (0, rows(&["s11,s12,s13", "s21,s22,s23", "s31,s32,s33", "d11,d12,d13", "d21,d22,d23", "d31,d32,d33"])),
    ] {
        let mut s = dest(row, 0);
        copy(&mut s, table("s", 3, 3));
        paste(&mut s);
        assert_eq!(grid(t0(&s)), want, "caret in row {row}");
        assert!(!nested(t0(&s)));
        assert_eq!(s.doc.body.iter().filter(|b| b.as_table().is_some()).count(), 1);
    }
}

#[test]
fn a_whole_table_with_the_caret_further_right_overwrites_cells_and_grows_the_table() {
    let mut s = dest(1, 2);
    copy(&mut s, table("s", 3, 3));
    paste(&mut s);
    let want = rows(&["d11,d12,d13,,", "d21,d22,s11,s12,s13", "d31,d32,s21,s22,s23", ",,s31,s32,s33"]);
    assert_eq!(grid(t0(&s)), want);
    assert_eq!(t0(&s).grid.len(), 5);
    assert!(t0(&s).rows.iter().all(|r| r.cells.len() == 5));
    // The last cell of the last row: the table grows two columns and two rows.
    let mut s = dest(2, 2);
    copy(&mut s, table("s", 3, 3));
    paste(&mut s);
    let want = rows(&["d11,d12,d13,,", "d21,d22,d23,,", "d31,d32,s11,s12,s13", ",,s21,s22,s23", ",,s31,s32,s33"]);
    assert_eq!(grid(t0(&s)), want);
}

#[test]
fn cells_copied_from_part_of_a_table_overwrite_from_the_caret_in_any_column() {
    // Two cells of one column (Word: s12, s22 into the cells below the caret).
    let mut s = dest(1, 0);
    copy(&mut s, table("s", 2, 1));
    paste(&mut s);
    assert_eq!(grid(t0(&s)), rows(&["d11,d12,d13", "s11,d22,d23", "s21,d32,d33"]));
    // At the last row a row is added.
    let mut s = dest(2, 0);
    copy(&mut s, table("s", 2, 1));
    paste(&mut s);
    assert_eq!(grid(t0(&s)), rows(&["d11,d12,d13", "d21,d22,d23", "s11,d32,d33", "s21,,"]));
    // Two columns wide, from the middle column: it fits the table.
    let mut s = dest(0, 1);
    copy(&mut s, table("s", 2, 2));
    paste(&mut s);
    assert_eq!(grid(t0(&s)), rows(&["d11,s11,s12", "d21,s21,s22", "d31,d32,d33"]));
}

#[test]
fn one_copied_cell_replaces_the_cell_it_is_pasted_in() {
    for (r, c) in [(0, 0), (1, 2), (2, 2)] {
        let mut s = dest(r, c);
        copy(&mut s, table("s", 1, 1));
        paste(&mut s);
        let mut want = grid(&table("d", 3, 3));
        want[r][c] = "s11".into();
        assert_eq!(grid(t0(&s)), want, "cell ({r}, {c})");
    }
}

#[test]
fn the_caret_goes_to_the_first_pasted_cell() {
    let mut s = dest(1, 0);
    copy(&mut s, table("s", 3, 3));
    paste(&mut s);
    assert_eq!(s.sel.focus.path, Path(vec![0, 1, 0, 0]));
    assert!(s.sel.is_collapsed());
}

#[test]
fn paste_as_nested_table_nests_the_table_in_the_cell() {
    let mut s = dest(1, 0);
    copy(&mut s, table("s", 3, 3));
    s.run("edit.pasteNestedTable", &json!({})).unwrap();
    let t = t0(&s);
    assert_eq!(t.rows.len(), 3, "no rows inserted");
    assert!(t.rows[1].cells[0].blocks.iter().any(|b| b.as_table().is_some()), "a table nests in the cell");
    assert_eq!(cell_text(&t.rows[1].cells[1]), "d22");
}

#[test]
fn pasting_a_wrapped_table_keeps_its_source_controls() {
    use wordcraft_doc::{ContentControl, ControlKind};

    for on_row in [false, true] {
        // A same-width copy can insert rows at column zero or overwrite cells further right.
        for col in [0, 1] {
            let mut s = dest(1, col);
            let mut source = table("s", 2, 3);
            let controls = if on_row { &mut source.rows[0].controls } else { &mut source.controls };
            let mut wrapper = ContentControl::new(ControlKind::RichText);
            wrapper.tag = "source wrapper".into();
            controls.open.push(wrapper);
            controls.close = 1;
            let expected = controls.clone();

            copy(&mut s, source);
            paste(&mut s);

            let pasted = t0(&s).rows[1].cells[col]
                .blocks
                .iter()
                .find_map(|b| b.as_table())
                .unwrap_or_else(|| panic!("the wrapped source should nest (row wrapper: {on_row}, column: {col})"));
            let controls = if on_row { &pasted.rows[0].controls } else { &pasted.controls };
            assert_eq!(controls, &expected, "row wrapper: {on_row}, column: {col}");
        }
    }
}

#[test]
fn what_cannot_be_laid_over_the_cells_nests_as_before() {
    let nests = |s: &mut Session| {
        paste(s);
        assert!(nested(t0(s)), "nested");
        assert_eq!(t0(s).rows.len(), 3);
    };
    // Merged cells in the copy.
    let mut merged = table("s", 2, 3);
    merged.rows[0].cells[0].props.span = 2;
    merged.rows[0].cells.remove(1);
    let mut s = dest(1, 0);
    copy(&mut s, merged);
    nests(&mut s);
    // Ragged rows in the copy.
    let mut ragged = table("s", 2, 3);
    ragged.rows[1].cells.pop();
    let mut s = dest(1, 0);
    copy(&mut s, ragged);
    nests(&mut s);
    // More columns than a table may have.
    let mut s = dest(1, 2);
    copy(&mut s, table("s", 1, 62));
    nests(&mut s);
    // A table inside a copied cell.
    let mut inner = table("s", 1, 1);
    inner.rows[0].cells[0].blocks = vec![Arc::new(Block::Table(table("n", 1, 1)))];
    let mut s = dest(1, 0);
    copy(&mut s, inner);
    nests(&mut s);
    // Merged cells in the destination.
    let mut s = dest(1, 0);
    if let Some(Block::Table(t)) = s.doc.body.get_mut(0).map(Arc::make_mut) {
        t.rows[1].cells[0].props.span = 2;
        t.rows[1].cells.remove(1);
    }
    copy(&mut s, table("s", 1, 1));
    nests(&mut s);
    // A copied cell without any block (hostile fragment): nothing to lay over the cell.
    let mut hollow = table("s", 1, 1);
    hollow.rows[0].cells[0].blocks.clear();
    let mut s = dest(1, 0);
    copy(&mut s, hollow);
    let _ = s.run("edit.paste", &json!({}));
    assert_eq!(t0(&s).rows.len(), 3, "nothing laid over the cells");
    // A wider copy into a one-column table (a layout wrapper) nests instead of widening it.
    let mut s = dest(1, 0);
    s.doc.body[0] = Arc::new(Block::Table(table("w", 3, 1)));
    copy(&mut s, table("s", 2, 3));
    paste(&mut s);
    assert!(nested(t0(&s)));
    assert_eq!(t0(&s).grid.len(), 1);
    // A content control in a cell the copy would overwrite.
    let mut s = dest(1, 0);
    if let Some(Block::Table(t)) = s.doc.body.get_mut(0).map(Arc::make_mut) {
        t.rows[2].cells[0].controls.close = 1;
    }
    copy(&mut s, table("s", 3, 1));
    nests(&mut s);
    // Row exceptions on the destination.
    let mut s = dest(1, 0);
    if let Some(Block::Table(t)) = s.doc.body.get_mut(0).map(Arc::make_mut) {
        t.rows[0].props.exceptions = Some(Default::default());
    }
    copy(&mut s, table("s", 1, 1));
    nests(&mut s);
    // Track Changes on.
    let mut s = dest(1, 0);
    s.doc.settings.track_changes = true;
    copy(&mut s, table("s", 3, 3));
    nests(&mut s);
}

#[test]
fn hostile_shapes_never_panic() {
    // An empty table, a table with empty rows, 30,000 columns and 30,000 rows in a copy.
    let mut wide = table("s", 1, 1);
    wide.grid = vec![1.0; 30_000];
    wide.rows[0].cells = (0..30_000).map(|_| Cell::empty()).collect();
    let mut tall = table("s", 1, 2);
    tall.rows = (0..30_000).map(|_| tall.rows[0].clone()).collect();
    let mut no_rows = table("s", 1, 1);
    no_rows.rows.clear();
    let mut no_cells = table("s", 1, 1);
    no_cells.rows[0].cells.clear();
    // 30,000 columns and empty tables can't be laid over the cells: they nest (the old path).
    for t in [wide, no_rows, no_cells] {
        for (r, c) in [(0, 0), (2, 2)] {
            let mut s = dest(r, c);
            copy(&mut s, t.clone());
            let _ = s.run("edit.paste", &json!({}));
            assert_eq!((t0(&s).rows.len(), t0(&s).grid.len()), (3, 3), "unchanged table");
        }
    }
    // 30,000 rows are laid over the cells: rows are added up to the limit, never past it.
    for (r, c) in [(0, 0), (2, 1)] {
        let mut s = dest(r, c);
        copy(&mut s, tall.clone());
        s.run("edit.paste", &json!({})).unwrap();
        assert_eq!(t0(&s).rows.len(), r + 30_000);
        assert!(t0(&s).rows.iter().all(|row| row.cells.len() == t0(&s).grid.len()));
    }
    // More rows than a table may have nest instead.
    let mut huge = table("s", 1, 2);
    huge.rows = (0..wordcraft_doc::table::MAX_ROWS).map(|_| huge.rows[0].clone()).collect();
    let mut s = dest(1, 1);
    copy(&mut s, huge);
    let _ = s.run("edit.paste", &json!({}));
    assert_eq!(t0(&s).rows.len(), 3);
    // A caret path that is not a cell of the table.
    let mut s = dest(0, 0);
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 9, 9, 0]), 0));
    copy(&mut s, table("s", 1, 1));
    let _ = s.run("edit.paste", &json!({}));
    assert_eq!(t0(&s).rows.len(), 3);
}

#[test]
fn a_copied_table_with_empty_paragraphs_around_it_is_laid_over_the_cells_too() {
    let mut s = dest(1, 0);
    s.clipboard = Some(Fragment {
        blocks: vec![Block::Para(Paragraph::new()), Block::Table(table("s", 1, 3)), Block::Para(Paragraph::new())],
        ..Default::default()
    });
    paste(&mut s);
    assert_eq!(grid(t0(&s)), rows(&["d11,d12,d13", "s11,s12,s13", "d21,d22,d23", "d31,d32,d33"]));
}

#[test]
fn undo_takes_the_paste_back_in_one_step_and_redo_does_it_again() {
    for (row, col) in [(1, 0), (1, 2)] {
        let mut s = dest(row, col);
        let before = t0(&s).clone();
        copy(&mut s, table("s", 3, 3));
        paste(&mut s);
        let after = t0(&s).clone();
        assert_ne!(after, before);
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(t0(&s), &before, "one undo restores the table ({row}, {col})");
        s.run("edit.redo", &json!({})).unwrap();
        assert_eq!(t0(&s), &after);
    }
}

#[test]
fn the_pasted_rows_survive_saving_and_reopening_as_docx() {
    let mut s = dest(1, 0);
    copy(&mut s, table("s", 3, 3));
    paste(&mut s);
    let bytes = crate::io::save_bytes("t.docx", &s.doc).unwrap();
    let back = crate::io::open_bytes("t.docx", &bytes).unwrap();
    let t = back.body.iter().find_map(|b| b.as_table()).expect("a table");
    assert_eq!(grid(t), grid(t0(&s)));
    let mut s = dest(1, 2);
    copy(&mut s, table("s", 3, 3));
    paste(&mut s);
    let back = crate::io::open_bytes("t.docx", &crate::io::save_bytes("t.docx", &s.doc).unwrap()).unwrap();
    let t = back.body.iter().find_map(|b| b.as_table()).expect("a table");
    assert_eq!((t.rows.len(), t.grid.len()), (4, 5));
    assert_eq!(grid(t), grid(t0(&s)));
}

#[test]
fn a_paste_into_a_cell_of_a_table_inside_a_cell_works_on_that_table() {
    let mut inner_host = table("o", 1, 1);
    inner_host.rows[0].cells[0].blocks = vec![Arc::new(Block::Table(table("d", 3, 3))), para_block(Paragraph::new())];
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(inner_host)), para_block(Paragraph::new())];
    let mut s = Session::new(d);
    // Body table 0, row 0, cell 0, its block 0 (the inner table), row 1, cell 0, block 0.
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0, 1, 0, 0]), 0));
    copy(&mut s, table("s", 3, 3));
    paste(&mut s);
    let inner = t0(&s).rows[0].cells[0].blocks[0].as_table().expect("the inner table").clone();
    assert_eq!(grid(&inner).len(), 6);
    assert!(!nested(&inner));
}

/// A locked control in a table nested in a cell the copy would overwrite is not deleted: the
/// paste nests instead (#6739 review).
#[test]
fn a_locked_control_in_a_nested_table_of_an_overwritten_cell_is_kept() {
    use wordcraft_doc::{ContentControl, ControlKind, ControlLock};
    let mut s = dest(0, 0);
    let mut inner = table("n", 1, 1);
    let mut ctl = ContentControl::new(ControlKind::RichText);
    ctl.lock = ControlLock::SdtContentLocked;
    inner.rows[0].cells[0].controls.open.push(ctl);
    inner.rows[0].cells[0].controls.close = 1;
    if let Some(Block::Table(t)) = s.doc.body.get_mut(0).map(Arc::make_mut) {
        t.rows[1].cells[0].blocks = vec![Arc::new(Block::Table(inner)), para_block(Paragraph::new())];
    }
    let before = t0(&s).rows[1].cells[0].clone();
    // Copy one cell, caret above the cell with the nested table: it would be overwritten.
    copy(&mut s, table("s", 2, 1));
    paste(&mut s);
    let kept = |t: &Table| t.rows.iter().flat_map(|r| &r.cells).any(|c| c.blocks == before.blocks);
    assert!(kept(t0(&s)), "the cell with the locked control survives");
}

/// A caret in a control that shows its placeholder: no rows are inserted above the caret's row;
/// the table nests in the cell as the ordinary paste does (#6739 review).
#[test]
fn a_control_showing_its_placeholder_gets_the_ordinary_paste_not_inserted_rows() {
    let mut s = dest(1, 0);
    s.run("developer.plainText", &json!({})).unwrap();
    let before = s.doc.body[0].as_table().map(|t| t.rows.len());
    copy(&mut s, table("s", 3, 3));
    paste(&mut s);
    assert_eq!(t0(&s).rows.len(), before.unwrap_or(0), "no rows inserted above the caret's row");
    assert!(nested(t0(&s)), "the table nests in the cell");
}

#[test]
fn paste_from_before_a_block_control_replaces_its_placeholder() {
    let mut s = dest(1, 0);
    let start = s.sel.focus.clone();
    s.run("developer.plainText", &json!({})).unwrap();
    let range = s.doc.control_at(&s.sel.focus).unwrap();
    assert_eq!(range.start, start);
    assert!(range.control.showing_placeholder);
    let placeholder = s.doc.para_at(&start).unwrap().text[range.content_start().off..range.end.off].to_owned();
    assert!(!placeholder.is_empty());
    s.doc.control_mut(&start).unwrap().block = true;
    s.sel = Selection::caret(start);
    copy(&mut s, table("s", 1, 3));
    paste(&mut s);
    assert_eq!(t0(&s).rows.len(), 3);
    assert!(nested(t0(&s)));
    assert!(!cell_text(&t0(&s).rows[1].cells[0]).contains(&placeholder));
}

/// A locked control in a text box anchored in a cell the copy would overwrite is not lost: the
/// paste nests instead (#6739 review).
#[test]
fn a_locked_control_in_a_text_box_of_an_overwritten_cell_is_kept() {
    use wordcraft_doc::para::ShapeKind;
    use wordcraft_doc::{ContentControl, ControlKind, ControlLock, InlineObject, PartKind};
    let mut s = dest(0, 0);
    let mut ctl = ContentControl::new(ControlKind::RichText);
    ctl.lock = ControlLock::SdtContentLocked;
    let mut inner = Paragraph::with_text("Locked", Default::default());
    inner.insert_object(0, InlineObject::ControlStart { control: Box::new(ctl) }, &Default::default()).unwrap();
    let end = inner.text.chars().count();
    inner.insert_object(end, InlineObject::ControlEnd, &Default::default()).unwrap();
    let id = s.doc.add_part(PartKind::TextBox, vec![para_block(inner)]);
    let shape = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 72.0,
        h: 36.0,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story: Some(id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let mut anchor = Paragraph::with_text("Box", Default::default());
    anchor.insert_object(0, shape, &Default::default()).unwrap();
    if let Some(Block::Table(t)) = s.doc.body.get_mut(0).map(Arc::make_mut) {
        t.rows[1].cells[0].blocks = vec![para_block(anchor)];
    }
    let before = t0(&s).rows[1].cells[0].blocks.clone();
    // One column of two cells from the caret in the top cell: the cell with the box is overwritten.
    copy(&mut s, table("s", 2, 1));
    paste(&mut s);
    let kept = |t: &Table| t.rows.iter().flat_map(|r| &r.cells).any(|c| c.blocks == before);
    assert!(kept(t0(&s)), "the cell anchoring the text box survives");
    assert!(nested(t0(&s)), "the table nests in the cell instead");
}

/// A row-level content control on a row the copy would overwrite is not bypassed: the paste nests
/// instead (#6739 review).
#[test]
fn a_row_level_control_on_an_overwritten_row_declines_the_overwrite() {
    use wordcraft_doc::{ContentControl, ControlKind, ControlLock};
    let mut s = dest(0, 0);
    let mut ctl = ContentControl::new(ControlKind::RichText);
    ctl.lock = ControlLock::SdtContentLocked;
    if let Some(Block::Table(t)) = s.doc.body.get_mut(0).map(Arc::make_mut) {
        t.rows[1].controls.open.push(ctl);
        t.rows[1].controls.close = 1;
    }
    let before = grid(t0(&s));
    copy(&mut s, table("s", 2, 1));
    paste(&mut s);
    assert_eq!(grid(t0(&s))[1], before[1], "the row under the control is not overwritten");
    assert!(nested(t0(&s)), "the table nests in the cell instead");
}

#[test]
fn an_overwrite_inside_a_spanning_row_control_preserves_the_row() {
    use wordcraft_doc::{ContentControl, ControlKind, ControlLock};

    let mut s = dest(1, 1);
    let mut control = ContentControl::new(ControlKind::RichText);
    control.lock = ControlLock::SdtContentLocked;
    let destination = s.doc.table_mut(StoryRef::Body, &Path(vec![0])).unwrap();
    destination.rows[0].controls.open.push(control);
    destination.rows[2].controls.close = 1;
    copy(&mut s, table("s", 1, 1));
    paste(&mut s);

    let table = t0(&s);
    let cell = &table.rows[1].cells[1];
    assert!(cell_text(cell).lines().any(|line| line == "d22"));
    assert!(cell.blocks.iter().any(|block| block.as_table().is_some()));
    assert_eq!(table.rows[0].controls.open[0].lock, ControlLock::SdtContentLocked);
    assert_eq!(table.rows[2].controls.close, 1);
}

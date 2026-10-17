use serde_json::{Value, json};
use wordcraft_doc::props::{BorderStyle, Rgb};
use wordcraft_doc::{Document, Path, Pos, StoryRef, Table};

use crate::{Selection, Session};

fn run(s: &mut Session, id: &str, params: Value) {
    s.run(id, &params).unwrap();
}

fn table(s: &Session) -> &Table {
    s.doc.body.iter().find_map(|b| b.as_table()).unwrap()
}

fn in_table() -> Session {
    let mut s = Session::new(Document::new());
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 3}));
    let mut path = s.sel.focus.path.cell().unwrap().0.0;
    path.extend([1, 1, 0]);
    s.sel = Selection::caret(Pos { story: StoryRef::Body, path: Path(path), off: 0 });
    s
}

fn no_paragraph_borders(s: &Session) {
    for path in s.doc.para_paths(StoryRef::Body) {
        assert!(s.doc.para(StoryRef::Body, &path).unwrap().props.borders.is_none());
    }
}

#[test]
fn home_borders_follow_table_design_for_caret_and_selected_cells() {
    for selection in [None, Some("table.selectCell"), Some("table.selectRow"), Some("table.selectColumn")] {
        for kind in ["bottom", "top", "left", "right", "none", "all", "outside", "inside"] {
            let mut home = in_table();
            let mut design = in_table();
            if let Some(id) = selection {
                run(&mut home, id, json!({}));
                run(&mut design, id, json!({}));
            }
            let before = table(&home).clone();
            let selection_before = home.sel.clone();
            let depth = home.undo_depth();
            let params = json!({"kind": kind, "width": 2.25, "color": "1256AB", "style": "double"});
            run(&mut home, "para.borders", params.clone());
            run(&mut design, "table.borders", params);
            assert_eq!(table(&home), table(&design), "{selection:?}: {kind}");
            assert_eq!(home.sel, selection_before);
            no_paragraph_borders(&home);
            assert_eq!(home.undo_depth(), depth + 1, "one undo step");
            let after = table(&home).clone();
            run(&mut home, "edit.undo", json!({}));
            assert_eq!(table(&home), &before);
            run(&mut home, "edit.redo", json!({}));
            assert_eq!(table(&home), &after);
        }
    }
}

#[test]
fn home_bottom_border_marks_b2_and_its_neighbour_and_round_trips() {
    let mut s = in_table();
    // No kind means Bottom for Home, whereas Table Design defaults to All.
    run(&mut s, "para.borders", json!({"width": 1.5, "color": "1256AB", "style": "double"}));
    let t = table(&s);
    assert!(t.props.borders.is_none());
    let bottom = t.rows[1].cells[1].props.borders.unwrap();
    let line = bottom.bottom.unwrap();
    assert_eq!((line.style, line.width, line.color), (BorderStyle::Double, 1.5, Some(Rgb(0x12, 0x56, 0xAB))));
    assert!(bottom.top.is_none() && bottom.left.is_none() && bottom.right.is_none());
    assert_eq!(t.rows[2].cells[1].props.borders.unwrap().top, Some(line));
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            if (r, c) != (1, 1) && (r, c) != (2, 1) {
                assert!(cell.props.borders.is_none(), "unrelated cell {r},{c}");
            }
        }
    }
    no_paragraph_borders(&s);
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let back = Session::new(wordcraft_docx::read(&bytes).unwrap());
    no_paragraph_borders(&back);
    assert_eq!(table(&back).rows[1].cells[1].props.borders.unwrap().bottom, Some(line));
}

#[test]
fn home_borders_keep_explicit_paragraph_cases() {
    for params in [json!({"kind": "horizontalLine"}), json!({"kind": "all", "sides": {"bottom": {"width": 2}}})] {
        let mut s = in_table();
        let before = table(&s).clone();
        run(&mut s, "para.borders", params);
        assert!(s.doc.para_at(&s.sel.focus).unwrap().props.borders.unwrap().bottom.is_some());
        assert_eq!(table(&s).props, before.props);
        for (row, old_row) in table(&s).rows.iter().zip(&before.rows) {
            for (cell, old_cell) in row.cells.iter().zip(&old_row.cells) {
                assert_eq!(cell.props.borders, old_cell.props.borders);
            }
        }
    }
    let mut s = Session::new(Document::new());
    run(&mut s, "para.borders", json!({"kind": "outside"}));
    let b = s.doc.para_at(&s.sel.focus).unwrap().props.borders.unwrap();
    assert!(b.top.is_some() && b.bottom.is_some() && b.left.is_some() && b.right.is_some());
    run(&mut s, "edit.undo", json!({}));
    no_paragraph_borders(&s);
}

#[test]
fn home_borders_keep_text_selection_within_one_cell_as_paragraph_formatting() {
    // Preserve the existing text-only path until this Word behavior is observed.
    let mut s = in_table();
    run(&mut s, "text.insert", json!({"text": "alpha beta"}));
    s.sel.anchor = Pos { off: 0, ..s.sel.focus.clone() };
    s.sel.focus.off = 5;
    run(&mut s, "para.borders", json!({"kind": "bottom"}));
    assert!(s.doc.para_at(&s.sel.focus).unwrap().props.borders.unwrap().bottom.is_some());
    assert!(table(&s).rows.iter().flat_map(|r| &r.cells).all(|c| c.props.borders.is_none()));
}

#[test]
fn home_borders_hostile_params_in_a_cell_never_panic() {
    for params in
        [json!(null), json!(true), json!(5), json!([]), json!("bottom"), json!({"width": -1e30}), json!({"color": 123}), json!({"kind": "bogus"})]
    {
        let mut s = in_table();
        if let Err(e) = s.run("para.borders", &params) {
            assert!(!e.to_string().contains("panicked"), "{params}: {e}");
            no_paragraph_borders(&s);
        }
    }
}

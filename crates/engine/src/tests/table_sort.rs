use super::{run, table_from, table_texts};
use serde_json::json;
use wordcraft_doc::{Block, StoryRef};

/// #5535: sorting individual rows must not detach vertical-merge continuations from their start.
#[test]
fn table_sort_refuses_vertical_merges_without_changing_history() {
    use wordcraft_doc::props::VMerge;
    for params in [
        json!({}),
        json!({"column": 0, "header": false}),
        json!({"column": 1, "header": false}),
        json!({"column": 1, "header": true}),
        json!({"column": 1, "header": false, "descending": true}),
    ] {
        let mut s = table_from("Anchor\tZulu\n\tAlpha\nSeparate\tMiddle");
        let t = s.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(vec![0])).unwrap();
        t.rows[0].cells[0].props.vmerge = VMerge::Restart;
        t.rows[1].cells[0].props.vmerge = VMerge::Continue;
        run(&mut s, "select.text", json!({"text": "Zulu"}));
        run(&mut s, "text.insert", json!({"text": "Edited"}));
        let edited = s.doc.clone();
        run(&mut s, "edit.undo", json!({}));
        let (doc, sel, dirty, depth) = (s.doc.clone(), s.sel.clone(), s.dirty, s.undo_depth());
        assert!(s.can_redo());
        let error = s.run("table.sort", &params).unwrap_err().to_string();
        assert!(error.contains("vertically merged"), "{error}");
        assert_eq!(s.doc, doc);
        assert_eq!(s.sel, sel);
        assert_eq!((s.dirty, s.undo_depth()), (dirty, depth));
        assert!(s.can_redo(), "a refused sort keeps the existing redo branch");
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(s.doc, edited);
    }
}

/// Only vertical merges in the target table are refused. Horizontal merges and a nested
/// table's own merges move safely with their containing row.
#[test]
fn table_sort_keeps_horizontal_and_nested_merges() {
    use wordcraft_doc::props::VMerge;
    for horizontal in [false, true] {
        let mut s = table_from("Charlie\nAlpha\nBravo");
        if horizontal {
            let t = s.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(vec![0])).unwrap();
            t.grid = vec![100.0, 100.0];
            for row in &mut t.rows {
                row.cells[0].props.span = 2;
                row.cells[0].props.width = Some(200.0);
            }
        }
        run(&mut s, "table.sort", json!({"header": false}));
        assert_eq!(table_texts(&s), [["Alpha"], ["Bravo"], ["Charlie"]]);
        if horizontal {
            let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
            assert!(t.rows.iter().all(|row| row.cells[0].span() == 2));
        }
    }
    let mut inner = table_from("Inner\nTail");
    let t = inner.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(vec![0])).unwrap();
    t.rows[0].cells[0].props.vmerge = VMerge::Restart;
    t.rows[1].cells[0].props.vmerge = VMerge::Continue;
    t.rows[1].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::new())];
    let nested = inner.doc.body[0].clone();
    let mut s = table_from("Zulu\nAlpha");
    s.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(vec![0])).unwrap().rows[0].cells[0].blocks.push(nested.clone());
    run(&mut s, "table.sort", json!({"header": false}));
    assert_eq!(table_texts(&s), [["Alpha"], ["Zulu"]]);
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    let moved = t.rows[1].cells[0].blocks.iter().find(|b| matches!(b.as_ref(), Block::Table(_))).unwrap();
    assert_eq!(moved, &nested);
}

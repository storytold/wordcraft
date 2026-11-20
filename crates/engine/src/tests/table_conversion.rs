use super::*;
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{Block, Path};

/// A conversion cannot split a form across cell stories and silently lose it on save.
#[test]
fn text_to_table_refuses_cross_cell_controls_without_editing() {
    for command in ["developer.richText", "developer.repeatingSection"] {
        for separator in ["tab", "paragraph", "comma"] {
            for reversed in [false, true] {
                let mut s = s();
                run(&mut s, "document.setText", json!({"text": "Zulu\tBeta\nAlpha\tDelta\nTail"}));
                run(
                    &mut s,
                    "select.range",
                    json!({"anchor": {"story": "body", "path": [0], "off": 0}, "focus": {"story": "body", "path": [1], "off": 11}}),
                );
                run(&mut s, command, json!({"tag": "cross-cell-original"}));
                run(&mut s, "select.all", json!({}));
                if reversed {
                    std::mem::swap(&mut s.sel.anchor, &mut s.sel.focus);
                }
                run(&mut s, "format.bold", json!({}));
                run(&mut s, "edit.undo", json!({}));
                let before = serde_json::to_value(&s.doc).unwrap();
                let selection = s.sel.clone();
                let dirty = s.dirty;
                let depth = s.undo_depth();
                assert!(s.can_redo());
                let error = s.run("table.fromText", &json!({"separator": separator})).unwrap_err();
                assert!(error.to_string().contains("content-control"), "{error}");
                assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
                assert_eq!(s.sel, selection);
                assert_eq!(s.dirty, dirty);
                assert_eq!(s.undo_depth(), depth);
                assert!(s.can_redo());
                run(&mut s, "edit.redo", json!({}));
            }
        }
    }
    // Even one paragraph can span cells when its control contains a separator.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Zulu\tAlpha"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "developer.richText", json!({"tag": "split-at-tab"}));
    run(&mut s, "select.all", json!({}));
    assert!(s.run("table.fromText", &json!({})).unwrap_err().to_string().contains("content-control"));
}

/// A complete control in one cell and untouched enclosing boundaries stay supported.
#[test]
fn text_to_table_keeps_cell_local_and_enclosing_controls() {
    for enclosing in [false, true] {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": "Before\nZulu\tBeta\nAlpha\tDelta\nAfter"}));
        let (a, b) = if enclosing {
            (json!({"story": "body", "path": [0], "off": 0}), json!({"story": "body", "path": [3], "off": 5}))
        } else {
            (json!({"story": "body", "path": [1], "off": 0}), json!({"story": "body", "path": [1], "off": 4}))
        };
        run(&mut s, "select.range", json!({"anchor": a, "focus": b}));
        run(&mut s, "developer.richText", json!({"tag": "safe-conversion"}));
        run(&mut s, "select.range", json!({"anchor": {"story": "body", "path": [1], "off": 0}, "focus": {"story": "body", "path": [3], "off": 0}}));
        let before = serde_json::to_value(&s.doc).unwrap();
        run(&mut s, "table.fromText", json!({}));
        assert_eq!(s.doc.body[1].as_table().unwrap().rows.len(), 2);
        assert!(!s.doc.has_unbalanced_controls());
        let docx = crate::io::save_bytes("safe.docx", &s.doc).unwrap();
        let reopened = crate::io::open_bytes("safe.docx", &docx).unwrap();
        assert!(!reopened.has_unbalanced_controls());
        assert_eq!(reopened.control_ranges(StoryRef::Body).len(), 1);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
        run(&mut s, "edit.redo", json!({}));
        assert!(s.doc.body[1].as_table().is_some());
    }
}

/// Conversion must not discard a selected section's page setup or its existing redo branch.
#[test]
fn text_to_table_refuses_selected_section_breaks_without_editing() {
    for start in [SectionStart::NextPage, SectionStart::Continuous, SectionStart::OddPage] {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": "Alpha\tOne\nBeta\tTwo\nTail"}));
        s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section =
            Some(Box::new(SectionProps { start, page_w: 432.0, page_h: 648.0, ..Default::default() }));
        run(&mut s, "select.all", json!({}));
        run(&mut s, "format.bold", json!({}));
        run(&mut s, "edit.undo", json!({}));
        assert!(s.can_redo());
        let before = serde_json::to_value(&s.doc).unwrap();
        let selection = s.sel.clone();
        let depth = s.undo_depth();
        let dirty = s.dirty;
        assert!(s.run("table.fromText", &json!({})).is_err());
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
        assert_eq!(s.sel, selection);
        assert_eq!(s.undo_depth(), depth);
        assert_eq!(s.dirty, dirty);
        assert!(s.can_redo());
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().props_of_char(0).bold, Some(true));
    }
}

/// A break outside the selected paragraphs must not disable conversion in that section.
#[test]
fn text_to_table_keeps_section_breaks_outside_the_selection() {
    for break_index in [0, 3] {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": "Before\nAlpha\tOne\nBeta\tTwo\nAfter"}));
        s.doc.para_mut(StoryRef::Body, &Path::top(break_index)).unwrap().section =
            Some(Box::new(SectionProps { page_w: 432.0, page_h: 648.0, ..Default::default() }));
        run(&mut s, "select.range", json!({"anchor": {"story": "body", "path": [1], "off": 0}, "focus": {"story": "body", "path": [2], "off": 8}}));
        let before = serde_json::to_value(&s.doc).unwrap();
        run(&mut s, "table.fromText", json!({}));
        let table = s.doc.body[1].as_table().unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells[0].blocks[0].as_para().unwrap().text, "Alpha");
        assert_eq!(table.rows[1].cells[1].blocks[0].as_para().unwrap().text, "Two");
        let sections: Vec<_> = s.doc.body.iter().filter_map(|b| b.as_para().and_then(|p| p.section.as_ref())).collect();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].page_w, 432.0);
        assert!(
            s.doc
                .body
                .iter()
                .filter_map(|b| b.as_table())
                .flat_map(|t| &t.rows)
                .flat_map(|r| &r.cells)
                .flat_map(|c| &c.blocks)
                .filter_map(|b| b.as_para())
                .all(|p| p.section.is_none())
        );
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
        run(&mut s, "edit.redo", json!({}));
        assert!(matches!(s.doc.body[1].as_ref(), Block::Table(_)));
    }
}

/// A selection ending at the next paragraph's start does not include that paragraph's section break.
#[test]
fn text_to_table_excludes_offset_zero_section_endpoint() {
    for reversed in [false, true] {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": "Alpha\tOne\nBeta\tTwo\nAfter"}));
        s.doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section =
            Some(Box::new(SectionProps { page_w: 432.0, page_h: 648.0, ..Default::default() }));
        let a = json!({"story": "body", "path": [0], "off": 0});
        let b = json!({"story": "body", "path": [2], "off": 0});
        run(&mut s, "select.range", if reversed { json!({"anchor": b, "focus": a}) } else { json!({"anchor": a, "focus": b}) });
        let before = serde_json::to_value(&s.doc).unwrap();
        run(&mut s, "table.fromText", json!({}));
        assert_eq!(s.doc.body.len(), 2);
        let table = s.doc.body[0].as_table().unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1].cells[1].blocks[0].as_para().unwrap().text, "Two");
        let after = s.doc.body[1].as_para().unwrap();
        assert_eq!(after.text, "After");
        assert_eq!(after.section.as_ref().unwrap().page_w, 432.0);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
        run(&mut s, "edit.redo", json!({}));
        assert!(s.doc.body[0].as_table().is_some());
        assert_eq!(s.doc.body[1].as_para().unwrap().section.as_ref().unwrap().page_w, 432.0);
    }
}

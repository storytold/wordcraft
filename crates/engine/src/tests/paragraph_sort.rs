use super::{run, text};
use crate::Session;
use serde_json::json;
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{Document, Path, StoryRef};

/// Original section-boundary reproduction: sorting must not move the page setup onto other text.
#[test]
fn paragraph_sort_refuses_section_breaks_without_changing_history() {
    for start in [SectionStart::NextPage, SectionStart::Continuous, SectionStart::OddPage] {
        for descending in [false, true] {
            let mut doc = Document::from_text("Zulu\nAlpha\nTail");
            doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section =
                Some(Box::new(SectionProps { start, page_w: 432.0, page_h: 648.0, ..Default::default() }));
            let original = serde_json::to_value(&doc).unwrap();
            let mut s = Session::new(doc);
            run(&mut s, "caret.docEnd", json!({}));
            run(&mut s, "text.insert", json!({"text": " edited"}));
            run(&mut s, "para.align", json!({"value": "center"}));
            assert!(s.undo());
            run(&mut s, "select.all", json!({}));
            let before = serde_json::to_value(&s.doc).unwrap();
            let selection = s.sel.clone();
            let dirty = s.dirty;
            let depth = s.undo_depth();
            assert!(s.can_redo());
            let error = s.run("para.sort", &json!({"descending": descending})).unwrap_err();
            assert!(error.to_string().contains("section break"), "{error}");
            assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
            assert_eq!(s.sel, selection);
            assert_eq!(s.dirty, dirty);
            assert_eq!(s.undo_depth(), depth);
            assert!(s.can_redo());
            assert!(s.redo());
            assert_ne!(serde_json::to_value(&s.doc).unwrap(), before);
            assert!(s.undo());
            assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
            assert!(s.undo());
            assert_eq!(serde_json::to_value(&s.doc).unwrap(), original);
        }
    }
}

/// A selection ends before the paragraph at offset zero, including any section break on it.
#[test]
fn paragraph_sort_excludes_an_unselected_endpoint_in_both_directions() {
    for outside_break in [false, true] {
        for reverse in [false, true] {
            let mut doc = Document::from_text("Zulu\nBravo\nAlpha\nTail");
            if outside_break {
                doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section = Some(Box::default());
            }
            let original = serde_json::to_value(&doc).unwrap();
            let outside = doc.body[2..].to_vec();
            let mut s = Session::new(doc);
            let a = json!({"story": "body", "path": [0], "off": 0});
            let b = json!({"story": "body", "path": [2], "off": 0});
            let (anchor, focus) = if reverse { (b, a) } else { (a, b) };
            run(&mut s, "select.range", json!({"anchor": anchor, "focus": focus}));
            run(&mut s, "para.sort", json!({}));
            assert_eq!(text(&s), "Bravo\nZulu\nAlpha\nTail");
            assert_eq!(s.doc.body[2..], outside);
            assert!(s.undo());
            assert_eq!(serde_json::to_value(&s.doc).unwrap(), original);
            assert!(s.redo());
            assert_eq!(text(&s), "Bravo\nZulu\nAlpha\nTail");
        }
    }
}

/// Ordinary sorting still moves complete formatted paragraphs and supports one-step Undo/Redo.
#[test]
fn paragraph_sort_keeps_formatted_single_section_sorting() {
    let mut doc = Document::from_text("Zulu\nAlpha\nBravo");
    doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().format(0, 4, &|p| p.bold = Some(true)).unwrap();
    let original = serde_json::to_value(&doc).unwrap();
    let mut s = Session::new(doc);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "para.sort", json!({"descending": true}));
    assert_eq!(text(&s), "Zulu\nBravo\nAlpha");
    assert_eq!(s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().props_of_char(0).bold, Some(true));
    assert!(s.undo());
    assert_eq!(serde_json::to_value(&s.doc).unwrap(), original);
    assert!(s.redo());
    assert_eq!(text(&s), "Zulu\nBravo\nAlpha");
}

/// Sorting a control's start or end apart must not lose the form or change its history.
#[test]
fn paragraph_sort_refuses_cross_paragraph_controls_without_changing_history() {
    for command in ["developer.richText", "developer.repeatingSection"] {
        for descending in [false, true] {
            for range in [None, Some((0, 1)), Some((1, 2))] {
                let mut s = Session::new(Document::from_text("Zulu\nAlpha\nTail"));
                run(
                    &mut s,
                    "select.range",
                    json!({"anchor": {"story": "body", "path": [0], "off": 0}, "focus": {"story": "body", "path": [1], "off": 5}}),
                );
                run(&mut s, command, json!({"title": "Original sorted form", "tag": "sort-control"}));
                run(&mut s, "caret.docEnd", json!({}));
                run(&mut s, "text.insert", json!({"text": " edited"}));
                run(&mut s, "para.align", json!({"value": "center"}));
                assert!(s.undo());
                if let Some((start, end)) = range {
                    run(
                        &mut s,
                        "select.range",
                        json!({"anchor": {"story": "body", "path": [start], "off": 0}, "focus": {"story": "body", "path": [end], "off": 0}}),
                    );
                } else {
                    run(&mut s, "select.all", json!({}));
                }
                let before = serde_json::to_value(&s.doc).unwrap();
                let selection = s.sel.clone();
                let (dirty, depth) = (s.dirty, s.undo_depth());
                assert!(s.can_redo());
                let error = s.run("para.sort", &json!({"descending": descending})).unwrap_err();
                assert!(error.to_string().contains("content control"), "{error}");
                assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
                assert_eq!(s.sel, selection);
                assert_eq!((s.dirty, s.undo_depth()), (dirty, depth));
                assert!(s.can_redo());
                assert!(s.redo());
                assert_ne!(serde_json::to_value(&s.doc).unwrap(), before);
                assert!(s.undo());
                assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
            }
        }
    }
}

/// Complete one-paragraph controls move with their text; interior sorts keep the outer form.
#[test]
fn paragraph_sort_keeps_local_controls_and_enclosing_controls() {
    let mut local = Session::new(Document::from_text("Zulu\nAlpha\nBravo"));
    run(&mut local, "select.range", json!({"anchor": {"story": "body", "path": [0], "off": 0}, "focus": {"story": "body", "path": [0], "off": 4}}));
    run(&mut local, "developer.richText", json!({"tag": "local-control"}));
    let original = serde_json::to_value(&local.doc).unwrap();
    run(&mut local, "select.all", json!({}));
    run(&mut local, "para.sort", json!({}));
    assert_eq!(text(&local), "Alpha\nBravo\nZulu");
    let ranges = local.doc.control_ranges(StoryRef::Body);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].control.tag, "local-control");
    assert_eq!(ranges[0].start.path, Path::top(2));
    assert_eq!(ranges[0].end.path, Path::top(2));
    assert!(local.undo());
    assert_eq!(serde_json::to_value(&local.doc).unwrap(), original);
    assert!(local.redo());
    assert_eq!(text(&local), "Alpha\nBravo\nZulu");

    for reverse in [false, true] {
        let mut enclosing = Session::new(Document::from_text("Outside\nZulu\nAlpha\nTail"));
        run(&mut enclosing, "select.all", json!({}));
        run(&mut enclosing, "developer.richText", json!({"tag": "enclosing-control"}));
        let original = serde_json::to_value(&enclosing.doc).unwrap();
        let boundary = enclosing.doc.control_ranges(StoryRef::Body);
        let (a, b) = (json!({"story": "body", "path": [1], "off": 0}), json!({"story": "body", "path": [3], "off": 0}));
        let (anchor, focus) = if reverse { (b, a) } else { (a, b) };
        run(&mut enclosing, "select.range", json!({"anchor": anchor, "focus": focus}));
        run(&mut enclosing, "para.sort", json!({}));
        assert_eq!(text(&enclosing), "Outside\nAlpha\nZulu\nTail");
        assert_eq!(enclosing.doc.control_ranges(StoryRef::Body), boundary);
        assert!(enclosing.undo());
        assert_eq!(serde_json::to_value(&enclosing.doc).unwrap(), original);
        assert!(enclosing.redo());
        assert_eq!(text(&enclosing), "Outside\nAlpha\nZulu\nTail");
    }
}

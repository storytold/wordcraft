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

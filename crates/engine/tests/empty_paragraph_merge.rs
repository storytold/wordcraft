use serde_json::{Value, json};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_engine::{Selection, Session};
use wordcraft_layout::outline;

/// Dispatch an engine command and report its ID if it fails.
fn run(s: &mut Session, id: &str, params: Value) {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// Create an empty heading, mixed-run body text and a genuine TOC heading.
fn fixture() -> Session {
    let mut s = Session::new(Document::from_text("\nBody paragraph.\nReal heading\n"));
    run(&mut s, "para.style", json!({"style": "Heading1"}));
    s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().mark =
        CharProps { font: Some("Source Sans 3".into()), size: Some(40.0), bold: Some(true), ..Default::default() };
    let p = s.doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    p.mark = CharProps { size: Some(11.0), bold: Some(false), ..Default::default() };
    p.format(5, 14, &|c| c.italic = Some(true)).unwrap();
    s.doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().props.style = Some("Heading2".into());
    s
}

/// Keyboard and selection deletes preserve body formatting through Undo and Redo.
#[test]
fn empty_heading_delete_backspace_and_selection_preserve_body_format_and_history() {
    for mode in ["delete", "backspace", "selection", "reverse selection"] {
        for pending in [false, true] {
            let mut s = fixture();
            let expected = s.doc.para_at(&Pos::body(1, 0)).unwrap().clone();
            s.sel = match mode {
                "backspace" => Selection::caret(Pos::body(1, 0)),
                "selection" => Selection { anchor: Pos::body(0, 0), focus: Pos::body(1, 0) },
                "reverse selection" => Selection { anchor: Pos::body(1, 0), focus: Pos::body(0, 0) },
                _ => Selection::caret(Pos::body(0, 0)),
            };
            if pending {
                s.pending = Some(CharProps { size: Some(48.0), bold: Some(true), ..Default::default() });
            }
            let before = serde_json::to_value(&s.doc.body).unwrap();
            run(&mut s, if mode == "backspace" { "text.backspace" } else { "text.delete" }, json!({}));
            let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
            assert_eq!(p.text, expected.text, "{mode}, pending={pending}");
            assert_eq!(p.props, expected.props);
            assert_eq!(p.mark, expected.mark);
            assert_eq!(serde_json::to_value(&p.runs).unwrap(), serde_json::to_value(&expected.runs).unwrap());
            let actual = s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(0));
            let old = s.doc.styles.resolve_char(expected.props.style.as_deref(), expected.props_of_char(0));
            assert_eq!((actual.font, actual.size, actual.bold, actual.italic), (old.font, old.size, old.bold, old.italic));
            assert!(s.pending.is_none());
            assert_eq!(outline::levels(&s.doc)[..2], [outline::BODY, 2]);
            let after = serde_json::to_value(&s.doc.body).unwrap();
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(serde_json::to_value(&s.doc.body).unwrap(), before);
            run(&mut s, "edit.redo", json!({}));
            assert_eq!(serde_json::to_value(&s.doc.body).unwrap(), after);
        }
    }
}

/// Saving to DOCX and generating a TOC must keep the surviving body text out of the outline.
#[test]
fn deleted_empty_heading_does_not_promote_body_text_into_toc_or_docx() {
    let mut s = fixture();
    run(&mut s, "text.delete", json!({}));
    assert_eq!(outline::levels(&s.doc)[..2], [outline::BODY, 2]);
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let mut s = Session::new(wordcraft_docx::read(&bytes).unwrap());
    assert_eq!(outline::levels(&s.doc)[..2], [outline::BODY, 2]);
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props.style.as_deref(), None);
    assert_eq!(p.mark.size, Some(11.0));
    assert_eq!(p.props_of_char(6).italic, Some(true));
    s.sel = Selection::caret(Pos::body(2, 0));
    run(&mut s, "references.toc", json!({"levels": 3}));
    let text = s.doc.plain_text(StoryRef::Body);
    assert_eq!(text.matches("Body paragraph.").count(), 1, "body text is not a TOC entry: {text}");
    assert_eq!(text.matches("Real heading").count(), 2, "the genuine heading stays in the TOC: {text}");
}

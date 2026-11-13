use serde_json::json;
use wordcraft_doc::{Document, props::ParaProps};
use wordcraft_engine::Session;

fn props(s: &Session) -> ParaProps {
    s.doc.body.first().and_then(|b| b.as_para()).unwrap().props.clone()
}

fn session(indent: f32) -> Session {
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": "Once upon a time"})).unwrap();
    s.run("para.indents", &json!({"firstLine": indent})).unwrap();
    s
}

#[test]
fn margin_indent_restored_after_reopen_and_position_change() {
    for indent in [-36.0, 36.0] {
        for position in ["none", "dropped"] {
            let mut s = session(indent);
            s.run("insert.dropCap", &json!({"position": "margin"})).unwrap();
            let bytes = wordcraft_docx::write(&s.doc).unwrap();
            let mut s = Session::new(wordcraft_docx::read(&bytes).unwrap());
            assert_eq!(props(&s).drop_cap_indent_first, Some(indent));
            // Updating the options while already In margin must keep the frame indent.
            s.run("insert.dropCap", &json!({"position": "margin", "lines": 4})).unwrap();
            let margin = props(&s);
            assert_eq!(margin.drop_cap_indent_first, Some(indent));
            assert_eq!(margin.indent_first, None);
            s.run("insert.dropCap", &json!({"position": position})).unwrap();
            assert_eq!(props(&s).indent_first, Some(indent));
            assert_eq!(props(&s).drop_cap_indent_first, None);
            s.run("edit.undo", &json!({})).unwrap();
            assert_eq!(props(&s), margin);
            s.run("edit.redo", &json!({})).unwrap();
            assert_eq!(props(&s).indent_first, Some(indent));
        }
    }
}

#[test]
fn margin_converts_character_indent_and_undo_restores_original_units() {
    let mut s = session(36.0);
    let focus = s.sel.focus.clone();
    let p = s.doc.para_mut(focus.story, &focus.path).unwrap();
    p.props.indent_first_chars = Some(-2.0);
    let original = props(&s);
    let expected = s.doc.styles.resolve_para(&original).indent_first;
    s.run("insert.dropCap", &json!({"position": "margin"})).unwrap();
    assert_eq!(props(&s).drop_cap_indent_first, Some(expected));
    assert_eq!(props(&s).indent_first_chars, None);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(props(&s), original);
    s.run("edit.redo", &json!({})).unwrap();
    s.run("insert.dropCap", &json!({"position": "none"})).unwrap();
    assert_eq!(props(&s).indent_first, Some(expected));
}

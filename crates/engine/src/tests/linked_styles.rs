use super::{para_style, run};
use serde_json::json;
use std::sync::Arc;
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{Document, Pos};

use crate::Session;

const PARAGRAPH: &str = "SampleLinked";
const CHARACTER: &str = "SampleLinkedChar";

fn sample() -> Session {
    let mut doc = Document::from_text("left sample right\nsecond paragraph");
    let styles = Arc::make_mut(&mut doc.styles);
    styles.upsert(Style {
        id: PARAGRAPH.into(),
        name: "Sample Linked".into(),
        kind: StyleKind::Paragraph,
        linked: Some(CHARACTER.into()),
        ..Default::default()
    });
    styles.upsert(Style {
        id: CHARACTER.into(),
        name: "Sample Linked Char".into(),
        kind: StyleKind::Character,
        linked: Some(PARAGRAPH.into()),
        ..Default::default()
    });
    let mut s = Session::new(doc);
    run(&mut s, "para.style", json!({"style": "Quote"}));
    s
}

fn select(s: &mut Session, anchor: Pos, focus: Pos) {
    run(s, "select.range", json!({"anchor": anchor, "focus": focus}));
}

#[test]
fn linked_style_on_partial_text_styles_only_the_selection() {
    let mut s = sample();
    select(&mut s, Pos::body(0, 0), Pos::body(0, "left sample right".len()));
    run(&mut s, "format.color", json!({"color": "1B1611"}));
    let para_at = Pos::body(0, 0);
    let direct_color = s.doc.para_at(&para_at).unwrap().props_of_char(5).color;
    assert!(direct_color.is_some());
    select(&mut s, Pos::body(0, 11), Pos::body(0, 5));
    let before = s.doc.clone();
    let selection = s.sel.clone();
    run(&mut s, "para.style", json!({"style": "Sample Linked"}));
    assert_eq!(s.sel, selection);

    let bytes = crate::io::save_bytes("linked.docx", &s.doc).unwrap();
    let reopened = crate::io::open_bytes("linked.docx", &bytes).unwrap();
    for doc in [&s.doc, &reopened] {
        let para = doc.para_at(&para_at).unwrap();
        assert_eq!(para.props.style.as_deref(), Some("Quote"));
        assert_eq!(para.text.as_str(), "left sample right");
        for offset in 0..para.len() {
            let props = para.props_of_char(offset);
            let style = (5..11).contains(&offset).then_some(CHARACTER);
            assert_eq!(props.style.as_deref(), style, "offset {offset}");
            assert_eq!(props.color, direct_color, "offset {offset}");
        }
    }

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc, before);
    assert_eq!(s.sel, selection);
}

#[test]
fn linked_style_keeps_paragraph_application_outside_a_partial_selection() {
    let length = "left sample right".len();
    let para_start = Pos::body(0, 0);
    let next_start = Pos::body(1, 0);
    for (anchor, focus, second) in [
        (Pos::body(0, 5), Pos::body(0, 5), None),
        (para_start.clone(), Pos::body(0, length), None),
        (Pos::body(0, 5), next_start, None),
        (Pos::body(0, 5), Pos::body(1, 6), Some(PARAGRAPH)),
    ] {
        let mut s = sample();
        select(&mut s, anchor, focus);
        run(&mut s, "para.style", json!({"style": PARAGRAPH}));
        assert_eq!(para_style(&s, 0).as_deref(), Some(PARAGRAPH));
        assert_eq!(para_style(&s, 1).as_deref(), second);
        assert_eq!(s.doc.para_at(&para_start).unwrap().props_of_char(5).style, None);
    }
}

#[test]
fn partial_selection_keeps_paragraph_application_without_a_character_link() {
    for target in [None, Some("MissingStyle"), Some("Quote")] {
        let mut s = sample();
        Arc::make_mut(&mut s.doc.styles).get_mut(PARAGRAPH).unwrap().linked = target.map(str::to_owned);
        select(&mut s, Pos::body(0, 5), Pos::body(0, 11));
        run(&mut s, "para.style", json!({"style": PARAGRAPH}));
        assert_eq!(para_style(&s, 0).as_deref(), Some(PARAGRAPH), "link {target:?}");
    }

    let mut s = sample();
    select(&mut s, Pos::body(0, 5), Pos::body(0, 11));
    run(&mut s, "para.heading1", json!({}));
    assert_eq!(para_style(&s, 0).as_deref(), Some("Heading1"));
}

#[test]
fn partial_selection_cannot_apply_a_locked_linked_character_style() {
    let mut s = sample();
    run(&mut s, "review.restrict", json!({"mode": "none", "formatting": true, "lockedStyles": [CHARACTER]}));
    select(&mut s, Pos::body(0, 5), Pos::body(0, 11));
    let before = s.doc.clone();
    assert!(s.run("para.style", &json!({"style": PARAGRAPH})).unwrap_err().to_string().contains("is locked"));
    assert_eq!(s.doc, before);
}

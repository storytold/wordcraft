//! Placeholders (ODF 1.3 §7.7.2 `text:placeholder`: "click here and type" text, as LibreOffice's
//! templates use): opened as plain text content controls showing their placeholder, so a click
//! selects one and typing replaces it, and written back as placeholders while they still show it.

use crate::model::Inline;
use crate::odt::tests::{NS_T, content_of, package, paras};
use crate::odt::{export, import, parse};

fn doc(body: &str) -> Vec<u8> {
    package(
        &format!(r#"<office:document-content {NS_T}><office:body><office:text>{body}</office:text></office:body></office:document-content>"#),
        None,
    )
}

/// Each control: (its placeholder text, showing it, the text shown).
fn controls(bytes: &[u8]) -> Vec<(String, bool, String)> {
    paras(&parse(bytes).unwrap())
        .into_iter()
        .flat_map(|p| &p.inlines)
        .filter_map(|i| match i {
            Inline::Control { control, text, .. } => Some((control.placeholder_text.clone(), control.showing_placeholder, text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn placeholders_open_as_controls_showing_them() {
    let bytes = doc(
        r#"<text:p>Dear <text:placeholder text:placeholder-type="text">&lt;Honorific&gt;</text:placeholder><text:placeholder text:placeholder-type="text">&lt;Name&gt;</text:placeholder>,</text:p>"#,
    );
    assert_eq!(controls(&bytes), [("<Honorific>".into(), true, "<Honorific>".into()), ("<Name>".into(), true, "<Name>".into())]);
    // The paragraph's own text around them.
    let d = import(&bytes).unwrap();
    let text = d.body[0].as_para().unwrap().text.replace(wordcraft_doc::para::OBJ, "");
    assert_eq!(text, "Dear <Honorific><Name>,");
}

#[test]
fn placeholders_are_written_back_while_they_show() {
    let bytes = doc(r#"<text:p>Dear <text:placeholder text:placeholder-type="text">&lt;Name&gt;</text:placeholder>,</text:p>"#);
    let saved = export(&import(&bytes).unwrap()).unwrap();
    let c = content_of(&saved);
    assert!(c.contains(r#"<text:placeholder text:placeholder-type="text">&lt;Name&gt;</text:placeholder>"#), "{c}");
    assert!(!c.contains("<form:"), "{c}");
    assert_eq!(controls(&saved), controls(&bytes));
}

#[test]
fn an_empty_or_hostile_placeholder_never_panics() {
    let long = "x".repeat(200_000);
    for body in [
        r#"<text:p><text:placeholder/></text:p>"#.to_string(),
        r#"<text:placeholder>outside a paragraph</text:placeholder><text:p>p</text:p>"#.to_string(),
        format!(r#"<text:p><text:placeholder>{long}</text:placeholder></text:p>"#),
        r#"<text:p><text:placeholder><text:placeholder>nested</text:placeholder></text:placeholder></text:p>"#.to_string(),
        r#"<text:p><text:placeholder>unclosed</text:p>"#.to_string(),
    ] {
        if let Ok(d) = import(&doc(&body)) {
            let _ = export(&d).unwrap();
        }
    }
}

#[test]
fn a_placeholders_type_is_kept_and_typed_text_is_text() {
    let bytes = doc(
        r#"<text:p><text:placeholder text:placeholder-type="image" text:description="Your logo &amp; mark">&lt;Logo&gt;</text:placeholder> and <text:placeholder text:placeholder-type="nonsense">&lt;X&gt;</text:placeholder></text:p>"#,
    );
    let c = content_of(&crate::odt::tests::export_model(&import(&bytes).unwrap()).unwrap());
    // Its description is the control's title, written back.
    assert!(
        c.contains(r#"<text:placeholder text:placeholder-type="image" text:description="Your logo &amp; mark">&lt;Logo&gt;</text:placeholder>"#),
        "{c}"
    );
    assert!(c.contains(r#"<text:placeholder text:placeholder-type="text">&lt;X&gt;</text:placeholder>"#), "{c}");
    // Something typed in its place is text.
    let mut flow = parse(&bytes).unwrap();
    for b in &mut flow.blocks {
        if let crate::model::FBlock::Para(p) = b {
            for i in &mut p.inlines {
                if let Inline::Control { control, text, .. } = i {
                    control.showing_placeholder = false;
                    *text = "Acme  Ltd".into();
                }
            }
        }
    }
    let c = content_of(&crate::odt::tests::export_model(&crate::model::to_doc(&flow)).unwrap());
    assert!(!c.contains("<text:placeholder") && c.contains(r#"Acme <text:s text:c="1"/>Ltd"#), "{c}");
}

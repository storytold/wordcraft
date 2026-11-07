use super::super::{json, run};
use super::{DisplayOptions, INK, Session, blue_pixels, document_with_ink};

#[test]
fn native_ink_omitted_channel_units_match_explicit_defaults_after_edits_and_saves() {
    let channels = r#"<traceFormat><channel name="X"/><channel name="Y"/><channel name="F"/></traceFormat>"#;
    let affine = r#"<mapping type="affine"><affine>1 0 0 0,0 1 0 0,0 0 1 0</affine></mapping>"#;
    for context in [
        channels.to_string(),
        format!(
            r#"<context><traceFormat><channel name="a"/><channel name="b"/><channel name="p"/></traceFormat><canvas>{channels}</canvas><canvasTransform>{affine}</canvasTransform></context>"#
        ),
        format!(
            r#"<context><traceFormat><channel name="a"/><channel name="b"/><channel name="p"/></traceFormat><canvas>{channels}</canvas><canvasTransform><mapping type="unknown"/>{affine}</canvasTransform></context>"#
        ),
    ] {
        let ink = INK.replace("<trace>0 0, 50 0, 50 25</trace>", &format!("{context}<trace>0 0 .5,25.4 0 1,25.4 12.7 .5</trace>"));
        let explicit = ink
            .replace("name=\"X\"", "name=\"X\" units=\"mm\"")
            .replace("name=\"Y\"", "name=\"Y\" units=\"mm\"")
            .replace("name=\"F\"", "name=\"F\" units=\"%\"");
        for placement in ["inline", "floating", "group", "canvas", "direct"] {
            let doc = crate::io::open_bytes("explicit.docx", &document_with_ink(placement, &explicit)).unwrap();
            let mut reference = Session::new(doc);
            let layout = reference.layout();
            let expected = wordcraft_render::render_page(&reference.doc, &layout.pages[0], 1.0, &Default::default());
            let doc = crate::io::open_bytes("defaults.docx", &document_with_ink(placement, &ink)).unwrap();
            let mut session = Session::new(serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap());
            let before = session.doc.body.clone();
            run(&mut session, "text.insert", json!({"text":"Edit "}));
            run(&mut session, "edit.undo", json!({}));
            assert_eq!(session.doc.body, before);
            for _ in 0..3 {
                let layout = session.layout();
                let page = &layout.pages[0];
                let actual = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
                assert!(blue_pixels(&actual) > 20);
                assert_eq!(actual.pixels, expected.pixels, "{placement}: omitted and explicit units must agree");
                let hidden = wordcraft_render::render_page(
                    &session.doc,
                    page,
                    1.0,
                    &wordcraft_render::RenderOptions { display: DisplayOptions { hide_ink: true, ..Default::default() }, ..Default::default() },
                );
                assert_eq!(blue_pixels(&hidden), 0);
                let pdf = wordcraft_pdf::export_layout(&session.doc, &layout, &Default::default()).unwrap();
                assert!(!String::from_utf8_lossy(&pdf).contains("/Subtype /Image"));
                assert!(session.doc.passthrough.values().any(|bytes| bytes.as_slice() == ink.as_bytes()));
                let saved = crate::io::save_bytes("defaults.docx", &session.doc).unwrap();
                session = Session::new(crate::io::open_bytes("defaults.docx", &saved).unwrap());
            }
        }
    }
}

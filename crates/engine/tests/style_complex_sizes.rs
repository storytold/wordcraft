//! Generated font sizes must survive exchange without erasing script-specific choices.
use serde_json::json;
use wordcraft_doc::{CharProps, Document, Style, StyleKind};
use wordcraft_engine::Session;

fn command(session: &mut Session, id: &str, params: serde_json::Value) {
    session.run(id, &params).unwrap();
}

fn reopened(session: &Session) -> Document {
    wordcraft_docx::read(&wordcraft_docx::write(&session.doc).unwrap()).unwrap()
}

#[test]
fn generated_complex_script_sizes_survive_docx_and_the_title_set() {
    let mut session = Session::new(Document::new());
    for (preset, title_size) in [(None, 28.0), (Some("title"), 36.0)] {
        if let Some(name) = preset {
            command(&mut session, "design.styleSet", json!({"name": name}));
        }
        let doc = reopened(&session);
        assert_eq!((doc.styles.default_chr.size, doc.styles.default_chr.size_cs), (Some(12.0), Some(12.0)));
        for (id, size) in [("Title", title_size), ("Heading1", 20.0), ("Subtitle", 14.0), ("FootnoteText", 10.0)] {
            let style = doc.styles.get(id).unwrap();
            assert_eq!((style.chr.size, style.chr.size_cs), (Some(size), Some(size)), "{id}");
        }
    }
}

#[test]
fn style_size_edits_update_linked_sizes_and_preserve_separate_sizes() {
    for (old_cs, requested_cs, expected_cs) in [(28.0, None, 32.0), (18.0, None, 18.0), (28.0, Some(16.0), 16.0)] {
        let mut session = Session::new(Document::from_text("Original mixed text: Latin العربية"));
        let title = session.doc.styles.get_mut("Title").unwrap();
        title.chr.size = Some(28.0);
        title.chr.size_cs = Some(old_cs);
        let mut delta = json!({"size": 32.0});
        if let Some(size) = requested_cs {
            delta["sizeCs"] = json!(size);
        }
        command(&mut session, "styles.modify", json!({"style": "Title", "chr": delta}));
        let doc = reopened(&session);
        let title = doc.styles.get("Title").unwrap();
        assert_eq!((title.chr.size, title.chr.size_cs), (Some(32.0), Some(expected_cs)));
        assert_eq!(doc.plain_text(wordcraft_doc::StoryRef::Body), session.doc.plain_text(wordcraft_doc::StoryRef::Body));
    }
}

#[test]
fn inherited_script_sizes_survive_noop_and_unrelated_style_edits() {
    for kind in [StyleKind::Paragraph, StyleKind::Character] {
        for parent_cs in [18.0, 28.0] {
            let mut session = Session::new(Document::new());
            session.doc.styles.upsert(Style {
                id: "SourceParent".into(),
                name: "Source parent".into(),
                kind,
                chr: CharProps { size: Some(28.0), size_cs: Some(parent_cs), ..Default::default() },
                ..Default::default()
            });
            session.doc.styles.upsert(Style {
                id: "SourceChild".into(),
                name: "Source child".into(),
                kind,
                based_on: Some("SourceParent".into()),
                ..Default::default()
            });
            // Exercise an imported style chain, rather than just our new-document defaults.
            session = Session::new(reopened(&session));
            command(&mut session, "styles.modify", json!({"style": "SourceChild", "chr": {"size": 28.0}}));
            command(&mut session, "styles.modify", json!({"style": "SourceChild", "chr": {"bold": true}}));
            assert_eq!(session.doc.styles.get("SourceChild").unwrap().chr.size_cs, None);
            command(&mut session, "styles.modify", json!({"style": "SourceChild", "chr": {"size": 32.0}}));
            let doc = reopened(&session);
            let child = doc.styles.get("SourceChild").unwrap();
            assert_eq!(child.chr.size, Some(32.0));
            assert_eq!(child.chr.size_cs, if parent_cs == 28.0 { Some(32.0) } else { None });
            assert_eq!(doc.styles.get("SourceParent").unwrap().chr.size_cs, Some(parent_cs));
        }
    }
}

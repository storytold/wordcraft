//! #2197: three-column header/footer presets are editable, borderless tables.
use serde_json::json;
use wordcraft_doc::props::Align;
use wordcraft_doc::{Block, Document, Path, Pos, StoryRef, Table};
use wordcraft_engine::Session;

fn table(doc: &Document, story: StoryRef) -> &Table {
    let Block::Table(table) = &*doc.story(story).unwrap()[0] else { panic!("three columns must be a table") };
    table
}

fn assert_empty_three_column_table(doc: &Document, story: StoryRef, style: &str, width: f32) {
    let t = table(doc, story);
    assert_eq!(t.rows.len(), 1);
    assert_eq!(t.rows[0].cells.len(), 3);
    assert_eq!(t.grid.len(), 3);
    assert!((t.grid.iter().sum::<f32>() - width).abs() < 0.01);
    assert!(t.grid.iter().all(|w| (*w - width / 3.0).abs() < 0.01));
    assert!(t.props.fixed);
    assert!(!t.props.borders.as_ref().unwrap().any_visible());
    assert_ne!(t.props.style.as_deref(), Some("TableGrid"));
    for (cell, align) in t.rows[0].cells.iter().zip([Align::Left, Align::Center, Align::Right]) {
        let Block::Para(p) = &*cell.blocks[0] else { panic!("cell paragraph missing") };
        assert!(p.text.is_empty());
        assert_eq!(p.props.style.as_deref(), Some(style));
        assert_eq!(p.props.align, Some(align));
    }
}

fn assert_filled_three_column_table(doc: &Document, story: StoryRef, style: &str, width: f32) {
    let t = table(doc, story);
    assert_eq!(t.rows.len(), 1);
    assert_eq!(t.rows[0].cells.len(), 3);
    assert!((t.grid.iter().sum::<f32>() - width).abs() < 0.1);
    assert!(t.props.fixed);
    assert!(!t.props.borders.as_ref().unwrap().any_visible());
    for ((cell, text), align) in
        t.rows[0].cells.iter().zip(["Left text", "Middle text", "Right text"]).zip([Align::Left, Align::Center, Align::Right])
    {
        let Block::Para(p) = &*cell.blocks[0] else { panic!("cell lost") };
        assert_eq!(p.text, text);
        assert_eq!(p.props.style.as_deref(), Some(style));
        assert_eq!(p.props.align, Some(align));
        assert_eq!(p.props_at(0).bold == Some(true), text == "Middle text");
    }
}

fn fill_and_check_undo(session: &mut Session, story: StoryRef) -> Document {
    for (cell, text) in ["Left text", "Middle text", "Right text"].into_iter().enumerate() {
        if cell > 0 {
            session.run("text.tab", &json!({})).unwrap();
        }
        assert_eq!(session.sel.focus, Pos { story, path: Path(vec![0, 0, cell as u32, 0]), off: 0 });
        session.run("text.insert", &json!({"text": text})).unwrap();
    }
    let filled = session.doc.clone();
    session.run("edit.undo", &json!({})).unwrap();
    assert!(!session.doc.plain_text(story).contains("Right text"));
    assert!(session.doc.plain_text(story).contains("Middle text"));
    session.run("edit.redo", &json!({})).unwrap();
    assert_eq!(session.doc, filled);
    session.run("select.text", &json!({"text": "Middle text"})).unwrap();
    session.run("format.bold", &json!({"value": true})).unwrap();
    session.doc.clone()
}

fn assert_roundtrip(session: &Session, header: bool, style: &str, width: f32) {
    let mut back = session.doc.clone();
    for _ in 0..2 {
        back = wordcraft_docx::read(&wordcraft_docx::write(&back).unwrap()).unwrap();
        let id = if header { back.last_section.headers.default } else { back.last_section.footers.default }.unwrap();
        let story = StoryRef::Part(id);
        assert_filled_three_column_table(&back, story, style, width);
        assert_eq!(back.plain_text(StoryRef::Body), "Body text");
    }
}

#[test]
fn three_columns_are_borderless_cells_with_the_story_style_and_section_width() {
    for (command, header, style, existing) in [
        ("insert.header", true, "Header", false),
        ("insert.header", true, "Header", true),
        ("insert.footer", false, "Footer", false),
        ("insert.footer", false, "Footer", true),
    ] {
        let mut doc = Document::from_text("Body text");
        doc.last_section.page_w = 540.0;
        doc.last_section.margin_left = 54.0;
        doc.last_section.margin_right = 54.0;
        let width = doc.last_section.text_width();
        let mut s = Session::new(doc);
        if existing {
            s.run(command, &json!({"text": "Original story content"})).unwrap();
            s.run("insert.closeHeader", &json!({})).unwrap();
        }
        let before = s.doc.clone();
        let depth = s.undo_depth();
        let id = s.run(command, &json!({"preset": "blankThree"})).unwrap()["story"].as_u64().unwrap() as u32;
        let story = StoryRef::Part(id);
        assert_empty_three_column_table(&s.doc, story, style, width);
        assert_eq!(s.sel.focus, Pos { story, path: Path(vec![0, 0, 0, 0]), off: 0 });
        let after = s.doc.clone();
        assert_eq!(s.undo_depth(), depth + 1);
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc, before);
        s.run("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc, after);
        fill_and_check_undo(&mut s, story);
        assert_roundtrip(&s, header, style, width);
    }
}

#[test]
fn explicit_text_keeps_precedence_over_the_preset() {
    let mut s = Session::new(Document::new());
    let id = s.run("insert.footer", &json!({"text": "Keep explicit text", "preset": "blankThree"})).unwrap()["story"].as_u64().unwrap() as u32;
    assert_eq!(s.doc.plain_text(StoryRef::Part(id)), "Keep explicit text");
    assert!(matches!(&*s.doc.story(StoryRef::Part(id)).unwrap()[0], Block::Para(_)));
}

//! #2201: inserting page numbers must preserve the header/footer story.
use std::sync::Arc;

use serde_json::json;
use wordcraft_doc::para::{GroupChild, InlineObject};
use wordcraft_doc::props::{Align, CharProps};
use wordcraft_doc::{Block, Document, Paragraph, PartKind, Path, StoryRef, Table, para_block};
use wordcraft_engine::{Selection, Session};

fn fields(doc: &Document, story: StoryRef, name: &str) -> usize {
    doc.para_paths(story)
        .iter()
        .filter_map(|path| doc.para(story, path))
        .flat_map(|p| &p.objects)
        .filter(|o| matches!(o, InlineObject::Field { instr, .. } | InlineObject::FieldStart { instr, .. } if instr.split_whitespace().next().is_some_and(|token| token.eq_ignore_ascii_case(name))))
        .count()
}

fn field(instr: &str) -> InlineObject {
    InlineObject::Field { instr: instr.into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() }
}

#[test]
fn numbering_preserves_content_formatting_selection_and_undo_redo() {
    for (command, position) in [("insert.header", "top"), ("insert.footer", "bottom")] {
        for format in ["plain", "x of y"] {
            let mut s = Session::new(Document::from_text("Body text"));
            let id = s.run(command, &json!({"text": "Keep this\nSecond line"})).unwrap()["story"].as_u64().unwrap() as u32;
            let story = StoryRef::Part(id);
            s.doc.para_mut(story, &Path::top(0)).unwrap().props.align = Some(Align::Right);
            let mut table = Table::new(1, 2, 300.0);
            table.rows[0].cells[0].blocks =
                vec![para_block(Paragraph::with_text("Keep table", CharProps { bold: Some(true), ..Default::default() }))];
            s.doc.story_mut(story).unwrap().push(Arc::new(Block::Table(table)));
            let before = s.doc.clone();
            let selection = s.sel.clone();
            let blocks = s.doc.story(story).unwrap().clone();
            let depth = s.undo_depth();
            s.run("insert.pageNumber", &json!({"position": position, "format": format})).unwrap();
            assert_eq!(&s.doc.story(story).unwrap()[..blocks.len()], blocks.as_slice());
            assert_eq!(s.sel, selection);
            assert_eq!(fields(&s.doc, story, "PAGE"), 1);
            assert_eq!(fields(&s.doc, story, "NUMPAGES"), usize::from(format == "x of y"));
            assert_eq!(s.undo_depth(), depth + 1);
            let after = s.doc.clone();
            s.run("edit.undo", &json!({})).unwrap();
            assert_eq!(s.doc, before);
            assert_eq!(s.sel, selection);
            s.run("edit.redo", &json!({})).unwrap();
            assert_eq!(s.doc, after);
            s.run("insert.pageNumber", &json!({"position": position, "format": format})).unwrap();
            assert_eq!(s.doc, after, "repeated numbering must not duplicate or replace content");
            let mut back = after.clone();
            for _ in 0..2 {
                back = wordcraft_docx::read(&wordcraft_docx::write(&back).unwrap()).unwrap();
                let id = if position == "top" { back.last_section.headers.default } else { back.last_section.footers.default }.unwrap();
                let reopened = StoryRef::Part(id);
                assert_eq!(back.plain_text(StoryRef::Body), "Body text");
                assert!(back.plain_text(reopened).starts_with("Keep this\nSecond line\nKeep table"));
                assert_eq!(back.para(reopened, &Path::top(0)).unwrap().props.align, Some(Align::Right));
                let Block::Table(table) = &*back.story(reopened).unwrap()[2] else { panic!("lost table") };
                let Block::Para(p) = &*table.rows[0].cells[0].blocks[0] else { panic!("lost cell") };
                assert_eq!(p.props_at(0).bold, Some(true));
                assert_eq!(fields(&back, reopened, "PAGE"), 1);
                assert_eq!(fields(&back, reopened, "NUMPAGES"), usize::from(format == "x of y"));
            }
        }
    }
}

#[test]
fn existing_page_field_in_a_table_is_preserved_with_its_switches() {
    for depth in [1, 17] {
        let mut s = Session::new(Document::from_text("Body"));
        let id = s.run("insert.footer", &json!({"text": "Keep footer"})).unwrap()["story"].as_u64().unwrap() as u32;
        let mut para = Paragraph::with_text("Existing page: ", CharProps::default());
        para.insert_object(para.len(), field(" page \\* MERGEFORMAT "), &CharProps::default()).unwrap();
        let mut blocks = vec![para_block(para)];
        for _ in 0..depth {
            let mut table = Table::new(1, 1, 300.0);
            table.rows[0].cells[0].blocks = blocks;
            blocks = vec![Arc::new(Block::Table(table))];
        }
        s.doc.story_mut(StoryRef::Part(id)).unwrap().extend(blocks);
        let before = s.doc.clone();
        s.run("insert.pageNumber", &json!({"position": "bottom", "format": "x of y"})).unwrap();
        assert!(s.doc == before, "PAGE in a table nested {depth} deep must not be duplicated");
    }
}

#[test]
fn existing_range_page_field_is_not_duplicated() {
    let mut s = Session::new(Document::new());
    let id = s.run("insert.header", &json!({"text": "Keep header"})).unwrap()["story"].as_u64().unwrap() as u32;
    let story = StoryRef::Part(id);
    let para = s.doc.para_mut(story, &Path::top(0)).unwrap();
    para.insert_object(para.len(), InlineObject::FieldStart { instr: "PAGE".into(), locked: true, dirty: true }, &CharProps::default()).unwrap();
    para.insert_text(para.len(), "7", &CharProps::default()).unwrap();
    para.insert_object(para.len(), InlineObject::FieldEnd, &CharProps::default()).unwrap();
    let before = s.doc.clone();
    s.run("insert.pageNumber", &json!({"position": "top"})).unwrap();
    assert_eq!(s.doc, before);
}

#[test]
fn unrelated_fields_do_not_prevent_numbering_and_formatted_empty_paragraphs_survive() {
    let mut s = Session::new(Document::new());
    let id = s.run("insert.header", &json!({"text": ""})).unwrap()["story"].as_u64().unwrap() as u32;
    let story = StoryRef::Part(id);
    let para = s.doc.para_mut(story, &Path::top(0)).unwrap();
    para.props.align = Some(Align::Right);
    let empty = para.clone();
    let mut para = Paragraph::new();
    para.insert_object(0, field("PAGEREF bookmark"), &CharProps::default()).unwrap();
    s.doc.story_mut(story).unwrap().push(para_block(para));
    s.run("insert.pageNumber", &json!({"position": "top"})).unwrap();
    assert_eq!(s.doc.para(story, &Path::top(0)).unwrap(), &empty);
    assert_eq!(fields(&s.doc, story, "PAGEREF"), 1);
    assert_eq!(fields(&s.doc, story, "PAGE"), 1);
}

#[test]
fn fresh_numbering_uses_one_paragraph_and_current_position_stays_inline() {
    for position in ["top", "bottom"] {
        let mut s = Session::new(Document::new());
        let id = s.run("insert.pageNumber", &json!({"position": position})).unwrap()["story"].as_u64().unwrap() as u32;
        assert_eq!(s.doc.story(StoryRef::Part(id)).unwrap().len(), 1);
    }
    let mut s = Session::new(Document::from_text("Before selected after"));
    s.run("select.text", &json!({"text": "selected"})).unwrap();
    let before = s.doc.clone();
    s.run("insert.pageNumber", &json!({"position": "current"})).unwrap();
    assert!(s.doc.parts.is_empty());
    assert_eq!(fields(&s.doc, StoryRef::Body, "PAGE"), 1);
    assert!(!s.doc.plain_text(StoryRef::Body).contains("selected"));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc, before);
}

#[test]
fn existing_page_field_in_nested_text_boxes_is_preserved() {
    for (command, position) in [("insert.header", "top"), ("insert.footer", "bottom")] {
        for levels in [1, 2] {
            let mut s = Session::new(Document::from_text("Body"));
            let id = s.run(command, &json!({"text": "Keep this"})).unwrap()["story"].as_u64().unwrap() as u32;
            let story = StoryRef::Part(id);
            let box_id = s.run("insert.textBox", &json!({"text": "Existing page: "})).unwrap()["story"].as_u64().unwrap() as u32;
            if levels == 2 {
                // Imported stories can contain a nested box, even though the insert command
                // does not offer one while the caret is already in a text box.
                let inner = s.doc.add_part(PartKind::TextBox, vec![para_block(Paragraph::new())]);
                let mut anchor = s.doc.para(story, &Path::top(0)).unwrap().objects.iter().find(|o| o.text_box() == Some(box_id)).unwrap().clone();
                if let InlineObject::Shape { story, .. } = &mut anchor {
                    *story = Some(inner);
                }
                s.doc.para_mut(StoryRef::Part(box_id), &Path::top(0)).unwrap().insert_object(0, anchor, &CharProps::default()).unwrap();
                s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(inner)));
            }
            s.run("insert.pageNumber", &json!({"position": "current"})).unwrap();
            let before = s.doc.clone();
            let selection = s.sel.clone();
            s.run("insert.pageNumber", &json!({"position": position, "format": "x of y"})).unwrap();
            assert_eq!(fields(&s.doc, story, "PAGE"), 0, "the text box already contains the page number");
            assert_eq!(s.doc, before);
            assert_eq!(s.sel, selection);
            let mut back = s.doc.clone();
            for _ in 0..2 {
                back = wordcraft_docx::read(&wordcraft_docx::write(&back).unwrap()).unwrap();
                assert_eq!(back.parts.keys().map(|id| fields(&back, StoryRef::Part(*id), "PAGE")).sum::<usize>(), 1);
                assert_eq!(back.parts.keys().map(|id| fields(&back, StoryRef::Part(*id), "NUMPAGES")).sum::<usize>(), 0);
            }
        }
    }
}

#[test]
fn existing_page_field_in_grouped_text_boxes_is_preserved() {
    for (command, position) in [("insert.header", "top"), ("insert.footer", "bottom")] {
        let mut s = Session::new(Document::from_text("Body"));
        let id = s.run(command, &json!({"text": "Keep this"})).unwrap()["story"].as_u64().unwrap() as u32;
        let story = StoryRef::Part(id);
        let box_id = s.run("insert.textBox", &json!({"text": "Existing page: "})).unwrap()["story"].as_u64().unwrap() as u32;
        s.run("insert.pageNumber", &json!({"position": "current"})).unwrap();
        let anchor = s.doc.para_mut(story, &Path::top(0)).unwrap().objects.iter_mut().find(|o| o.text_box() == Some(box_id)).unwrap();
        *anchor = InlineObject::Group {
            w: 144.0,
            h: 72.0,
            float: Default::default(),
            ch_w: 144.0,
            ch_h: 72.0,
            children: vec![GroupChild { x: 0.0, y: 0.0, obj: anchor.clone() }],
            canvas: None,
        };
        let before = s.doc.clone();
        let selection = s.sel.clone();
        s.run("insert.pageNumber", &json!({"position": position})).unwrap();
        assert_eq!(fields(&s.doc, story, "PAGE"), 0, "the grouped text box already contains the page number");
        assert_eq!(s.doc, before);
        assert_eq!(s.sel, selection);
        let mut back = s.doc.clone();
        for _ in 0..2 {
            back = wordcraft_docx::read(&wordcraft_docx::write(&back).unwrap()).unwrap();
            assert_eq!(back.parts.keys().map(|id| fields(&back, StoryRef::Part(*id), "PAGE")).sum::<usize>(), 1);
        }
    }
}

#[test]
fn page_fields_in_unrelated_text_boxes_do_not_prevent_numbering() {
    let mut s = Session::new(Document::from_text("Body"));
    let box_id = s.run("insert.textBox", &json!({"text": "Body page: "})).unwrap()["story"].as_u64().unwrap() as u32;
    s.run("insert.pageNumber", &json!({"position": "current"})).unwrap();
    let footer = s.run("insert.footer", &json!({"text": "Keep footer"})).unwrap()["story"].as_u64().unwrap() as u32;
    s.run("insert.pageNumber", &json!({"position": "bottom"})).unwrap();
    assert_eq!(fields(&s.doc, StoryRef::Part(box_id), "PAGE"), 1);
    assert_eq!(fields(&s.doc, StoryRef::Part(footer), "PAGE"), 1);
}

#[test]
fn cyclic_text_box_references_do_not_block_numbering() {
    let mut s = Session::new(Document::from_text("Body"));
    let footer = s.run("insert.footer", &json!({"text": "Keep footer"})).unwrap()["story"].as_u64().unwrap() as u32;
    let box_id = s.run("insert.textBox", &json!({"text": "A box"})).unwrap()["story"].as_u64().unwrap() as u32;
    let anchor = s.doc.para(StoryRef::Part(footer), &Path::top(0)).unwrap().objects.iter().find(|o| o.text_box() == Some(box_id)).unwrap().clone();
    s.doc.para_mut(StoryRef::Part(box_id), &Path::top(0)).unwrap().insert_object(0, anchor, &CharProps::default()).unwrap();
    s.run("insert.pageNumber", &json!({"position": "bottom"})).unwrap();
    assert_eq!(fields(&s.doc, StoryRef::Part(footer), "PAGE"), 1);
}

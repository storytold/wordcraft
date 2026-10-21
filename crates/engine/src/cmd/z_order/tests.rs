use super::*;
use crate::{CommandSpec, Registry, Selection};
use serde_json::json;
use wordcraft_doc::para::{Anchor, Float, GroupChild, ShapeKind};
use wordcraft_doc::{Paragraph, PartKind, Path, StoryRef, Table, para_block};

fn drawing(z_order: u32, wrap: Wrap) -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 80.0,
        h: 40.0,
        fill: Some(wordcraft_doc::Rgb(20, 40, 60)),
        stroke: None,
        stroke_width: 0.0,
        float: Float { z_order, wrap, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 90.0, ..Default::default() },
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}

fn document(ranks: &[u32]) -> Document {
    let mut p = Paragraph::new();
    for &rank in ranks {
        p.insert_object(p.len(), drawing(rank, Wrap::InFrontOfText), &Default::default()).unwrap();
    }
    let mut doc = Document::new();
    doc.body = vec![para_block(p)];
    doc
}

#[test]
fn new_wordart_orders_before_save() {
    let mut s = Session::new(document(&[42]));
    s.run("insert.wordArt", &json!({"text": "New"})).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [43, 42]);
    let l = s.layout();
    let order: Vec<_> = l.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            wordcraft_layout::Placed::Object { story: StoryRef::Body, path, off, .. } if *path == Path::top(0) => Some(*off),
            _ => None,
        })
        .collect();
    assert_eq!(order, [wordcraft_doc::para::OBJ.len_utf8(), 0]);
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(document_ranks(&back).unwrap(), [43, 42]);
}

#[test]
fn batch_rebases_without_source() {
    let source = document(&[7, 2, 7]);
    let frag = source.copy_range(&source.start_of(StoryRef::Body), &source.end_of(StoryRef::Body));
    let before = frag.clone();
    let mut owned = frag.clone();
    let mut target = document(&[100]);
    fragment(&mut target, &mut owned, Intent::Fresh).unwrap();
    assert_eq!(fragment_ranks(&owned).unwrap(), [102, 101, 102]);
    assert_eq!(frag, before);
    assert_eq!(document_ranks(&source).unwrap(), [7, 2, 7]);
    assert_eq!(document_ranks(&target).unwrap(), [100]);
    let mut s = Session::new(target);
    let at = s.doc.end_of(StoryRef::Body);
    s.sel = Selection::caret(at);
    super::super::paste::insert(&mut s, frag).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [100, 102, 101, 102]);
}

#[test]
fn exhaustion_keeps_ties() {
    let mut doc = document(&[7, u32::MAX, 7]);
    let assigned = reserve(&mut doc, &[8, 3, 8]).unwrap();
    assert_eq!(document_ranks(&doc).unwrap(), [1, 2, 1]);
    assert_eq!(assigned, [4, 3, 4]);
    let back = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(document_ranks(&back).unwrap(), [1, 2, 1]);
    let mut near = document(&[u32::MAX - 1]);
    assert_eq!(reserve(&mut near, &[0, 1]).unwrap(), [2, 3]);
    assert_eq!(document_ranks(&near).unwrap(), [1]);
}

#[test]
fn inline_transition_and_move() {
    let mut doc = document(&[12]);
    let at = doc.end_of(StoryRef::Body);
    doc.insert_object(&at, drawing(u32::MAX, Wrap::Inline), &Default::default()).unwrap();
    let mut s = Session::new(doc);
    s.sel = Selection { anchor: at.clone(), focus: Pos { off: at.off + wordcraft_doc::para::OBJ.len_utf8(), ..at.clone() } };
    s.run("arrange.wrap", &json!({"wrap": "square"})).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [12, 13]);
    s.run("arrange.rotation", &json!({"degrees": 90})).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [12, 13]);
    s.run("arrange.bounds", &json!({"page": 0, "x": 240, "y": 220})).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [12, 13]);
    let copy_source = s.doc.clone();
    s.run("arrange.copyTo", &json!({"page": 0, "x": 100, "y": 220})).unwrap();
    let ranks = document_ranks(&s.doc).unwrap();
    assert_eq!(ranks.len(), 3);
    assert!(ranks.contains(&14));
    assert_eq!(document_ranks(&copy_source).unwrap(), [12, 13]);
}

#[test]
fn normalization_undo_redo() {
    let before = document(&[7, 7, u32::MAX]);
    let mut s = Session::new(before.clone());
    s.run("insert.wordArt", &json!({"text": "New"})).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [3, 1, 1, 2]);
    let after = s.doc.clone();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc, before);
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc, after);
}

#[test]
fn failed_command_restores_ranks() {
    fn fail(s: &mut Session, _: &serde_json::Value) -> crate::CmdResult {
        let mut obj = drawing(0, Wrap::InFrontOfText);
        object(&mut s.doc, &mut obj, Intent::Fresh)?;
        assert_eq!(document_ranks(&s.doc).unwrap(), [1, 1, 2]);
        Err(CmdError::Failed("synthetic failure after normalization".into()))
    }
    let before = document(&[7, 7, u32::MAX]);
    let mut s = Session::new(before.clone());
    s.registry = Arc::new(Registry::new(vec![CommandSpec::new("test.orderFailure", "", "", fail)]));
    assert!(s.run("test.orderFailure", &json!({})).is_err());
    assert_eq!(s.doc, before);
}

#[test]
fn bounds_precede_mutation() {
    let mut doc = document(&[u32::MAX]);
    let mut block = Block::Para(Paragraph::new());
    for _ in 0..=MAX_ORDER_DEPTH {
        let mut table = Table::new(1, 1, 40.0);
        table.rows[0].cells[0].blocks = vec![Arc::new(block)];
        block = Block::Table(table);
    }
    doc.body.push(Arc::new(block));
    let before = doc.clone();
    let mut obj = drawing(0, Wrap::InFrontOfText);
    let original = obj.clone();
    assert!(object(&mut doc, &mut obj, Intent::Fresh).is_err());
    assert_eq!(doc, before);
    assert_eq!(obj, original);
    let mut ranks = Vec::new();
    assert!(scan_block(&Block::Para(Paragraph::new()), 0, &mut Budget(0), &mut ranks).is_err());
    let too_many = vec![0; MAX_ORDER_ITEMS + 1];
    assert!(reserve(&mut doc, &too_many).is_err());
    assert_eq!(doc, before);
}

#[test]
fn preserve_and_group_scope() {
    let mut doc = document(&[u32::MAX]);
    let mut existing = drawing(4, Wrap::InFrontOfText);
    object(&mut doc, &mut existing, Intent::Preserve).unwrap();
    assert_eq!(existing.frame().unwrap().2.z_order, 4);
    assert_eq!(document_ranks(&doc).unwrap(), [u32::MAX]);
    let child = drawing(u32::MAX, Wrap::InFrontOfText);
    let group = InlineObject::Group {
        w: 80.0,
        h: 40.0,
        float: Float { wrap: Wrap::InFrontOfText, z_order: 2, ..Default::default() },
        ch_w: 80.0,
        ch_h: 40.0,
        children: vec![GroupChild { x: 0.0, y: 0.0, obj: child.clone() }],
        canvas: None,
    };
    let mut clean = Document::new();
    clean.insert_object(&Pos::body(0, 0), group, &Default::default()).unwrap();
    assert_eq!(reserve(&mut clean, &[0]).unwrap(), [3]);
    let InlineObject::Group { children, .. } = &clean.para_at(&Pos::body(0, 0)).unwrap().objects[0] else { panic!("group") };
    assert_eq!(children[0].obj, child);
}

#[test]
fn pasted_story_source_is_private() {
    let mut source = document(&[9]);
    let part = source.add_part(PartKind::TextBox, document(&[6, 6]).body);
    let p = source.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    let InlineObject::Shape { story, .. } = &mut p.objects[0] else { panic!("shape") };
    *story = Some(part);
    p.touch();
    let frag = source.copy_range(&source.start_of(StoryRef::Body), &source.end_of(StoryRef::Body));
    let before = source.clone();
    let mut owned = frag.clone();
    fragment(&mut document(&[20]), &mut owned, Intent::Fresh).unwrap();
    assert_eq!(fragment_ranks(&owned).unwrap(), [22, 21, 21]);
    assert_eq!(source, before);
    assert_eq!(fragment_ranks(&frag).unwrap(), [9, 6, 6]);
}

#[test]
fn protected_ranks_roll_back() {
    let mut doc = document(&[u32::MAX]);
    doc.body.push(para_block(Paragraph::with_text("Editable", Default::default())));
    let mut s = Session::new(doc);
    s.sel = Selection { anchor: Pos::body(1, 0), focus: Pos::body(1, 8) };
    s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    // Permission markers are three-byte objects; derive the caret after the start marker.
    let editable_start = s.sel.ordered().0;
    let at = Pos { off: editable_start.off + 2, ..editable_start };
    let permit = s.doc.perm_ranges().into_iter().next().unwrap();
    assert!(permit.start <= at && at < permit.end, "the insertion starts inside the editable exception");
    assert!(s.doc.para_at(&at).unwrap().text.is_char_boundary(at.off));
    s.sel = Selection::caret(at);
    let before = s.doc.clone();
    let err = s.run("insert.shape", &json!({"kind": "straightConnector"})).unwrap_err();
    assert!(err.to_string().contains("only the editable regions"), "{err}");
    assert_eq!(s.doc, before);
}

#[test]
fn tracked_move_keeps_rank() {
    let mut doc = document(&[7, 12]);
    doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().insert_text(0, "Move this sentence. ", &Default::default()).unwrap();
    doc.body.push(para_block(Paragraph::with_text("Destination", Default::default())));
    let mut s = Session::new(doc);
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(1, 0) };
    s.run("edit.cut", &json!({})).unwrap();
    assert!(s.pending_move.is_some());
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
    s.run("edit.paste", &json!({})).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [7, 12, 7, 12]);
    assert!(s.doc.revisions.iter().any(|r| r.move_name.is_some()));
}

#[test]
fn fresh_connector_and_inline_drag() {
    let mut s = Session::new(document(&[42]));
    // Append explicitly; a new session otherwise starts before the existing drawing.
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
    s.run("insert.shape", &json!({"kind": "straightConnector"})).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [42, 43]);
    s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [42, 43]);
    s.run("arrange.bounds", &json!({"page": 0, "x": 200, "y": 240})).unwrap();
    assert_eq!(document_ranks(&s.doc).unwrap(), [42, 43, 44]);
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert_eq!(document_ranks(&back).unwrap(), [42, 43, 44]);
}

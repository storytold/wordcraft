use super::*;

#[test]
fn new_document_is_valid() {
    let d = Document::new();
    assert_eq!(d.body.len(), 1);
    assert_eq!(d.para_paths(StoryRef::Body).len(), 1);
    assert_eq!(d.plain_text(StoryRef::Body), "");
    assert_eq!(d.sections().len(), 1);
}

#[test]
fn word_count() {
    let d = Document::from_text("Hello, world!\nThis is — a test 42.\n\n");
    assert_eq!(d.word_count(), 7);
    assert_eq!(count_words(" -- "), 0);
}

#[test]
fn sections_and_section_mut() {
    let mut d = Document::from_text("a\nb\nc");
    if let Some(Block::Para(p)) = d.body.get_mut(0).map(Arc::make_mut) {
        p.section = Some(Box::new(SectionProps { landscape: true, ..Default::default() }));
    }
    let s = d.sections();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].0, 0);
    assert_eq!(d.section_index_of(0), 0);
    assert_eq!(d.section_index_of(2), 1);
    d.section_mut(0).margin_left = 10.0;
    d.section_mut(2).margin_left = 20.0;
    assert_eq!(d.sections()[0].1.margin_left, 10.0);
    assert_eq!(d.last_section.margin_left, 20.0);
}

#[test]
fn parts_and_media() {
    let mut d = Document::new();
    let id = d.add_part(PartKind::Header, Vec::new());
    assert_eq!(d.story(StoryRef::Part(id)).map(|b| b.len()), Some(1));
    let k1 = d.add_media(vec![1, 2, 3], "png");
    let k2 = d.add_media(vec![1, 2, 3], "png");
    let k3 = d.add_media(vec![4], "png");
    assert_eq!(k1, k2);
    assert_ne!(k1, k3);
}

#[test]
fn clone_is_shallow() {
    let d = Document::from_text("a\nb");
    let mut e = d.clone();
    assert!(Arc::ptr_eq(&d.body[0], &e.body[0]));
    e.insert_text(&Pos::body(1, 0), "x", &CharProps::default()).unwrap();
    assert!(Arc::ptr_eq(&d.body[0], &e.body[0]));
    assert!(!Arc::ptr_eq(&d.body[1], &e.body[1]));
    assert_eq!(d.plain_text(StoryRef::Body), "a\nb");
}

#[test]
fn json_round_trip() {
    let mut d = Document::from_text("Hello\nWorld");
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 5), &|c| c.bold = Some(true)).unwrap();
    let j = serde_json::to_string(&d).unwrap();
    let back: Document = serde_json::from_str(&j).unwrap();
    assert_eq!(back.plain_text(StoryRef::Body), "Hello\nWorld");
    assert_eq!(back.para(StoryRef::Body, &Path::top(0)).unwrap().runs[0].props.bold, Some(true));
}

#[test]
fn ensure_nonempty_repairs() {
    let mut d = Document::new();
    d.body.clear();
    d.body.push(Arc::new(Block::Table(Table::new(1, 1, 100.0))));
    d.ensure_nonempty();
    assert!(matches!(d.body.last().map(|b| &**b), Some(Block::Para(_))));
}

#[test]
fn text_box_anchor_finds_the_owning_shape() {
    let mut d = Document::from_text("Before after");
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Inside", CharProps::default()))]);
    let shape = |story| InlineObject::Shape {
        kind: para::ShapeKind::TextBox,
        w: 144.0,
        h: 72.0,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story,
    };
    // A plain shape before it doesn't count.
    d.insert_object(&Pos::body(0, 0), shape(None), &CharProps::default()).unwrap();
    let end = d.insert_object(&Pos::body(0, 10), shape(Some(id)), &CharProps::default()).unwrap();
    assert_eq!(d.text_box_anchor(id), Some(end));
    assert_eq!(d.text_box_anchor(id + 1), None);
}

#[test]
fn word_count_including_notes_counts_used_boxes_and_notes_only() {
    let mut d = Document::from_text("one two");
    let shape = |story| InlineObject::Shape {
        kind: para::ShapeKind::TextBox,
        w: 144.0,
        h: 72.0,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story: Some(story),
    };
    let para_of = |t: &str| vec![para_block(Paragraph::with_text(t, CharProps::default()))];
    let boxed = d.add_part(PartKind::TextBox, para_of("three four"));
    let nested = d.add_part(PartKind::TextBox, para_of("five"));
    let note = d.add_part(PartKind::Footnote, para_of("six seven"));
    // Not counted: an orphaned box (deleted or cut), a header, a comment.
    d.add_part(PartKind::TextBox, para_of("orphan words here"));
    d.add_part(PartKind::Header, para_of("header words"));
    d.add_part(PartKind::Comment, para_of("comment words"));
    d.insert_object(&Pos::body(0, 3), shape(boxed), &CharProps::default()).unwrap();
    d.insert_object(&Pos { story: StoryRef::Part(boxed), path: Path::top(0), off: 0 }, shape(nested), &CharProps::default()).unwrap();
    d.insert_object(
        &Pos::body(0, 0),
        InlineObject::NoteRef { kind: para::NoteKind::Footnote, id: note, custom: String::new() },
        &CharProps::default(),
    )
    .unwrap();
    // A box pointing at a header doesn't make the header count.
    let hdr = d.parts.iter().find(|(_, p)| p.kind == PartKind::Header).map(|(k, _)| *k).unwrap();
    d.insert_object(&Pos::body(0, 0), shape(hdr), &CharProps::default()).unwrap();
    assert_eq!(d.word_count(), 2);
    assert_eq!(d.word_count_including_notes(), 7);
    assert_eq!(d.counted_stories(), vec![StoryRef::Body, StoryRef::Part(note), StoryRef::Part(boxed), StoryRef::Part(nested)]);
}

#[test]
fn counted_stories_survive_self_nested_boxes() {
    let mut d = Document::from_text("a");
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("b", CharProps::default()))]);
    let shape = InlineObject::Shape {
        kind: para::ShapeKind::TextBox,
        w: 10.0,
        h: 10.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: Some(id),
    };
    d.insert_object(&Pos::body(0, 0), shape.clone(), &CharProps::default()).unwrap();
    d.insert_object(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 }, shape, &CharProps::default()).unwrap();
    assert_eq!(d.word_count_including_notes(), 2);
}

#[test]
fn prune_text_boxes_drops_only_unshown_ones() {
    let shape = |story| InlineObject::Shape {
        kind: para::ShapeKind::TextBox,
        w: 50.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: Some(story),
    };
    let para_of = |t: &str| vec![para_block(Paragraph::with_text(t, CharProps::default()))];
    let mut d = Document::from_text("body");
    assert_eq!(d.prune_text_boxes(), 0, "nothing to do without text boxes");
    let shown = d.add_part(PartKind::TextBox, para_of("shown"));
    let nested = d.add_part(PartKind::TextBox, para_of("inside shown"));
    let orphan = d.add_part(PartKind::TextBox, para_of("deleted box"));
    let orphan_child = d.add_part(PartKind::TextBox, para_of("inside the deleted box"));
    let hdr = d.add_part(PartKind::Header, para_of("header"));
    let in_header = d.add_part(PartKind::TextBox, para_of("box in the header"));
    d.insert_object(&Pos::body(0, 0), shape(shown), &CharProps::default()).unwrap();
    let at = |id| Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 };
    d.insert_object(&at(shown), shape(nested), &CharProps::default()).unwrap();
    d.insert_object(&at(orphan), shape(orphan_child), &CharProps::default()).unwrap();
    d.insert_object(&at(hdr), shape(in_header), &CharProps::default()).unwrap();
    assert_eq!(d.prune_text_boxes(), 2);
    let kept: Vec<u32> = d.parts.keys().copied().collect();
    assert_eq!(kept, vec![shown, nested, hdr, in_header]);
    assert_eq!(d.prune_text_boxes(), 0);
}

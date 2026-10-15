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
fn word_count_includes_numbered_list_labels_but_not_bullets() {
    let mut d = Document::new();
    let num_id = d.numbering.add_list(ListKind::Numbered);
    let bul_id = d.numbering.add_list(ListKind::Bullet);

    let mut p1 = Paragraph::with_text("Apple", CharProps::default());
    p1.props.numbering = Some(props::NumRef { num: num_id, level: 0 });
    let mut p2 = Paragraph::with_text("Banana", CharProps::default());
    p2.props.numbering = Some(props::NumRef { num: num_id, level: 0 });

    let mut p3 = Paragraph::with_text("Cherry", CharProps::default());
    p3.props.numbering = Some(props::NumRef { num: bul_id, level: 0 });
    let mut p4 = Paragraph::with_text("Date", CharProps::default());
    p4.props.numbering = Some(props::NumRef { num: bul_id, level: 0 });

    d.body = vec![Arc::new(Block::Para(p1)), Arc::new(Block::Para(p2)), Arc::new(Block::Para(p3)), Arc::new(Block::Para(p4))];

    // Numbered items contribute 2 words each (label + text) = 4 words.
    // Bullet items contribute 1 word each (bullet marker is not a word + text) = 2 words.
    assert_eq!(d.word_count(), 6);
    let plain = d.plain_text(StoryRef::Body);
    assert_eq!(plain, "Apple\nBanana\nCherry\nDate");
}

#[test]
fn word_count_includes_numbered_labels_with_nothing_suffix() {
    let mut d = Document::new();
    let num_id = d.numbering.add_list(ListKind::Numbered);
    let aid = d.numbering.num(num_id).expect("list num exists").abstract_id;
    let a = d.numbering.abstracts.iter_mut().find(|a| a.id == aid).expect("abstract num exists");
    a.levels[0].suffix = numbering::LevelSuffix::Nothing;

    let mut p1 = Paragraph::with_text("Apple", CharProps::default());
    p1.props.numbering = Some(props::NumRef { num: num_id, level: 0 });
    let mut p2 = Paragraph::with_text("Banana Cherry", CharProps::default());
    p2.props.numbering = Some(props::NumRef { num: num_id, level: 0 });
    d.body = vec![Arc::new(Block::Para(p1)), Arc::new(Block::Para(p2))];

    // Label "1." and text "Apple" must each be counted as a separate word (2 words),
    // and label "2." and text "Banana Cherry" as 3 words (total: 5 words),
    // even though LevelSuffix::Nothing produces no visible separator between label and text.
    assert_eq!(d.word_count(), 5);
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
fn free_ids_skip_taken_ones_instead_of_overflowing() {
    assert_eq!(free_id(None, std::iter::empty(), 1), Some(1));
    assert_eq!(free_id(Some(7), [3, 7].into_iter(), 1), Some(8));
    assert_eq!(free_id(Some(0), [0].into_iter(), 1), Some(1));
    // A file used the largest id: the lowest free one, not a wrapped 0 or a taken id.
    assert_eq!(free_id(Some(u32::MAX), [1, 2, 4, u32::MAX].into_iter(), 1), Some(3));
    assert_eq!(free_id(Some(u32::MAX), [u32::MAX, 1, 0, 1].into_iter(), 0), Some(2));
    // Every id from `min` up taken: none, rather than a duplicate.
    assert_eq!(free_id(Some(u32::MAX), [u32::MAX, u32::MAX - 1, u32::MAX - 2].into_iter(), u32::MAX - 2), None);
}

#[test]
fn parts_and_lists_after_the_largest_ids_take_free_ones() {
    // A file can carry any ids, 0 and u32::MAX included; new ones must not overflow or replace them.
    let mut d = Document::new();
    for id in [0, 1, u32::MAX] {
        let blocks = vec![para_block(Paragraph::with_text(&id.to_string(), CharProps::default()))];
        d.parts.insert(id, Part { kind: PartKind::Header, blocks, ..Default::default() });
    }
    let parts = d.parts.clone();
    assert_eq!(d.try_add_part(PartKind::Comment, Vec::new()), Ok(2));
    assert_eq!(d.add_part(PartKind::Footer, Vec::new()), 3);
    assert!(parts.iter().all(|(k, p)| d.parts.get(k) == Some(p)));

    let n = &mut d.numbering;
    n.add_list(ListKind::Bullet);
    n.add_list(ListKind::Bullet);
    n.abstracts[1].id = u32::MAX;
    n.nums[1].id = u32::MAX;
    n.nums[1].abstract_id = u32::MAX;
    let lists = n.clone();
    let num = n.try_add_list(ListKind::Numbered).unwrap();
    assert_eq!((num, n.num(num).map(|x| x.abstract_id)), (2, Some(1)));
    assert_eq!(n.abstract_of(num).map(|a| &a.levels), Some(&numbering::levels_for(ListKind::Numbered)));
    assert_eq!(n.restart(num), Some(3));
    assert_eq!(n.num(3).map(|x| x.abstract_id), Some(1));
    assert_eq!((&n.abstracts[..2], &n.nums[..2]), (&lists.abstracts[..], &lists.nums[..]));
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
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
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
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
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
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
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
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
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

/// `blocks` in the only cell of `levels` tables, each nested in the next.
fn nested_tables(levels: usize, blocks: Blocks) -> Blocks {
    (0..levels).fold(blocks, |inner, _| {
        let mut t = Table::new(1, 1, 100.0);
        t.rows[0].cells[0].blocks = inner;
        vec![Arc::new(Block::Table(t))]
    })
}

/// A paragraph showing text box story `story` (drawing id `id`).
fn box_para(story: u32, id: u32) -> Blocks {
    let shape = InlineObject::Shape {
        kind: para::ShapeKind::TextBox,
        w: 50.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: connector::ShapeExtra { id, ..Default::default() },
    };
    let mut p = Paragraph::with_text("cell", CharProps::default());
    p.insert_object(0, shape, &CharProps::default()).unwrap();
    vec![para_block(p)]
}

#[test]
fn adopting_relinks_lists_in_deeply_nested_tables() {
    // As #1339: a list paragraph 20 tables down takes the list it is brought in with.
    let mut ours = Document::from_text("ours");
    let our_list = ours.numbering.add_list(numbering::ListKind::Bullet);
    let mut theirs = Document::from_text("theirs");
    let list = theirs.numbering.add_list(numbering::ListKind::Bullet);
    assert_eq!(list, our_list);
    let mut p = Paragraph::with_text("item", CharProps::default());
    p.props.numbering = Some(props::NumRef { num: list, level: 0 });
    theirs.body = nested_tables(20, vec![para_block(p)]);
    let mut nums = Vec::new();
    for b in &ours.adopt(theirs).blocks {
        edit::each_para(b, 0, &mut |p| nums.extend(p.props.numbering.map(|n| n.num)));
    }
    assert_eq!(nums.len(), 1);
    assert_ne!(nums[0], our_list);
}

#[test]
fn text_boxes_shown_from_deeply_nested_tables_are_kept() {
    // #1339: the paragraph walk stopped 16 tables down, so a box shown only from deeper was taken
    // for unshown and the next edit pruned its story.
    let para_of = |t: &str| vec![para_block(Paragraph::with_text(t, CharProps::default()))];
    let mut d = Document::from_text("body");
    let at_17 = d.add_part(PartKind::TextBox, para_of("17 tables down"));
    let at_40 = d.add_part(PartKind::TextBox, para_of("40 tables down"));
    let inner = d.add_part(PartKind::TextBox, para_of("deep in a box"));
    let outer = d.add_part(PartKind::TextBox, nested_tables(18, box_para(inner, 0)));
    let in_header = d.add_part(PartKind::TextBox, para_of("deep in a header"));
    let hdr = d.add_part(PartKind::Header, nested_tables(20, box_para(in_header, 0)));
    let unshown = d.add_part(PartKind::TextBox, para_of("shown nowhere"));
    d.body.extend(nested_tables(17, box_para(at_17, 0)));
    d.body.extend(nested_tables(40, box_para(at_40, 7)));
    d.body.extend(box_para(outer, 0));
    assert_eq!(d.prune_text_boxes(), 1);
    assert!(!d.parts.contains_key(&unshown));
    let kept: Vec<u32> = d.parts.keys().copied().collect();
    assert_eq!(kept, vec![at_17, at_40, inner, outer, in_header, hdr]);
    assert_eq!(d.prune_text_boxes(), 0);
    // Counted in reading order, the nested box after its own; new drawing ids above the deep one.
    let part = StoryRef::Part;
    assert_eq!(d.counted_stories(), vec![StoryRef::Body, part(at_17), part(at_40), part(outer), part(inner)]);
    assert_eq!(d.next_shape_id(), 8);
}

#[test]
fn a_deeply_nested_table_is_copied_with_its_text_boxes() {
    // A copy takes the box 17 tables down along, so pasting it into another document shows its
    // text, not the other document's box that happens to have the same id.
    let para_of = |t: &str| vec![para_block(Paragraph::with_text(t, CharProps::default()))];
    let mut d = Document::from_text("body");
    let deep = d.add_part(PartKind::TextBox, para_of("deep box"));
    d.body.extend(nested_tables(17, box_para(deep, 0)));
    let frag = d.body_fragment();
    assert_eq!(frag.parts.keys().copied().collect::<Vec<_>>(), vec![deep]);
    let mut e = Document::new();
    let own = e.add_part(PartKind::TextBox, para_of("its own box"));
    assert_eq!(own, deep, "the same id in both documents");
    e.body = box_para(own, 0);
    let end = e.end_of(StoryRef::Body);
    e.insert_fragment(&end, &frag).unwrap();
    let mut shown = Vec::new();
    e.objects_in_reading_order(&mut |o| shown.extend(o.text_box()));
    let texts: Vec<String> = shown.iter().map(|id| e.plain_text(StoryRef::Part(*id))).collect();
    assert_eq!(texts, vec!["its own box", "deep box"]);
}

#[test]
fn notes_in_text_box_follow_nested_boxes_once() {
    let shape = |story| InlineObject::Shape {
        kind: para::ShapeKind::TextBox,
        w: 50.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let note = |id| InlineObject::NoteRef { kind: para::NoteKind::Footnote, id, custom: String::new() };
    let mut d = Document::new();
    let outer = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::new())]);
    let inner = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::new())]);
    let (n1, n2) = (d.add_part(PartKind::Footnote, Vec::new()), d.add_part(PartKind::Footnote, Vec::new()));
    let at = |id| Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 };
    // outer: [note n1][box inner][box outer (itself)]; inner: [note n2]
    d.insert_object(&at(outer), shape(outer), &CharProps::default()).unwrap();
    d.insert_object(&at(outer), shape(inner), &CharProps::default()).unwrap();
    d.insert_object(&at(outer), note(n1), &CharProps::default()).unwrap();
    d.insert_object(&at(inner), note(n2), &CharProps::default()).unwrap();
    assert_eq!(d.notes_in_text_box(outer), vec![n1, n2]);
    assert_eq!(d.notes_in_text_box(inner), vec![n2]);
    assert!(d.notes_in_text_box(n1).is_empty(), "not a text box");
}

fn fs(instr: &str) -> InlineObject {
    InlineObject::FieldStart { instr: instr.into(), locked: false, dirty: false }
}

#[test]
fn field_ranges_pair_across_paragraphs_and_nest() {
    let mut d = Document::from_text("See .\nBib one\nBib two");
    let c = CharProps::default();
    // Citation in paragraph 0: "See ␣[cite](Smith 2020)␣." with a nested field inside.
    let p0 = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p0.insert_object(4, fs("ADDIN ZOTERO_ITEM CSL_CITATION {}"), &c).unwrap();
    let n = p0.insert_text(7, "(Smith 2020)", &c).unwrap();
    p0.insert_object(7 + n, InlineObject::FieldEnd, &c).unwrap();
    p0.insert_object(7, fs("ADDIN inner"), &c).unwrap();
    p0.insert_object(10, InlineObject::FieldEnd, &c).unwrap();
    // Bibliography from paragraph 1 to the end of paragraph 2.
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().insert_object(0, fs("ADDIN ZOTERO_BIBL {} CSL_BIBLIOGRAPHY"), &c).unwrap();
    let p2 = d.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    let end = p2.len();
    p2.insert_object(end, InlineObject::FieldEnd, &c).unwrap();

    let r = d.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 3);
    assert!(r[0].instr.starts_with("ADDIN ZOTERO_ITEM"));
    assert_eq!((r[0].start.off, r[0].depth), (4, 0));
    assert_eq!((r[1].instr.as_str(), r[1].depth), ("ADDIN inner", 1));
    assert_eq!(r[2].start.path, Path::top(1));
    assert_eq!(r[2].end.path, Path::top(2));
    assert!(!d.has_unbalanced_field_ranges());
    assert_eq!(d.balance_field_ranges(), 0);
    // Markers contribute no text.
    assert_eq!(d.plain_text(StoryRef::Body), "See (Smith 2020).\nBib one\nBib two");
}

#[test]
fn orphan_field_markers_are_removed() {
    let mut d = Document::from_text("ab\ncd");
    let c = CharProps::default();
    let p0 = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p0.insert_object(0, InlineObject::FieldEnd, &c).unwrap();
    // OBJ is three bytes: "␣a[x]b]" → start after "␣a", end after "b".
    p0.insert_object(4, fs("ADDIN x"), &c).unwrap();
    p0.insert_object(8, InlineObject::FieldEnd, &c).unwrap();
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().insert_object(1, fs("ADDIN open"), &c).unwrap();
    assert!(d.has_unbalanced_field_ranges());
    assert_eq!(d.balance_field_ranges(), 2);
    assert!(!d.has_unbalanced_field_ranges());
    let r = d.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].instr, "ADDIN x");
    assert_eq!(d.plain_text(StoryRef::Body), "ab\ncd");
}

#[test]
fn custom_props_set_get_remove() {
    let mut d = Document::new();
    assert_eq!(d.custom_prop("ZOTERO_PREF_1"), None);
    d.set_custom_prop("ZOTERO_PREF_1", "<data/>");
    d.set_custom_prop("Other", "x");
    d.set_custom_prop("zotero_pref_1", "<data2/>");
    assert_eq!(d.custom_props.len(), 2);
    assert_eq!(d.custom_props[0].name, "ZOTERO_PREF_1");
    assert_eq!(d.custom_prop("Zotero_Pref_1"), Some("<data2/>"));
    assert!(d.remove_custom_prop("other"));
    assert!(!d.remove_custom_prop("other"));
    assert_eq!(d.custom_props.len(), 1);
}

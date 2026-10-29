use super::*;
use crate::{StoryRef, Table};

fn doc(lines: &[&str]) -> Document {
    Document::from_text(&lines.join("\n"))
}

/// Paths between two positions are those of all the story's paragraphs between them.
#[test]
fn paths_between_matches_every_path_filtered() {
    let mut d = doc(&["a", "b", "c", "d", "e"]);
    let mut inner = Table::new(2, 2, 100.0);
    inner.rows[1].cells[0].blocks.push(crate::para_block(crate::Paragraph::new()));
    let mut t = Table::new(3, 2, 200.0);
    t.rows[1].cells[1].blocks.insert(0, Arc::new(Block::Table(inner)));
    d.insert_block(StoryRef::Body, &Path::top(2), Block::Table(t)).unwrap();
    d.insert_block(StoryRef::Body, &Path::top(4), Block::Table(Table::new(1, 3, 200.0))).unwrap();
    let all = d.para_paths(StoryRef::Body);
    for a in &all {
        for b in &all {
            let (pa, pb) = (Pos { story: StoryRef::Body, path: a.clone(), off: 0 }, Pos { story: StoryRef::Body, path: b.clone(), off: 0 });
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let want: Vec<Path> = all.iter().filter(|p| *p >= lo && *p <= hi).cloned().collect();
            assert_eq!(d.paths_between(&pa, &pb), want, "{a:?} {b:?}");
        }
    }
}

#[test]
fn split_and_join() {
    let mut d = doc(&["Hello world"]);
    let p = d.split_paragraph(&Pos::body(0, 5)).unwrap();
    assert_eq!(p, Pos::body(1, 0));
    assert_eq!(d.plain_text(StoryRef::Body), "Hello\n world");
    let at = d.delete_range(&Pos::body(0, 5), &Pos::body(1, 0)).unwrap();
    assert_eq!(at, Pos::body(0, 5));
    assert_eq!(d.plain_text(StoryRef::Body), "Hello world");
}

#[test]
fn delete_range_with_bad_end_leaves_document_unchanged() {
    let mut d = doc(&["one", "two", "three"]);
    let before = d.plain_text(StoryRef::Body);
    // Missing paragraph as the second end.
    assert!(d.delete_range(&Pos::body(0, 1), &Pos::body(9, 0)).is_err());
    assert_eq!(d.plain_text(StoryRef::Body), before);
    // Offset past the end of the second paragraph.
    assert!(d.delete_range(&Pos::body(0, 1), &Pos::body(1, 99)).is_err());
    assert_eq!(d.plain_text(StoryRef::Body), before);
    // Offset inside a multi-byte character.
    let mut d = doc(&["abc", "\u{e9}x"]);
    assert!(d.delete_range(&Pos::body(0, 1), &Pos::body(1, 1)).is_err());
    assert_eq!(d.plain_text(StoryRef::Body), "abc\n\u{e9}x");
    // Range entering a table with a bad offset at its start: the cell text stays.
    let mut d = doc(&["before", "after"]);
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(Table::new(2, 1, 200.0))).unwrap();
    d.insert_text(&Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 0 }, "x", &CharProps::default()).unwrap();
    let before = d.plain_text(StoryRef::Body);
    let cell = Pos { story: StoryRef::Body, path: Path(vec![1, 1, 0, 0]), off: 0 };
    assert!(d.delete_range(&Pos::body(0, 99), &cell).is_err());
    assert_eq!(d.plain_text(StoryRef::Body), before);
}

#[test]
fn delete_multi() {
    let mut d = doc(&["one", "two", "three"]);
    d.delete_range(&Pos::body(2, 2), &Pos::body(0, 1)).unwrap();
    assert_eq!(d.plain_text(StoryRef::Body), "oree");
    assert_eq!(d.body.len(), 1);
}

/// #2127: a range over two section breaks joins three sections into the last; the headers
/// and footers it showed by linking to earlier ones stay, each from the nearest section that
/// named it, as they resolved before the delete.
#[test]
fn delete_over_two_section_breaks_keeps_the_linked_headers() {
    let mut d = doc(&["A", "B", "C"]);
    let set = |default| crate::section::HeaderSet { default: Some(default), ..Default::default() };
    let s1 = crate::section::SectionProps { headers: set(1), footers: set(5), ..Default::default() };
    let s2 = crate::section::SectionProps { headers: set(2), ..Default::default() };
    let s3 = crate::section::SectionProps { margin_left: 99.0, ..Default::default() };
    for (i, s) in [(0, s1), (1, s2), (2, s3.clone())] {
        if let Some(Block::Para(p)) = d.body.get_mut(i).map(Arc::make_mut) {
            p.section = Some(Box::new(s));
        }
    }
    assert_eq!(d.sections().len(), 4);
    d.delete_range(&Pos::body(0, 1), &Pos::body(2, 0)).unwrap();
    assert_eq!(d.plain_text(StoryRef::Body), "AC");
    let sections = d.sections();
    assert_eq!(sections.len(), 2);
    assert_eq!(*sections[0].1, crate::section::SectionProps { headers: set(2), footers: set(5), ..s3 });
}

#[test]
fn format_across() {
    let mut d = doc(&["abc", "def"]);
    d.format_range(&Pos::body(0, 1), &Pos::body(1, 2), &|c| c.bold = Some(true)).unwrap();
    let p0 = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(p0.props_of_char(0).bold, None);
    assert_eq!(p0.props_of_char(1).bold, Some(true));
    let p1 = d.para(StoryRef::Body, &Path::top(1)).unwrap();
    assert_eq!(p1.props_of_char(1).bold, Some(true));
    assert_eq!(p1.props_of_char(2).bold, None);
}

#[test]
fn copy_paste_round_trip() {
    let mut d = doc(&["alpha", "beta", "gamma"]);
    let f = d.copy_range(&Pos::body(0, 2), &Pos::body(2, 3));
    assert_eq!(f.plain_text(), "pha\nbeta\ngam");
    let end = d.insert_fragment(&Pos::body(2, 5), &f).unwrap();
    assert_eq!(d.plain_text(StoryRef::Body), "alpha\nbeta\ngammapha\nbeta\ngam");
    assert_eq!(end, Pos::body(4, 3));
    let one = Fragment::from_text("XY");
    let e = d.insert_fragment(&Pos::body(0, 0), &one).unwrap();
    assert_eq!(e, Pos::body(0, 2));
    assert!(d.plain_text(StoryRef::Body).starts_with("XYalpha"));
}

/// The first paragraph of the deepest cell, following the first table at each level.
fn deepest(d: &Document) -> Path {
    let (mut path, mut blocks) = (Vec::new(), &d.body);
    while let Some((i, t)) = blocks.iter().enumerate().find_map(|(i, b)| if let Block::Table(t) = &**b { Some((i, t)) } else { None }) {
        path.extend([i as u32, 0, 0]);
        blocks = &t.rows[0].cells[0].blocks;
    }
    path.push(0);
    Path(path)
}

fn body_height(d: &Document) -> usize {
    d.body.iter().filter_map(|b| if let Block::Table(t) = &**b { Some(table_height(t)) } else { None }).max().unwrap_or(0)
}

/// How many times `c` is in `blocks`, the deepest cells too.
fn count(blocks: &[Arc<Block>], c: char) -> usize {
    blocks
        .iter()
        .map(|b| match &**b {
            Block::Para(p) => p.text.matches(c).count(),
            Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).map(|cell| count(&cell.blocks, c)).sum(),
        })
        .sum()
}

#[test]
fn tables_nest_at_most_max_table_depth() {
    let mut d = doc(&["x", ""]);
    for _ in 0..MAX_TABLE_DEPTH {
        let at = deepest(&d);
        d.insert_block(StoryRef::Body, &at, Block::Table(Table::new(1, 1, 100.0))).unwrap();
    }
    assert_eq!(body_height(&d), MAX_TABLE_DEPTH);
    // One more in the deepest cell is refused and changes nothing; a paragraph is fine.
    let (at, before) = (deepest(&d), d.clone());
    assert_eq!(at.depth(), MAX_TABLE_DEPTH);
    assert!(d.insert_block(StoryRef::Body, &at, Block::Table(Table::new(1, 1, 100.0))).is_err());
    assert_eq!(d, before);
    d.insert_block(StoryRef::Body, &at, Block::Para(Paragraph::new())).unwrap();
    // One level up, a table fits but a table holding a table doesn't.
    let up = Path(at.0[..at.0.len() - 3].to_vec());
    let mut two = Table::new(1, 1, 100.0);
    two.rows[0].cells[0].blocks.insert(0, Arc::new(Block::Table(Table::new(1, 1, 100.0))));
    assert!(d.insert_block(StoryRef::Body, &up, Block::Table(two)).is_err());
    d.insert_block(StoryRef::Body, &up, Block::Table(Table::new(1, 1, 100.0))).unwrap();
    assert_eq!(body_height(&d), MAX_TABLE_DEPTH);
}

#[test]
fn pasting_a_table_into_its_own_cell_stops_at_max_table_depth() {
    // #1596: copy everything and paste it into the deepest cell, again and again: each round
    // doubled the nesting, until saving, copying or closing the document overflowed the stack.
    let mut d = doc(&["x", ""]);
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(Table::new(1, 1, 100.0))).unwrap();
    d.insert_text(&Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 0 }, "a", &CharProps::default()).unwrap();
    for _ in 0..8 {
        let all = d.copy_range(&Pos::body(0, 0), &Pos::body(d.body.len() - 1, 0));
        let at = deepest(&d);
        let off = d.para(StoryRef::Body, &at).map_or(0, Paragraph::len);
        d.insert_fragment(&Pos { story: StoryRef::Body, path: at, off }, &all).unwrap();
    }
    assert_eq!(body_height(&d), MAX_TABLE_DEPTH);
    // Nothing is lost: every copy's text is there, below the limit as paragraphs.
    assert_eq!((count(&d.body, 'x'), count(&d.body, 'a')), (256, 256));
}

fn text_box(d: &mut Document, pos: &Pos, text: &str) -> u32 {
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(text, CharProps::default()))]);
    d.insert_object(pos, box_shape(Some(id)), &CharProps::default()).unwrap();
    id
}

fn box_shape(story: Option<u32>) -> InlineObject {
    InlineObject::Shape {
        kind: crate::para::ShapeKind::TextBox,
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
    }
}

/// Story ids of the boxes in a body paragraph, in order.
fn boxes_in(d: &Document, path: &Path) -> Vec<Option<u32>> {
    let p = d.para(StoryRef::Body, path).unwrap();
    p.objects.iter().filter_map(|o| if let InlineObject::Shape { story, .. } = o { Some(*story) } else { None }).collect()
}

#[test]
fn pasted_text_box_gets_its_own_story() {
    let mut d = doc(&["ab", "cd"]);
    let id = text_box(&mut d, &Pos::body(0, 1), "Inside");
    let f = d.copy_range(&Pos::body(0, 0), &Pos::body(0, 5));
    assert_eq!(f.parts.keys().copied().collect::<Vec<_>>(), vec![id]);
    // Edit the original after copying: the paste keeps the copied text.
    d.insert_text(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 6 }, " later", &CharProps::default()).unwrap();
    d.insert_fragment(&Pos::body(1, 2), &f).unwrap();
    let pasted = boxes_in(&d, &Path::top(1));
    let [Some(new)] = pasted[..] else { panic!("{pasted:?}") };
    assert_ne!(new, id);
    assert_eq!(d.parts.get(&new).map(|p| p.kind), Some(PartKind::TextBox));
    assert_eq!(d.plain_text(StoryRef::Part(new)), "Inside");
    assert_eq!(d.plain_text(StoryRef::Part(id)), "Inside later");
    // Editing the copy leaves the original alone; a second paste is a third story.
    d.insert_text(&Pos { story: StoryRef::Part(new), path: Path::top(0), off: 0 }, "Copy: ", &CharProps::default()).unwrap();
    assert_eq!(d.plain_text(StoryRef::Part(id)), "Inside later");
    d.insert_fragment(&Pos::body(1, 0), &f).unwrap();
    let again = boxes_in(&d, &Path::top(1));
    assert_eq!(again.len(), 2);
    assert!(again.iter().all(|s| s.is_some_and(|s| s != id)) && again[0] != again[1], "{again:?}");
    // Multi-paragraph pastes too.
    let f2 = d.copy_range(&Pos::body(0, 0), &Pos::body(1, 1));
    let before = d.parts.len();
    d.insert_fragment(&Pos::body(1, 0), &f2).unwrap();
    assert_eq!(d.parts.len(), before + 1);
}

#[test]
fn text_box_pasted_into_another_document_brings_its_text() {
    let mut src = doc(&["x"]);
    let id = text_box(&mut src, &Pos::body(0, 1), "From the source");
    let f = src.copy_range(&Pos::body(0, 0), &Pos::body(0, 1 + 3));
    // The target already has a header with the same part id.
    let mut dst = doc(&["target"]);
    let hdr = dst.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Header text", CharProps::default()))]);
    assert_eq!(hdr, id);
    dst.insert_fragment(&Pos::body(0, 6), &f).unwrap();
    let [Some(new)] = boxes_in(&dst, &Path::top(0))[..] else { panic!() };
    assert_ne!(new, hdr);
    assert_eq!(dst.plain_text(StoryRef::Part(new)), "From the source");
    assert_eq!(dst.plain_text(StoryRef::Part(hdr)), "Header text");
    // The whole body of a document, as Insert › Text from File uses it.
    let mut dst2 = doc(&["t"]);
    dst2.insert_fragment(&Pos::body(0, 1), &src.body_fragment()).unwrap();
    let [Some(n2)] = boxes_in(&dst2, &Path::top(0))[..] else { panic!() };
    assert_eq!(dst2.plain_text(StoryRef::Part(n2)), "From the source");
}

#[test]
fn text_box_in_a_copied_table_gets_its_own_story() {
    let mut d = doc(&["before", "after"]);
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(Table::new(1, 1, 200.0))).unwrap();
    let cell = Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 0 };
    let id = text_box(&mut d, &cell, "In a cell");
    let f = d.copy_range(&Pos::body(0, 0), &Pos::body(2, 5));
    assert!(f.parts.contains_key(&id));
    d.insert_fragment(&Pos::body(2, 5), &f).unwrap();
    let ids: Vec<u32> = d
        .para_paths(StoryRef::Body)
        .iter()
        .filter_map(|p| d.para(StoryRef::Body, p))
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let InlineObject::Shape { story, .. } = o { *story } else { None })
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1]);
    assert!(ids.iter().all(|i| d.plain_text(StoryRef::Part(*i)) == "In a cell"));
}

#[test]
fn fragment_without_parts_never_shows_an_unrelated_story() {
    let mut d = doc(&["x"]);
    let hdr = d.add_part(PartKind::Header, Vec::new());
    let own = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Mine", CharProps::default()))]);
    let mut p = Paragraph::new();
    p.insert_object(0, box_shape(Some(hdr)), &CharProps::default()).unwrap();
    p.insert_object(0, box_shape(Some(own)), &CharProps::default()).unwrap();
    p.insert_object(0, box_shape(Some(999)), &CharProps::default()).unwrap();
    let f = Fragment { blocks: vec![Block::Para(p)], ..Default::default() };
    d.insert_fragment(&Pos::body(0, 1), &f).unwrap();
    let got = boxes_in(&d, &Path::top(0));
    assert_eq!(got.len(), 3);
    assert_eq!(got[0], None, "missing story");
    let Some(copy) = got[1] else { panic!("{got:?}") };
    assert_ne!(copy, own);
    assert_eq!(d.plain_text(StoryRef::Part(copy)), "Mine");
    assert_eq!(got[2], None, "a header isn't a text box");
}

#[test]
fn self_nested_text_box_paste_is_bounded() {
    let mut d = doc(&["x"]);
    let id = text_box(&mut d, &Pos::body(0, 1), "loop");
    // The box contains a box showing itself.
    d.insert_object(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 0 }, box_shape(Some(id)), &CharProps::default()).unwrap();
    let f = d.copy_range(&Pos::body(0, 0), &Pos::body(0, 1 + 3));
    let before = d.parts.len();
    d.insert_fragment(&Pos::body(0, 0), &f).unwrap();
    let added = d.parts.len() - before;
    // The copy's inner box would show the story being copied: a cycle, so it gets none.
    assert_eq!(added, 1);
}

/// A fragment whose body is one paragraph of `refs` text boxes all showing story 1.
fn fan_out(story: Vec<Arc<Block>>, refs: usize) -> Fragment {
    let mut p = Paragraph::new();
    for _ in 0..refs {
        p.insert_object(0, box_shape(Some(1)), &CharProps::default()).unwrap();
    }
    let mut parts = BTreeMap::new();
    parts.insert(1, crate::Part { kind: PartKind::TextBox, blocks: story, body: Default::default() });
    Fragment { blocks: vec![Block::Para(p)], parts }
}

#[test]
fn pasted_boxes_share_the_blocks_they_need_not_copy() {
    let big = para_block(Paragraph::with_text(&"x".repeat(100_000), CharProps::default()));
    let f = fan_out(vec![big.clone()], 3);
    let mut d = doc(&["x"]);
    d.insert_fragment(&Pos::body(0, 1), &f).unwrap();
    let got = boxes_in(&d, &Path::top(0));
    assert_eq!(got.len(), 3);
    let ids: Vec<u32> = got.into_iter().flatten().collect();
    assert_eq!(ids.len(), 3, "each box has a story");
    assert!(ids.windows(2).all(|w| w[0] != w[1]), "stories of their own: {ids:?}");
    for id in ids {
        let b = d.parts[&id].blocks.first().unwrap();
        assert!(Arc::ptr_eq(b, &big), "no text box inside: shared, not copied");
    }
}

#[test]
fn a_box_over_the_budget_leaves_later_smaller_boxes_their_stories() {
    // Review of #3874: once one story didn't fit, every later box in the paste was left
    // empty, however small its story. Only the box that doesn't fit is.
    // (Story 1 holds a text box, so each box showing it needs a real copy.)
    let mut bp = Paragraph::with_text(&"x".repeat(100_000), CharProps::default());
    bp.insert_object(0, box_shape(Some(999)), &CharProps::default()).unwrap();
    let big = para_block(bp);
    let small = para_block(Paragraph::with_text("small", CharProps::default()));
    let mut p = Paragraph::new();
    for story in [2, 1, 2, 1] {
        p.insert_object(0, box_shape(Some(story)), &CharProps::default()).unwrap();
    }
    let mut parts = BTreeMap::new();
    parts.insert(1, crate::Part { kind: PartKind::TextBox, blocks: vec![big], body: Default::default() });
    parts.insert(2, crate::Part { kind: PartKind::TextBox, blocks: vec![small], body: Default::default() });
    let mut blocks = vec![Block::Para(p)];
    let mut d = doc(&["x"]);
    let mut budget = BoxBudget { bytes: 50_000, ..BoxBudget::new() };
    d.adopt_text_boxes(&mut blocks, &parts, 0, &mut budget);
    let Some(Block::Para(p)) = blocks.first() else { panic!() };
    let got: Vec<Option<u32>> = p.objects.iter().map(|o| if let InlineObject::Shape { story, .. } = o { *story } else { None }).collect();
    assert_eq!(got.iter().map(Option::is_some).collect::<Vec<_>>(), [false, true, false, true], "{got:?}");
    assert_eq!(budget.skipped, 2);
    assert_eq!(budget.costs.len(), 2, "each story costed once");
}

#[test]
fn many_boxes_sharing_one_story_paste_with_their_text() {
    // Review of #3874: a document converted from PDF has thousands of boxes showing story
    // data they share. 64 boxes x a 1 MiB block is 64 MiB counted per box (over the 32 MiB
    // cap) but the block is shared, not copied, so every box keeps its text.
    let big = para_block(Paragraph::with_text(&"x".repeat(1 << 20), CharProps::default()));
    let f = fan_out(vec![big.clone()], 64);
    let mut d = doc(&["x"]);
    let (_, warned) = crate::test_log::warnings(|| d.insert_fragment(&Pos::body(0, 1), &f).unwrap());
    assert!(!warned.iter().any(|w| w.contains("text boxes are left empty")), "{warned:?}");
    let ids: Vec<u32> = boxes_in(&d, &Path::top(0)).into_iter().flatten().collect();
    assert_eq!(ids.len(), 64, "every box keeps a story");
    for id in ids {
        let b = d.parts[&id].blocks.first().unwrap();
        assert!(Arc::ptr_eq(b, &big), "shared, so not charged per box");
        assert_eq!(d.parts[&id].blocks.len(), 1);
    }
}

#[test]
fn pasted_box_copies_are_capped_in_bytes() {
    // Story 1 is a 1 MiB paragraph that holds a text box itself, so each box showing it needs
    // its own copy; 64 of them used to copy 64 MiB.
    let mut p = Paragraph::with_text(&"x".repeat(1 << 20), CharProps::default());
    p.insert_object(0, box_shape(Some(999)), &CharProps::default()).unwrap();
    let f = fan_out(vec![para_block(p)], 64);
    let mut d = doc(&["x"]);
    let before = d.parts.len();
    let (_, warned) = crate::test_log::warnings(|| d.insert_fragment(&Pos::body(0, 1), &f).unwrap());
    let added = d.parts.len() - before;
    // The boxes left empty are said to be, once for the paste (review of #3874).
    let empty: Vec<_> = warned.iter().filter(|w| w.contains("text boxes are left empty")).collect();
    assert_eq!(empty.len(), 1, "{warned:?}");
    assert!(empty[0].starts_with(&format!("paste: {} text boxes", 64 - added)), "{empty:?}");
    let copied: usize = d.parts.values().flat_map(|p| p.blocks.iter()).map(|b| if let Block::Para(p) = &**b { p.text.len() } else { 0 }).sum();
    assert!((1..64).contains(&added), "{added}");
    assert!(copied <= MAX_PASTED_BOX_BYTES, "{copied}");
    // The boxes past the cap keep no story rather than a shared one.
    let got = boxes_in(&d, &Path::top(0));
    assert_eq!(got.iter().flatten().count(), added);
}

#[test]
fn pasted_box_copies_count_the_objects_they_copy() {
    // #3614 review: a story holding a text box and a 1 MiB kept-for-round-trip object costs
    // the object's XML too, not only its slot: 64 copies used to copy 64 MiB of it.
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(0, box_shape(Some(999)), &CharProps::default()).unwrap();
    let opaque = InlineObject::Opaque { format: "docx".into(), xml: "x".repeat(1 << 20), text: String::new() };
    p.insert_object(0, opaque, &CharProps::default()).unwrap();
    let f = fan_out(vec![para_block(p)], 64);
    let mut d = doc(&["x"]);
    let before = d.parts.len();
    d.insert_fragment(&Pos::body(0, 1), &f).unwrap();
    let added = d.parts.len() - before;
    let xml = |o: &InlineObject| if let InlineObject::Opaque { xml, .. } = o { xml.len() } else { 0 };
    let copied: usize =
        d.parts.values().flat_map(|p| p.blocks.iter()).map(|b| if let Block::Para(p) = &**b { p.objects.iter().map(xml).sum() } else { 0 }).sum();
    assert!((1..64).contains(&added), "{added}");
    assert!(copied <= MAX_PASTED_BOX_BYTES, "{copied}");
    // Nested group members and equations count too.
    let member = |obj| crate::para::GroupChild { x: 0.0, y: 0.0, obj };
    let deep = InlineObject::Opaque { format: String::new(), xml: "y".repeat(5000), text: String::new() };
    let inner = InlineObject::Group { w: 1.0, h: 1.0, float: Default::default(), ch_w: 1.0, ch_h: 1.0, children: vec![member(deep)], canvas: None };
    let outer = InlineObject::Group { w: 1.0, h: 1.0, float: Default::default(), ch_w: 1.0, ch_h: 1.0, children: vec![member(inner)], canvas: None };
    assert!(object_cost(&outer) >= 5000);
    let eq = InlineObject::Equation {
        linear: String::new(),
        display: false,
        math: crate::math::Math { nodes: crate::math::parse_linear(&"z".repeat(5000)), ..Default::default() },
    };
    assert!(object_cost(&eq) >= 5000);
}

#[test]
fn tables_in_paths() {
    let mut d = doc(&["before", "after"]);
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(Table::new(2, 2, 200.0))).unwrap();
    let paths = d.para_paths(StoryRef::Body);
    assert_eq!(paths.len(), 6);
    assert_eq!(paths[1], Path(vec![1, 0, 0, 0]));
    let cell = Pos { story: StoryRef::Body, path: Path(vec![1, 0, 1, 0]), off: 0 };
    let e = d.insert_text(&cell, "x", &CharProps::default()).unwrap();
    assert_eq!(e.off, 1);
    assert_eq!(d.next_para(StoryRef::Body, &Path(vec![1, 0, 1, 0])), Some(Path(vec![1, 1, 0, 0])));
    assert!(d.plain_text(StoryRef::Body).contains("\tx"));
    // Delete from before the table into a cell: text cleared, table kept (partially covered).
    d.delete_range(&Pos::body(0, 2), &cell).unwrap();
    assert_eq!(d.para(StoryRef::Body, &Path::top(0)).unwrap().text, "be");
    assert!(d.container(StoryRef::Body, &Path(vec![1, 0, 0, 0])).is_some());
    assert_eq!(Path(vec![1, 0, 1, 0]).cell(), Some((Path::top(1), 0, 1)));
}

fn para(text: &str) -> Block {
    Block::Para(Paragraph::with_text(text, CharProps::default()))
}

#[test]
fn appended_pieces_are_each_followed_by_one_paragraph_mark() {
    let mut spike = Fragment::default();
    spike.append_piece(Fragment { blocks: vec![para("a")], ..Default::default() });
    spike.append_piece(Fragment { blocks: vec![para("b")], ..Default::default() });
    assert_eq!(spike.plain_text(), "a\nb\n");
    // A piece that already ends with its paragraph mark gets no second one.
    let mut spike = Fragment::default();
    spike.append_piece(Fragment { blocks: vec![para("a"), para("")], ..Default::default() });
    assert_eq!(spike.plain_text(), "a\n");
    spike.append_piece(Fragment { blocks: vec![para("b"), para("c")], ..Default::default() });
    assert_eq!(spike.plain_text(), "a\nb\nc\n");
}

#[test]
fn appended_pieces_keep_their_own_text_boxes() {
    // Two documents give their first text box the same id.
    let mut one = doc(&["x"]);
    let id1 = text_box(&mut one, &Pos::body(0, 1), "One");
    let mut two = doc(&["y"]);
    let id2 = text_box(&mut two, &Pos::body(0, 1), "Two");
    assert_eq!(id1, id2);
    let mut spike = Fragment::default();
    spike.append_piece(one.copy_range(&Pos::body(0, 0), &Pos::body(0, 1 + 3)));
    spike.append_piece(two.copy_range(&Pos::body(0, 0), &Pos::body(0, 1 + 3)));
    let mut dst = doc(&[""]);
    dst.insert_fragment(&Pos::body(0, 0), &spike).unwrap();
    let texts: Vec<String> = [0, 1].iter().flat_map(|i| boxes_in(&dst, &Path::top(*i))).map(|b| dst.plain_text(StoryRef::Part(b.unwrap()))).collect();
    assert_eq!(texts, ["One", "Two"]);
}

#[test]
fn bad_positions_error() {
    let mut d = doc(&["é"]);
    assert!(d.insert_text(&Pos::body(0, 1), "x", &CharProps::default()).is_err());
    assert!(d.insert_text(&Pos::body(5, 0), "x", &CharProps::default()).is_err());
    assert!(d.split_paragraph(&Pos::body(0, 99)).is_err());
    let c = d.clamp(&Pos::body(9, 9));
    assert_eq!(c, Pos::body(0, 2));
    assert!(d.delete_range(&Pos { story: StoryRef::Part(3), path: Path::top(0), off: 0 }, &Pos::body(0, 0)).is_err());
}

// delete_range over whole tables (#1844), in the body, a header and a footnote.

/// A document whose story (the body, or a header or footnote part) holds `lines`.
fn story_doc(kind: Option<PartKind>, lines: &[&str]) -> (Document, StoryRef) {
    match kind {
        None => (doc(lines), StoryRef::Body),
        Some(k) => {
            let mut d = doc(&["body"]);
            let blocks = lines.iter().map(|l| para_block(Paragraph::with_text(l, CharProps::default()))).collect();
            let id = d.add_part(k, blocks);
            (d, StoryRef::Part(id))
        }
    }
}

const STORIES: [Option<PartKind>; 3] = [None, Some(PartKind::Header), Some(PartKind::Footnote)];

fn at(story: StoryRef, path: &[u32], off: usize) -> Pos {
    Pos { story, path: Path(path.to_vec()), off }
}

/// Insert an `rows`×`cols` table at `path` whose cells hold `texts`, row by row.
fn put_table(d: &mut Document, story: StoryRef, path: &[u32], rows: usize, cols: usize, texts: &[&str]) {
    d.insert_block(story, &Path(path.to_vec()), Block::Table(Table::new(rows, cols, 200.0))).unwrap();
    for (k, t) in texts.iter().enumerate() {
        let mut p = path.to_vec();
        p.extend([(k / cols) as u32, (k % cols) as u32, 0]);
        d.insert_text(&at(story, &p, 0), t, &CharProps::default()).unwrap();
    }
}

/// The story's top-level blocks: a paragraph's text, or `[table]`.
fn shape(d: &Document, story: StoryRef) -> Vec<String> {
    d.story(story).unwrap().iter().map(|b| b.as_para().map_or("[table]".to_string(), |p| p.text.clone())).collect()
}

fn cell_text(d: &Document, story: StoryRef, table: &[u32], r: u32, c: u32) -> String {
    let mut p = table.to_vec();
    p.extend([r, c, 0]);
    d.para(story, &Path(p)).map(|p| p.text.clone()).unwrap_or_else(|| "<none>".into())
}

#[test]
fn a_range_ending_at_the_end_of_a_cell_that_isnt_the_last_keeps_the_table() {
    for kind in STORIES {
        for (r, c) in [(0, 1), (1, 0)] {
            let (mut d, st) = story_doc(kind, &["before", "after"]);
            put_table(&mut d, st, &[1], 2, 2, &["a", "b", "c", "d"]);
            d.delete_range(&at(st, &[0], 0), &at(st, &[1, r, c, 0], 1)).unwrap();
            assert_eq!(shape(&d, st), ["", "[table]", "after"], "{kind:?} ({r},{c})");
            assert_eq!(cell_text(&d, st, &[1], 1, 1), "d", "{kind:?} ({r},{c})");
        }
    }
}

#[test]
fn a_range_from_the_start_of_one_table_into_the_middle_of_the_next_removes_only_the_first() {
    for kind in STORIES {
        let (mut d, st) = story_doc(kind, &["x", "mid", "y"]);
        put_table(&mut d, st, &[2], 2, 2, &["e", "f", "g", "h"]);
        put_table(&mut d, st, &[1], 2, 2, &["a", "b", "c", "d"]);
        assert_eq!(shape(&d, st), ["x", "[table]", "mid", "[table]", "y"]);
        let to = d.delete_range(&at(st, &[1, 0, 0, 0], 0), &at(st, &[3, 0, 1, 0], 1)).unwrap();
        assert_eq!(shape(&d, st), ["x", "", "[table]", "y"], "{kind:?}");
        assert_eq!([cell_text(&d, st, &[2], 0, 0), cell_text(&d, st, &[2], 0, 1), cell_text(&d, st, &[2], 1, 0)], ["", "", "g"], "{kind:?}");
        assert!(d.para(st, &to.path).is_some(), "{kind:?}: {to:?}");
    }
}

#[test]
fn a_range_from_a_cell_paragraph_to_the_end_of_a_table_nested_in_that_cell_removes_only_the_nested_table() {
    for kind in STORIES {
        let (mut d, st) = story_doc(kind, &["before", "after"]);
        put_table(&mut d, st, &[1], 1, 1, &["p"]);
        put_table(&mut d, st, &[1, 0, 0, 1], 1, 1, &["n"]);
        d.insert_block(st, &Path(vec![1, 0, 0, 2]), Block::Para(Paragraph::with_text("q", CharProps::default()))).unwrap();
        let to = d.delete_range(&at(st, &[1, 0, 0, 0], 1), &at(st, &[1, 0, 0, 1, 0, 0, 0], 1)).unwrap();
        assert_eq!(shape(&d, st), ["before", "[table]", "after"], "{kind:?}");
        let cell = d.container(st, &Path(vec![1, 0, 0, 0])).unwrap();
        let cell: Vec<String> = cell.iter().map(|b| b.as_para().map_or("[table]".to_string(), |p| p.text.clone())).collect();
        assert_eq!(cell, ["pq"], "{kind:?}");
        assert_eq!(to, at(st, &[1, 0, 0, 0], 1), "{kind:?}");
    }
}

#[test]
fn two_whole_tables_at_the_end_of_a_story_leave_one_empty_paragraph() {
    for kind in STORIES {
        for trailing in [true, false] {
            let (mut d, st) = story_doc(kind, &[""]);
            put_table(&mut d, st, &[0], 1, 2, &["a", "b"]);
            put_table(&mut d, st, &[1], 2, 1, &["c", "d"]);
            if !trailing {
                // A story that ends in a table (a hostile file).
                d.story_mut(st).unwrap().pop();
            }
            let to = d.delete_range(&at(st, &[0, 0, 0, 0], 0), &at(st, &[1, 1, 0, 0], 1)).unwrap();
            assert_eq!(shape(&d, st), [""], "{kind:?} trailing {trailing}");
            assert_eq!(to, at(st, &[0], 0), "{kind:?} trailing {trailing}");
        }
    }
}

/// Content controls pair their markers within one block list (a story, or a cell), so a
/// marker in a table never has its partner outside it: a control inside a table removed whole
/// goes with it, and one the range only enters keeps both markers (the control is emptied).
#[test]
fn removing_a_whole_table_keeps_a_control_the_range_only_partly_covers() {
    use crate::control::ContentControl;
    let start = || InlineObject::ControlStart { control: Box::new(ContentControl::default()) };
    for kind in STORIES {
        let (mut d, st) = story_doc(kind, &["before", "after"]);
        put_table(&mut d, st, &[1], 1, 1, &["c"]);
        // One control around "efo" in "before", one around the cell's text.
        d.insert_object(&at(st, &[0], 4), InlineObject::ControlEnd, &CharProps::default()).unwrap();
        let in_before = d.insert_object(&at(st, &[0], 1), start(), &CharProps::default()).unwrap();
        d.insert_object(&at(st, &[1, 0, 0, 0], 1), InlineObject::ControlEnd, &CharProps::default()).unwrap();
        d.insert_object(&at(st, &[1, 0, 0, 0], 0), start(), &CharProps::default()).unwrap();
        assert_eq!(d.control_ranges(st).len(), 2, "setup");
        let end = d.para(st, &Path(vec![1, 0, 0, 0])).unwrap().len();
        // From inside the first control to the end of the table.
        d.delete_range(&Pos { off: in_before.off + 1, ..in_before }, &at(st, &[1, 0, 0, 0], end)).unwrap();
        assert_eq!(shape(&d, st), ["b\u{FFFC}e\u{FFFC}after"], "{kind:?}: the table goes, the paragraphs join");
        let ranges = d.control_ranges(st);
        assert_eq!(ranges.len(), 1, "{kind:?}: {ranges:?}");
        assert!(ranges.iter().all(|r| r.start.path == Path::top(0) && r.end.path == Path::top(0)), "{kind:?}");
        let objects = d.para(st, &Path::top(0)).map_or(0, |p| p.objects.len());
        assert_eq!(objects, 2, "{kind:?}: no orphan markers left");
    }
}

#[test]
fn a_table_nested_deeper_than_the_paragraph_walk_is_never_removed_whole() {
    // 20 tables, each in the first cell of the one around it, ahead of that cell's paragraph:
    // the innermost paragraphs come first in the document but the paragraph walk stops short.
    let mut t = Table::new(1, 1, 200.0);
    if let Some(cell) = t.rows.get_mut(0).and_then(|r| r.cells.get_mut(0)) {
        cell.blocks = vec![para_block(Paragraph::with_text("deep", CharProps::default()))];
    }
    for _ in 0..20 {
        let mut outer = Table::new(1, 1, 200.0);
        if let Some(cell) = outer.rows.get_mut(0).and_then(|r| r.cells.get_mut(0)) {
            cell.blocks.insert(0, Arc::new(Block::Table(t)));
        }
        t = outer;
    }
    let mut d = doc(&["before", "after"]);
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t)).unwrap();
    let first = d.para_paths(StoryRef::Body).into_iter().find(|p| p.0.first() == Some(&1)).unwrap();
    d.delete_range(&Pos { story: StoryRef::Body, path: first, off: 0 }, &Pos::body(2, 0)).unwrap();
    assert!(d.body.get(1).is_some_and(|b| b.as_table().is_some()), "a table with paragraphs the walk never saw was removed");
}

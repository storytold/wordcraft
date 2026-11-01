//! A layout started again from the page an edit is on ([`crate::resume`]) has the pages of a
//! layout from the first page.

use super::*;
use wordcraft_doc::numbering::ListKind;
use wordcraft_doc::props::NumRef;
use wordcraft_doc::{Revision, RevisionKind};

/// Some 40 pages of the pagination fixture's paragraphs: headings kept with the next paragraph,
/// paragraphs kept together, a numbered list, bordered paragraphs joined in one box, a page
/// break, and a second section starting on a new page.
fn long_doc() -> Document {
    let words = "words that wrap across the line and on ";
    let text: Vec<String> =
        (0..600).map(|i| if i % 9 == 0 { format!("Heading {i}") } else { format!("Paragraph {i} {}", words.repeat(i % 7 + 1)) }).collect();
    let mut doc = pagination_fixture_document(&text.join("\n"));
    let list = doc.numbering.add_list(ListKind::Numbered);
    for (i, b) in doc.body.iter_mut().enumerate() {
        let Block::Para(p) = Arc::make_mut(b) else { continue };
        p.props.keep_next = Some(i % 9 == 0);
        p.props.keep_lines = Some(i % 13 == 5);
        p.props.page_break_before = Some(i == 250);
        if i % 40 >= 30 && i % 9 != 0 {
            p.props.numbering = Some(NumRef { num: list, level: 0 });
        }
        if (100..106).contains(&i) {
            p.props.borders = Some(Borders { top: Some(Border::single(1.0)), bottom: Some(Border::single(1.0)), ..Default::default() });
        }
        if i == 400 {
            p.section = Some(Box::new(SectionProps::default()));
        }
    }
    doc
}

/// Lay `doc` out with `cache` (its pass before kept), and from scratch: the same pages.
fn assert_fresh(doc: &Document, cache: &mut LayoutCache, what: &str) {
    let ours = layout(doc, cache, &LayoutOptions::default());
    let fresh = layout(doc, &mut LayoutCache::new(), &LayoutOptions::default());
    assert!(format!("{:?}", ours.pages) == format!("{:?}", fresh.pages), "{what}: not the pages of a fresh layout");
    assert_eq!(ours.text_boxes, fresh.text_boxes, "{what}: text boxes laid out");
    assert_eq!(
        format!("{:?}", { ours.index.into_iter().collect::<std::collections::BTreeMap<_, _>>() }),
        format!("{:?}", { fresh.index.into_iter().collect::<std::collections::BTreeMap<_, _>>() })
    );
}

fn at(block: usize, off: usize) -> Pos {
    Pos { story: StoryRef::Body, path: Path::top(block), off }
}

#[test]
fn edits_lay_out_as_a_fresh_layout() {
    let mut doc = long_doc();
    let mut cache = LayoutCache::new();
    let pages = layout(&doc, &mut cache, &LayoutOptions::default()).pages.len();
    assert!(pages > 30, "{pages} pages");
    let starts = crate::resume::STARTS.get();
    let cp = CharProps::default();
    // In the first block, on the first page, in the middle, in the list, the bordered box and near
    // the end.
    for block in [0, 1, 2, 120, 102, 333, 405, 570] {
        doc.insert_text(&at(block, 3), "typed", &cp).unwrap();
        assert_fresh(&doc, &mut cache, "typing");
        doc.insert_text(&at(block, 8), &" more words".repeat(30), &cp).unwrap();
        assert_fresh(&doc, &mut cache, "a line more");
        doc.split_paragraph(&at(block, 5)).unwrap();
        assert_fresh(&doc, &mut cache, "a paragraph split");
        let len = doc.para(StoryRef::Body, &Path::top(block)).unwrap().text.len();
        doc.delete_range(&at(block, len), &at(block + 1, 0)).unwrap();
        assert_fresh(&doc, &mut cache, "two paragraphs joined");
        let len = doc.para(StoryRef::Body, &Path::top(block)).unwrap().text.len();
        doc.delete_range(&at(block, 3), &at(block, len.min(40))).unwrap();
        assert_fresh(&doc, &mut cache, "words deleted");
        doc.delete_range(&at(block, 2), &at(block + 2, 3)).unwrap();
        assert_fresh(&doc, &mut cache, "paragraphs deleted");
    }
    // A list item taken out of the list: the items after it, on later pages, number one less
    // although they sit where they did.
    if let Block::Para(p) = Arc::make_mut(&mut doc.body[33]) {
        p.props.numbering = None;
        p.touch();
    }
    assert_fresh(&doc, &mut cache, "a list item out of its list");
    // A heading kept with the paragraph after it gets that paragraph's first lines from the
    // paragraph after it: editing the paragraph moves the heading.
    if let Block::Para(p) = Arc::make_mut(&mut doc.body[180]) {
        p.props.keep_next = Some(true);
        p.touch();
    }
    assert_fresh(&doc, &mut cache, "kept with the next");
    doc.insert_text(&at(181, 3), &"longer ".repeat(200), &cp).unwrap();
    assert_fresh(&doc, &mut cache, "the paragraph a heading is kept with");
    // Not only the body: a style change lays out everything again.
    doc.styles.default_chr.size = Some(13.0);
    assert_fresh(&doc, &mut cache, "a style");
    // A bordered paragraph running on to another page, in one box with the next: the next is the
    // first to begin on that page, but the paragraph before looked ahead at it to leave out its
    // bottom border. Without the next's border, that border takes its room again.
    let mut doc = long_doc();
    let border = Some(Borders { top: Some(Border::single(1.0)), bottom: Some(Border::single(1.0)), ..Default::default() });
    for (i, text) in [(300, "words that wrap across the line and on ".repeat(150)), (301, "Boxed with the one before".to_string())] {
        if let Block::Para(p) = Arc::make_mut(&mut doc.body[i]) {
            *p = Paragraph { props: p.props.clone(), ..Paragraph::with_text(&text, CharProps::default()) };
            p.props.borders = border;
        }
    }
    let mut cache = LayoutCache::new();
    let page_of = |l: &DocLayout, b: usize| l.index.get(&(StoryRef::Body, Path::top(b))).and_then(|v| v.first()).map(|(page, _)| *page);
    let l = layout(&doc, &mut cache, &LayoutOptions::default());
    let (first, next) = (page_of(&l, 300), page_of(&l, 301));
    assert!(first < next, "the bordered paragraph runs on to the next one's page: {first:?}, {next:?}");
    if let Block::Para(p) = Arc::make_mut(&mut doc.body[301]) {
        p.props.borders = None;
        p.touch();
    }
    assert_fresh(&doc, &mut cache, "the paragraph after a bordered one");
    // A heading at the top of a page kept with a paragraph kept together that only fits on the
    // next page; a shorter paragraph comes up under the heading.
    let mut doc = long_doc();
    for (i, text) in [(250, "Heading".to_string()), (251, "words that wrap across the line and on ".repeat(105))] {
        if let Block::Para(p) = Arc::make_mut(&mut doc.body[i]) {
            *p = Paragraph { props: p.props.clone(), ..Paragraph::with_text(&text, CharProps::default()) };
            p.props.keep_next = Some(i == 250);
            p.props.keep_lines = Some(i == 251);
        }
    }
    let mut cache = LayoutCache::new();
    let page_of = |l: &DocLayout, b: usize| l.index.get(&(StoryRef::Body, Path::top(b))).and_then(|v| v.first()).map(|(page, _)| *page);
    let l = layout(&doc, &mut cache, &LayoutOptions::default());
    assert_eq!(page_of(&l, 251), page_of(&l, 250).map(|p| p + 1), "the kept paragraph starts the next page");
    let len = doc.para(StoryRef::Body, &Path::top(251)).unwrap().text.len();
    doc.delete_range(&at(251, 20), &at(251, len)).unwrap();
    assert_fresh(&doc, &mut cache, "a paragraph a heading looked ahead at");
    let l = layout(&doc, &mut cache, &LayoutOptions::default());
    assert_eq!(page_of(&l, 251), page_of(&l, 250), "the shorter paragraph comes up under its heading");
    let (resumed, joined) = crate::resume::STARTS.get();
    assert!(resumed - starts.0 >= 40 && joined - starts.1 >= 30, "started again {} times, joined {}", resumed - starts.0, joined - starts.1);
}

/// A floating box sized to its text that shows itself inline: inside itself it can't be measured,
/// which the layout notes for the box at that width; the body showing it inline at that width
/// further on takes that note. A pass started again after that would measure it there.
#[test]
fn a_box_not_measured_inside_itself_lays_out_as_a_fresh_layout() {
    let mut doc = long_doc();
    let id = doc.add_part(wordcraft_doc::PartKind::TextBox, vec![wordcraft_doc::para_block(Paragraph::with_text("Box", Default::default()))]);
    if let Some(p) = doc.parts.get_mut(&id) {
        p.body.auto_fit = true;
    }
    let shape = |w: f32, float: Float| InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w,
        h: 40.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float,
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let inside = Pos { story: StoryRef::Part(id), path: Path::top(0), off: 3 };
    doc.insert_object(&inside, shape(60.0, Float::default()), &Default::default()).unwrap();
    let floating = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, dist: 9.0, ..Default::default() };
    doc.insert_object(&at(40, 0), shape(150.0, floating), &Default::default()).unwrap();
    doc.insert_object(&at(300, 0), shape(60.0, Float::default()), &Default::default()).unwrap();
    let mut cache = LayoutCache::new();
    layout(&doc, &mut cache, &LayoutOptions::default());
    let cp = CharProps::default();
    for block in [2, 120, 333, 500] {
        doc.insert_text(&at(block, 3), "typed", &cp).unwrap();
        assert_fresh(&doc, &mut cache, "typing");
        doc.split_paragraph(&at(block, 5)).unwrap();
        assert_fresh(&doc, &mut cache, "a paragraph split");
    }
}

/// Typing with changes tracked adds revisions as it goes: the layout still starts again, and a
/// revision changed under text laid out before lays it out from the first page.
#[test]
fn tracked_typing_lays_out_as_a_fresh_layout() {
    let mut doc = long_doc();
    let insert = |author: &str| Revision { kind: RevisionKind::Insert, author: author.into(), date: String::new(), move_name: None };
    doc.revisions.push(insert("First"));
    let mut cache = LayoutCache::new();
    layout(&doc, &mut cache, &LayoutOptions::default());
    let starts = crate::resume::STARTS.get();
    for (n, block) in [(1u32, 120), (2, 333), (3, 2), (4, 570)] {
        doc.revisions.push(insert(if n % 2 == 0 { "Second" } else { "First" }));
        let cp = CharProps { ins: Some(n), ..Default::default() };
        doc.insert_text(&at(block, 3), "typed", &cp).unwrap();
        assert_fresh(&doc, &mut cache, "tracked typing");
        doc.insert_text(&at(block, 8), &" more words".repeat(30), &cp).unwrap();
        assert_fresh(&doc, &mut cache, "a line more, tracked");
    }
    let (resumed, joined) = crate::resume::STARTS.get();
    assert!(resumed - starts.0 >= 8 && joined - starts.1 >= 6, "started again {} times, joined {}", resumed - starts.0, joined - starts.1);
    // The first revision's author changed: text shown in its colour may be anywhere.
    let cp = CharProps { ins: Some(0), ..Default::default() };
    doc.insert_text(&at(450, 3), "early", &cp).unwrap();
    layout(&doc, &mut cache, &LayoutOptions::default());
    let starts = crate::resume::STARTS.get();
    if let Some(r) = doc.revisions.first_mut() {
        r.author = "Second".into();
    }
    assert_fresh(&doc, &mut cache, "an author changed");
    assert_eq!(crate::resume::STARTS.get().0, starts.0, "laid out from the first page");
}

/// A text box `w` x `h` showing `text`, inline at the end of body paragraph `para`, not resized to
/// fit it; its story id.
fn text_box(d: &mut Document, para: usize, text: &str, (w, h): (f32, f32)) -> u32 {
    let id = d.add_part(wordcraft_doc::PartKind::TextBox, vec![wordcraft_doc::para_block(Paragraph::with_text(text, Default::default()))]);
    // A box of its own size (one that resizes to fit its text keeps a layout from marking pages).
    if let Some(p) = d.parts.get_mut(&id) {
        p.body.auto_fit = false;
    }
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let off = d.para(StoryRef::Body, &Path::top(para)).map_or(0, |p| p.len());
    d.insert_object(&Pos::body(para, off), shape, &Default::default()).unwrap();
    id
}

/// Text boxes on their own, in paragraphs all through the document, don't keep a layout from
/// starting again: its pages, and the boxes it counts, are a fresh layout's. Two linked boxes near
/// the end still stop it marking pages past them.
#[test]
fn edits_among_text_boxes_lay_out_as_a_fresh_layout() {
    let mut doc = long_doc();
    for block in (5..560).step_by(17) {
        text_box(&mut doc, block, &format!("Box at {block}, words that wrap in the box"), (120.0, 40.0));
    }
    let a = text_box(&mut doc, 580, &"Linked words that run on past the first box. ".repeat(4), (120.0, 30.0));
    let b = text_box(&mut doc, 585, "", (120.0, 200.0));
    if let Some(p) = doc.parts.get_mut(&a) {
        p.body.next = Some(b);
    }
    let mut cache = LayoutCache::new();
    let first = layout(&doc, &mut cache, &LayoutOptions::default());
    assert!(first.text_boxes > 30, "{} boxes", first.text_boxes);
    let starts = crate::resume::STARTS.get();
    let cp = CharProps::default();
    for block in [2, 120, 333, 590] {
        doc.insert_text(&at(block, 3), "typed", &cp).unwrap();
        assert_fresh(&doc, &mut cache, "typing");
        doc.insert_text(&at(block, 8), &" more words".repeat(30), &cp).unwrap();
        assert_fresh(&doc, &mut cache, "a line more");
        doc.split_paragraph(&at(block, 5)).unwrap();
        assert_fresh(&doc, &mut cache, "a paragraph split");
        let len = doc.para(StoryRef::Body, &Path::top(block)).unwrap().text.len();
        doc.delete_range(&at(block, len), &at(block + 1, 0)).unwrap();
        assert_fresh(&doc, &mut cache, "two paragraphs joined");
    }
    // A box's own text edited: a part, not the body, so the layout starts from the first page.
    if let Some(p) = doc.parts.get_mut(&b) {
        p.blocks = vec![wordcraft_doc::para_block(Paragraph::with_text("Its own text now", Default::default()))];
    }
    assert_fresh(&doc, &mut cache, "a box's text");
    let (resumed, joined) = crate::resume::STARTS.get();
    assert!(resumed - starts.0 >= 8 && joined - starts.1 >= 6, "started again {} times, joined {}", resumed - starts.0, joined - starts.1);
}

/// A header's box sized to its text is measured where its section's top is worked out: a pass
/// that joins an earlier one's pages before that takes the box's height from it, as the headers
/// laid out after the body read it.
#[test]
fn a_header_box_sized_to_its_text_lays_out_as_a_fresh_layout() {
    let mut doc = long_doc();
    let header = doc.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(Paragraph::with_text("Header", Default::default()))]);
    let in_header = Pos { story: StoryRef::Part(header), path: Path::top(0), off: 0 };
    let fitted = text_box_at(&mut doc, &in_header, "Fitted", 60.0, 20.0, Float::default());
    if let Some(p) = doc.parts.get_mut(&fitted) {
        p.body.auto_fit = true;
    }
    // Sections every 40 blocks, the seventh and those after it showing the header.
    for i in (40..600).step_by(40) {
        let mut s = SectionProps::default();
        s.headers.default = (i == 280).then_some(header);
        if let Block::Para(p) = Arc::make_mut(&mut doc.body[i]) {
            p.section = Some(Box::new(s));
        }
    }
    let mut cache = LayoutCache::new();
    layout(&doc, &mut cache, &LayoutOptions::default());
    let joined = crate::resume::STARTS.get().1;
    let cp = CharProps::default();
    for block in [0, 2, 120] {
        doc.insert_text(&at(block, 3), "typed", &cp).unwrap();
        assert_fresh(&doc, &mut cache, "typing");
    }
    assert!(crate::resume::STARTS.get().1 > joined, "no pass joined the pages before");
}

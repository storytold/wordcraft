//! A layout started again from the page an edit is on ([`crate::resume`]) has the pages of a
//! layout from the first page.

use super::*;
use wordcraft_doc::numbering::ListKind;
use wordcraft_doc::props::NumRef;

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

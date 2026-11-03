//! Page breaks at the end of the body or of a section, and the empty paragraph closing a
//! section after one. Expected pages measured in Word for Mac 16.114 on synthetic documents
//! (A4, one paragraph per line, compatibility modes 12 and 15 alike).
use super::*;
use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{Pos, Table, para_block};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// One body block in a case: `text` (`\u{c}` a page break, `\u{e}` a column break), what it
/// carries.
#[derive(Clone, Copy)]
enum Item {
    Text(&'static str),
    /// A paragraph closing its section (next page, the default), with this text.
    Closing(&'static str),
    /// The same with page break before.
    ClosingBreakBefore,
    /// A bookmark after this text.
    Bookmarked(&'static str),
    /// The same, closing its section.
    ClosingBookmarked,
    /// A shape anchored to the paragraph, in front of the text, after this text.
    Shape(&'static str),
    /// The same, closing its section.
    ClosingShape,
    /// A one-cell table holding these paragraphs.
    Cell(&'static [&'static str]),
}

fn shape() -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 100.0,
        h: 30.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Float { wrap: Wrap::InFrontOfText, v_rel: Anchor::Paragraph, ..Default::default() },
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    }
}

/// The body of `items`, its last section starting as `next`.
fn doc(items: &[Item], next: SectionStart) -> Document {
    let mut d = Document::from_text("");
    d.body = items
        .iter()
        .map(|it| match it {
            Item::Cell(texts) => {
                let mut t = Table::new(1, 1, 200.0);
                if let Some(c) = t.rows.first_mut().and_then(|r| r.cells.first_mut()) {
                    c.blocks = texts.iter().map(|s| para_block(Paragraph::with_text(s, Default::default()))).collect();
                }
                Arc::new(Block::Table(t))
            }
            Item::Text(s) | Item::Closing(s) | Item::Bookmarked(s) | Item::Shape(s) => para_block(Paragraph::with_text(s, Default::default())),
            Item::ClosingBreakBefore | Item::ClosingBookmarked | Item::ClosingShape => para_block(Paragraph::with_text("", Default::default())),
        })
        .collect();
    for (i, it) in items.iter().enumerate() {
        let end = d.para(StoryRef::Body, &Path::top(i)).map_or(0, |p| p.len());
        match it {
            Item::Bookmarked(_) | Item::ClosingBookmarked => {
                d.insert_object(&Pos::body(i, end), InlineObject::BookmarkEnd { name: "mark".into() }, &Default::default()).unwrap();
                d.insert_object(&Pos::body(i, end), InlineObject::BookmarkStart { name: "mark".into() }, &Default::default()).unwrap();
            }
            Item::Shape(_) | Item::ClosingShape => {
                d.insert_object(&Pos::body(i, end), shape(), &Default::default()).unwrap();
            }
            _ => {}
        }
        if let Ok(p) = d.para_mut(StoryRef::Body, &Path::top(i)) {
            if matches!(it, Item::Closing(_) | Item::ClosingBreakBefore | Item::ClosingBookmarked | Item::ClosingShape) {
                p.section = Some(Box::new(SectionProps::default()));
            }
            if matches!(it, Item::ClosingBreakBefore) {
                p.props.page_break_before = Some(true);
            }
        }
    }
    d.last_section.start = next;
    d
}

/// The page count, and the number of the page each block starts on.
fn pages(d: &Document) -> (usize, Vec<Option<u32>>) {
    let l = lay(d);
    let first = (0..u32::try_from(d.body.len()).unwrap())
        .map(|b| {
            l.pages
                .iter()
                .find(|pg| pg.items.iter().any(|it| matches!(it, Placed::Lines { story: StoryRef::Body, path, .. } if path.0.first() == Some(&b))))
                .map(|pg| pg.number)
        })
        .collect();
    (l.pages.len(), first)
}

use Item::*;

/// (body, Word's page count): the break is the last thing in the body's last paragraph,
/// after text or alone, followed only by a bookmark; an empty paragraph after the break gets
/// the new page, and so does a space, a second break or a paragraph holding a bookmark or a
/// shape.
#[test]
fn a_page_break_ending_the_body_starts_no_page() {
    let cases: &[(&[Item], usize)] = &[
        (&[Text("AAA"), Text("ZZZ\u{c}")], 1),
        (&[Text("AAA"), Text("\u{c}")], 1),
        (&[Text("AAA"), Bookmarked("\u{c}")], 1),
        (&[Text("AAA\u{c}"), Text("")], 2),
        (&[Text("AAA\u{c}"), Text(""), Text("")], 2),
        (&[Text("AAA"), Text("\u{c}"), Text("\u{c}")], 2),
        (&[Text("AAA"), Text("\u{c}\u{c}")], 2),
        (&[Text("AAA"), Text("\u{c} ")], 2),
        (&[Text("AAA\u{c}"), Bookmarked("")], 2),
        (&[Text("AAA\u{c}"), Shape("")], 2),
        (&[Text("AAA"), Text("ZZZ\u{e}")], 2),
        (&[Text("AAA"), Text("\u{e}")], 2),
        (&[Text("AAA"), Cell(&["CCC\u{c}"]), Text("")], 1),
        (&[Text("AAA"), Cell(&["CCC", "\u{c}"]), Text("")], 1),
    ];
    for (i, (items, want)) in cases.iter().enumerate() {
        let (n, firsts) = pages(&doc(items, SectionStart::NextPage));
        assert_eq!(n, *want, "case {i}: pages start at {firsts:?}");
    }
}

/// (body, how the section after starts, Word's page count, the page number "BBB" is on): an
/// empty paragraph (bookmarks don't count) closing a section after a page break stays with
/// the break; the next section starts on the page after it, on the next even or odd page if
/// it asks for one. Text or a shape in the closing paragraph, an empty paragraph before it or
/// a column break instead of the page break start a page as usual.
#[test]
fn an_empty_paragraph_closing_a_section_after_a_page_break_starts_no_page() {
    let cases: &[(&[Item], SectionStart, usize, u32)] = &[
        (&[Text("AAA"), Text("\u{c}"), Closing(""), Text("BBB")], SectionStart::NextPage, 2, 2),
        (&[Text("AAA\u{c}"), Closing(""), Text("BBB")], SectionStart::NextPage, 2, 2),
        (&[Text("AAA"), Text("\u{c}"), ClosingBookmarked, Text("BBB")], SectionStart::NextPage, 2, 2),
        (&[Text("AAA"), Bookmarked("\u{c}"), Closing(""), Text("BBB")], SectionStart::NextPage, 2, 2),
        (&[Text("AAA"), Text("\u{c}"), Closing(""), Text("BBB")], SectionStart::EvenPage, 2, 2),
        (&[Text("AAA\u{c}"), Text("CCC"), Text("\u{c}"), Closing(""), Text("BBB")], SectionStart::OddPage, 3, 3),
        (&[Text("AAA"), Text("\u{c}"), Closing(""), Text("BBB")], SectionStart::OddPage, 3, 3),
        (&[Text("AAA"), Text("\u{c}"), Closing(""), Text("BBB")], SectionStart::Continuous, 2, 2),
        (&[Text("AAA"), Text("\u{c}"), Text(""), Closing(""), Text("BBB")], SectionStart::NextPage, 3, 3),
        (&[Text("AAA"), Text("\u{c}"), Closing("SSS"), Text("BBB")], SectionStart::NextPage, 3, 3),
        (&[Text("AAA"), Text("\u{c}"), ClosingShape, Text("BBB")], SectionStart::NextPage, 3, 3),
        (&[Text("AAA"), Text("\u{e}"), Closing(""), Text("BBB")], SectionStart::NextPage, 3, 3),
    ];
    for (i, (items, next, want, bbb)) in cases.iter().enumerate() {
        let (n, firsts) = pages(&doc(items, *next));
        assert_eq!((n, firsts.last().copied().flatten()), (*want, Some(*bbb)), "case {i}: pages start at {firsts:?}");
    }
}

/// (how the section after starts, Word's page count, the page number "BBB" is on): a page
/// break in the paragraph closing its section starts no page of its own: a section starting
/// on a new page (or an even one) starts on the next, a continuous one goes on on the page.
#[test]
fn a_page_break_ending_a_section_starts_no_page() {
    let cases = [(SectionStart::NextPage, 2, 2), (SectionStart::EvenPage, 2, 2), (SectionStart::Continuous, 1, 1)];
    for (next, want, bbb) in cases {
        let (n, firsts) = pages(&doc(&[Text("AAA"), Closing("\u{c}"), Text("BBB")], next));
        assert_eq!((n, firsts.last().copied().flatten()), (want, Some(bbb)), "{next:?}: pages start at {firsts:?}");
    }
}

/// (body, how the section after starts, Word's page count, the page number "BBB" is on); an
/// empty last paragraph that closes no section keeps its page break before.
#[test]
fn an_empty_paragraph_closing_a_section_ignores_its_page_break_before() {
    let cases: &[(&[Item], SectionStart, usize, u32)] = &[
        (&[Text("AAA"), ClosingBreakBefore, Text("BBB")], SectionStart::NextPage, 2, 2),
        (&[Text("AAA"), ClosingBreakBefore, Text("BBB")], SectionStart::EvenPage, 2, 2),
        (&[Text("AAA"), ClosingBreakBefore, Text("BBB")], SectionStart::Continuous, 1, 1),
        (&[Text("AAA"), Text("\u{c}"), ClosingBreakBefore, Text("BBB")], SectionStart::NextPage, 2, 2),
        (&[Text("AAA"), Text("\u{c}"), ClosingBreakBefore, Text("BBB")], SectionStart::Continuous, 2, 2),
    ];
    for (i, (items, next, want, bbb)) in cases.iter().enumerate() {
        let (n, firsts) = pages(&doc(items, *next));
        assert_eq!((n, firsts.last().copied().flatten()), (*want, Some(*bbb)), "case {i}: pages start at {firsts:?}");
    }
    let mut d = doc(&[Text("AAA"), Text("")], SectionStart::NextPage);
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.page_break_before = Some(true);
    assert_eq!(pages(&d).0, 2);
}

/// After the break, the caret (the paragraph's mark) is on the break's page, below it, where
/// Word shows the mark; the break is kept for saving.
#[test]
fn the_caret_after_a_final_page_break_stays_on_its_page() {
    for text in ["ZZZ\u{c}", "\u{c}"] {
        let d = doc(&[Text("AAA"), Text(text)], SectionStart::NextPage);
        let l = lay(&d);
        let end = Pos::body(1, text.len());
        let (before, after) = (l.caret(&Pos::body(1, 0)).unwrap(), l.caret(&end).unwrap());
        assert_eq!((l.pages.len(), after.page), (1, 0), "{text:?}");
        assert!(after.top > before.top, "{text:?}: the caret after the break at {}, before it at {}", after.top, before.top);
        assert_eq!(d.para(StoryRef::Body, &Path::top(1)).unwrap().text, text);
    }
    let d = doc(&[Text("AAA"), Text("\u{c}"), Closing(""), Text("BBB")], SectionStart::NextPage);
    assert_eq!(lay(&d).caret(&Pos::body(2, 0)).map(|c| c.page), Some(0));
}

/// `items` with the blocks at `listed` in one numbered list, laid out: the page count, the
/// labels drawn with the page each is on, and the top of the last block on its page.
fn numbered(items: &[Item], next: SectionStart, listed: &[usize]) -> (usize, Vec<(usize, String)>, Option<f32>) {
    in_list(wordcraft_doc::ListKind::Numbered, items, next, listed)
}

/// [`numbered`] with a list of `kind`.
fn in_list(kind: wordcraft_doc::ListKind, items: &[Item], next: SectionStart, listed: &[usize]) -> (usize, Vec<(usize, String)>, Option<f32>) {
    let mut d = doc(items, next);
    let id = d.numbering.add_list(kind);
    for &i in listed {
        d.para_mut(StoryRef::Body, &Path::top(i)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 });
    }
    let l = lay(&d);
    let mut labels = Vec::new();
    let mut last_top = None;
    let last = u32::try_from(items.len().saturating_sub(1)).unwrap();
    for (i, pg) in l.pages.iter().enumerate() {
        for it in &pg.items {
            if let Placed::Lines { story: StoryRef::Body, path, para, l0, y, .. } = it {
                if *l0 == 0
                    && let Some(label) = &para.label
                {
                    labels.push((i, label.text.clone()));
                }
                if path.0.first() == Some(&last) {
                    last_top = Some(*y - pg.body.y);
                }
            }
        }
    }
    (l.pages.len(), labels, last_top)
}

/// Word draws no label on an empty numbered paragraph that closes a section, and it doesn't
/// count: after a page break, with page break before, in the middle of a page, before a
/// continuous section (taking no room: BBB right under AAA), alone in its section; the next
/// item is 2 (3 when a numbered break paragraph, which counts, comes first). An empty
/// numbered paragraph that closes no section keeps its label.
#[test]
fn an_empty_paragraph_closing_a_section_has_no_list_label() {
    let one = |p: usize| vec![(p, "1.".to_string())];
    let cases: Vec<(&[Item], SectionStart, &[usize], usize, Vec<(usize, String)>)> = vec![
        (&[Text("AAA"), Text("\u{c}"), Closing(""), Text("BBB")], SectionStart::NextPage, &[0, 1, 2], 2, one(0)),
        (&[Text("AAA\u{c}"), Closing(""), Text("BBB")], SectionStart::NextPage, &[0, 1], 2, one(0)),
        (&[Text("AAA"), Text("\u{c}"), Closing(""), Text("BBB")], SectionStart::EvenPage, &[0, 1, 2], 2, one(0)),
        (&[Text("AAA"), ClosingBreakBefore, Text("BBB")], SectionStart::NextPage, &[0, 1], 2, one(0)),
        (&[Text("AAA"), Closing(""), Text("BBB")], SectionStart::NextPage, &[0, 1], 2, one(0)),
        (&[Text("AAA"), Closing(""), Text("BBB")], SectionStart::Continuous, &[0, 1], 1, one(0)),
        (&[Text("AAA"), ClosingBreakBefore, Text("BBB")], SectionStart::Continuous, &[0, 1], 1, one(0)),
        (&[Closing("AAA"), Closing(""), Text("BBB")], SectionStart::NextPage, &[0, 1], 3, one(0)),
        (&[Text("AAA"), Closing(""), Text("CCC")], SectionStart::NextPage, &[0, 1, 2], 2, vec![(0, "1.".into()), (1, "2.".into())]),
        (&[Text("AAA"), Text("\u{c}"), Closing(""), Text("CCC")], SectionStart::NextPage, &[0, 1, 2, 3], 2, vec![(0, "1.".into()), (1, "3.".into())]),
        (&[Text("AAA"), Text(""), Closing(""), Text("BBB")], SectionStart::NextPage, &[0, 1], 2, vec![(0, "1.".into()), (0, "2.".into())]),
        (&[Text("AAA\u{c}"), Text("")], SectionStart::NextPage, &[0, 1], 2, vec![(0, "1.".into()), (1, "2.".into())]),
    ];
    for (i, (items, next, listed, pages, labels)) in cases.iter().enumerate() {
        let (n, drawn, _) = numbered(items, *next, listed);
        assert_eq!((n, &drawn), (*pages, labels), "case {i}");
    }
    let bbb = |listed: &[usize]| numbered(&[Text("AAA"), Closing(""), Text("BBB")], SectionStart::Continuous, listed).2;
    assert_eq!(bbb(&[0, 1]), bbb(&[0]));
}

/// An empty paragraph alone in a section starting a new page, closing it before a continuous
/// section, keeps its line in a list as out of one, and draws no label. Word: `BBB` 21.5 pt
/// below the top of page 2 after `AAA` (an item or not) and at the top of the document, the
/// closing paragraph numbered, bulleted or neither; a numbered `BBB` there is `1.`.
#[test]
fn an_empty_list_paragraph_alone_in_its_section_keeps_its_line() {
    for kind in [wordcraft_doc::ListKind::Numbered, wordcraft_doc::ListKind::Bullet] {
        let after_aaa: &[Item] = &[Closing("AAA"), Closing(""), Text("BBB")];
        for (aaa, mark) in [(&[0][..], &[0, 1][..]), (&[], &[1])] {
            let (n, labels, top) = in_list(kind, after_aaa, SectionStart::Continuous, mark);
            let (plain_n, plain_labels, plain_top) = in_list(kind, after_aaa, SectionStart::Continuous, aaa);
            assert!(plain_top.is_some_and(|t| t > 0.0), "{kind:?}: an unlisted mark keeps its line: {plain_top:?}");
            assert_eq!((n, &labels, top), (plain_n, &plain_labels, plain_top), "{kind:?}, AAA listed: {}", !aaa.is_empty());
        }
        let first: &[Item] = &[Closing(""), Text("BBB")];
        for (mark, plain) in [(&[0][..], &[][..]), (&[0, 1], &[1])] {
            let (n, labels, top) = in_list(kind, first, SectionStart::Continuous, mark);
            let (plain_n, plain_labels, plain_top) = in_list(kind, first, SectionStart::Continuous, plain);
            assert!(plain_top.is_some_and(|t| t > 0.0), "{kind:?}: an unlisted mark keeps its line: {plain_top:?}");
            assert_eq!((n, &labels, top), (plain_n, &plain_labels, plain_top), "{kind:?}, first in the document, BBB listed: {}", plain.len() == 1);
        }
    }
    assert_eq!(numbered(&[Closing(""), Text("BBB")], SectionStart::Continuous, &[0, 1]).1, vec![(0, "1.".to_string())]);
}

/// A bundled font for the text, the marks and the list labels, so that lines don't depend on the
/// installed fonts.
const FONT: &str = "Source Sans 3";

/// `Intro` (with a footnote), `Fill` `before` points below it, list items `AAA` and `AAB` (of
/// `kind`, automatic space after) and `rest`, the blocks of `rest` at `listed` in their list;
/// every paragraph and label in [`FONT`], lines exactly 16 pt.
fn after_items(kind: wordcraft_doc::ListKind, before: f32, rest: &[Item], listed: &[usize], next: SectionStart) -> Document {
    let mut items = vec![Text("Intro"), Text("Fill"), Text("AAA"), Text("AAB")];
    items.extend_from_slice(rest);
    let mut d = doc(&items, next);
    let id = d.numbering.add_list(kind);
    let note = d.add_part(wordcraft_doc::PartKind::Footnote, vec![para_block(Paragraph::with_text("Note", Default::default()))]);
    d.insert_object(
        &Pos::body(0, 5),
        InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id: note, custom: String::new() },
        &Default::default(),
    )
    .unwrap();
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.space_before = Some(before);
    for i in [2, 3] {
        let p = d.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
        p.props.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 });
        p.props.space_after_auto = Some(true);
    }
    for &i in listed {
        d.para_mut(StoryRef::Body, &Path::top(4 + i)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 });
    }
    for level in d.numbering.abstracts.iter_mut().flat_map(|a| a.levels.iter_mut()) {
        level.chr.font = Some(FONT.into());
    }
    let last = d.body.len() - 1;
    let end = d.para(StoryRef::Body, &Path::top(last)).map_or(0, |p| p.len());
    d.format_range(&Pos::body(0, 0), &Pos::body(last, end), &|c| c.font = Some(FONT.into())).unwrap();
    for i in 0..=last {
        d.para_mut(StoryRef::Body, &Path::top(i)).unwrap().props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Exactly(16.0));
    }
    d
}

/// Where the lines of body block `b` are placed on page 1 (their origin), if they are.
fn placed_y(l: &DocLayout, b: u32) -> Option<f32> {
    l.pages.first()?.items.iter().find_map(|it| match it {
        Placed::Lines { story: StoryRef::Body, path, l0: 0, y, .. } if path.0.first() == Some(&b) => Some(*y),
        _ => None,
    })
}

/// On a page with a footnote, a list item's last line keeps its automatic space after (14 pt)
/// clear of the separator, unless the next paragraph is a list item. Word, moving `Fill` down
/// in 5 pt steps: `AAB` goes to page 2 at 650 pt before a numbered `CCC` and before an empty
/// numbered or bulleted paragraph closing a section (next one new page or continuous), at
/// 635 pt before a plain `CCC` or an empty unnumbered closing paragraph.
#[test]
fn an_empty_list_paragraph_closing_a_section_is_an_item_for_the_space_after_the_item_before() {
    let moves_at = |build: &dyn Fn(f32) -> Document| {
        let on_page_1 = |x: u32| pages(&build(x as f32)).1.get(3).copied().flatten() == Some(1);
        let (mut lo, mut hi) = (0u32, 800u32);
        assert!(on_page_1(lo) && !on_page_1(hi));
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if on_page_1(mid) { lo = mid } else { hi = mid }
        }
        hi
    };
    for kind in [wordcraft_doc::ListKind::Numbered, wordcraft_doc::ListKind::Bullet] {
        let next = SectionStart::NextPage;
        let item = moves_at(&|x| after_items(kind, x, &[Text("CCC")], &[0], next));
        let text = moves_at(&|x| after_items(kind, x, &[Text("CCC")], &[], next));
        assert!((item as f32 - text as f32 - wordcraft_doc::resolve::AUTO_SPACING).abs() <= 1.0, "{kind:?}: {item} and {text}");
        for next in [SectionStart::NextPage, SectionStart::Continuous] {
            let mark = moves_at(&|x| after_items(kind, x, &[Closing(""), Text("CCC")], &[0], next));
            let plain = moves_at(&|x| after_items(kind, x, &[Closing(""), Text("CCC")], &[], next));
            assert_eq!((mark, plain), (item, text), "{kind:?}, {next:?}");
        }
    }
}

/// Word drops the item's automatic space after before an empty numbered or bulleted paragraph
/// closing a section as before another item: on a page with a footnote `AAB` moves to the next
/// page at the same offset as before a list item (the test above), and before a continuous
/// section `BBB` comes 13.8 pt (one line) below `AAB` after such a paragraph, 27.8 pt after an
/// unnumbered one. Here the closing list paragraph is placed where a list item would be (the
/// item's automatic space after dropped), an unnumbered one where a plain paragraph would. Before
/// a continuous section the paragraph after it still comes 14 pt lower than in Word: the
/// closing paragraph takes no room and the item's space after comes back (the same on `main`).
#[test]
fn an_empty_list_paragraph_closing_a_section_drops_the_automatic_space_after_the_item_before() {
    for kind in [wordcraft_doc::ListKind::Numbered, wordcraft_doc::ListKind::Bullet] {
        for next in [SectionStart::NextPage, SectionStart::Continuous] {
            let below_aab = |rest: &[Item], listed: &[usize]| {
                let l = lay(&after_items(kind, 0.0, rest, listed, next));
                let (aab, it) = (placed_y(&l, 3), placed_y(&l, 4));
                aab.zip(it).map(|(a, b)| b - a).unwrap_or_else(|| panic!("{kind:?}, {next:?}: AAB at {aab:?}, block 4 at {it:?} on page 1"))
            };
            let (mark, item) = (below_aab(&[Closing(""), Text("BBB")], &[0]), below_aab(&[Text("CCC"), Text("BBB")], &[0]));
            let (plain, text) = (below_aab(&[Closing(""), Text("BBB")], &[]), below_aab(&[Text("CCC"), Text("BBB")], &[]));
            assert!((item - 16.0).abs() < 0.01, "{kind:?}, {next:?}: a list item one line below: {item}");
            assert!((mark - item).abs() < 0.01 && (plain - text).abs() < 0.01, "{kind:?}, {next:?}: {mark} {item}, {plain} {text}");
            assert!((text - item - wordcraft_doc::resolve::AUTO_SPACING).abs() < 0.01, "{kind:?}, {next:?}: {text} {item}");
        }
    }
}

//! Page geometry: mirrored margins, headers and footers taller than their margin (also when
//! linked to the previous section), endnotes continuing over pages, and the caret in a footnote
//! continued on the next page.
use crate::*;
use wordcraft_doc::para::{InlineObject, NoteKind};
use wordcraft_doc::{PartKind, Pos, para_block};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn paras(text: &str, n: usize) -> Vec<Arc<Block>> {
    (1..=n).map(|i| para_block(Paragraph::with_text(&format!("{text} {i}"), Default::default()))).collect()
}

/// Top and bottom of the lines of `story` among `items`.
fn extent(items: &[Placed], story: StoryRef) -> Option<(f32, f32)> {
    items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: s, y, para, l0, l1, .. } if *s == story => Some((*y, item_bottom(*y, para, *l0, *l1)?)),
            _ => None,
        })
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

/// The left edge of the body text on each page.
fn body_left(l: &DocLayout) -> Vec<f32> {
    l.pages
        .iter()
        .map(|p| p.items.iter().find_map(|it| if let Placed::Lines { story: StoryRef::Body, x, .. } = it { Some(*x) } else { None }).unwrap())
        .collect()
}

/// #1836: 2 pages per sheet lays pages out on half the sheet (cut across its long side), a book
/// fold on half the paper's short side by its long side (Word for Mac 16.113.4, user-checked);
/// the margins apply to that half page. Mirrored margins follow a book
/// fold. Tiny and zero sheets don't panic.
#[test]
fn two_per_sheet_and_book_fold_lay_out_on_half_the_sheet() {
    use wordcraft_doc::section::MultiplePages;
    let mut d = Document::from_text(&"Body line\n".repeat(90));
    d.last_section.margin_left = 54.0;
    d.last_section.margin_right = 36.0;
    d.last_section.margin_top = 36.0;
    d.last_section.margin_bottom = 36.0;
    let normal = lay(&d);
    assert_eq!((normal.pages[0].w, normal.pages[0].h), (612.0, 792.0));
    d.settings.set_multiple_pages(MultiplePages::TwoPerSheet);
    let l = lay(&d);
    let p = &l.pages[0];
    assert_eq!((p.w, p.h), (612.0, 396.0), "letter cut across its long side");
    assert!((p.body.x - 54.0).abs() < 0.01 && (p.body.w - (612.0 - 90.0)).abs() < 0.01, "{:?}", p.body);
    assert!((p.body.h - (396.0 - 72.0)).abs() < 0.5, "{:?}", p.body);
    assert!(l.pages.len() > normal.pages.len());
    // The document's own section keeps the sheet.
    assert_eq!((d.last_section.page_w, d.last_section.page_h), (612.0, 792.0));

    d.last_section.set_landscape(true);
    (d.last_section.margin_left, d.last_section.margin_right) = (54.0, 36.0);
    (d.last_section.margin_top, d.last_section.margin_bottom) = (36.0, 36.0);
    d.settings.set_multiple_pages(MultiplePages::BookFold);
    let l = lay(&d);
    assert_eq!((l.pages[0].w, l.pages[0].h), (306.0, 792.0), "landscape letter folded");
    d.settings.set_multiple_pages(MultiplePages::TwoPerSheet);
    assert_eq!((lay(&d).pages[0].w, lay(&d).pages[0].h), (396.0, 612.0), "two pages side by side on a landscape sheet");
    d.settings.set_multiple_pages(MultiplePages::BookFold);
    let l = lay(&d);
    // A booklet mirrors: the inside margin is toward the fold, the left of page 1 and the right of page 2.
    let lefts = body_left(&l);
    assert!(lefts.len() >= 2 && (lefts[0] - 54.0).abs() < 0.01 && (lefts[1] - 36.0).abs() < 0.01, "{lefts:?}");

    for (w, h) in [(0.0, 0.0), (1.0, 1.0), (7.2, 7.2), (f32::NAN, 100.0)] {
        for mode in [MultiplePages::TwoPerSheet, MultiplePages::BookFold] {
            d.last_section.page_w = w;
            d.last_section.page_h = h;
            d.settings.set_multiple_pages(mode);
            let _ = lay(&d);
        }
    }
}

#[test]
fn mirrored_margins_put_the_inside_margin_toward_the_binding() {
    // #452: inside 108 (plus a 10pt gutter), outside 36. Odd pages: inside on the left; even
    // pages: on the right.
    let mut d = Document::from_text(&"Body line\n".repeat(90));
    d.last_section.margin_left = 108.0;
    d.last_section.margin_right = 36.0;
    d.last_section.gutter = 10.0;
    let plain = body_left(&lay(&d));
    assert!(plain.len() >= 2 && plain.iter().all(|x| (x - 118.0).abs() < 0.01), "not mirrored: {plain:?}");
    d.settings.mirror_margins = true;
    let l = lay(&d);
    let lefts = body_left(&l);
    for (i, (x, p)) in lefts.iter().zip(&l.pages).enumerate() {
        let want = if p.number % 2 == 1 { 118.0 } else { 36.0 };
        assert!((x - want).abs() < 0.01, "page {i} (number {}): text at {x}, want {want}", p.number);
        assert!((p.body.x - want).abs() < 0.01 && (p.body.w - d.last_section.text_width()).abs() < 0.01, "page {i}: {:?}", p.body);
    }
}

#[test]
fn inside_and_outside_floats_swap_sides_on_even_pages() {
    // #1181: shapes Word places in the outside margin of a book (mirrored margins) all came out
    // in the right margin. Inside 108 plus a 10pt gutter, outside 36; the paragraph on page 2
    // carries the same six shapes as page 1's, told apart by their widths.
    use super::{rect_shape, shapes};
    use wordcraft_doc::para::{Anchor, Float, FloatAlign, Wrap};
    let mut d = Document::from_text("Odd page\nEven page");
    d.last_section.margin_left = 108.0;
    d.last_section.margin_right = 36.0;
    d.last_section.gutter = 10.0;
    d.settings.mirror_margins = true;
    if let Some(Block::Para(p)) = d.body.get_mut(1).map(Arc::make_mut) {
        p.props.page_break_before = Some(true);
    }
    let floats = [
        (Anchor::Margin, Some(FloatAlign::Outside), 0.0),
        (Anchor::OutsideMargin, Some(FloatAlign::Center), 0.0),
        (Anchor::InsideMargin, Some(FloatAlign::Center), 0.0),
        (Anchor::Page, Some(FloatAlign::Inside), 0.0),
        (Anchor::Page, None, 10.0),
        (Anchor::LeftMargin, Some(FloatAlign::End), 0.0),
    ];
    for block in 0..2 {
        for (i, (h_rel, h_align, x)) in floats.iter().enumerate() {
            let f = Float { wrap: Wrap::InFrontOfText, h_rel: *h_rel, h_align: *h_align, x: *x, ..Default::default() };
            d.insert_object(&Pos::body(block, 0), rect_shape(20.0 + 2.0 * i as f32, 10.0, f), &Default::default()).unwrap();
        }
    }
    // Left edge of shape `i` on an odd and on an even page (page coordinates).
    let want = |i: usize, even: bool| {
        let w = 20.0 + 2.0 * i as f32;
        match (i, even) {
            (0, false) => 576.0 - w,
            (0, true) => 36.0,
            (1, false) => 576.0 + (36.0 - w) / 2.0,
            (1, true) => (36.0 - w) / 2.0,
            (2, false) => (118.0 - w) / 2.0,
            (2, true) => 494.0 + (118.0 - w) / 2.0,
            (3, false) => 0.0,
            (3, true) => 612.0 - w,
            (4, _) => 10.0,
            (5, false) => 118.0 - w,
            _ => 36.0 - w,
        }
    };
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    for (pi, page) in l.pages.iter().enumerate() {
        let r = shapes(&page.items);
        assert_eq!(r.len(), floats.len(), "page {pi}: {r:?}");
        for (i, f) in floats.iter().enumerate() {
            let w = 20.0 + 2.0 * i as f32;
            let got = r.iter().find(|r| (r.w - w).abs() < 0.01).unwrap();
            let x = want(i, page.number % 2 == 0);
            assert!((got.x - x).abs() < 0.01, "page {} shape {i} ({:?}): at {}, want {x}", page.number, f, got.x);
        }
    }
    // Without mirrored margins the margins stay put, but inside and outside still follow the
    // page (ECMA-376 §20.4.3.4): the inside of an even page is its right side.
    d.settings.mirror_margins = false;
    let l = lay(&d);
    let r = shapes(&l.pages[1].items);
    let at = |w: f32| r.iter().find(|r| (r.w - w).abs() < 0.01).unwrap().x;
    assert!((at(20.0) - 118.0).abs() < 0.01, "outside of the margins on page 2: {}", at(20.0));
    assert!((at(26.0) - (612.0 - 26.0)).abs() < 0.01, "inside of page 2: {}", at(26.0));
    assert!((at(28.0) - 10.0).abs() < 0.01, "{}", at(28.0));
}

#[test]
fn outside_floating_tables_swap_sides_on_even_pages() {
    // #1181, for a floating table (`w:tblpXSpec="outside"`): on page 2 of a book the outside is
    // the left. One positioned from the page edge stays there on both pages.
    use wordcraft_doc::para::{Anchor, FloatAlign};
    use wordcraft_doc::props::TableFloat;
    let table_x = |page_two: bool, float: TableFloat| {
        let mut d = Document::from_text("First\nSecond\nAfter");
        d.last_section.margin_left = 108.0;
        d.last_section.margin_right = 36.0;
        d.last_section.gutter = 10.0;
        d.settings.mirror_margins = true;
        if let Some(Block::Para(p)) = d.body.get_mut(1).map(Arc::make_mut) {
            p.props.page_break_before = Some(page_two);
        }
        let mut t = wordcraft_doc::Table::new(1, 1, 100.0);
        t.props.float = Some(float);
        d.insert_block(StoryRef::Body, &Path::top(2), Block::Table(t)).unwrap();
        let l = lay(&d);
        let page = &l.pages[page_two as usize];
        let x = page.items.iter().find_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, x, .. } if path.0.len() > 1 => Some(*x),
            _ => None,
        });
        x.unwrap()
    };
    let outside = TableFloat { h_rel: Anchor::Margin, v_rel: Anchor::Paragraph, h_align: Some(FloatAlign::Outside), ..Default::default() };
    let (odd, even) = (table_x(false, outside), table_x(true, outside));
    assert!(odd > 576.0 - 100.0 - 0.01 && odd < 576.0, "odd page: at the right margin, text at {odd}");
    assert!(even > 36.0 - 0.01 && even < 36.0 + 20.0, "even page: at the left margin, text at {even}");
    let from_page = TableFloat { h_rel: Anchor::Page, v_rel: Anchor::Paragraph, x: 200.0, ..Default::default() };
    let (odd, even) = (table_x(false, from_page), table_x(true, from_page));
    assert!((odd - even).abs() < 0.01, "same place from the page edge: {odd} and {even}");
}

#[test]
fn a_tall_footer_pushes_the_body_up() {
    // #453: an eight-line footer reaches far above the 72pt bottom margin.
    let mut d = Document::from_text(&"Body line\n".repeat(60));
    let id = d.add_part(PartKind::Footer, paras("Footer line", 8));
    d.last_section.footers.default = Some(id);
    let l = lay(&d);
    assert!(l.pages.len() >= 2);
    let foot_top = extent(&l.pages[0].footer, StoryRef::Part(id)).unwrap().0;
    assert!(foot_top < 792.0 - 72.0, "the footer is taller than the margin: {foot_top}");
    for (i, p) in l.pages.iter().enumerate() {
        let (_, body_bottom) = extent(&p.items, StoryRef::Body).unwrap();
        let (top, _) = extent(&p.footer, StoryRef::Part(id)).unwrap();
        assert!(body_bottom <= top + 0.01, "page {i}: body ends at {body_bottom}, footer starts at {top}");
        assert!(p.body.bottom() <= top + 0.01, "page {i}: {:?}", p.body);
    }
    // Every body paragraph is still there, once.
    let placed: usize = l.pages.iter().flat_map(|p| &p.items).filter(|it| matches!(it, Placed::Lines { story: StoryRef::Body, l0: 0, .. })).count();
    assert_eq!(placed, d.body.len());
    // A one-line footer fits in the margin: the body keeps its full height.
    d.parts.insert(id, wordcraft_doc::Part { kind: PartKind::Footer, blocks: paras("Footer", 1), body: Default::default() });
    let l = lay(&d);
    assert!((l.pages[0].body.bottom() - (792.0 - 72.0)).abs() < 0.01, "{:?}", l.pages[0].body);
}

#[test]
fn a_linked_tall_header_pushes_the_next_sections_body_down() {
    // #454: section 2 has no header of its own and shows section 1's (link to previous).
    let mut d = Document::from_text("First section\nSecond section");
    let id = d.add_part(PartKind::Header, paras("Header line", 8));
    let first = SectionProps { headers: wordcraft_doc::section::HeaderSet { default: Some(id), ..Default::default() }, ..d.last_section.clone() };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(first));
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2);
    for (i, p) in l.pages.iter().enumerate() {
        assert_eq!(p.header_story, Some(id), "page {i}");
        let (_, header_bottom) = extent(&p.header, StoryRef::Part(id)).unwrap();
        assert!(header_bottom > 72.0, "the header reaches past the top margin");
        let (body_top, _) = extent(&p.items, StoryRef::Body).unwrap();
        assert!(body_top >= header_bottom - 0.01, "page {i}: body at {body_top}, header ends at {header_bottom}");
    }
    assert_eq!(l.pages[0].body.y, l.pages[1].body.y, "same header, same body top");
}

#[test]
fn long_endnotes_continue_on_new_pages() {
    // #456: an 80-paragraph endnote after a one-line body.
    let mut d = Document::from_text("Body");
    let id = d.add_part(PartKind::Endnote, paras("Endnote paragraph", 80));
    d.insert_object(&Pos::body(0, 4), InlineObject::NoteRef { kind: NoteKind::Endnote, id, custom: String::new() }, &Default::default()).unwrap();
    let l = lay(&d);
    assert!(l.pages.len() >= 2, "{} pages", l.pages.len());
    let mut seen = Vec::new();
    for (i, p) in l.pages.iter().enumerate() {
        if let Some((top, bottom)) = extent(&p.items, StoryRef::Part(id)) {
            assert!(top >= p.body.y - 0.01 && bottom <= p.body.bottom() + 0.01, "page {i}: note at {top}..{bottom}, body {:?}", p.body);
        }
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Part(s), path, l0: 0, .. } = it
                && *s == id
            {
                seen.push(path.0.clone());
            }
        }
    }
    assert_eq!(seen, (0..80u32).map(|i| vec![i]).collect::<Vec<_>>(), "every paragraph once, in order");
    // The caret reaches the last paragraph, on a page.
    let c = l.caret(&Pos { story: StoryRef::Part(id), path: Path::top(79), off: 0 }).unwrap();
    assert!(c.top < 792.0 - 72.0, "{c:?}");
}

#[test]
fn caret_in_a_continued_footnote_is_found_on_its_page() {
    // #457: one long footnote paragraph split over pages 1 and 2.
    let mut d = Document::from_text(&"Body text line.\n".repeat(16));
    let id = d.add_part(
        PartKind::Footnote,
        vec![para_block(Paragraph::with_text(&"A long footnote that runs on and on. ".repeat(100), Default::default()))],
    );
    d.insert_object(&Pos::body(4, 4), InlineObject::NoteRef { kind: NoteKind::Footnote, id, custom: String::new() }, &Default::default()).unwrap();
    let l = lay(&d);
    // The first line of the piece on page 2.
    let (para, k) = l.pages[1]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), para, l0, .. } if *s == id && *l0 > 0 => Some((para.clone(), *l0)),
            _ => None,
        })
        .expect("the note continues on page 2");
    let at = |off: usize| Pos { story: StoryRef::Part(id), path: Path::top(0), off };
    let cont = at(para.lines[k + 1].start);
    for hint in [0, 1] {
        assert_eq!(l.caret_on(&at(0), hint).map(|c| c.page), Some(0), "start, hint {hint}");
        let c = l.caret_on(&cont, hint).unwrap_or_else(|| panic!("no caret with hint {hint}"));
        assert_eq!(c.page, 1, "hint {hint}");
        // Up goes to the line above, not to the note's start.
        let (up, _) = l.vertical(&cont, None, -1, hint).unwrap();
        assert_eq!(para.line_of(up.off), k, "hint {hint}: {up:?}");
    }
}

/// A letter per section, each restarting its page numbers, with "Page X of Y" in its footer
/// where Y is a `PAGEREF` to a bookmark at the letter's end (how mail-merged letters count their
/// own pages). The footer shows each letter's last page, not the file's stale result.
#[test]
fn page_references_in_footers_show_their_bookmarks_page() {
    let field =
        |instr: &str| InlineObject::Field { instr: instr.into(), result: "9".into(), locked: false, dirty: false, code: false, form: String::new() };
    let footer = |d: &mut Document, mark: &str| {
        let mut p = Paragraph::with_text("Page  of ", Default::default());
        p.insert_object(5, field("PAGE"), &Default::default()).unwrap();
        let n = p.len();
        p.insert_object(n, field(&format!("PAGEREF {mark} \\* Arabic \\* MERGEFORMAT")), &Default::default()).unwrap();
        d.add_part(PartKind::Footer, vec![para_block(p)])
    };
    let mut d = Document::from_text("Letter A 1\nLetter A 2\nLetter A 3\nLetter B 1\nLetter B 2");
    let (fa, fb) = (footer(&mut d, "End_A"), footer(&mut d, "End_B"));
    for i in [1, 2, 4] {
        d.para_mut(StoryRef::Body, &Path::top(i)).unwrap().props.page_break_before = Some(true);
    }
    for (i, mark) in [(2, "End_A"), (4, "End_B")] {
        let p = d.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
        let n = p.len();
        p.insert_object(n, InlineObject::BookmarkStart { name: mark.into() }, &Default::default()).unwrap();
    }
    let restart = |id| SectionProps {
        page_num_start: Some(1),
        footers: wordcraft_doc::section::HeaderSet { default: Some(id), ..Default::default() },
        ..d.last_section.clone()
    };
    let (a, b) = (restart(fa), restart(fb));
    d.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section = Some(Box::new(a));
    d.last_section = b;
    let shown = |l: &DocLayout| -> Vec<String> {
        l.pages
            .iter()
            .map(|p| {
                p.footer
                    .iter()
                    .flat_map(|it| if let Placed::Lines { para, .. } = it { para.shown.iter().map(|s| s.1.clone()).collect() } else { vec![] })
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect()
    };
    let mut cache = LayoutCache::new();
    let l = layout(&d, &mut cache, &LayoutOptions::default());
    assert_eq!(shown(&l), ["1/3", "2/3", "3/3", "1/2", "2/2"]);
    // The bookmark moves to the letter's first page: only the reference changes, and the
    // footers laid out before (same page, same totals) aren't reused.
    let last = d.para_mut(StoryRef::Body, &Path::top(4)).unwrap();
    let n = last.len();
    last.delete(n - '\u{FFFC}'.len_utf8(), n).unwrap();
    let first = d.para_mut(StoryRef::Body, &Path::top(3)).unwrap();
    let n = first.len();
    first.insert_object(n, InlineObject::BookmarkStart { name: "End_B".into() }, &Default::default()).unwrap();
    assert_eq!(shown(&layout(&d, &mut cache, &LayoutOptions::default())), ["1/3", "2/3", "3/3", "1/1", "2/1"]);
    // In the body, a page reference keeps its result until fields are updated.
    let mut body = Document::from_text("Intro\nSee page ");
    let p = body.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.insert_object(0, InlineObject::BookmarkStart { name: "Intro".into() }, &Default::default()).unwrap();
    let p = body.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    let n = p.len();
    p.insert_object(n, field("PAGEREF Intro"), &Default::default()).unwrap();
    // The footer's reference to the same bookmark is resolved.
    let mut fp = Paragraph::new();
    fp.insert_object(0, field("PAGEREF Intro"), &Default::default()).unwrap();
    let id = body.add_part(PartKind::Footer, vec![para_block(fp)]);
    body.last_section.footers.default = Some(id);
    let l = lay(&body);
    assert_eq!(footer_shown(&l), ["1"]);
    let texts: Vec<String> = l.pages[0]
        .items
        .iter()
        .flat_map(|it| if let Placed::Lines { para, .. } = it { para.shown.iter().map(|s| s.1.clone()).collect() } else { vec![] })
        .collect();
    assert_eq!(texts, ["9"]);
}

/// The fields' text in each page's footer, joined with "/".
fn footer_shown(l: &DocLayout) -> Vec<String> {
    l.pages
        .iter()
        .map(|p| {
            p.footer
                .iter()
                .flat_map(|it| if let Placed::Lines { para, .. } = it { para.shown.iter().map(|s| s.1.clone()).collect() } else { vec![] })
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

/// A footer of `PAGEREF` fields to `marks` (stored result "9"), set on the last section.
fn page_ref_footer(d: &mut Document, marks: &[&str]) -> u32 {
    let mut p = Paragraph::new();
    for mark in marks {
        let n = p.len();
        let field = InlineObject::Field {
            instr: format!("PAGEREF {mark}"),
            result: "9".into(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        };
        p.insert_object(n, field, &Default::default()).unwrap();
    }
    let id = d.add_part(PartKind::Footer, vec![para_block(p)]);
    d.last_section.footers.default = Some(id);
    id
}

fn mark(d: &mut Document, path: &Path, off: usize, name: &str) {
    d.para_mut(StoryRef::Body, path).unwrap().insert_object(off, InlineObject::BookmarkStart { name: name.into() }, &Default::default()).unwrap();
}

/// Bookmarks in the second half of a paragraph split over two pages, in a table cell and in a
/// section numbered in roman numerals are found on the page and in the format they land in.
#[test]
fn page_references_find_bookmarks_in_split_paragraphs_tables_and_roman_sections() {
    // One long paragraph over two pages, marked at its start and its end.
    let mut d = Document::from_text(&"Long paragraph text running on. ".repeat(160));
    let n = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();
    mark(&mut d, &Path::top(0), n, "End");
    mark(&mut d, &Path::top(0), 0, "Start");
    page_ref_footer(&mut d, &["Start", "End"]);
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2, "the paragraph runs over two pages");
    assert_eq!(footer_shown(&l), ["1/2", "1/2"]);

    // A table cell on the second page.
    let mut d = Document::from_text("First\nSecond");
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.page_break_before = Some(true);
    d.insert_block(StoryRef::Body, &Path::top(2), Block::Table(wordcraft_doc::Table::new(1, 1, 300.0))).unwrap();
    let cell = Path(vec![2, 0, 0, 0]);
    d.para_mut(StoryRef::Body, &cell).unwrap().insert_text(0, "In a cell", &Default::default()).unwrap();
    mark(&mut d, &cell, 0, "Cell");
    page_ref_footer(&mut d, &["Cell"]);
    assert_eq!(footer_shown(&lay(&d)), ["2", "2"]);

    // The second section numbers its pages in lower-case roman numerals.
    let mut d = Document::from_text("Arabic\nRoman 1\nRoman 2");
    d.para_mut(StoryRef::Body, &Path::top(2)).unwrap().props.page_break_before = Some(true);
    let n = d.para(StoryRef::Body, &Path::top(2)).unwrap().len();
    mark(&mut d, &Path::top(2), n, "End");
    let first = d.last_section.clone();
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(first));
    d.last_section.page_num_start = Some(1);
    d.last_section.page_num_format = wordcraft_doc::section::NumFormat::LowerRoman;
    page_ref_footer(&mut d, &["End"]);
    assert_eq!(footer_shown(&lay(&d)), ["", "ii", "ii"], "the first section has no footer of its own");

    // Two bookmarks of the same name: the first in the document counts.
    let mut d = Document::from_text("First\nSecond");
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.page_break_before = Some(true);
    mark(&mut d, &Path::top(1), 0, "Twice");
    mark(&mut d, &Path::top(0), 0, "Twice");
    page_ref_footer(&mut d, &["Twice"]);
    assert_eq!(footer_shown(&lay(&d)), ["1", "1"]);
}

/// A text box in a footer resolves its page references; one in the body keeps its result.
#[test]
fn page_references_in_footer_text_boxes_are_resolved() {
    use wordcraft_doc::para::{Float, ShapeKind, Wrap};
    let text_box = |d: &mut Document| {
        let mut p = Paragraph::new();
        let field =
            InlineObject::Field { instr: "PAGEREF End".into(), result: "9".into(), locked: false, dirty: false, code: false, form: String::new() };
        p.insert_object(0, field, &Default::default()).unwrap();
        let story = d.add_part(PartKind::TextBox, vec![para_block(p)]);
        InlineObject::Shape {
            kind: ShapeKind::TextBox,
            w: 100.0,
            h: 40.0,
            fill: None,
            stroke: None,
            stroke_width: 0.0,
            float: Float { wrap: Wrap::Inline, ..Default::default() },
            story: Some(story),
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        }
    };
    let box_text = |l: &DocLayout, story: Option<u32>| -> Vec<String> {
        l.pages
            .iter()
            .flat_map(|p| p.items.iter().chain(&p.footer))
            .flat_map(|it| match it {
                Placed::Lines { story: StoryRef::Part(id), para, .. } if Some(*id) == story => para.shown.iter().map(|s| s.1.clone()).collect(),
                _ => vec![],
            })
            .collect()
    };
    let mut d = Document::from_text("First\nSecond");
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.page_break_before = Some(true);
    let n = d.para(StoryRef::Body, &Path::top(1)).unwrap().len();
    mark(&mut d, &Path::top(1), n, "End");
    let in_footer = text_box(&mut d);
    let footer_box = in_footer.text_box();
    let mut fp = Paragraph::with_text("Footer ", Default::default());
    let n = fp.len();
    fp.insert_object(n, in_footer, &Default::default()).unwrap();
    let id = d.add_part(PartKind::Footer, vec![para_block(fp)]);
    d.last_section.footers.default = Some(id);
    let in_body = text_box(&mut d);
    let body_box = in_body.text_box();
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().insert_object(0, in_body, &Default::default()).unwrap();
    let l = lay(&d);
    assert!(box_text(&l, footer_box).iter().all(|t| t == "2") && !box_text(&l, footer_box).is_empty(), "{:?}", box_text(&l, footer_box));
    assert_eq!(box_text(&l, body_box), ["9"]);
}

#[test]
fn widow_control_moves_a_three_line_paragraph_whole() {
    // A three-line paragraph with room for two at the bottom of a page can't leave a widow (two
    // and one) nor an orphan (one and two): Word moves it whole. A four-line one splits two and
    // two. Sweep the paragraph down the page a point at a time and check every split.
    let mut split4 = false;
    for (want, words) in [(3usize, 45usize), (4, 55)] {
        for (fill, shift) in (30usize..60).flat_map(|f| (0..15).step_by(2).map(move |s| (f, s))) {
            let filler = "Filler line.\n".repeat(fill);
            let mut d = Document::from_text(&format!("{filler}{}\nAfter.", "word ".repeat(words)));
            d.styles.default_chr.font = Some("Source Sans 3".into());
            d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.space_before = Some(shift as f32)).unwrap();
            let l = lay(&d);
            let pieces: Vec<(usize, usize)> = l
                .pages
                .iter()
                .filter_map(|p| {
                    p.items.iter().find_map(|it| match it {
                        Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } if *path == Path::top(fill) => {
                            Some((l1 - l0, para.lines.len()))
                        }
                        _ => None,
                    })
                })
                .collect();
            assert!(pieces.iter().all(|&(_, n)| n == want), "{words} words: {pieces:?}");
            assert!(pieces.iter().all(|&(c, _)| c >= 2), "shift {shift}, {want} lines split as {pieces:?}");
            split4 |= want == 4 && pieces.len() == 2;
        }
    }
    assert!(split4, "the sweep never took the paragraph across a page break");
}

#[test]
fn footer_border_limits_body() {
    use wordcraft_doc::props::{Border, Borders, HeightRule};
    let mut d = Document::from_text(&"Body line\n".repeat(40));
    d.last_section.page_h = 400.0;
    d.last_section.margin_bottom = 36.0;
    d.last_section.footer = 16.0;
    let mut table = wordcraft_doc::Table::new(1, 1, d.last_section.text_width());
    table.rows[0].props.height = Some(160.0);
    table.rows[0].props.height_rule = HeightRule::Exact;
    table.props.borders = Some(Borders::all(Border::single(4.0)));
    let id = d.add_part(PartKind::Footer, vec![Arc::new(Block::Table(table))]);
    d.last_section.footers.default = Some(id);
    for page in &lay(&d).pages {
        let top = page
            .footer
            .iter()
            .filter_map(|item| match item {
                Placed::Rule { y0, y1, border, .. } if (y0 - y1).abs() < 0.01 => Some(y0 - border.width / 2.0),
                _ => None,
            })
            .fold(f32::MAX, f32::min);
        assert!(page.body.bottom() <= top + 0.01, "body {}, footer stroke {top}", page.body.bottom());
    }
}

/// A section numbering its pages in lower-case roman shows them so in its footer when the body
/// shows the page count (`NUMPAGES`): the footer laid out while paginating, before the section's
/// number format is known, isn't taken for the page's own footer.
#[test]
fn a_roman_page_number_in_the_footer_stays_roman_with_the_page_count_in_the_body() {
    let field = |instr: &str| InlineObject::Field {
        instr: instr.into(),
        result: String::new(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let mut d = Document::new();
    let mut fp = Paragraph::new();
    fp.insert_object(0, field("PAGE"), &Default::default()).unwrap();
    let id = d.add_part(PartKind::Footer, vec![para_block(fp)]);
    d.last_section.footers.default = Some(id);
    let mut np = Paragraph::with_text(" pages", Default::default());
    np.insert_object(0, field("NUMPAGES"), &Default::default()).unwrap();
    d.body = vec![para_block(np)];
    d.body.extend(paras(&"word ".repeat(40), 40));
    let mut end = Paragraph::with_text("end", Default::default());
    end.section = Some(Box::new(d.last_section.clone()));
    d.body.push(para_block(end));
    d.body.extend(paras(&"word ".repeat(40), 120));
    d.last_section.page_num_start = Some(1);
    d.last_section.page_num_format = wordcraft_doc::section::NumFormat::LowerRoman;
    let l = lay(&d);
    let roman: Vec<_> = l.pages.iter().filter(|p| p.section == 1).collect();
    assert!(roman.len() > 5);
    for p in roman {
        let glyphs: usize = p.footer.iter().map(|it| if let Placed::Lines { para, .. } = it { para.glyphs.len() } else { 0 }).sum();
        let want = wordcraft_doc::section::NumFormat::LowerRoman.format(p.number);
        assert_eq!(glyphs, want.chars().count(), "page {}: the footer should show {want}", p.number);
    }
}

#[test]
fn a_page_ends_when_a_multiple_spaced_lines_text_no_longer_fits() {
    // Multiple spacing puts its extra leading below the text, and Word lets that leading of a
    // page's last line run into the bottom margin: a multiple-spaced line stays on the page while
    // its text ends above the margin (observed in Word at 1.15, 1.5 and double spacing: the page
    // holds one more line than when the whole line box must fit, for shifts up to about the
    // leading). Each shift moves the text down; on every page the last line's text ends within
    // the margin, and the next line's would not.
    use wordcraft_doc::props::LineSpacing;
    for multiple in [1.15, 1.5, 2.0] {
        let mut past_margin = 0;
        for shift in (0..28).step_by(2) {
            let mut d = Document::from_text(&"Several words wrap onto many lines of a long paragraph. ".repeat(60));
            if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(Arc::make_mut) {
                p.props.line_spacing = Some(LineSpacing::Multiple(multiple));
                p.props.space_before = Some(shift as f32);
                p.props.widow_control = Some(false);
            }
            let bottom = d.last_section.page_h - d.last_section.margin_bottom;
            let l = lay(&d);
            assert!(l.pages.len() >= 2, "{multiple} shift {shift}: the paragraph should run onto a second page");
            let Some(Placed::Lines { y, para, l0, l1, .. }) =
                l.pages[0].items.iter().find(|i| matches!(i, Placed::Lines { story: StoryRef::Body, .. }))
            else {
                panic!("{multiple} shift {shift}: no body lines on page 1")
            };
            let (first, last, next) = (&para.lines[*l0], &para.lines[*l1 - 1], &para.lines[*l1]);
            let top = y + last.top - first.top;
            let text_end = top + (last.baseline - last.top) + last.text_desc;
            assert!(text_end <= bottom + 0.01, "{multiple} shift {shift}: the last line's text ends at {text_end}, below the margin at {bottom}");
            let next_text_end = top + last.height + (next.baseline - next.top) + next.text_desc;
            assert!(
                next_text_end > bottom + 0.01,
                "{multiple} shift {shift}: the next line's text would end at {next_text_end}, within the margin at {bottom}"
            );
            if top + last.height > bottom + 0.01 {
                past_margin += 1;
            }
        }
        assert!(past_margin > 0, "{multiple}: some shift should leave the last line's leading past the margin");
    }
}

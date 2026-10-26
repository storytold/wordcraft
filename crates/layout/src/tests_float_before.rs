//! Text before a floating table or object on its page wraps around it like the text after it
//! (see `float_before`): it moves below one placed over it, beside a narrow one, and stays above
//! one lower down. The cases are those measured in Word (`docs/layout-parity.md`).
//!
//! Paragraphs have exact 17 pt lines and no space before or after (`exact_text`) in the bundled
//! Source Sans 3 ([`pinned`]), so the results don't depend on the installed fonts.
use super::tests::{rect_shape, shapes};
use super::*;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::para::{Anchor, Float, FloatAlign, Wrap};
use wordcraft_doc::props::{HeightRule, TableFloat};
use wordcraft_doc::{PartKind, Pos, Table};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// The lines of top-level body block `block` on page `page`: top and bottom of its lines and the
/// left edge of its first line's room.
fn lines(l: &DocLayout, page: usize, block: u32) -> Option<(f32, f32, f32)> {
    let mut found: Option<(f32, f32, f32)> = None;
    for it in &l.pages.get(page)?.items {
        let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, .. } = it else { continue };
        if path.0 != [block] {
            continue;
        }
        let shown = para.lines.get(*l0..*l1).unwrap_or(&[]);
        let first = shown.first().map_or(0.0, |l| l.top);
        for line in shown {
            let (top, bottom) = (y + line.top - first, y + line.top - first + line.height);
            found = Some(match found {
                Some((t, b, left)) => (t.min(top), b.max(bottom), left),
                None => (top, bottom, x + line.left),
            });
        }
    }
    found
}

fn top(l: &DocLayout, page: usize, block: u32) -> f32 {
    lines(l, page, block).unwrap().0
}

/// The page block `block` starts on and its top there.
fn page_top(l: &DocLayout, block: u32) -> (usize, f32) {
    (0..l.pages.len()).find_map(|page| lines(l, page, block).map(|(t, ..)| (page, t))).unwrap()
}

/// Whether blocks `0..n` stand in order, each at least `line` below the one before on its page.
fn in_order(l: &DocLayout, n: u32, line: f32) -> bool {
    let at: Vec<(usize, f32)> = (0..n).map(|b| page_top(l, b)).collect();
    at.windows(2).all(|w| w[1].0 > w[0].0 || (w[1].0 == w[0].0 && w[1].1 - w[0].1 >= line - 0.01))
}

/// Where table `block` stands on page `page` (its cells' union).
fn table_rect(l: &DocLayout, page: usize, block: u32) -> Option<Rect> {
    l.pages
        .get(page)?
        .items
        .iter()
        .filter_map(|it| if let Placed::Cell { rect, table, .. } = it { (table.0 == [block]).then_some(*rect) } else { None })
        .reduce(|a, b| a.union(&b))
}

/// The bundled font the documents here are set in: its own line (15.9 pt at the default 12 pt) is
/// shorter than the 17 pt lines and nothing in them depends on the fonts a machine has installed.
const TEST_FONT: &str = "Source Sans 3";

/// The paragraphs of `text` (one per line) in [`TEST_FONT`].
fn pinned(text: &str) -> Document {
    let mut d = Document::from_text(text);
    d.styles.default_chr.font = Some(TEST_FONT.into());
    d
}

/// The paragraphs of `text` (one per line), each with exact 17 pt lines and no space before or
/// after, in [`TEST_FONT`].
fn exact_text(text: &str) -> Document {
    use wordcraft_doc::props::LineSpacing;
    let mut d = pinned(text);
    for b in &mut d.body {
        if let Block::Para(p) = Arc::make_mut(b) {
            p.props.line_spacing = Some(LineSpacing::Exactly(17.0));
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(0.0);
        }
    }
    d
}

/// How far an empty paragraph moves the next one down.
fn empty_pitch() -> f32 {
    let plain = lay(&exact_text("\nAfter"));
    top(&plain, 0, 1) - top(&plain, 0, 0)
}

/// A one-cell floating table `w` wide with one row exactly `h` high, placed by `float`.
fn float_table(w: f32, h: f32, float: TableFloat) -> Table {
    let mut t = Table::new(1, 1, w);
    t.rows[0].props.height = Some(h);
    t.rows[0].props.height_rule = HeightRule::Exact;
    t.props.float = Some(float);
    t
}

/// Placed `y` from the top of the page, across the page.
fn on_page(y: f32) -> TableFloat {
    TableFloat { h_rel: Anchor::Page, v_rel: Anchor::Page, y, ..Default::default() }
}

/// The paragraphs of `text` with floating table `t` inserted before paragraph `at`.
fn doc_with_table(text: &str, at: usize, t: Table) -> Document {
    let mut d = exact_text(text);
    d.insert_block(StoryRef::Body, &Path::top(at), Block::Table(t)).unwrap();
    d
}

/// Every body line on page `page` clear of `r`.
fn clear_of(l: &DocLayout, page: usize, r: &Rect) -> bool {
    l.pages[page].items.iter().all(|it| match it {
        Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, .. } if path.0.len() == 1 => {
            let shown = para.lines.get(*l0..*l1).unwrap_or(&[]);
            let first = shown.first().map_or(0.0, |l| l.top);
            // (Rounding: a line ending a hair inside the table is clear of it.)
            shown.iter().all(|line| !Rect::new(x + line.left, y + line.top - first + 0.01, line.right - line.left, line.height - 0.02).intersects(r))
        }
        _ => true,
    })
}

#[test]
fn a_paragraph_before_a_floating_table_over_it_moves_below_it() {
    // Word: `ZZZ`, a full-width table placed on the page 23.3pt down over it, `AAA`: ZZZ goes
    // below the table and AAA after it (both used to stay at the top margin, under the table).
    for text in ["Before\nAfter", "\nAfter"] {
        let l = lay(&doc_with_table(text, 1, float_table(612.0, 100.0, on_page(40.0))));
        assert_eq!(l.pages.len(), 1);
        let table = table_rect(&l, 0, 1).unwrap();
        assert!((table.y - 40.0).abs() < 0.01, "{text:?}: the table stays put: {table:?}");
        let (before, after) = (lines(&l, 0, 0).unwrap(), lines(&l, 0, 2).unwrap());
        assert!(before.0 >= table.bottom() - 0.01, "{text:?}: the paragraph before at {before:?}, the table ends at {}", table.bottom());
        assert!(after.0 >= before.1 - 0.01, "{text:?}: after at {after:?}, below the one before ({before:?})");
    }
    // The empty paragraph before the table takes its room below it: the text after starts that
    // much lower than with no paragraph before the table.
    let alone = lay(&doc_with_table("After", 0, float_table(612.0, 100.0, on_page(40.0))));
    let l = lay(&doc_with_table("\nAfter", 1, float_table(612.0, 100.0, on_page(40.0))));
    let after = top(&l, 0, 2);
    assert!((after - top(&alone, 0, 1) - empty_pitch()).abs() < 0.5, "after at {after}, alone {}", top(&alone, 0, 1));
}

#[test]
fn a_paragraph_before_a_floating_table_lower_down_stays_above_it() {
    // Word: a table 150pt down leaves room above it for the paragraph before and the one after.
    let l = lay(&doc_with_table("Before\nAfter", 1, float_table(612.0, 100.0, on_page(300.0))));
    assert!((top(&l, 0, 0) - 72.0).abs() < 0.5, "{}", top(&l, 0, 0));
    assert!(lines(&l, 0, 2).unwrap().1 <= 300.0, "{:?}", lines(&l, 0, 2));
}

/// Word: eight paragraphs, then a table 100pt down: the first stay above it, the rest go below it
/// in order. A line the table's top lands on anywhere moves below it: its 17pt (exactly or at
/// least) are in the way, however short the font's own line (two bundled fonts, 15.9 and 15.8 pt;
/// see the next test for Word's measurements).
#[test]
fn of_the_paragraphs_before_a_floating_table_those_in_its_way_move_below_it() {
    use wordcraft_doc::props::LineSpacing;
    let text_8 = (1..=8).map(|i| format!("Paragraph {i}")).collect::<Vec<_>>().join("\n") + "\nAfter";
    for font in [TEST_FONT, "JetBrains Mono"] {
        for spacing in [LineSpacing::Exactly(17.0), LineSpacing::AtLeast(17.0)] {
            for y in (0..17).map(|k| 106.5 + k as f32).chain([160.0]) {
                let mut d = doc_with_table(&text_8, 8, float_table(612.0, 100.0, on_page(y)));
                d.styles.default_chr.font = Some(font.into());
                for b in &mut d.body {
                    if let Block::Para(p) = Arc::make_mut(b) {
                        p.props.line_spacing = Some(spacing);
                    }
                }
                let l = lay(&d);
                let table = table_rect(&l, 0, 8).unwrap();
                let boxes: Vec<(f32, f32, f32)> = (0..8).chain(9..10).map(|b| lines(&l, 0, b).unwrap()).collect();
                let case = format!("{font:?} {spacing:?} y {y}");
                assert!((boxes[0].0 - 72.0).abs() < 0.5, "{case}: the first paragraph stays at the top: {boxes:?}");
                assert!(boxes.windows(2).all(|w| w[1].0 >= w[0].1 - 0.01), "{case}: in order: {boxes:?}");
                assert!(boxes.iter().all(|(t, b, _)| *b <= table.y + 0.01 || *t >= table.bottom() - 0.01), "{case}: {boxes:?} around {table:?}");
                assert!(boxes.iter().any(|(t, ..)| *t >= table.bottom() - 0.01), "{case}: some below: {boxes:?}");
            }
        }
    }
}

/// Word (Calibri 11 pt, whose own line is 13.4 pt, in exact or at-least 17 pt lines): a full-width
/// table or square-wrapped shape whose top lands anywhere in the third line, 106 to 123 pt, moves
/// that line below it, also 16.5 pt into it; a shape at 123 leaves it in place. The same whether
/// the float comes after the line in the document (a table after it, a shape anchored in a later
/// paragraph) or before it. Here in two bundled fonts whose own lines (15.9 and 15.8 pt) are
/// shorter than 16.5 pt, so that the font's line alone would leave the line over a float there.
#[test]
fn exact_and_at_least_lines_keep_their_whole_height_clear_of_a_float_before_or_after_its_anchor() {
    use wordcraft_doc::props::LineSpacing;
    let text_8 = (1..=8).map(|i| format!("P{i} word")).collect::<Vec<_>>().join("\n");
    let square = |y: f32| Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, y, ..Default::default() };
    let in_font = |mut d: Document, font: &str, spacing: LineSpacing| {
        d.styles.default_chr.font = Some(font.into());
        for b in &mut d.body {
            if let Block::Para(p) = Arc::make_mut(b) {
                p.props.line_spacing = Some(spacing);
            }
        }
        d
    };
    for font in [TEST_FONT, "JetBrains Mono"] {
        for spacing in [LineSpacing::Exactly(17.0), LineSpacing::AtLeast(17.0)] {
            let plain = lay(&in_font(exact_text(&text_8), font, spacing));
            let own = plain.pages[0].items.iter().find_map(|it| match it {
                Placed::Lines { story: StoryRef::Body, para, .. } => para.styles.first().map(|s| s.ascent + s.descent),
                _ => None,
            });
            assert!(own.is_some_and(|h| h < 16.5), "{font}: its own line {own:?} is shorter than 16.5 pt");
            assert_eq!(lines(&plain, 0, 2).map(|(t, b, _)| (t, b)), Some((106.0, 123.0)), "{font} {spacing:?}: the third line");
            for float_first in [false, true] {
                for table in [true, false] {
                    for y in (0..17).map(|k| 106.5 + k as f32).chain([123.0]) {
                        if table && y >= 123.0 {
                            continue;
                        }
                        let d = match (table, float_first) {
                            (true, first) => doc_with_table(&text_8, if first { 0 } else { 5 }, float_table(612.0, 50.0, on_page(y))),
                            (false, first) => doc_with_shape(&text_8, if first { 0 } else { 5 }, 612.0, 50.0, square(y)),
                        };
                        let l = lay(&in_font(d, font, spacing));
                        let float =
                            if table { table_rect(&l, 0, if float_first { 0 } else { 5 }) } else { shapes(&l.pages[0].items).first().copied() };
                        let case = format!("{font:?} {spacing:?} float first {float_first}, table {table}, y {y}");
                        let float = float.unwrap_or_else(|| panic!("{case}: no float"));
                        assert!((float.y - y).abs() < 0.01, "{case}: the float stays put: {float:?}");
                        let third = lines(&l, 0, if table && float_first { 3 } else { 2 }).unwrap();
                        if y < 123.0 {
                            assert!(third.0 >= float.bottom() - 0.01, "{case}: the third line at {third:?}, the float ends at {}", float.bottom());
                        } else {
                            assert!((third.0 - 106.0).abs() < 0.01, "{case}: the third line stays at {third:?}");
                        }
                    }
                }
            }
        }
    }
}

/// Word for Mac (exact 12 pt lines, a full-width table or square-wrapped shape whose top is at
/// the bottom of the second line or a twip either side, compatibility modes 12 and 15): a table
/// touching the line's bottom moves the line below it, whether the table comes before the line in
/// the document (#3977) or after it; a shape touching it leaves it in place, and so does a table
/// placed from the text, here one line up from where it comes in the document (the line before
/// that, which it overlaps, moves below it). So the text before a table placed on the page or
/// margin is laid out again for a table touching one of its lines, and not for a shape or a table
/// placed from the text: any vertical anchor other than the page or margin, as placement takes it.
#[test]
fn a_floating_table_touching_the_bottom_of_a_line_before_it_moves_that_line_below_it() {
    let text = "P1\nP2\nP3\nP4";
    let square = |y: f32| Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, y, ..Default::default() };
    for mode in [11, 12, 14, 15] {
        for twips in [-1, 0, 1] {
            let y = 106.0 + twips as f32 / 20.0;
            let on_margin = TableFloat { v_rel: Anchor::Margin, ..on_page(y - 72.0) };
            let column_across = TableFloat { h_rel: Anchor::Column, ..on_page(y) };
            for (case, at, table) in [
                ("table first", 0, on_page(y)),
                ("table after", 2, on_page(y)),
                ("table after, on the margin", 2, on_margin),
                ("table after, across from the column", 2, column_across),
                ("shape", 3, on_page(y)),
            ] {
                let mut d = if case == "shape" {
                    doc_with_shape(text, at, 612.0, 50.0, square(y))
                } else {
                    doc_with_table(text, at, float_table(612.0, 50.0, table))
                };
                d.settings.compat_mode = mode;
                let l = lay(&d);
                let float = if case == "shape" { shapes(&l.pages[0].items).first().copied() } else { table_rect(&l, 0, at as u32) };
                let float = float.unwrap_or_else(|| panic!("mode {mode} {case} {twips}: no float"));
                assert!((float.y - y).abs() < 0.001, "mode {mode} {case} {twips}: the float stays put: {float:?}");
                let (p1, p2) = if at == 0 { (1, 2) } else { (0, 1) };
                assert!((top(&l, 0, p1) - 72.0).abs() < 0.001, "mode {mode} {case} {twips}: P1 at {}", top(&l, 0, p1));
                let moves = twips < 0 || (twips == 0 && case != "shape");
                let expected = if moves { float.bottom() } else { 89.0 };
                assert!((top(&l, 0, p2) - expected).abs() < 0.001, "mode {mode} {case} {twips}: P2 at {}, expected {expected}", top(&l, 0, p2));
            }
            for v_rel in [
                Anchor::Paragraph,
                Anchor::Line,
                Anchor::Column,
                Anchor::Character,
                Anchor::TopMargin,
                Anchor::BottomMargin,
                Anchor::LeftMargin,
                Anchor::RightMargin,
                Anchor::InsideMargin,
                Anchor::OutsideMargin,
            ] {
                let from_text = TableFloat { v_rel, ..on_page(y - 106.0 - 17.0) };
                let mut d = doc_with_table(text, 2, float_table(612.0, 50.0, from_text));
                d.settings.compat_mode = mode;
                let l = lay(&d);
                let case = format!("mode {mode} from text ({v_rel:?}) {twips}");
                let table = table_rect(&l, 0, 2).unwrap_or_else(|| panic!("{case}: no table"));
                assert!((table.y - (y - 17.0)).abs() < 0.001, "{case}: the table stays put: {table:?}");
                let expected = if twips < 0 { table.bottom() } else { 72.0 };
                assert!((top(&l, 0, 0) - expected).abs() < 0.001, "{case}: P1 at {}, expected {expected}", top(&l, 0, 0));
                assert!(top(&l, 0, 1) >= table.bottom() - 0.001, "{case}: P2 at {}", top(&l, 0, 1));
            }
        }
    }
}

#[test]
fn a_paragraph_before_a_narrow_floating_table_wraps_beside_it() {
    // Word: a table half the text wide at the left margin; the paragraph before it runs beside it
    // like the one after it. One at the right leaves it where it was.
    let left = TableFloat { h_rel: Anchor::Margin, dist: [7.1, 0.0, 7.1, 0.0], ..on_page(40.0) };
    let l = lay(&doc_with_table("Before\nAfter", 1, float_table(234.0, 100.0, left)));
    let table = table_rect(&l, 0, 1).unwrap();
    for block in [0, 2] {
        let (t, _, x) = lines(&l, 0, block).unwrap();
        assert!(x >= table.right() + 7.1 - 0.01 && t < table.bottom(), "block {block}: at ({x}, {t}), beside {table:?}");
    }
    let right = TableFloat { h_rel: Anchor::Margin, x: 300.0, ..left };
    let l = lay(&doc_with_table("Before\nAfter", 1, float_table(168.0, 100.0, right)));
    let (t, _, x) = lines(&l, 0, 0).unwrap();
    assert!((t - 72.0).abs() < 0.5 && (x - 72.0).abs() < 0.01, "at ({x}, {t})");
}

#[test]
fn margin_and_text_anchored_tables_move_the_paragraph_before_them_too() {
    // Word: 15pt below the top margin, or 15pt above where the text after it starts: the
    // paragraph before goes below the table, which stays where it was placed.
    let margin = TableFloat { v_rel: Anchor::Margin, y: 10.0, ..on_page(0.0) };
    let l = lay(&doc_with_table("Before\nAfter", 1, float_table(612.0, 100.0, margin)));
    let table = table_rect(&l, 0, 1).unwrap();
    assert!((table.y - 82.0).abs() < 0.01 && top(&l, 0, 0) >= table.bottom() - 0.01, "{table:?}, before at {}", top(&l, 0, 0));
    // Where the table's text position is: below the paragraph before, as laid out first.
    let plain = lay(&exact_text("Before\nAfter"));
    let starts = top(&plain, 0, 1);
    let text_anchored = TableFloat { v_rel: Anchor::Paragraph, y: -15.0, ..on_page(0.0) };
    let l = lay(&doc_with_table("Before\nAfter", 1, float_table(612.0, 100.0, text_anchored)));
    let table = table_rect(&l, 0, 1).unwrap();
    assert!((table.y - (starts - 15.0)).abs() < 0.5, "{table:?}, the text after started at {starts}");
    assert!(top(&l, 0, 0) >= table.bottom() - 0.01 && top(&l, 0, 2) > top(&l, 0, 0), "{} {}", top(&l, 0, 0), top(&l, 0, 2));
}

#[test]
fn on_a_later_page_the_paragraph_before_moves_below_the_table_too() {
    // Word: the same on page 2, after a page break before the paragraph (empty or not).
    for text in ["First\nBefore\nAfter", "First\n\nAfter"] {
        let mut d = doc_with_table(text, 2, float_table(612.0, 100.0, on_page(40.0)));
        if let Some(Block::Para(p)) = d.body.get_mut(1).map(Arc::make_mut) {
            p.props.page_break_before = Some(true);
        }
        let l = lay(&d);
        assert_eq!(l.pages.len(), 2, "{text:?}");
        let table = table_rect(&l, 1, 2).unwrap();
        assert!(top(&l, 1, 1) >= table.bottom() - 0.01, "{text:?}: {} above the table's bottom {}", top(&l, 1, 1), table.bottom());
        assert!(top(&l, 1, 3) >= lines(&l, 1, 1).unwrap().1 - 0.01, "{text:?}");
    }
}

#[test]
fn a_table_with_no_vertical_offset_stands_where_the_text_is() {
    // Word: `w:tblpY` 0 (or none) from the page or margin, with no `w:tblpYSpec`, puts the table
    // where the text is, below the paragraph before it; aligned at the top it goes to the top.
    for v_rel in [Anchor::Page, Anchor::Margin] {
        let l = lay(&doc_with_table("Before\nAfter", 1, float_table(612.0, 100.0, TableFloat { v_rel, ..on_page(0.0) })));
        let table = table_rect(&l, 0, 1).unwrap();
        let before = lines(&l, 0, 0).unwrap();
        assert!((before.0 - 72.0).abs() < 0.5 && table.y >= before.1 - 0.01 && table.y < before.1 + 20.0, "{v_rel:?}: {before:?} {table:?}");
        assert!(top(&l, 0, 2) >= table.bottom() - 0.01);
    }
    let top_aligned = TableFloat { v_align: Some(FloatAlign::Start), ..on_page(0.0) };
    let l = lay(&doc_with_table("Before\nAfter", 1, float_table(612.0, 100.0, top_aligned)));
    let table = table_rect(&l, 0, 1).unwrap();
    assert!(table.y.abs() < 0.01 && top(&l, 0, 0) >= table.bottom() - 0.01, "{table:?} {}", top(&l, 0, 0));
}

/// Body paragraphs `texts`, with floating shape `w` × `h` (`float`) in paragraph `anchor`.
fn doc_with_shape(texts: &str, anchor: usize, w: f32, h: f32, float: Float) -> Document {
    let mut d = exact_text(texts);
    d.insert_object(&Pos::body(anchor, 0), rect_shape(w, h, float), &Default::default()).unwrap();
    d
}

#[test]
fn a_paragraph_before_a_floating_shapes_anchor_wraps_around_the_shape() {
    // Word: a shape placed on the page over the paragraph before its anchor moves that paragraph
    // below it (wrapping top and bottom) or beside it (square wrap); one placed from its
    // paragraph, above it and over the paragraph before (Word: 20 pt; here 10 pt, with 17 pt
    // lines), stays there and the paragraph before goes below it.
    let full = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 40.0, ..Default::default() };
    let l = lay(&doc_with_shape("Before\nAnchor\nAfter", 1, 612.0, 100.0, full.clone()));
    let shape = shapes(&l.pages[0].items)[0];
    assert!((shape.y - 40.0).abs() < 0.01, "{shape:?}");
    assert!(top(&l, 0, 0) >= shape.bottom() - 0.01, "{} {shape:?}", top(&l, 0, 0));
    assert!(top(&l, 0, 1) >= lines(&l, 0, 0).unwrap().1 - 0.01);

    let square = Float { wrap: Wrap::Square, h_rel: Anchor::Margin, ..full.clone() };
    let l = lay(&doc_with_shape("Before\nAnchor\nAfter", 1, 200.0, 100.0, square));
    let shape = shapes(&l.pages[0].items)[0];
    let (t, _, x) = lines(&l, 0, 0).unwrap();
    assert!(x >= shape.right() - 0.01 && t < shape.bottom(), "beside: ({x}, {t}), {shape:?}");

    let plain = lay(&exact_text("First\nBefore\nAnchor\nAfter"));
    let anchor_top = top(&plain, 0, 2);
    let from_para = Float { v_rel: Anchor::Paragraph, y: -10.0, ..full };
    let l = lay(&doc_with_shape("First\nBefore\nAnchor\nAfter", 2, 612.0, 100.0, from_para));
    let shape = shapes(&l.pages[0].items)[0];
    assert!((top(&l, 0, 0) - 72.0).abs() < 0.5, "the first paragraph stays: {}", top(&l, 0, 0));
    assert!((shape.y - (anchor_top - 10.0)).abs() < 0.01, "{shape:?}, its paragraph started at {anchor_top}");
    assert!(top(&l, 0, 1) >= shape.bottom() - 0.01, "{} {shape:?}", top(&l, 0, 1));
}

#[test]
fn text_laid_out_again_right_below_a_shape_is_not_under_it() {
    // Swept over offsets and heights that aren't whole points (and lines of 16.4634 pt, the
    // height of the line Word's documents without styles have here): the paragraph before goes
    // below the shape, which stays where its paragraph placed it. Only text before the first
    // block laid out again counts as under the shape: the line laid out again right below it
    // (touching it, give or take rounding), or the first line where line breaking leaves it 0.1
    // pt into the shape, would leave the shape to the layout passes of #2048 and move it down.
    use wordcraft_doc::props::LineSpacing;
    for k in 0..40 {
        let (dy, h) = (-10.0 - 0.173 * k as f32, 100.0 + 0.311 * k as f32);
        let mut d = exact_text("First\nBefore\nAnchor\nAfter");
        for b in &mut d.body {
            if let Block::Para(p) = Arc::make_mut(b) {
                p.props.line_spacing = Some(LineSpacing::Exactly(16.4634));
            }
        }
        let anchor_top = top(&lay(&d), 0, 2);
        let from_para = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Paragraph, y: dy, ..Default::default() };
        d.insert_object(&Pos::body(2, 0), rect_shape(612.0, h, from_para), &Default::default()).unwrap();
        let l = lay(&d);
        let shape = shapes(&l.pages[0].items)[0];
        assert!((shape.y - (anchor_top + dy)).abs() < 0.01, "{dy}, {h}: {shape:?}, its paragraph started at {anchor_top}");
        assert!(top(&l, 0, 1) >= shape.bottom() - 0.01 && in_order(&l, 4, 16.46), "{dy}, {h}: {} {shape:?}", top(&l, 0, 1));
    }
}

#[test]
fn an_empty_paragraph_before_a_letterhead_table_ends_up_below_the_address_block() {
    // A letter: an empty paragraph, a full-width table at the top of the page, then a paragraph
    // anchoring an address block placed on the page below the table. Word moves the empty
    // paragraph below the table and then below the block, so the subject starts a line lower.
    let block = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 127.6, ..Default::default() };
    let letter = |text: &str, at: usize| {
        let mut d = doc_with_table(text, at, float_table(598.4, 99.25, on_page(23.3)));
        d.insert_object(&Pos::body(at + 1, 0), rect_shape(468.0, 198.4, block.clone()), &Default::default()).unwrap();
        d
    };
    let l = lay(&letter("\nSubject\nAfter", 1));
    let shape = shapes(&l.pages[0].items)[0];
    let empty = lines(&l, 0, 0).unwrap();
    assert!(empty.0 >= shape.bottom() - 0.01, "{empty:?} {shape:?}");
    let without = lay(&letter("Subject\nAfter", 0));
    assert!((top(&l, 0, 2) - top(&without, 0, 1) - empty_pitch()).abs() < 0.5, "{} {}", top(&l, 0, 2), top(&without, 0, 1));
}

#[test]
fn text_before_floating_tables_never_overlaps_them_nor_gets_lost() {
    // Swept over where the table stands, how much text comes before it and how wide it is: every
    // paragraph is placed once, and none of the text on the table's page overlaps it when it all
    // still fits there.
    for y in (0..=720).step_by(45) {
        for n in [0usize, 1, 3, 12, 30, 45] {
            for w in [612.0, 200.0] {
                let mut text: Vec<String> =
                    (0..n).map(|i| format!("Paragraph {i} before the table, long enough to run over a line or two when it is narrowed.")).collect();
                text.push("After".into());
                // (Automatic line spacing: a line is as tall as checked against the table.)
                let mut d = pinned(&text.join("\n"));
                d.insert_block(
                    StoryRef::Body,
                    &Path::top(n),
                    Block::Table(float_table(w, 100.0, TableFloat { h_rel: Anchor::Margin, ..on_page(y as f32) })),
                )
                .unwrap();
                let l = lay(&d);
                let firsts = l
                    .pages
                    .iter()
                    .flat_map(|p| &p.items)
                    .filter(|it| matches!(it, Placed::Lines { story: StoryRef::Body, path, l0: 0, .. } if path.0.len() == 1))
                    .count();
                assert_eq!(firsts, n + 1, "y {y}, {n} before, {w} wide");
                let Some((page, table)) = (0..l.pages.len()).find_map(|p| table_rect(&l, p, n as u32).map(|r| (p, r))) else {
                    panic!("y {y}, {n}: no table")
                };
                if l.pages.len() == 1 {
                    assert!(clear_of(&l, page, &table), "y {y}, {n} before, {w} wide: text over {table:?}");
                }
            }
        }
    }
}

#[test]
fn two_floating_tables_on_a_page_keep_the_text_before_both_clear() {
    let text: Vec<String> = (0..12).map(|i| format!("Paragraph {i}")).collect();
    let mut d = doc_with_table(&text.join("\n"), 6, float_table(612.0, 60.0, on_page(150.0)));
    d.insert_block(StoryRef::Body, &Path::top(3), Block::Table(float_table(612.0, 60.0, on_page(60.0)))).unwrap();
    let l = lay(&d);
    let (first, second) = (table_rect(&l, 0, 3).unwrap(), table_rect(&l, 0, 7).unwrap());
    assert!((first.y - 60.0).abs() < 0.01 && (second.y - 150.0).abs() < 0.01, "{first:?} {second:?}");
    assert!(clear_of(&l, 0, &first) && clear_of(&l, 0, &second));
    let tops: Vec<f32> = (0..14).filter(|b| ![3, 7].contains(b)).map(|b| top(&l, 0, b)).collect();
    assert!(tops.windows(2).all(|w| w[1] > w[0]), "in order: {tops:?}");
}

#[test]
fn hostile_floating_table_positions_lay_out() {
    for y in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e30, 1e30, -50.0] {
        let l = lay(&doc_with_table("Before\n\nAfter", 2, float_table(612.0, 100.0, on_page(y))));
        let firsts = l
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter(|it| matches!(it, Placed::Lines { story: StoryRef::Body, path, l0: 0, .. } if path.0.len() == 1))
            .count();
        assert_eq!(firsts, 3, "y {y}");
    }
}

#[test]
fn a_paragraph_whose_shape_would_push_the_text_before_it_or_itself_off_the_page_starts_the_next_page() {
    // Word (A4, the same geometry 1.8 pt further down): paragraphs of 17 pt lines from 72, then
    // one anchoring a shape across the page from 80 to 280. With 24 paragraphs before it they all
    // go below the shape and the anchor paragraph ends at 705. With 25 the text before still fits
    // below the shape (to 705) but the anchor paragraph doesn't (722); with 26 the text before
    // doesn't fit either. Then the text before stays where it was and the anchor paragraph starts
    // the next page with its shape, below it.
    let full = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 80.0, ..Default::default() };
    for n in [24usize, 25, 26] {
        let mut text: Vec<String> = (0..n).map(|i| format!("P{i}")).collect();
        text.extend(["Anchor".to_string(), "After".to_string()]);
        let l = lay(&doc_with_shape(&text.join("\n"), n, 612.0, 200.0, full.clone()));
        let anchor = n as u32;
        if n == 24 {
            let shape = shapes(&l.pages[0].items)[0];
            assert!((top(&l, 0, 0) - 280.0).abs() < 0.01 && (top(&l, 0, anchor) - 688.0).abs() < 0.01, "{n}: {shape:?}");
            continue;
        }
        for b in 0..anchor {
            let want = 72.0 + 17.0 * b as f32;
            assert!((top(&l, 0, b) - want).abs() < 0.01, "{n}: paragraph {b} at {}, not {want}", top(&l, 0, b));
        }
        assert!(shapes(&l.pages[0].items).is_empty(), "{n}: no shape left on page 1");
        let shape = shapes(&l.pages[1].items)[0];
        assert!((shape.y - 80.0).abs() < 0.01, "{n}: {shape:?}");
        assert!(lines(&l, 0, anchor).is_none(), "{n}: the anchor paragraph left page 1");
        assert!((top(&l, 1, anchor) - 280.0).abs() < 0.01, "{n}: anchor at {} on page 2, below its shape", top(&l, 1, anchor));
    }
}

#[test]
fn pages_aligned_in_the_middle_keep_their_text_in_order_around_a_float() {
    // Vertical alignment moves a page's text when the page is left; laying text out again could
    // not move it back, so such pages are not laid out again block by block: the text stays in
    // order, at least 17 pt apart.
    for valign in [wordcraft_doc::section::PageVAlign::Center, wordcraft_doc::section::PageVAlign::Bottom] {
        let mut text: Vec<String> = (0..26).map(|i| format!("P{i}")).collect();
        text.push("Anchor".to_string());
        let full = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 100.0, ..Default::default() };
        let mut d = doc_with_shape(&text.join("\n"), 26, 612.0, 300.0, full);
        d.last_section.valign = valign;
        let l = lay(&d);
        let at: Vec<(usize, f32)> = (0..26).map(|b| page_top(&l, b)).collect();
        assert!(in_order(&l, 26, 17.0), "{valign:?}: {at:?}");
    }
}

#[test]
fn text_laid_out_again_keeps_its_footnote_once_and_its_paragraphs_kept_together() {
    // A paragraph with a footnote and a heading kept with the next paragraph, both under a table
    // placed on the page later: laid out again below the table, the footnote is on the page once,
    // and the heading still starts right above its paragraph.
    let mut d = exact_text("Noted\nHeading\nBody\nAfter");
    let id = {
        let id = d.add_part(PartKind::Footnote, vec![wordcraft_doc::para_block(Paragraph::with_text("The note.", Default::default()))]);
        d.insert_object(
            &Pos::body(0, 5),
            InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id, custom: String::new() },
            &Default::default(),
        )
        .unwrap();
        id
    };
    if let Some(Block::Para(p)) = d.body.get_mut(1).map(Arc::make_mut) {
        p.props.keep_next = Some(true);
    }
    d.insert_block(StoryRef::Body, &Path::top(3), Block::Table(float_table(612.0, 100.0, on_page(40.0)))).unwrap();
    let l = lay(&d);
    let table = table_rect(&l, 0, 3).unwrap();
    assert!(clear_of(&l, 0, &table));
    assert!((top(&l, 0, 0) - table.bottom()).abs() < 0.01, "{}", top(&l, 0, 0));
    assert!((top(&l, 0, 2) - lines(&l, 0, 1).unwrap().1).abs() < 0.01, "heading and body apart");
    let notes = l.pages.iter().flat_map(|p| &p.items).filter(|it| matches!(it, Placed::Lines { story: StoryRef::Part(n), .. } if *n == id)).count();
    assert_eq!(notes, 1, "the footnote is placed once");
}

#[test]
fn text_of_an_earlier_section_on_the_page_goes_below_a_later_shape_too() {
    // Word (A4, top margin 28.35 pt): `ZZZ`, a continuous section break, `YYY`, then a paragraph
    // anchoring a shape across the page at 20–120 pt: ZZZ goes below the shape (122.7) and YYY
    // after it (147.7), as without the section break. With `XXX` before ZZZ in the first section
    // and the shape at 45–145 pt, ZZZ goes below the shape too. The walk restarts at a section
    // break, so the earlier section's text is left to the pass that lays the page out again with
    // the shape known from its start (#2048): here the shape at 80–180 pt is over the earlier
    // section's first line (72–89) or second (89–106).
    for (first, y) in [(None, 80.0), (Some("First"), 95.0)] {
        let text = first.map_or(String::new(), |f| format!("{f}\n")) + "Earlier section\nBefore\nAnchor\nAfter";
        let mut d = exact_text(&text);
        let earlier = u32::from(first.is_some());
        if let Some(Block::Para(p)) = d.body.get_mut(earlier as usize).map(Arc::make_mut) {
            p.section = Some(Box::default());
        }
        d.last_section.start = wordcraft_doc::section::SectionStart::Continuous;
        let full = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y, ..Default::default() };
        d.insert_object(&Pos::body(earlier as usize + 2, 0), rect_shape(612.0, 100.0, full), &Default::default()).unwrap();
        let l = lay(&d);
        let shape = shapes(&l.pages[0].items)[0];
        assert!((shape.y - y).abs() < 0.01, "{first:?}: {shape:?}");
        assert!(clear_of(&l, 0, &shape), "{first:?}: text under the shape {shape:?}");
        if first.is_some() {
            assert!((top(&l, 0, 0) - 72.0).abs() < 0.01, "the first line stays above the shape: {}", top(&l, 0, 0));
        }
        assert!((top(&l, 0, earlier) - shape.bottom()).abs() < 0.01, "{first:?}: earlier section at {}, {shape:?}", top(&l, 0, earlier));
        assert!(in_order(&l, earlier + 4, 17.0), "{first:?}");
    }
}

#[test]
fn an_anchor_paragraph_that_runs_over_the_page_keeps_its_shape_and_first_lines_there() {
    // Word (A4, the same geometry 1.8 pt further down): a four-line anchor paragraph (line breaks)
    // after 23 paragraphs of 17 pt lines, its shape across the page from 80 to 280: the text
    // before goes below the shape, the anchor's first two lines end page 1 (671, 688) and the
    // other two start page 2, the shape staying on page 1. After 24 paragraphs one line stays
    // on page 1; with widow control that line can't stay alone, so the text before stays where
    // it was and the anchor paragraph starts page 2 with its shape (Word: its first three lines
    // above the shape there, which starts 80 pt down; here no line fits above it).
    let full = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 80.0, ..Default::default() };
    for (n, widow, on_first) in [(23usize, false, 2usize), (24, false, 1), (24, true, 0)] {
        let mut text: Vec<String> = (0..n).map(|i| format!("P{i}")).collect();
        text.extend(["Anchor".to_string(), "After".to_string()]);
        let mut d = exact_text(&text.join("\n"));
        if let Some(Block::Para(p)) = d.body.get_mut(n).map(Arc::make_mut) {
            let q = Paragraph::with_text("A1\nA2\nA3\nA4", Default::default());
            (p.text, p.runs) = (q.text, q.runs);
        }
        d.insert_object(&Pos::body(n, 0), rect_shape(612.0, 200.0, full.clone()), &Default::default()).unwrap();
        for b in &mut d.body {
            if let Block::Para(p) = Arc::make_mut(b) {
                p.props.widow_control = Some(widow);
            }
        }
        let l = lay(&d);
        let anchor = n as u32;
        let tops = |page: usize| -> Vec<f32> {
            l.pages[page]
                .items
                .iter()
                .filter_map(|it| match it {
                    Placed::Lines { story: StoryRef::Body, path, para, l0, l1, y, .. } if path.0 == [anchor] => {
                        Some(para.lines.get(*l0..*l1).unwrap_or(&[]).iter().map(|ln| y + ln.top - para.lines[*l0].top).collect::<Vec<_>>())
                    }
                    _ => None,
                })
                .flatten()
                .collect()
        };
        let (first, second) = (tops(0), tops(1));
        assert_eq!((first.len(), second.len()), (on_first, 4 - on_first), "{n} {widow}: {first:?} {second:?}");
        if on_first > 0 {
            let want: Vec<f32> = (0..on_first).map(|k| 280.0 + 17.0 * (n + k) as f32).collect();
            assert!(first.iter().zip(&want).all(|(a, b)| (a - b).abs() < 0.01), "{n} {widow}: {first:?}, not {want:?}");
            assert!((second[0] - 72.0).abs() < 0.01 && (top(&l, 0, 0) - 280.0).abs() < 0.01, "{n} {widow}: {second:?}");
            assert_eq!((shapes(&l.pages[0].items).len(), shapes(&l.pages[1].items).len()), (1, 0), "{n} {widow}: the shape stays on page 1");
        } else {
            assert!((0..anchor).all(|b| (top(&l, 0, b) - (72.0 + 17.0 * b as f32)).abs() < 0.01), "{n} {widow}: the text before stays");
            assert_eq!((shapes(&l.pages[0].items).len(), shapes(&l.pages[1].items).len()), (0, 1), "{n} {widow}: the shape goes with it");
            assert!((second[0] - 280.0).abs() < 0.01, "{n} {widow}: below its shape: {second:?}");
        }
    }
}

#[test]
fn a_page_whose_first_section_is_centred_keeps_the_text_before_a_float_in_order() {
    // A centred section and a continuous top-aligned one on the same page: the page is aligned
    // as its first section says when it is left, so the second section's text before a float
    // is not laid out again there block by block either (in order, at least 17 pt apart).
    let mut text: Vec<String> = vec!["Centred".to_string()];
    text.extend((0..26).map(|i| format!("P{i}")));
    text.push("Anchor".to_string());
    let full = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 100.0, ..Default::default() };
    let mut d = doc_with_shape(&text.join("\n"), 27, 612.0, 300.0, full);
    if let Some(Block::Para(p)) = d.body.get_mut(0).map(Arc::make_mut) {
        p.section = Some(Box::new(wordcraft_doc::section::SectionProps { valign: wordcraft_doc::section::PageVAlign::Center, ..Default::default() }));
    }
    d.last_section.start = wordcraft_doc::section::SectionStart::Continuous;
    let l = lay(&d);
    let at: Vec<(usize, f32)> = (0..27).map(|b| page_top(&l, b)).collect();
    assert!(in_order(&l, 27, 17.0), "{at:?}");
}

#[test]
fn text_before_a_float_is_laid_out_again_only_up_to_512_blocks() {
    // 600 paragraphs of exact 1 pt lines, the last anchoring a shape across the page at 80–100:
    // laying out again from the first paragraph under the shape would take 592 blocks, past the
    // bound, so it is not done (the text still goes below the shape, by a layout pass started
    // again with the shape known from the page's start, #2048); with 400 paragraphs it is.
    use wordcraft_doc::props::LineSpacing;
    let full = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 80.0, ..Default::default() };
    for (n, replayed) in [(400usize, 392usize), (600, 0)] {
        let text = vec![""; n].join("\n");
        let mut d = doc_with_shape(&format!("{text}\nAnchor"), n, 612.0, 20.0, full.clone());
        for b in &mut d.body {
            if let Block::Para(p) = Arc::make_mut(b) {
                p.props.line_spacing = Some(LineSpacing::Exactly(1.0));
            }
        }
        crate::float_before::MOST.with(|c| c.set((0, 0)));
        let l = lay(&d);
        assert_eq!(crate::float_before::MOST.with(|c| c.get().1), replayed, "{n}: blocks laid out again before the shape");
        assert_eq!(l.pages.len(), 1, "{n}");
        let t = top(&l, 0, 8);
        assert!(t >= 100.0 - 0.01, "{n}: paragraph 8 at {t}");
    }
}

#[test]
fn text_before_floats_is_laid_out_again_for_at_most_16_floats_on_a_page_across_sections() {
    // 17 continuous sections on one page, each a paragraph and one anchoring a shape placed from
    // it over the paragraph before: the budget of 16 is the page's, not each section's, so the
    // text before the first 16 shapes is laid out again below them, the 17th's is not (it goes
    // below it by a layout pass started again with the shape known from the page's start, #2048).
    use wordcraft_doc::section::{SectionProps, SectionStart};
    let text: Vec<String> = (0..17).flat_map(|k| [format!("B{k}"), format!("A{k}")]).collect();
    let mut d = exact_text(&text.join("\n"));
    let over = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Paragraph, y: -17.0, ..Default::default() };
    for k in 0..17usize {
        let anchor = 2 * k + 1;
        d.insert_object(&Pos::body(anchor, 0), rect_shape(612.0, 2.0, over.clone()), &Default::default()).unwrap();
        if k < 16
            && let Some(Block::Para(p)) = d.body.get_mut(anchor).map(Arc::make_mut)
        {
            p.section = Some(Box::new(SectionProps { start: SectionStart::Continuous, ..Default::default() }));
        }
    }
    d.last_section.start = SectionStart::Continuous;
    crate::float_before::MOST.with(|c| c.set((0, 0)));
    let l = lay(&d);
    assert_eq!(crate::float_before::MOST.with(|c| c.get().0), 16, "floats the text before was laid out again for");
    assert_eq!(l.pages.len(), 1);
    let mut shapes = shapes(&l.pages[0].items);
    shapes.sort_by(|a, b| a.y.total_cmp(&b.y));
    assert_eq!(shapes.len(), 17);
    for (k, shape) in shapes.iter().enumerate() {
        let before = top(&l, 0, 2 * k as u32);
        assert!(before >= shape.bottom() - 0.01, "section {k}: paragraph before at {before}, shape {shape:?}");
    }
}

/// A floating table whose text before, laid out again around it, runs on into a narrower column
/// of the same page: there the table (as wide as its column) would be laid out too tall for the
/// page and run in the text. It stays where it was placed, as wide as the first column, where it
/// stands with text before it that fits that column, and the text before is left as it was (37
/// lines of 17 pt from the first column's top), as when that text no longer fits the page.
#[test]
fn a_floating_table_stays_put_when_its_text_before_runs_into_a_narrower_column() {
    let mut t = Table::new(1, 1, 400.0);
    t.props.width = None;
    t.props.width_pct = Some(100.0);
    t.rows[0].cells[0].props.width = None;
    let words = (0..200).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(Paragraph::with_text(&words, Default::default()))];
    t.props.float = Some(TableFloat { h_rel: Anchor::Margin, ..on_page(150.0) });
    let doc = |n: usize| {
        let text: Vec<String> = (0..n).map(|i| format!("P{i}")).collect();
        let mut d = exact_text(&text.join("\n"));
        d.last_section.columns =
            wordcraft_doc::section::Columns { count: 2, space: 18.0, separator: false, widths: vec![(400.0, 18.0), (50.0, 0.0)] };
        d.insert_block(StoryRef::Body, &Path::top(n), Block::Table(t.clone())).unwrap();
        lay(&d)
    };
    let fits = table_rect(&doc(10), 0, 10).unwrap();
    assert!((fits.y - 150.0).abs() < 0.01 && fits.w > 399.0 && fits.w < 401.0, "{fits:?}");
    let n = 37usize;
    let l = doc(n);
    assert_eq!(l.pages.len(), 1);
    let table = table_rect(&l, 0, n as u32).unwrap();
    assert!(
        (table.x - fits.x).abs() < 0.01 && (table.y - 150.0).abs() < 0.01 && (table.w - fits.w).abs() < 0.01,
        "{table:?}, with less text before {fits:?}"
    );
    let at: Vec<(f32, f32)> = (0..n as u32).map(|b| lines(&l, 0, b).map_or((f32::NAN, f32::NAN), |(t, _, x)| (t, x))).collect();
    assert!(at.iter().enumerate().all(|(i, (t, x))| (t - (72.0 + 17.0 * i as f32)).abs() < 0.01 && (x - 72.0).abs() < 0.01), "{at:?}");
}

/// A floating table whose text before, laid out again around it, runs on into a narrower column
/// of the same page, where the table (as wide as its column) comes out taller but still fits the
/// page: placed at the rectangle it had, its rows would be the narrower, taller ones and the text
/// before would wrap around the smaller area it left. It stays where and as it was placed, and the
/// text before is left as it was, as when the table is too tall for the page there.
#[test]
fn a_floating_table_that_comes_out_taller_in_a_narrower_column_stays_as_it_was_placed() {
    let mut t = Table::new(1, 1, 300.0);
    t.props.width = None;
    t.props.width_pct = Some(100.0);
    t.rows[0].cells[0].props.width = None;
    let words = (0..40).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
    t.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(Paragraph::with_text(&words, Default::default()))];
    t.props.float = Some(TableFloat { h_rel: Anchor::Margin, ..on_page(150.0) });
    let doc = |n: usize, widths: Vec<(f32, f32)>| {
        let text: Vec<String> = (0..n).map(|i| format!("P{i}")).collect();
        let mut d = exact_text(&text.join("\n"));
        d.last_section.columns = wordcraft_doc::section::Columns { count: 2, space: 18.0, separator: false, widths };
        d.insert_block(StoryRef::Body, &Path::top(n), Block::Table(t.clone())).unwrap();
        lay(&d)
    };
    let columns = vec![(300.0, 18.0), (150.0, 0.0)];
    let fits = table_rect(&doc(10, columns.clone()), 0, 10).unwrap();
    assert!((fits.y - 150.0).abs() < 0.01 && fits.w > 299.0 && fits.w < 301.0, "{fits:?}");
    let narrow = table_rect(&doc(10, vec![(150.0, 18.0), (300.0, 0.0)]), 0, 10).unwrap();
    assert!(narrow.h > fits.h + 17.0 && narrow.bottom() < 720.0, "taller in the narrower column, and on the page: {narrow:?}, {fits:?}");
    let n = 37usize;
    let l = doc(n, columns);
    assert_eq!(l.pages.len(), 1);
    let table = table_rect(&l, 0, n as u32).unwrap();
    assert!(
        (table.x - fits.x).abs() < 0.01 && (table.y - fits.y).abs() < 0.01 && (table.w - fits.w).abs() < 0.01 && (table.h - fits.h).abs() < 0.01,
        "{table:?}, with less text before {fits:?}"
    );
    let at: Vec<(f32, f32)> = (0..n as u32).map(|b| lines(&l, 0, b).map_or((f32::NAN, f32::NAN), |(t, _, x)| (t, x))).collect();
    assert!(at.iter().enumerate().all(|(i, (t, x))| (t - (72.0 + 17.0 * i as f32)).abs() < 0.01 && (x - 72.0).abs() < 0.01), "{at:?}");
}

/// Word (Word 2007 layout, Calibri 11 pt on a 24 pt line grid, `lines` or `linesAndChars`): a line
/// that snaps to the grid keeps its whole snapped line clear of a full-width floating table or
/// square-wrapped shape, before or after the float's anchor: its third line, 120 to 144 pt with
/// single spacing (or to 147.6, 27.6 pt, with 1.15), moves below a shape whose top lands 23.75 pt
/// into it (27.5 with 1.15) and stays for one at its bottom (here 0.05 pt below it). Here in [`TEST_FONT`], whose own line
/// (15.9 pt, 18.3 with 1.15) is shorter than the snapped one, swept through the snapped line of a
/// layout without the float.
#[test]
fn a_line_on_a_line_grid_keeps_its_snapped_height_clear_of_a_float() {
    use wordcraft_doc::props::LineSpacing;
    use wordcraft_doc::section::{DocGrid, DocGridType};
    let text_8 = (1..=8).map(|i| format!("P{i} word")).collect::<Vec<_>>().join("\n");
    let square = |y: f32| Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, y, ..Default::default() };
    let on_grid = |mut d: Document, m: f32| {
        d.settings.compat_mode = wordcraft_doc::LEGACY_COMPAT_MODE;
        d.last_section.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: 24.0, char_space: 0 });
        for b in &mut d.body {
            if let Block::Para(p) = Arc::make_mut(b) {
                p.props.line_spacing = Some(LineSpacing::Multiple(m));
            }
        }
        d
    };
    for m in [1.0, 1.15] {
        let plain = lay(&on_grid(exact_text(&text_8), m));
        let (top, bottom, _) = lines(&plain, 0, 2).unwrap();
        assert!((bottom - top - 24.0 * m).abs() < 0.01 && (top - (72.0 + 48.0 * m)).abs() < 0.01, "{m}: the third line {top}–{bottom}");
        let steps = ((bottom - top) as usize).saturating_sub(1);
        for float_first in [false, true] {
            for table in [true, false] {
                for y in (0..=steps).map(|k| top + 0.5 + k as f32).chain([bottom + 0.05]) {
                    if table && y >= bottom {
                        continue;
                    }
                    let d = match (table, float_first) {
                        (true, first) => doc_with_table(&text_8, if first { 0 } else { 5 }, float_table(612.0, 50.0, on_page(y))),
                        (false, first) => doc_with_shape(&text_8, if first { 0 } else { 5 }, 612.0, 50.0, square(y)),
                    };
                    let l = lay(&on_grid(d, m));
                    let float = if table { table_rect(&l, 0, if float_first { 0 } else { 5 }) } else { shapes(&l.pages[0].items).first().copied() };
                    let case = format!("{m} lines, float first {float_first}, table {table}, y {y}");
                    let float = float.unwrap_or_else(|| panic!("{case}: no float"));
                    let third = lines(&l, 0, if table && float_first { 3 } else { 2 }).unwrap();
                    if y < bottom {
                        assert!(third.0 >= float.bottom() - 0.01, "{case}: the third line at {third:?}, the float ends at {}", float.bottom());
                    } else {
                        assert!((third.0 - top).abs() < 0.01, "{case}: the third line stays at {third:?}");
                    }
                }
            }
        }
    }
}

/// A paragraph whose shape stands over the paragraph before it, and that runs on across a column
/// break into a column of another width (its text after the break laid out again for that width),
/// still has the text before it wrap around the shape: the shape stays with the paragraph's first
/// piece, and so does the record of where it was placed. Equal columns, where nothing is laid out
/// again at the break, give the same.
#[test]
fn text_before_a_shape_wraps_around_it_when_its_paragraph_breaks_into_a_column_of_another_width() {
    use wordcraft_doc::section::Columns;
    let brk = wordcraft_doc::para::COLUMN_BREAK;
    let float = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, y: -10.0, ..Default::default() };
    let laid = |widths: Vec<(f32, f32)>| {
        let mut d = doc_with_shape(&format!("First\nBefore\nAnchor{brk}After the break\nAfter"), 2, 150.0, 100.0, float.clone());
        d.last_section.columns = Columns { count: 2, space: 36.0, separator: false, widths };
        lay(&d)
    };
    for (widths, second) in [(vec![(216.0, 36.0), (216.0, 0.0)], 72.0 + 216.0 + 36.0), (vec![(260.0, 36.0), (172.0, 0.0)], 72.0 + 260.0 + 36.0)] {
        let l = laid(widths.clone());
        let drawn: Vec<Rect> = l.pages.iter().flat_map(|pg| shapes(&pg.items)).collect();
        let [shape] = drawn[..] else { panic!("{widths:?}: the shape is placed once: {drawn:?}") };
        assert_eq!(l.pages.len(), 1, "{widths:?}");
        assert!((top(&l, 0, 0) - 72.0).abs() < 0.5, "{widths:?}: the first paragraph stays: {}", top(&l, 0, 0));
        let before = top(&l, 0, 1);
        assert!(
            before >= shape.bottom() - 0.01 && before < shape.bottom() + 17.0,
            "{widths:?}: the paragraph before goes right below the shape: {before} {shape:?}"
        );
        let anchor = l.pages[0].items.iter().find_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, l0: 0, y, .. } if path.0 == [2] => Some(*y),
            _ => None,
        });
        assert!(
            anchor.is_some_and(|y| (y - (before + 17.0)).abs() < 0.01),
            "{widths:?}: the shape's paragraph follows the paragraph before: {anchor:?}, {before}"
        );
        assert!(clear_of(&l, 0, &shape), "{widths:?}: no text under the shape {shape:?}");
        let after_break = l.pages[0].items.iter().any(|it| {
            matches!(it, Placed::Lines { story: StoryRef::Body, path, l0, x, para, .. } if path.0 == [2] && *l0 > 0 && (x + para.lines[*l0].left - second).abs() < 0.5)
        });
        assert!(after_break, "{widths:?}: the text after the break is in the second column");
    }
}

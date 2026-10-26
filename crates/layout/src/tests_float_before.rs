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

/// Every body line's text on page `page` (its own ascent and descent about the baseline, without
/// the extra of multiple line spacing) clear of `r`.
fn text_clear_of(l: &DocLayout, page: usize, r: &Rect) -> bool {
    l.pages[page].items.iter().all(|it| match it {
        Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, .. } if path.0.len() == 1 => {
            let shown = para.lines.get(*l0..*l1).unwrap_or(&[]);
            let first = shown.first().map_or(0.0, |l| l.top);
            shown.iter().all(|line| {
                let top = y + line.baseline - line.text_asc - first;
                !Rect::new(x + line.left, top + 0.01, line.right - line.left, line.text_asc + line.text_desc - 0.02).intersects(r)
            })
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

/// Swept over where the table stands, how much text comes before it and how wide it is: every
/// paragraph is placed once, and none of the text on the table's page overlaps it when it all
/// still fits there. The lines have automatic 1.15 spacing: in Word 2013 layout a line keeps its
/// text clear of the table, its extra spacing may run over it ([`text_clear_of`]).
#[test]
fn text_before_floating_tables_never_overlaps_them_nor_gets_lost() {
    for y in (0..=720).step_by(45) {
        for n in [0usize, 1, 3, 12, 30, 45] {
            for w in [612.0, 200.0] {
                let mut text: Vec<String> =
                    (0..n).map(|i| format!("Paragraph {i} before the table, long enough to run over a line or two when it is narrowed.")).collect();
                text.push("After".into());
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
                    assert!(text_clear_of(&l, page, &table), "y {y}, {n} before, {w} wide: text over {table:?}");
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

// A paragraph's last line keeps its space after clear of floating tables and objects before Word
// 2013 layout (Word for Mac, exact 17 pt lines, documents without settings, so compatibility mode
// 12: one whose line bottom plus space after reaches a float's top goes below it, or beside a
// narrow one; the line alone decides for the other lines). In Word 2013 layout only the line does.

/// `doc` laid out in compatibility mode 12 (Word 2007), like the documents measured in Word.
fn lay_12(doc: &Document) -> DocLayout {
    let mut d = doc.clone();
    d.settings.compat_mode = wordcraft_doc::LEGACY_COMPAT_MODE;
    lay(&d)
}

/// A paragraph of `text` with exact 17 pt lines, no space before and `after` after.
fn spaced(text: &str, after: f32) -> Paragraph {
    use wordcraft_doc::props::LineSpacing;
    let mut p = Paragraph::with_text(text, Default::default());
    p.props.line_spacing = Some(LineSpacing::Exactly(17.0));
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(after);
    p
}

/// Body paragraphs `paras` with floating table `t` inserted before paragraph `at`.
fn spaced_doc(paras: Vec<Paragraph>, at: usize, t: Table) -> Document {
    let mut d = pinned("");
    d.body = paras.into_iter().map(wordcraft_doc::para_block).collect();
    d.insert_block(StoryRef::Body, &Path::top(at), Block::Table(t)).unwrap();
    d
}

#[test]
fn a_last_line_whose_space_after_reaches_a_floating_table_goes_below_it() {
    // Word: line 72–89, 8 pt after; a table at 97.5 leaves it (89 + 8 = 97 fits), one at 96.5
    // takes it below; with no space after, one at 89.5 leaves it too. Swept around the boundary.
    for (after, y, stays) in [(8.0, 97.5, true), (8.0, 96.5, false), (8.0, 90.0, false), (0.0, 89.5, true), (4.0, 93.5, true), (4.0, 92.5, false)] {
        let l = lay_12(&spaced_doc(vec![spaced("Before", after), spaced("After", 0.0)], 1, float_table(612.0, 100.0, on_page(y))));
        let table = table_rect(&l, 0, 1).unwrap();
        let t = top(&l, 0, 0);
        if stays {
            assert!((t - 72.0).abs() < 0.01, "after {after}, table at {y}: moved to {t}");
        } else {
            assert!(t >= table.bottom() - 0.01, "after {after}, table at {y}: at {t}, table ends {}", table.bottom());
        }
    }
}

/// Word for Mac (exact 12 pt lines, 8 pt after the paragraph, a full-width table placed on the page
/// after it in the document, at the line's bottom plus its space after or a twip either side):
/// before Word 2013 layout a table touching the space after moves the line below it, as a twip
/// higher does, and a twip lower leaves it; in Word 2013 layout, where only the line counts, it
/// stays. The same with the table first in the document (#3977); a shape touching the space after
/// leaves the line in place.
#[test]
fn a_floating_table_touching_the_space_after_of_a_line_before_it_moves_that_line_below_it() {
    let paras = || vec![spaced("P1", 0.0), spaced("P2", 8.0), spaced("P3", 0.0), spaced("P4", 0.0)];
    let square = |y: f32| Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, y, ..Default::default() };
    for mode in [11, 12, 14, 15] {
        for twips in [-1, 0, 1] {
            let y = 114.0 + twips as f32 / 20.0;
            for case in ["table first", "table after", "shape"] {
                let mut d = match case {
                    "table first" => spaced_doc(paras(), 0, float_table(612.0, 50.0, on_page(y))),
                    "table after" => spaced_doc(paras(), 2, float_table(612.0, 50.0, on_page(y))),
                    _ => {
                        let mut d = pinned("");
                        d.body = paras().into_iter().map(wordcraft_doc::para_block).collect();
                        d.insert_object(&Pos::body(3, 0), rect_shape(612.0, 50.0, square(y)), &Default::default()).unwrap();
                        d
                    }
                };
                d.settings.compat_mode = mode;
                let l = lay(&d);
                let float = match case {
                    "table first" => table_rect(&l, 0, 0),
                    "table after" => table_rect(&l, 0, 2),
                    _ => shapes(&l.pages[0].items).first().copied(),
                };
                let float = float.unwrap_or_else(|| panic!("mode {mode} {case} {twips}: no float"));
                assert!((float.y - y).abs() < 0.001, "mode {mode} {case} {twips}: the float stays put: {float:?}");
                let p2 = if case == "table first" { 2 } else { 1 };
                let moves = mode < 15 && (twips < 0 || (twips == 0 && case != "shape"));
                let expected = if moves { float.bottom() } else { 89.0 };
                assert!((top(&l, 0, p2) - expected).abs() < 0.001, "mode {mode} {case} {twips}: P2 at {}, expected {expected}", top(&l, 0, p2));
            }
        }
    }
}

/// An exact 12 pt line from 72 pt with 8 pt after, its paragraph mark far taller (62.5 pt) or
/// smaller (4 pt) than the line, below a full-width floating table placed first in the document:
/// the table is checked against the line's 12 pt (#3977), so before Word 2013 layout a table
/// reaching the line's space after (from 92 pt down, contact included) moves the line below it
/// whatever the mark's height; one a twip lower, or no space after, leaves it.
#[test]
fn a_table_reaching_the_space_after_of_an_exact_line_moves_it_whatever_its_marks_height() {
    use wordcraft_doc::props::LineSpacing;
    for mode in [11, 12, 14, 15] {
        for mark in [4.0, 62.5] {
            for (after, y) in [(8.0, 88.0), (8.0, 92.0), (8.0, 92.05), (0.0, 88.0), (0.0, 84.05)] {
                let ch = CharProps { font: Some(TEST_FONT.into()), size: Some(11.0), ..Default::default() };
                let mut p = Paragraph::with_text("P1", ch.clone());
                p.mark = CharProps { size: Some(mark), ..ch };
                p.props.line_spacing = Some(LineSpacing::Exactly(12.0));
                p.props.space_before = Some(0.0);
                p.props.space_after = Some(after);
                let mut d = spaced_doc(vec![p, spaced("After", 0.0)], 0, float_table(612.0, 50.0, on_page(y)));
                d.settings.compat_mode = mode;
                let l = lay(&d);
                let table = table_rect(&l, 0, 0).unwrap();
                let case = format!("mode {mode}, mark {mark}, {after} after, table at {y}");
                assert!((table.y - y).abs() < 0.001, "{case}: the table stays put: {table:?}");
                let moves = mode < 15 && after > 0.0 && y <= 92.0;
                let expected = if moves { table.bottom() } else { 72.0 };
                assert!((top(&l, 0, 1) - expected).abs() < 0.001, "{case}: the line at {}, expected {expected}", top(&l, 0, 1));
            }
        }
    }
}

#[test]
fn only_the_last_line_of_a_paragraph_keeps_its_space_after_clear() {
    // Word: a two-line paragraph (72–89–106, 8 pt after) and a table at 110: the first line stays,
    // the last goes below the table; with no space after both stay.
    // (Two lines by a line break, whatever the font.)
    for (after, moves) in [(8.0, true), (0.0, false)] {
        let l =
            lay_12(&spaced_doc(vec![spaced("First line\nSecond line", after), spaced("After", 0.0)], 1, float_table(612.0, 100.0, on_page(110.0))));
        let lines = l.pages[0].items.iter().find_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, para, .. } if path.0 == [0] => Some(para.lines.len()),
            _ => None,
        });
        assert_eq!(lines, Some(2), "two lines");
        let (first, last) = (lines_at(&l, 0, 0, 0), lines_at(&l, 0, 0, 1));
        assert!((first - 72.0).abs() < 0.01, "after {after}: first line at {first}");
        if moves {
            assert!(last >= 210.0 - 0.01, "after {after}: last line at {last}");
        } else {
            assert!((last - 89.0).abs() < 0.01, "after {after}: last line at {last}");
        }
    }
}

/// The top of line `k` of top-level body block `block` on page `page`.
fn lines_at(l: &DocLayout, page: usize, block: u32, k: usize) -> f32 {
    l.pages[page]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, para, l0, l1, y, .. } if path.0 == [block] && (*l0..*l1).contains(&k) => {
                Some(y + para.lines.get(k)?.top - para.lines.get(*l0)?.top)
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn text_after_a_floating_table_keeps_its_space_after_clear_of_the_next_one() {
    // Word: a table across the top (40–140), then a paragraph (140–157) with 4 or 6 pt after, then
    // a table at 162: 4 pt fits above it, 6 pt doesn't.
    for (after, stays) in [(4.0, true), (6.0, false)] {
        let mut d = spaced_doc(vec![spaced("After the first", after), spaced("Last", 0.0)], 1, float_table(612.0, 60.0, on_page(162.0)));
        d.insert_block(StoryRef::Body, &Path::top(0), Block::Table(float_table(612.0, 100.0, on_page(40.0)))).unwrap();
        let l = lay_12(&d);
        let (t, second) = (top(&l, 0, 1), table_rect(&l, 0, 2).unwrap());
        if stays {
            assert!((t - 140.0).abs() < 0.01, "after {after}: at {t}");
        } else {
            assert!(t >= second.bottom() - 0.01, "after {after}: at {t}, second table ends {}", second.bottom());
        }
    }
}

#[test]
fn contextual_and_automatic_space_after_near_a_floating_table() {
    // Word: contextual spacing drops the space after before a paragraph of the same style, so the
    // line stays above a table at 92; automatic spacing after is 14 pt (89 + 14 = 103).
    let mut ctx = spaced("Before", 8.0);
    ctx.props.contextual_spacing = Some(true);
    let l = lay_12(&spaced_doc(vec![ctx, spaced("After", 0.0)], 1, float_table(612.0, 100.0, on_page(92.0))));
    assert!((top(&l, 0, 0) - 72.0).abs() < 0.01, "{}", top(&l, 0, 0));
    for (y, stays) in [(103.5, true), (102.5, false)] {
        let mut auto = spaced("Before", 0.0);
        auto.props.space_after_auto = Some(true);
        let l = lay_12(&spaced_doc(vec![auto, spaced("After", 0.0)], 1, float_table(612.0, 100.0, on_page(y))));
        assert_eq!((top(&l, 0, 0) - 72.0).abs() < 0.01, stays, "table at {y}: {}", top(&l, 0, 0));
    }
}

#[test]
fn the_next_paragraphs_space_before_does_not_count_near_a_floating_table() {
    // Word: 4 pt after and 8 pt before the next paragraph are 8 pt apart, but only the 4 pt keep
    // clear of a table at 93.5.
    let mut next = spaced("After", 0.0);
    next.props.space_before = Some(8.0);
    let l = lay_12(&spaced_doc(vec![spaced("Before", 4.0), next], 1, float_table(612.0, 100.0, on_page(93.5))));
    assert!((top(&l, 0, 0) - 72.0).abs() < 0.01, "{}", top(&l, 0, 0));
}

#[test]
fn a_last_lines_space_after_keeps_clear_of_a_floating_shape_and_goes_beside_a_narrow_table() {
    // Word: a shape placed on the page at 95 (top and bottom wrap) takes the line (72–89) with 8 pt
    // after below it, and leaves it with 4 pt; a table half the text wide at the margin, at 92,
    // takes it beside itself.
    for (after, stays) in [(8.0, false), (4.0, true)] {
        let float = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 95.0, ..Default::default() };
        let mut d = pinned("");
        d.body = [spaced("Before", after), spaced("Anchor", 0.0), spaced("After", 0.0)].into_iter().map(wordcraft_doc::para_block).collect();
        d.insert_object(&Pos::body(1, 0), rect_shape(612.0, 100.0, float), &Default::default()).unwrap();
        let l = lay_12(&d);
        let shape = shapes(&l.pages[0].items)[0];
        let t = top(&l, 0, 0);
        assert_eq!((t - 72.0).abs() < 0.01, stays, "after {after}: at {t}, shape {shape:?}");
        if !stays {
            assert!(t >= shape.bottom() - 0.01, "after {after}: at {t}, shape {shape:?}");
        }
    }
    let left = TableFloat { h_rel: Anchor::Margin, dist: [7.1, 0.0, 7.1, 0.0], ..on_page(92.0) };
    let l = lay_12(&spaced_doc(vec![spaced("Before", 8.0), spaced("After", 0.0)], 1, float_table(234.0, 100.0, left)));
    let table = table_rect(&l, 0, 1).unwrap();
    let (t, _, x) = lines(&l, 0, 0).unwrap();
    assert!((t - 72.0).abs() < 0.01 && x >= table.right() + 7.1 - 0.01, "at ({x}, {t}), {table:?}");
}

#[test]
fn text_of_an_earlier_section_keeps_its_space_after_clear_of_a_later_shape_too() {
    // Word (mode 12, A4, top margin 28.35 pt, Aptos 12 with 8 pt after): `XXX`, `ZZZ`, a
    // continuous section break, `YYY`, then a paragraph anchoring a shape across the page at
    // 45–145 pt: XXX, whose line ends above the shape and whose space after reaches it, goes below
    // the shape (147.7) with ZZZ (172.7). Here the earlier section is one paragraph (72–89) with 8
    // pt after, the shape at 95–195: below it in mode 12, where it stays with 4 pt after or in
    // Word 2013 layout.
    use wordcraft_doc::section::SectionStart;
    for (after, mode, stays) in [(8.0, wordcraft_doc::LEGACY_COMPAT_MODE, false), (4.0, wordcraft_doc::LEGACY_COMPAT_MODE, true), (8.0, 15, true)] {
        let float = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 95.0, ..Default::default() };
        let mut d = pinned("");
        let mut earlier = spaced("Earlier", after);
        earlier.section = Some(Box::default());
        d.body = [earlier, spaced("Before", 0.0), spaced("Anchor", 0.0), spaced("After", 0.0)].into_iter().map(wordcraft_doc::para_block).collect();
        d.last_section.start = SectionStart::Continuous;
        d.insert_object(&Pos::body(2, 0), rect_shape(612.0, 100.0, float), &Default::default()).unwrap();
        let l = in_mode(d, mode);
        let shape = shapes(&l.pages[0].items)[0];
        let t = top(&l, 0, 0);
        assert_eq!((t - 72.0).abs() < 0.01, stays, "{after} pt after, mode {mode}: at {t}, shape {shape:?}");
        if !stays {
            assert!((t - shape.bottom()).abs() < 0.01, "{after} pt after, mode {mode}: at {t}, shape {shape:?}");
        }
        assert!(clear_of(&l, 0, &shape) && top(&l, 0, 1) >= shape.bottom() - 0.01, "{after} pt after, mode {mode}: {}", top(&l, 0, 1));
    }
}

#[test]
fn automatic_line_spacing_keeps_the_whole_line_and_its_space_after_clear() {
    // The same with automatic (single) line spacing: the line's own height plus 8 pt.
    use wordcraft_doc::props::LineSpacing;
    let single = |text: &str, after: f32| {
        let mut p = spaced(text, after);
        p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
        p
    };
    let mut plain = pinned("");
    plain.body = vec![wordcraft_doc::para_block(single("Before", 0.0))];
    let bottom = lines(&lay_12(&plain), 0, 0).unwrap().1;
    for (y, stays) in [(bottom + 8.5, true), (bottom + 7.5, false), (bottom + 0.5, false)] {
        let l = lay_12(&spaced_doc(vec![single("Before", 8.0), single("After", 0.0)], 1, float_table(612.0, 100.0, on_page(y))));
        assert_eq!((top(&l, 0, 0) - 72.0).abs() < 0.01, stays, "line ends {bottom}, table at {y}: {}", top(&l, 0, 0));
    }
}

#[test]
fn automatic_space_after_counts_before_a_paragraph_that_shows_no_list_label() {
    // Automatic spacing after a list item drops between two list items only when the next one
    // shows a label: before a paragraph whose list is missing, or whose level shows no number,
    // the 14 pt stay and keep the line (72–89) clear of a table at 100.
    use wordcraft_doc::props::NumRef;
    for (next, stays) in [("same list", true), ("missing list", false), ("no number", false)] {
        let mut d = spaced_doc(vec![spaced("Item", 0.0), spaced("Next", 0.0)], 0, float_table(612.0, 100.0, on_page(100.0)));
        let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        let other = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        if next == "no number" {
            let abs = d.numbering.abstract_of(other).map(|a| a.id);
            for a in d.numbering.abstracts.iter_mut().filter(|a| Some(a.id) == abs) {
                for level in &mut a.levels {
                    level.format = wordcraft_doc::section::NumFormat::None;
                }
            }
        }
        let next_num = match next {
            "same list" => num,
            "missing list" => 9999,
            _ => other,
        };
        if let Some(Block::Para(p)) = d.body.get_mut(1).map(Arc::make_mut) {
            p.props.numbering = Some(NumRef { num, level: 0 });
            p.props.space_after_auto = Some(true);
        }
        if let Some(Block::Para(p)) = d.body.get_mut(2).map(Arc::make_mut) {
            p.props.numbering = Some(NumRef { num: next_num, level: 0 });
        }
        let l = lay_12(&d);
        let t = top(&l, 0, 1);
        assert_eq!((t - 72.0).abs() < 0.01, stays, "{next}: item at {t}");
        if !stays {
            assert!(t >= 200.0 - 0.01, "{next}: item at {t}");
        }
    }
}

/// A paragraph of two inline drawings 150 pt wide with a space between, exact 17 pt lines and
/// `after` after: font-independent widths that fill one strip each beside a shape in the middle.
fn two_drawings(after: f32) -> Document {
    let mut d = pinned("");
    d.body = vec![wordcraft_doc::para_block(spaced(" ", after))];
    d.insert_object(&Pos::body(0, 1), rect_shape(150.0, 10.0, Float::default()), &Default::default()).unwrap();
    d.insert_object(&Pos::body(0, 0), rect_shape(150.0, 10.0, Float::default()), &Default::default()).unwrap();
    d
}

#[test]
fn a_last_line_beside_a_shape_keeps_its_space_after_clear_and_its_row_moves_with_it() {
    // Word: a row split by a square-wrapped shape in the middle (250–350 across), its second
    // strip the paragraph's last line, and a shape across the page from 92: with 8 pt after the
    // whole row (72–89, + 8) goes below the second shape; with none it stays, a drawing in each strip.
    for (after, moves) in [(8.0, true), (0.0, false)] {
        let mut d = two_drawings(after);
        let square = Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 250.0, y: 40.0, ..Default::default() };
        let full = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 92.0, ..Default::default() };
        d.insert_object(&Pos::body(0, 0), rect_shape(100.0, 100.0, square), &Default::default()).unwrap();
        d.insert_object(&Pos::body(0, 0), rect_shape(612.0, 100.0, full), &Default::default()).unwrap();
        let l = lay_12(&d);
        let rows: Vec<(f32, f32)> = l.pages[0]
            .items
            .iter()
            .filter_map(|it| match it {
                Placed::Lines { story: StoryRef::Body, para, l0, l1, x, y, .. } => {
                    Some(para.lines.get(*l0..*l1).unwrap_or(&[]).iter().map(|ln| (x + ln.left, y + ln.top - para.lines[*l0].top)).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();
        if moves {
            assert!(rows.iter().all(|(_, t)| *t >= 192.0 - 0.01), "after {after}: {rows:?}");
        } else {
            assert!(rows.len() == 2 && rows.iter().all(|(_, t)| (*t - 72.0).abs() < 0.01), "after {after}: {rows:?}");
            assert!(rows.iter().any(|(x, _)| *x >= 350.0 - 0.01), "after {after}: one strip right of the shape: {rows:?}");
        }
    }
}

#[test]
fn a_paragraph_ending_in_a_line_break_keeps_its_space_after_clear() {
    // Word: a paragraph ending in a line break has an empty last line (89–106 here); with 8 pt
    // after that line goes below a table at 110, with none it stays.
    for (after, moves) in [(8.0, true), (0.0, false)] {
        let l = lay_12(&spaced_doc(vec![spaced("Before\n", after), spaced("After", 0.0)], 1, float_table(612.0, 100.0, on_page(110.0))));
        let (first, last) = (lines_at(&l, 0, 0, 0), lines_at(&l, 0, 0, 1));
        assert!((first - 72.0).abs() < 0.01, "after {after}: {first}");
        if moves {
            assert!(last >= 210.0 - 0.01, "after {after}: empty last line at {last}");
        } else {
            assert!((last - 89.0).abs() < 0.01, "after {after}: empty last line at {last}");
        }
    }
}

#[test]
fn the_empty_line_after_a_final_line_break_goes_beside_a_narrow_float_in_its_strip() {
    // Word: `Before` and a line break beside a table or shape at the left margin, then `After`:
    // the empty line takes the next row beside the float (`After` one row lower, 8 pt lower
    // still with 8 pt after), in the strip right of it like the other lines (where in the row
    // an empty line sits doesn't show in Word's PDF).
    let left_edge = |l: &DocLayout, block: u32, k: usize| {
        l.pages[0].items.iter().find_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, .. } if path.0 == [block] && (*l0..*l1).contains(&k) => {
                Some(x + para.lines.get(k)?.left)
            }
            _ => None,
        })
    };
    for after in [0.0, 8.0] {
        let table = TableFloat { h_rel: Anchor::Margin, dist: [7.1, 0.0, 7.1, 0.0], ..on_page(40.0) };
        let l = lay_12(&spaced_doc(vec![spaced("Before\n", after), spaced("After", 0.0)], 0, float_table(234.0, 100.0, table)));
        let strip = table_rect(&l, 0, 0).unwrap().right() + 7.1;
        assert!((lines_at(&l, 0, 1, 1) - 89.0).abs() < 0.01, "after {after}: {}", lines_at(&l, 0, 1, 1));
        assert!(left_edge(&l, 1, 1).is_some_and(|x| x >= strip - 0.01), "after {after}: {:?}, strip from {strip}", left_edge(&l, 1, 1));
        assert!((top(&l, 0, 2) - (106.0 + after)).abs() < 0.01, "after {after}: {}", top(&l, 0, 2));

        let square = Float { wrap: Wrap::Square, h_rel: Anchor::Margin, v_rel: Anchor::Page, y: 40.0, ..Default::default() };
        let mut d = pinned("");
        d.body = [spaced("Before\n", after), spaced("After", 0.0)].into_iter().map(wordcraft_doc::para_block).collect();
        d.insert_object(&Pos::body(0, 0), rect_shape(200.0, 100.0, square), &Default::default()).unwrap();
        let l = lay_12(&d);
        let shape = shapes(&l.pages[0].items)[0];
        assert!((lines_at(&l, 0, 0, 1) - 89.0).abs() < 0.01, "shape, after {after}: {}", lines_at(&l, 0, 0, 1));
        assert!(left_edge(&l, 0, 1).is_some_and(|x| x >= shape.right() - 0.01), "shape, after {after}: {:?}, {shape:?}", left_edge(&l, 0, 1));
        assert!((top(&l, 0, 1) - (106.0 + after)).abs() < 0.01, "shape, after {after}: {}", top(&l, 0, 1));
    }
}

#[test]
fn in_word_2013_layout_only_the_line_keeps_clear_of_a_float_not_its_space_after() {
    // Word for Mac, compatibility mode 15 against 12 (and 14): a line (72–89) with 8 pt after
    // stays above a table or shape at 96.5 in mode 15 and goes below it before; a shape 3.6 pt
    // from text at 93 (its wrap area from 89.4) the same; at 92 (from 88.4) the line itself
    // reaches it and goes below in both, 3.6 pt below the shape (195.6).
    let table = |mode: u32, y: f32| {
        let mut d = spaced_doc(vec![spaced("Before", 8.0), spaced("After", 0.0)], 1, float_table(612.0, 100.0, on_page(y)));
        d.settings.compat_mode = mode;
        top(&lay(&d), 0, 0)
    };
    let shape = |mode: u32, y: f32, dist: f32| {
        let float =
            Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y, dist_top: dist, dist_bottom: dist, ..Default::default() };
        let mut d = pinned("");
        d.body = [spaced("Before", 8.0), spaced("Anchor", 0.0)].into_iter().map(wordcraft_doc::para_block).collect();
        d.insert_object(&Pos::body(1, 0), rect_shape(612.0, 100.0, float), &Default::default()).unwrap();
        d.settings.compat_mode = mode;
        top(&lay(&d), 0, 0)
    };
    for mode in [12, 14, 15] {
        let stays = mode >= 15;
        let t = table(mode, 96.5);
        assert_eq!((t - 72.0).abs() < 0.01, stays, "mode {mode}, table at 96.5: {t}");
        let t = shape(mode, 96.5, 0.0);
        assert_eq!((t - 72.0).abs() < 0.01, stays, "mode {mode}, shape at 96.5: {t}");
        let t = shape(mode, 93.0, 3.6);
        assert_eq!((t - 72.0).abs() < 0.01, stays, "mode {mode}, shape at 93 with 3.6 from text: {t}");
        let t = shape(mode, 92.0, 3.6);
        assert!((t - 195.6).abs() < 0.01, "mode {mode}, shape at 92 with 3.6 from text: {t}");
    }
}

#[test]
fn an_empty_paragraph_below_a_box_its_paragraph_before_anchors_keeps_only_its_own_space_after() {
    // Word for Mac, an agenda letter: a paragraph of two lines (a line break; 72–106) with 30 pt
    // after anchors a box 87.5 pt high, 3.6 pt from text, placed 65.75 pt below the paragraph's
    // top (137.75, its wrap area 134.15–228.85); then an empty paragraph with 30 pt after, then
    // the text. Word 2013 layout: the two lines stay (only the space after reaches the box), the
    // empty paragraph goes below the box (228.85) and the text follows it and its 30 pt (275.85).
    // Before (mode 12 and 14): the last line goes below the box too, so the text is at 322.85.
    for (mode, empty, text) in [(15, 228.85, 275.85), (12, 275.85, 322.85), (14, 275.85, 322.85)] {
        let float = Float {
            wrap: Wrap::TopAndBottom,
            h_rel: Anchor::Page,
            v_rel: Anchor::Paragraph,
            y: 65.75,
            dist_top: 3.6,
            dist_bottom: 3.6,
            ..Default::default()
        };
        let mut d = pinned("");
        d.body = [spaced("Address\n", 30.0), spaced("", 30.0), spaced("Item", 0.0)].into_iter().map(wordcraft_doc::para_block).collect();
        d.insert_object(&Pos::body(0, 0), rect_shape(468.0, 87.5, float), &Default::default()).unwrap();
        d.settings.compat_mode = mode;
        let l = lay(&d);
        let shape = shapes(&l.pages[0].items)[0];
        assert!((shape.y - 137.75).abs() < 0.01, "mode {mode}: {shape:?}");
        assert!((lines_at(&l, 0, 0, 0) - 72.0).abs() < 0.01, "mode {mode}: {}", lines_at(&l, 0, 0, 0));
        let (e, t) = (top(&l, 0, 1), top(&l, 0, 2));
        assert!((e - empty).abs() < 0.01 && (t - text).abs() < 0.01, "mode {mode}: empty paragraph at {e}, text at {t}, not {empty} / {text}");
    }
}

#[test]
fn a_paragraph_moved_below_a_floating_table_or_shape_keeps_its_space_before_there() {
    // Word: a paragraph with 8 pt before whose first line would land on a table at 92 (or on its
    // own shape placed there) starts 8 pt below it; with no space before, right below it. A
    // paragraph that fits where it is keeps its space before above it as usual.
    for before in [8.0, 0.0] {
        let mut next = spaced("After the table", 0.0);
        next.props.space_before = Some(before);
        let l = lay(&spaced_doc(vec![spaced("Before", 0.0), next], 1, float_table(612.0, 100.0, on_page(92.0))));
        let table = table_rect(&l, 0, 1).unwrap();
        let t = top(&l, 0, 2);
        assert!((t - (table.bottom() + before)).abs() < 0.01, "before {before}: at {t}, table ends {}", table.bottom());

        let float = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, y: 92.0, ..Default::default() };
        let mut anchor = spaced("Anchor", 0.0);
        anchor.props.space_before = Some(before);
        let mut d = pinned("");
        d.body = [spaced("Before", 0.0), anchor].into_iter().map(wordcraft_doc::para_block).collect();
        d.insert_object(&Pos::body(1, 0), rect_shape(612.0, 100.0, float), &Default::default()).unwrap();
        let l = lay(&d);
        let shape = shapes(&l.pages[0].items)[0];
        let t = top(&l, 0, 1);
        assert!((t - (shape.bottom() + before)).abs() < 0.01, "before {before}: at {t}, shape ends {}", shape.bottom());
    }
    let mut next = spaced("After", 0.0);
    next.props.space_before = Some(8.0);
    let l = lay(&spaced_doc(vec![spaced("Before", 0.0), next], 1, float_table(612.0, 100.0, on_page(300.0))));
    assert!((top(&l, 0, 2) - 97.0).abs() < 0.01, "{}", top(&l, 0, 2));
}

fn multiple(text: &str, m: f32) -> Paragraph {
    use wordcraft_doc::props::LineSpacing;
    let mut p = spaced(text, 0.0);
    p.props.line_spacing = Some(LineSpacing::Multiple(m));
    p
}

fn in_mode(mut d: Document, mode: u32) -> DocLayout {
    d.settings.compat_mode = mode;
    lay(&d)
}

#[test]
fn in_word_2013_layout_a_line_keeps_its_text_clear_of_a_float_not_its_extra_line_spacing() {
    let mut single = pinned("");
    single.body = vec![wordcraft_doc::para_block(multiple("Before", 1.0))];
    let natural = lines(&lay(&single), 0, 0).unwrap().1 - 72.0;
    for m in [1.15, 2.0] {
        let at = |y: f32, mode: u32| {
            top(&in_mode(spaced_doc(vec![multiple("Before", m), multiple("After", m)], 1, float_table(612.0, 100.0, on_page(y))), mode), 0, 0)
        };
        for (y, mode, stays) in [
            (72.0 + natural + 0.3, 15, true),
            (72.0 + natural - 0.9, 15, false),
            (72.0 + natural + 0.3, 12, false),
            (72.0 + natural + 0.3, 14, false),
            (72.0 + natural * m + 0.4, 12, true),
        ] {
            let t = at(y, mode);
            assert_eq!((t - 72.0).abs() < 0.01, stays, "{m} lines, mode {mode}, natural {natural}, table at {y}: {t}");
        }
        let last = 72.0 + natural * m + natural;
        for (y, mode, stays) in [(last + 1.1, 15, true), (last - 0.9, 15, false), (last + 1.1, 12, false)] {
            let l = in_mode(spaced_doc(vec![multiple("Before\n", m), multiple("After", m)], 1, float_table(612.0, 100.0, on_page(y))), mode);
            let t = lines_at(&l, 0, 0, 1);
            assert_eq!((t - (72.0 + natural * m)).abs() < 0.01, stays, "{m} lines, mode {mode}, empty line, table at {y}: {t}");
        }
    }
}

#[test]
fn a_line_of_pictures_keeps_only_them_clear_of_a_float_in_word_2013_layout_and_their_paragraph_mark_before() {
    let picture = |text: &str, w: f32, m: f32| {
        let mut d = pinned("");
        d.body = vec![wordcraft_doc::para_block(multiple(text, m)), wordcraft_doc::para_block(multiple("After", m))];
        d.insert_object(&Pos::body(0, 0), rect_shape(w, 33.2, Float::default()), &Default::default()).unwrap();
        d
    };
    let with_table = |text: &str, w: f32, y: f32, at: usize| {
        let mut d = picture(text, w, 1.15);
        d.insert_block(StoryRef::Body, &Path::top(at), Block::Table(float_table(612.0, 100.0, on_page(y)))).unwrap();
        d
    };
    for at in [0, 1] {
        let para = if at == 0 { 1 } else { 0 };
        for (text, w, y, mode, stays) in [
            (" ", 400.0, 105.4, 15, true),
            (" ", 400.0, 104.8, 15, false),
            ("", 500.0, 105.6, 15, true),
            (" ", 500.0, 105.4, 15, true),
            (" ", 400.0, 105.7, 12, false),
            (" ", 400.0, 105.7, 14, false),
            ("", 500.0, 105.7, 12, false),
            ("", 500.0, 105.7, 14, false),
            (" ", 500.0, 105.4, 12, true),
            (" ", 500.0, 105.4, 14, true),
        ] {
            let t = top(&in_mode(with_table(text, w, y, at), mode), 0, para);
            assert_eq!((t - 72.0).abs() < 0.01, stays, "table {at}, {w} wide picture and {text:?}, mode {mode}, table at {y}: {t}");
        }
        let bottom = lines(&lay(&picture("ZZZ", 400.0, 1.0)), 0, 0).unwrap().1;
        for (y, stays) in [(bottom + 0.9, true), (bottom - 0.6, false)] {
            let t = top(&in_mode(with_table("ZZZ", 400.0, y, at), 15), 0, para);
            assert_eq!((t - 72.0).abs() < 0.01, stays, "table {at}, text after the picture to {bottom}, table at {y}: {t}");
        }
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

/// Word (Word 2013 layout, Calibri 11 pt on a 24 pt line grid, `lines` or `linesAndChars`, single
/// and 1.15 spacing, tables and square-wrapped shapes, before and after the float's anchor): a
/// line on the grid keeps only its text clear of a float below it, the text centred in the
/// snapped line (13.43 pt in 24, so to 18.7 pt from the line's top; 20.5 in 27.6 with 1.15):
/// the third line moves below a shape whose top lands 18.5 pt into it and stays for one 18.75
/// pt in (20.25 and 20.5 with 1.15). Here in [`TEST_FONT`], the text's bottom taken from a layout
/// without the float; before Word 2013 layout the whole snapped line counts
/// ([`a_line_on_a_line_grid_keeps_its_snapped_height_clear_of_a_float`]).
#[test]
fn in_word_2013_layout_a_line_on_a_line_grid_keeps_its_text_clear_of_a_float() {
    use wordcraft_doc::props::LineSpacing;
    use wordcraft_doc::section::{DocGrid, DocGridType};
    let text_8 = (1..=8).map(|i| format!("P{i} word")).collect::<Vec<_>>().join("\n");
    let square = |y: f32| Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, y, ..Default::default() };
    let on_grid = |mut d: Document, m: f32| {
        d.settings.compat_mode = 15;
        d.last_section.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: 24.0, char_space: 0 });
        for b in &mut d.body {
            if let Block::Para(p) = Arc::make_mut(b) {
                p.props.line_spacing = Some(LineSpacing::Multiple(m));
            }
        }
        d
    };
    let text_bottom = |l: &DocLayout, block: u32| {
        l.pages[0].items.iter().find_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, para, l0, y, .. } if path.0 == [block] => {
                para.lines.get(*l0).map(|ln| y + ln.baseline - ln.top + ln.text_desc)
            }
            _ => None,
        })
    };
    for m in [1.0, 1.15] {
        let plain = lay(&on_grid(exact_text(&text_8), m));
        let (top, bottom, _) = lines(&plain, 0, 2).unwrap();
        let text = text_bottom(&plain, 2).unwrap();
        assert!(text > top + 15.0 && text < bottom - 2.0, "{m}: the third line {top}–{bottom}, its text to {text}");
        for float_first in [false, true] {
            for table in [true, false] {
                let below_text = ((text - top) as usize).saturating_sub(1);
                for (y, moves) in
                    (0..=below_text).map(|k| (top + 0.5 + k as f32, true)).chain([(text - 0.25, true), (text + 0.05, false), (bottom - 0.5, false)])
                {
                    let d = match (table, float_first) {
                        (true, first) => doc_with_table(&text_8, if first { 0 } else { 5 }, float_table(612.0, 50.0, on_page(y))),
                        (false, first) => doc_with_shape(&text_8, if first { 0 } else { 5 }, 612.0, 50.0, square(y)),
                    };
                    let l = lay(&on_grid(d, m));
                    let float = if table { table_rect(&l, 0, if float_first { 0 } else { 5 }) } else { shapes(&l.pages[0].items).first().copied() };
                    let case = format!("{m} lines, float first {float_first}, table {table}, y {y} (text to {text})");
                    let float = float.unwrap_or_else(|| panic!("{case}: no float"));
                    let third = lines(&l, 0, if table && float_first { 3 } else { 2 }).unwrap();
                    if moves {
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

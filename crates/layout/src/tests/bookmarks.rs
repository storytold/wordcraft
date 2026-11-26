//! File › Options › Advanced › Show bookmarks (#3572): grey brackets where bookmarks start and
//! end, and an I-beam where one marks a place, drawn only when the option asks for them.

use crate::display::{self, BOOKMARK_COLOR, DisplayOptions, Draw};
use crate::{DocLayout, Document, LayoutCache, LayoutOptions, layout};
use wordcraft_doc::Pos;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{CharProps, ParaProps};

/// One bracket as drawn: its upright's page x, top and bottom, and the x span of its two ticks.
#[derive(Debug, Clone, Copy)]
struct Bracket {
    x: f32,
    top: f32,
    bottom: f32,
    ticks: (f32, f32),
}

/// Puts the start (`start`) or end of bookmark `name` at `pos`.
fn mark(d: &mut Document, pos: Pos, name: &str, start: bool) {
    let name = name.to_string();
    let o = if start { InlineObject::BookmarkStart { name } } else { InlineObject::BookmarkEnd { name } };
    d.insert_object(&pos, o, &CharProps::default()).unwrap();
}

/// Puts bookmark `name` round the bytes `a..b` of body paragraph `para` (offsets before the
/// markers go in); the text it holds then starts at `a + 3` (past the start marker).
fn bookmark(d: &mut Document, para: usize, (a, b): (usize, usize), name: &str) {
    mark(d, Pos::body(para, b), name, false);
    mark(d, Pos::body(para, a), name, true);
}

fn lay(d: &Document) -> DocLayout {
    layout(d, &mut LayoutCache::new(), &LayoutOptions::default())
}

/// The first page's draws with Show bookmarks `on`.
fn draws(d: &Document, l: &DocLayout, on: bool) -> Vec<Draw> {
    display::page_display(d, &l.pages[0], &DisplayOptions { bookmarks: on, ..Default::default() })
}

/// The bookmark brackets among `items`, in drawing order. Each is an upright and a tick at its
/// top and at its bottom, both spanning the same x.
fn brackets(items: &[Draw]) -> Vec<Bracket> {
    let lines: Vec<(f32, f32, f32, f32)> = items
        .iter()
        .filter_map(|i| match i {
            Draw::Line { x0, y0, x1, y1, color, .. } if *color == BOOKMARK_COLOR => Some((*x0, *y0, *x1, *y1)),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len() % 3, 0, "each bracket is three rules: {lines:?}");
    lines
        .chunks(3)
        .map(|c| {
            let [(x, top, ux, bottom), (a, ty, b, ty1), (a2, by, b2, by1)] = [c[0], c[1], c[2]];
            assert_eq!(x, ux, "the upright is upright: {c:?}");
            assert!(bottom > top, "the upright runs down: {c:?}");
            assert_eq!((ty, ty1, by, by1), (top, top, bottom, bottom), "the ticks sit at its ends: {c:?}");
            assert_eq!((a, b), (a2, b2), "both ticks point the same way: {c:?}");
            Bracket { x, top, bottom, ticks: (a, b) }
        })
        .collect()
}

/// Asserts `b` stands where the caret at `pos` does, as tall as it.
fn at_caret(l: &DocLayout, b: &Bracket, pos: Pos) {
    let c = l.caret(&pos).unwrap();
    let close = |p: f32, q: f32| (p - q).abs() < 0.01;
    assert!(close(b.x, c.x) && close(b.top, c.top) && close(b.bottom, c.top + c.height), "{b:?} should stand at the caret {c:?} at {pos:?}");
}

/// Which way `b`'s ticks point from its upright: 1 right, -1 left, 0 both ways (an I-beam).
fn facing(b: &Bracket) -> i32 {
    let (a, c) = b.ticks;
    assert!(c > a, "ticks have length: {b:?}");
    match (a == b.x, c == b.x) {
        (true, false) => 1,
        (false, true) => -1,
        _ => {
            assert!((b.x - a - (c - b.x)).abs() < 0.01, "an I-beam's ticks are centred: {b:?}");
            0
        }
    }
}

#[test]
fn bookmarks_show_as_brackets_round_their_text_only_when_asked() {
    let mut d = Document::from_text("Hello brave world");
    bookmark(&mut d, 0, (6, 11), "Pick");
    let l = lay(&d);
    // Off (the default, and what prints and PDFs use): none.
    assert!(brackets(&display::page_display(&d, &l.pages[0], &DisplayOptions::default())).is_empty());
    let (off, on) = (draws(&d, &l, false), draws(&d, &l, true));
    assert!(brackets(&off).is_empty());
    let b = brackets(&on);
    assert_eq!(b.len(), 2, "{b:?}");
    // "[" before "brave", "]" after it: "brave" is bytes 9..14 once the start marker is in.
    at_caret(&l, &b[0], Pos::body(0, 9));
    at_caret(&l, &b[1], Pos::body(0, 14));
    assert_eq!((facing(&b[0]), facing(&b[1])), (1, -1));
    assert!(b[0].x < b[1].x);
    // They take no room: everything else is drawn the same, in the same places.
    let rest: Vec<String> =
        on.iter().filter(|i| !matches!(i, Draw::Line { color, .. } if *color == BOOKMARK_COLOR)).map(|i| format!("{i:?}")).collect();
    assert_eq!(rest, off.iter().map(|i| format!("{i:?}")).collect::<Vec<_>>());
}

#[test]
fn a_bookmark_marking_a_place_is_an_i_beam_and_hidden_ones_show_nothing() {
    let mut d = Document::from_text("Hello world");
    // "Here" marks the place after "Hello".
    bookmark(&mut d, 0, (5, 5), "Here");
    // Word's own hidden bookmarks: one round "world" (bytes 12..17 now), one marking the start.
    bookmark(&mut d, 0, (12, 17), "_Toc123");
    bookmark(&mut d, 0, (0, 0), "_GoBack");
    let l = lay(&d);
    let b = brackets(&draws(&d, &l, true));
    assert_eq!(b.len(), 1, "one I-beam, nothing for the hidden bookmarks: {b:?}");
    assert_eq!(facing(&b[0]), 0);
    // After _GoBack's two markers and "Hello".
    at_caret(&l, &b[0], Pos::body(0, 11));
    // Text between its start and end makes it a pair of brackets again.
    let mut d = Document::from_text("Hello world");
    bookmark(&mut d, 0, (5, 6), "Space");
    let l = lay(&d);
    let b = brackets(&draws(&d, &l, true));
    assert_eq!(b.iter().map(facing).collect::<Vec<_>>(), [1, -1]);
    // So does a picture: a bookmark round an inline picture alone isn't a place.
    let mut d = Document::from_text("Hello world");
    let pic = InlineObject::Image {
        media: "m".into(),
        w: 30.0,
        h: 20.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
        fx: Default::default(),
    };
    d.insert_object(&Pos::body(0, 5), pic, &CharProps::default()).unwrap();
    bookmark(&mut d, 0, (5, 8), "Picture");
    let l = lay(&d);
    let b = brackets(&draws(&d, &l, true));
    assert_eq!(b.iter().map(facing).collect::<Vec<_>>(), [1, -1], "{b:?}");
    // Either side of the picture (bytes 8..11 once the start marker is in).
    at_caret(&l, &b[0], Pos::body(0, 8));
    at_caret(&l, &b[1], Pos::body(0, 11));
}

#[test]
fn a_bookmark_across_paragraphs_opens_in_one_and_closes_in_the_other() {
    let mut d = Document::from_text("First line\nSecond line");
    mark(&mut d, Pos::body(1, 6), "Span", false);
    mark(&mut d, Pos::body(0, 6), "Span", true);
    let l = lay(&d);
    let b = brackets(&draws(&d, &l, true));
    assert_eq!(b.len(), 2, "{b:?}");
    at_caret(&l, &b[0], Pos::body(0, 9));
    at_caret(&l, &b[1], Pos::body(1, 6));
    assert_eq!((facing(&b[0]), facing(&b[1])), (1, -1));
    assert!(b[1].top > b[0].bottom, "the end is on the next paragraph's line");
}

/// Hidden text that isn't shown takes no room, so a bookmark round only that marks a place: an
/// I-beam, as in Word, and its two brackets again once hidden text is shown.
#[test]
fn a_bookmark_round_hidden_text_is_an_i_beam_until_the_text_is_shown() {
    let mut d = Document::from_text("Hello secret world");
    bookmark(&mut d, 0, (6, 12), "Secret");
    // "secret" and both its markers hidden (bytes 6..18).
    d.format_range(&Pos::body(0, 6), &Pos::body(0, 18), &|c| c.hidden = Some(true)).unwrap();
    // Word: with the hidden text not shown, an I-beam where it is, between "Hello " and "world".
    let l = lay(&d);
    let b = brackets(&draws(&d, &l, true));
    assert_eq!(b.iter().map(facing).collect::<Vec<_>>(), [0], "{b:?}");
    at_caret(&l, &b[0], Pos::body(0, 6));
    at_caret(&l, &b[0], Pos::body(0, 18));
    let shown = layout(&d, &mut LayoutCache::new(), &LayoutOptions { show_hidden: true, ..Default::default() });
    let b = brackets(&draws(&d, &shown, true));
    assert_eq!(b.iter().map(facing).collect::<Vec<_>>(), [1, -1]);
    at_caret(&shown, &b[0], Pos::body(0, 9));
    at_caret(&shown, &b[1], Pos::body(0, 15));
}

/// The P3 on #5422, checked in Word 16: brackets round tracked-deleted text in markup view, an
/// I-beam where it was in No Markup.
#[test]
fn a_bookmark_round_tracked_deleted_text_is_bracketed_in_markup_and_an_i_beam_without() {
    // As Word 16 shows it: in All Markup the struck-through text is on screen and the brackets go
    // round it; in No Markup it is gone and the bookmark is an I-beam where it was. Either with
    // the markers deleted too (the text was bookmarked, then deleted) or not (the deleted text
    // was bookmarked).
    for (markers, (a, b)) in [("deleted too", (6, 17)), ("not deleted", (9, 14))] {
        let mut d = Document::from_text("Hello brave world");
        bookmark(&mut d, 0, (6, 11), "Pick");
        d.format_range(&Pos::body(0, a), &Pos::body(0, b), &|c| c.del = Some(0)).unwrap();
        let markup = lay(&d);
        let br = brackets(&draws(&d, &markup, true));
        assert_eq!(br.iter().map(facing).collect::<Vec<_>>(), [1, -1], "markers {markers}: {br:?}");
        at_caret(&markup, &br[0], Pos::body(0, 9));
        at_caret(&markup, &br[1], Pos::body(0, 14));
        let no_markup = layout(&d, &mut LayoutCache::new(), &LayoutOptions { hide_deleted: true, ..Default::default() });
        let br = brackets(&draws(&d, &no_markup, true));
        assert_eq!(br.iter().map(facing).collect::<Vec<_>>(), [0], "markers {markers}: {br:?}");
        // Between "Hello " and "world", which now close up.
        at_caret(&no_markup, &br[0], Pos::body(0, 6));
        at_caret(&no_markup, &br[0], Pos::body(0, 17));
    }
}

/// A paragraph that takes no room at all shows no bookmark mark either, not even one of no height.
#[test]
fn a_bookmark_in_a_paragraph_left_out_altogether_shows_nothing() {
    // A paragraph whose text and mark are all a tracked deletion takes no room in No Markup.
    let mut d = Document::from_text("Gone\nKept");
    bookmark(&mut d, 0, (0, 4), "Gone");
    d.format_range(&Pos::body(0, 0), &Pos::body(1, 0), &|c| c.del = Some(0)).unwrap();
    let no_markup = layout(&d, &mut LayoutCache::new(), &LayoutOptions { hide_deleted: true, ..Default::default() });
    assert!(brackets(&draws(&d, &no_markup, true)).is_empty());
    // With markup the deletion is on screen, and so is its bookmark.
    let b = brackets(&draws(&d, &lay(&d), true));
    assert_eq!(b.iter().map(facing).collect::<Vec<_>>(), [1, -1], "{b:?}");
}

#[test]
fn right_to_left_text_turns_the_brackets_round() {
    // A right-to-left paragraph, the bookmark round its second word (bytes 9..17).
    let mut d = Document::from_text("\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd}");
    let at = Pos::body(0, 0);
    d.format_paragraphs(&at, &at, &|p: &mut ParaProps| p.bidi = Some(true)).unwrap();
    bookmark(&mut d, 0, (9, 17), "Olam");
    let l = lay(&d);
    let b = brackets(&draws(&d, &l, true));
    assert_eq!(b.len(), 2, "{b:?}");
    at_caret(&l, &b[0], Pos::body(0, 12));
    at_caret(&l, &b[1], Pos::body(0, 20));
    // The word reads leftwards: its start bracket is on its right and opens to the left.
    assert_eq!((facing(&b[0]), facing(&b[1])), (-1, 1));
    assert!(b[0].x > b[1].x);
}

/// Many bookmarks over many lines of one paragraph: each line draws its own, in place (#3614).
#[test]
fn every_bookmark_of_a_long_paragraph_shows_on_its_own_line() {
    // 300 words over many lines, each its own bookmark.
    let words: Vec<String> = (0..300).map(|i| format!("w{i:03}")).collect();
    let mut d = Document::from_text(&words.join(" "));
    // From the end, so the offsets before each word stay put.
    for i in (0..words.len()).rev() {
        let a = i * 5;
        bookmark(&mut d, 0, (a, a + 4), &format!("b{i}"));
    }
    let l = lay(&d);
    assert!(l.pages[0].items.iter().any(|it| matches!(it, crate::Placed::Lines { para, .. } if para.lines.len() > 10)));
    let b = brackets(&draws(&d, &l, true));
    assert_eq!(b.len(), 600);
    // Each word with its markers and the space after it is 11 bytes: word i is at 11i + 3..11i + 7.
    for (i, pair) in b.chunks(2).enumerate() {
        at_caret(&l, &pair[0], Pos::body(0, 11 * i + 3));
        at_caret(&l, &pair[1], Pos::body(0, 11 * i + 7));
        assert_eq!((facing(&pair[0]), facing(&pair[1])), (1, -1));
    }
}

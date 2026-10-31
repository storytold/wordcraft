//! Text boxes' text direction, alignment and linked boxes (Shape Format › Text).

use crate::boxes::natural_width;
#[allow(unused_imports)]
use crate::*;
use wordcraft_doc::props::{TextVert, VAlign};
use wordcraft_doc::{PartKind, Pos, TextBody, para_block};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// A text box `w` × `h` showing `text`, inline at the end of body paragraph `para`; its story id.
fn text_box(d: &mut Document, para: usize, text: &str, w: f32, h: f32, body: TextBody) -> u32 {
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(text, Default::default()))]);
    if let Some(p) = d.parts.get_mut(&id) {
        p.body = body;
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

/// The lines of story `id` on the first page: (x, y, turn, l0, l1).
fn lines(l: &DocLayout, id: u32) -> Vec<(f32, f32, TextDirection, usize, usize)> {
    l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), x, y, turn, l0, l1, .. } if *s == id => Some((*x, *y, *turn, *l0, *l1)),
            _ => None,
        })
        .collect()
}

fn frame_of(l: &DocLayout, id: u32) -> Rect {
    l.find_object(0, |o| o.text_box == Some(id)).unwrap().rect
}

#[test]
fn align_text_and_text_direction_place_the_text_in_the_box() {
    let at = |body: TextBody| {
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, "Hi", 144.0, 144.0, body);
        let l = lay(&d);
        (lines(&l, id)[0], frame_of(&l, id))
    };
    let ((_, top, turn, ..), r) = at(TextBody::default());
    assert_eq!(turn, TextDirection::Horizontal);
    assert!((top - (r.y + BOX_INSET_Y)).abs() < 1.0, "top-aligned text starts under the top inset: {top} in {r:?}");
    let ((_, mid, ..), _) = at(TextBody { anchor: VAlign::Center, ..Default::default() });
    let ((_, bottom, ..), _) = at(TextBody { anchor: VAlign::Bottom, ..Default::default() });
    assert!(mid > top + 40.0 && bottom > mid + 40.0, "{top} {mid} {bottom}");
    assert!(bottom < r.bottom() - BOX_INSET_Y, "the line stays inside the box");
    // Rotated 90°: lines run down from the top right corner; 270°: up from the bottom left.
    let ((x, y, turn, ..), r) = at(TextBody { vert: TextVert::Vert, ..Default::default() });
    assert_eq!(turn, TextDirection::Down);
    assert!((x - (r.right() - BOX_INSET_X)).abs() < 1.0 && (y - (r.y + BOX_INSET_Y)).abs() < 1.0, "{x},{y} in {r:?}");
    let ((x, y, turn, ..), r) = at(TextBody { vert: TextVert::Vert270, anchor: VAlign::Bottom, ..Default::default() });
    assert_eq!(turn, TextDirection::Up);
    assert!(x > r.x + 100.0 && (y - (r.bottom() - BOX_INSET_Y)).abs() < 1.0, "bottom-aligned turned text sits at the far side: {x},{y} in {r:?}");
    // Stacked: one letter to a line, upright.
    let mut d = Document::from_text("x");
    let id = text_box(&mut d, 0, "Hi", 144.0, 144.0, TextBody { vert: TextVert::Stacked, ..Default::default() });
    let l = lay(&d);
    let (.., l0, l1) = lines(&l, id)[0];
    assert_eq!(l1 - l0, 2, "two letters, two lines");
}

#[test]
fn linked_boxes_continue_the_text() {
    let mut d = Document::from_text("one\ntwo");
    let text = "Words that run on well past the first little box. ".repeat(6);
    let a = text_box(&mut d, 0, &text, 144.0, 48.0, TextBody::default());
    let b = text_box(&mut d, 1, "", 144.0, 400.0, TextBody::default());
    let unlinked = lay(&d);
    let shown = |l: &DocLayout| lines(l, a).iter().map(|(.., l0, l1)| l1 - l0).sum::<usize>();
    let in_a = shown(&unlinked);
    d.parts.get_mut(&a).unwrap().body.next = Some(b);
    let l = lay(&d);
    // Both boxes show (and edit) the first box's story.
    let frames: Vec<Rect> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { rect, text_box: Some(s), .. } if *s == a => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(frames.len(), 2);
    let rb = frames[1];
    let ls = lines(&l, a);
    // The first box shows what fits; the second picks up at the next line, at its top.
    let (first, rest): (Vec<(f32, f32, TextDirection, usize, usize)>, Vec<_>) = ls.iter().partition(|(_, y, ..)| *y < rb.y);
    assert_eq!(first.iter().map(|(.., l0, l1)| l1 - l0).sum::<usize>(), in_a);
    let (_, y, _, l0, _) = rest[0];
    assert_eq!(l0, in_a, "continues with the first line the first box couldn't show");
    assert!((y - (rb.y + BOX_INSET_Y)).abs() < 1.0);
    // Every line shows once.
    let total = ls.iter().map(|(.., l0, l1)| l1 - l0).sum::<usize>();
    let all = l.pages[0].items.iter().find_map(|it| match it {
        Placed::Lines { story: StoryRef::Part(s), para, .. } if *s == a => Some(para.lines.len()),
        _ => None,
    });
    assert_eq!(Some(total), all);
}

#[test]
fn hostile_link_chains_stay_bounded() {
    // Boxes linking in a circle show their own text; a box linking to itself too.
    let mut d = Document::from_text("one\ntwo\nthree");
    let a = text_box(&mut d, 0, "A", 72.0, 36.0, TextBody::default());
    let b = text_box(&mut d, 1, "B", 72.0, 36.0, TextBody::default());
    let c = text_box(&mut d, 2, "C", 72.0, 36.0, TextBody::default());
    d.parts.get_mut(&a).unwrap().body.next = Some(b);
    d.parts.get_mut(&b).unwrap().body.next = Some(a);
    d.parts.get_mut(&c).unwrap().body.next = Some(c);
    assert!(d.text_box_chains().is_empty());
    let l = lay(&d);
    for id in [a, b, c] {
        assert_eq!(lines(&l, id).len(), 1, "box {id} shows its own text");
    }
}

#[test]
fn own_margins_and_resize_to_fit_keep_the_text_in_view() {
    // #743: a cover page's two-line box with no margins (`lIns="0"`…) is just tall enough for
    // its text; with Word's default margins its second line would be cut.
    let two = |h: f32, body: TextBody| {
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, "One", 144.0, h, body);
        d.parts.get_mut(&id).unwrap().blocks =
            vec![para_block(Paragraph::with_text("One", Default::default())), para_block(Paragraph::with_text("Two", Default::default()))];
        let l = lay(&d);
        let shown: usize = lines(&l, id).iter().map(|(.., l0, l1)| l1 - l0).sum();
        (shown, lines(&l, id).first().map(|(x, y, ..)| (*x, *y)), frame_of(&l, id))
    };
    // How deep the two lines reach without margins, from a box with room to spare.
    let zero = TextBody { inset: Some([0; 4]), ..Default::default() };
    let mut d = Document::from_text("x");
    let id = text_box(&mut d, 0, "One", 144.0, 400.0, zero);
    d.parts.get_mut(&id).unwrap().blocks =
        vec![para_block(Paragraph::with_text("One", Default::default())), para_block(Paragraph::with_text("Two", Default::default()))];
    let l = lay(&d);
    let r = frame_of(&l, id);
    let need = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), para, l0, l1, y, .. } if *s == id => {
                let (first, last) = (para.lines.get(*l0)?, para.lines.get(l1.checked_sub(1)?)?);
                Some(y + last.top + last.height - first.top - r.y)
            }
            _ => None,
        })
        .fold(0.0f32, f32::max);
    assert!(need > 10.0, "{need}");
    let (shown, at, r) = two(need + 1.0, zero);
    assert_eq!(shown, 2, "both lines fit without margins");
    let (x, y) = at.unwrap();
    assert!((x - r.x).abs() < 0.5 && (y - r.y).abs() < 0.5, "text starts at the box's corner: {x},{y} in {r:?}");
    assert_eq!(two(need + 1.0, TextBody::default()).0, 1, "Word's default margins cut the second line");
    let (shown, ..) = two(need / 2.0, TextBody { auto_fit: true, ..Default::default() });
    assert_eq!(shown, 2, "a box that resizes to fit its text shows it all");
    // Hostile margins wider than the box still lay out.
    let (shown, ..) = two(20.0, TextBody { inset: Some([u32::MAX; 4]), ..Default::default() });
    assert!(shown >= 1);
}

/// #2191: a floating box that resizes to fit its text and doesn't wrap it (Wrap text in shape
/// off) is as wide as its line and as deep as its text, whatever size the file gives it: Word's
/// figure caption on one line, not a 144 pt square of three lines pushing the text beside it.
/// Without Resize shape to fit text the box keeps its size and the line runs on past its side;
/// a box whose text wraps keeps its size too.
#[test]
fn a_box_fitting_text_it_doesnt_wrap_takes_the_size_of_its_line() {
    let [l, t, r, b] = TextBody::default().insets_pt();
    let ((n, wide), frame) = square_caption(TextBody { auto_fit: true, no_wrap: true, ..Default::default() }, None, 0.0);
    assert_eq!(n, 1, "one line");
    assert!(wide > 200.0, "the line is wider than the box was: {wide}");
    assert!((frame.w - (wide + l + r)).abs() < 0.05, "as wide as its line: {frame:?}, line {wide}");
    assert!(frame.h > t + b + 5.0 && frame.h < t + b + 20.0, "one line deep: {frame:?}");
    let ((n, wide), frame) = square_caption(TextBody { no_wrap: true, ..Default::default() }, None, 0.0);
    assert_eq!(n, 1, "one line, past the box's side");
    assert!(wide > 200.0 && (frame.w - 144.0).abs() < 0.01 && (frame.h - 144.0).abs() < 0.01, "its own size: {frame:?}");
    let ((n, _), frame) = square_caption(TextBody { auto_fit: true, ..Default::default() }, None, 0.0);
    assert!(n > 1, "the text wraps");
    assert!((frame.w - 144.0).abs() < 0.01 && (frame.h - 144.0).abs() < 0.01, "its own size: {frame:?}");
}

/// Word adds a fitted box's outline width to its size, drawn or not: no outline 354.6 × 19.6 pt,
/// a 0.5 pt one 355.1 × 20.1, a 3 pt one (drawn or `a:noFill`) 357.6 × 22.6 (Word for Windows
/// 16.0.17932, a synthetic caption box, #2191 review). The text stays the same size.
#[test]
fn a_fitted_box_also_takes_its_outlines_width() {
    let body = TextBody { auto_fit: true, no_wrap: true, ..Default::default() };
    let ((_, wide), bare) = square_caption(body, None, 0.0);
    let grey = Some(wordcraft_doc::props::Rgb(0x80, 0x80, 0x80));
    for (stroke, width) in [(grey, 0.5), (grey, 3.0), (None, 3.0)] {
        let ((n, w), frame) = square_caption(body, stroke, width);
        assert_eq!((n, w), (1, wide), "the same line ({stroke:?}, {width} pt)");
        let grown = (frame.w - bare.w, frame.h - bare.h);
        assert!((grown.0 - width).abs() < 0.01 && (grown.1 - width).abs() < 0.01, "{width} pt wider and deeper ({stroke:?}): {grown:?}");
    }
}

/// A square-wrapped box holding a one-line caption, with outline `stroke` `width` pt wide: its
/// first paragraph's line count and widest line, and its frame.
fn square_caption(body: TextBody, stroke: Option<wordcraft_doc::props::Rgb>, width: f32) -> ((usize, f32), Rect) {
    let mut d = Document::from_text("x");
    let id = text_box(&mut d, 0, "(Figure 1.1) A time line of the important vaccines, on one line", 144.0, 144.0, body);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    for o in &mut p.objects {
        if let InlineObject::Shape { stroke: s, stroke_width, .. } = o {
            (*s, *stroke_width) = (stroke, width);
        }
    }
    if let Some(f) = p.objects.iter_mut().find_map(InlineObject::float_mut) {
        f.wrap = wordcraft_doc::para::Wrap::Square;
    }
    p.touch();
    let l = lay(&d);
    let line = l.pages[0].items.iter().find_map(|it| match it {
        Placed::Lines { story: StoryRef::Part(s), para, .. } if *s == id => Some((para.lines.len(), natural_width(para))),
        _ => None,
    });
    (line.unwrap(), frame_of(&l, id))
}

/// A box whose text doesn't wrap but that also holds a table keeps its own width, also when it
/// resizes to fit its text (an inline one is then as deep as its text, #2791); its paragraphs
/// still run on one line past its side, and the table keeps the box's width (#2191 review).
#[test]
fn text_that_doesnt_wrap_beside_a_table_stays_on_one_line() {
    let caption = "(Figure 1.1) A time line of the important vaccines, on one line";
    for body in [TextBody { no_wrap: true, ..Default::default() }, TextBody { no_wrap: true, auto_fit: true, ..Default::default() }] {
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, caption, 144.0, 144.0, body);
        if let Some(p) = d.parts.get_mut(&id) {
            p.blocks.push(std::sync::Arc::new(Block::Table(wordcraft_doc::Table::new(1, 2, 100.0))));
        }
        let l = lay(&d);
        let frame = frame_of(&l, id);
        assert!((frame.w - 144.0).abs() < 0.01, "its own width: {frame:?} ({body:?})");
        if body.auto_fit {
            assert!(frame.h < 100.0, "as deep as one line and the table: {frame:?}");
        } else {
            assert!((frame.h - 144.0).abs() < 0.01, "its own height: {frame:?}");
        }
        let caption_lines = l.pages[0].items.iter().find_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), para, .. } if *s == id => Some((para.lines.len(), natural_width(para))),
            _ => None,
        });
        let (n, wide) = caption_lines.unwrap();
        assert_eq!(n, 1, "one line, past the box's side ({body:?})");
        assert!(wide > 200.0, "{wide}");
        let table_right = l.pages[0].items.iter().filter_map(|it| match it {
            Placed::Rule { x0, x1, .. } => Some(x0.max(*x1)),
            _ => None,
        });
        for right in table_right {
            assert!(right <= frame.right() + 1.0, "the table stays in the box: {right} > {}", frame.right());
        }
    }
}

/// An auto-fit inline box shrinks to its three lines and keeps the explicit page break on page
/// one; a fixed 10 cm box keeps its saved height and pushes the break and following text to
/// later pages.
#[test]
fn inline_box_that_resizes_to_fit_takes_its_texts_height() {
    // #2791: an inline box with Resize shape to fit text, saved 10 cm tall around three short
    // lines, shrinks to them as in Word; the page break after it stays on the first page.
    let brk = wordcraft_doc::para::PAGE_BREAK;
    let doc = |auto_fit: bool| {
        let filler: Vec<String> = (1..=FILLER).map(|i| format!("Filler line {i}")).collect();
        let mut d = super::pagination_fixture_document(&format!("{}\n\n\n{brk}\nSecond page text.", filler.join("\n")));
        // Exact spacing is only for the filler/short text. The object paragraph must retain
        // automatic spacing so its inline box really contributes its physical height.
        d.para_mut(StoryRef::Body, &Path::top(FILLER)).unwrap().props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.15));
        let probe = lay(&d);
        super::assert_pagination_fixture_font(&probe);
        let body_para = |layout: &DocLayout, index: usize| {
            layout
                .pages
                .iter()
                .enumerate()
                .find_map(|(page, p)| {
                    p.items.iter().find_map(|it| match it {
                        Placed::Lines { story: StoryRef::Body, path, para, y, .. } if path.0 == [index as u32] => {
                            Some((page, *y, para.height, para.rp.space_after))
                        }
                        _ => None,
                    })
                })
                .unwrap()
        };
        let (anchor_page, anchor_y, _, _) = body_para(&probe, FILLER);
        assert_eq!(anchor_page, 0, "all twenty-two original filler paragraphs must precede the box on page one");
        assert_eq!(d.body.len(), FILLER + 4, "all original blank, break and following paragraphs remain");
        assert_eq!(d.para(StoryRef::Body, &Path::top(FILLER + 1)).unwrap().text, "");
        assert_eq!(d.para(StoryRef::Body, &Path::top(FILLER + 2)).unwrap().text, brk.to_string());
        let available = probe.pages[0].body.bottom() - anchor_y;
        let (_, _, blank_h, blank_after) = body_para(&probe, FILLER + 1);
        let (_, _, break_h, break_after) = body_para(&probe, FILLER + 2);
        let id = text_box(&mut d, FILLER, "", 453.5, 283.46, TextBody { auto_fit, anchor: VAlign::Center, ..Default::default() });
        d.parts.get_mut(&id).unwrap().blocks = (1..=3)
            .map(|i| {
                let mut p = Paragraph::with_text(&format!("Text box line {i}"), Default::default());
                p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Exactly(14.0));
                p.props.space_before = Some(0.0);
                p.props.space_after = Some(8.0);
                para_block(p)
            })
            .collect();
        let l = lay(&d);
        super::assert_pagination_fixture_font(&l);
        let (object_page, r) = l
            .pages
            .iter()
            .enumerate()
            .find_map(|(page, p)| {
                p.items.iter().find_map(|it| match it {
                    Placed::Object { rect, text_box: Some(s), .. } if *s == id => Some((page, *rect)),
                    _ => None,
                })
            })
            .unwrap();
        let (_, _, object_line_h, object_after) = body_para(&l, FILLER);
        let needed = object_line_h + object_after + blank_h + blank_after + break_h + break_after;
        assert!(available > 0.0, "the box anchor must have measured space available");
        if auto_fit {
            assert!(needed < available - 1.0, "auto-fit box plus blank and break need {needed}pt, only {available}pt available");
        } else {
            assert!(r.h > available + 1.0, "the fixed 10cm box itself must exceed the {available}pt remaining");
        }
        let shown: Vec<_> = l
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| match it {
                Placed::Lines { story: StoryRef::Part(s), path, l0, l1, .. } if *s == id => Some((path.0.clone(), l1 - l0)),
                _ => None,
            })
            .collect();
        assert_eq!(shown.iter().map(|(_, count)| count).sum::<usize>(), 3, "all three original text-box paragraphs remain visible");
        assert_eq!(shown.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(), vec![vec![0], vec![1], vec![2]]);
        assert_eq!(object_page, usize::from(!auto_fit), "the real inline box moves only when its saved height cannot fit");
        assert_eq!(body_para(&l, FILLER + 2).0, usize::from(!auto_fit), "the explicit page break follows the box on its page");
        assert_eq!(body_para(&l, FILLER + 3).0, if auto_fit { 1 } else { 2 }, "the following text stays after the explicit break");
        (l.pages.len(), r, d, l)
    };
    let (pages, r, d, l) = doc(true);
    assert!(
        r.h > 3.0 * 12.0 && r.h < 80.0,
        "the box fits its three lines: {r:?}{}",
        super::pagination_fixture_diagnostics("inline_autofit/height", Some(&d), &l)
    );
    assert_eq!(
        pages,
        2,
        "the page break stays on the first page; filler={FILLER} box={r:?}{}",
        super::pagination_fixture_diagnostics("inline_autofit/pages", Some(&d), &l)
    );
    let (pages, saved, d, l) = doc(false);
    assert_eq!(
        pages,
        3,
        "a box keeping its 10 cm pushes the page break over; filler={FILLER} box={saved:?}{}",
        super::pagination_fixture_diagnostics("inline_fixed/pages", Some(&d), &l)
    );
    assert!(
        (saved.h - 283.46).abs() < 1.0,
        "without auto-fit the box keeps its saved height: {saved:?}{}",
        super::pagination_fixture_diagnostics("inline_fixed/height", Some(&d), &l)
    );
}

/// Filler paragraphs before the box in [`inline_box_that_resizes_to_fit_takes_its_texts_height`]:
/// enough that a 10 cm box no longer leaves room for the page break on the first page.
const FILLER: usize = 22;

/// An inline box that resizes to fit text it doesn't wrap is one line deep (#2191 with #2791's
/// inline fitting); with its text wrapping it is as deep as the wrapped lines.
#[test]
fn inline_box_fitting_text_it_doesnt_wrap_is_one_line_deep() {
    let caption = "(Figure 1.1) A time line of the important vaccines, on one line";
    let height = |no_wrap: bool| {
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, caption, 144.0, 283.0, TextBody { auto_fit: true, no_wrap, ..Default::default() });
        frame_of(&lay(&d), id).h
    };
    let [_, t, _, b] = TextBody::default().insets_pt();
    let one = height(true);
    assert!(one > t + b + 5.0 && one < t + b + 20.0, "one line deep: {one}");
    assert!(height(false) > one + 10.0, "wrapped, it is deeper: {} vs {one}", height(false));
}

/// Measuring text that doesn't wrap lays its nested boxes out once more than showing it does: the
/// measuring mustn't use up the nested-box budget, or the boxes after it show blank (#2191 review).
#[test]
fn measuring_text_that_doesnt_wrap_leaves_the_nested_box_budget() {
    let n = 40;
    assert!(n * 2 > wordcraft_doc::BoxBudget::MAX_NESTED && n < wordcraft_doc::BoxBudget::MAX_NESTED);
    let mut d = Document::from_text("x");
    let outer = text_box(&mut d, 0, "Boxes", 144.0, 144.0, TextBody { no_wrap: true, ..Default::default() });
    let inner: Vec<u32> = (0..n)
        .map(|i| {
            let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(&format!("n{i}"), Default::default()))]);
            let shape = InlineObject::Shape {
                kind: wordcraft_doc::para::ShapeKind::TextBox,
                w: 20.0,
                h: 20.0,
                fill: None,
                stroke: None,
                stroke_width: 1.0,
                float: Default::default(),
                story: Some(id),
                effects: Default::default(),
                freeform: None,
                extra: Default::default(),
            };
            d.insert_object(&Pos { story: StoryRef::Part(outer), path: Path::top(0), off: 0 }, shape, &Default::default()).unwrap();
            id
        })
        .collect();
    let l = lay(&d);
    let shown: Vec<u32> = inner.iter().copied().filter(|id| !lines(&l, *id).is_empty()).collect();
    assert_eq!(shown.len(), n, "every nested box shows its text");
}

/// A linked box whose text doesn't wrap runs its paragraphs on past its side too, and the next box
/// picks up at the next paragraph (#2191 review).
#[test]
fn a_linked_box_whose_text_doesnt_wrap_keeps_each_paragraph_on_one_line() {
    let mut d = Document::from_text("one\ntwo");
    let a = text_box(
        &mut d,
        0,
        "(Figure 1.1) A time line of the important vaccines, on one line",
        144.0,
        30.0,
        TextBody { no_wrap: true, ..Default::default() },
    );
    let b = text_box(&mut d, 1, "", 144.0, 200.0, TextBody::default());
    d.parts.get_mut(&a).unwrap().blocks.push(para_block(Paragraph::with_text("The next paragraph.", Default::default())));
    d.parts.get_mut(&a).unwrap().body.next = Some(b);
    let l = lay(&d);
    let first = l.pages[0].items.iter().find_map(|it| match it {
        Placed::Lines { story: StoryRef::Part(s), path, para, .. } if *s == a && path.0 == [0] => Some(para.lines.len()),
        _ => None,
    });
    assert_eq!(first, Some(1), "the caption stays on one line in the linked box");
}

/// A linked box whose text doesn't wrap, picking up partway through a paragraph the box before it
/// wrapped, carries on from the next line rather than showing that paragraph again from its start
/// (#2191 review).
#[test]
fn a_no_wrap_box_continuing_a_wrapped_paragraph_shows_only_the_rest() {
    let mut d = Document::from_text("one\ntwo");
    let text = "Words that run on well past the first little box. ".repeat(6);
    let a = text_box(&mut d, 0, &text, 144.0, 48.0, TextBody::default());
    let b = text_box(&mut d, 1, "", 144.0, 400.0, TextBody { no_wrap: true, ..Default::default() });
    let in_a = {
        let l = lay(&d);
        lines(&l, a).iter().map(|(.., l0, l1)| l1 - l0).sum::<usize>()
    };
    d.parts.get_mut(&a).unwrap().body.next = Some(b);
    let l = lay(&d);
    // Both boxes show the first box's story: the second frame is the no-wrap box.
    let frames: Vec<Rect> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { rect, text_box: Some(s), .. } if *s == a => Some(*rect),
            _ => None,
        })
        .collect();
    let rb = frames[1];
    let ls = lines(&l, a);
    let rest: Vec<_> = ls.iter().filter(|(_, y, ..)| *y >= rb.y).collect();
    assert_eq!(rest.first().map(|(.., l0, _)| *l0), Some(in_a), "carries on from the first line the first box couldn't show: {ls:?}");
}

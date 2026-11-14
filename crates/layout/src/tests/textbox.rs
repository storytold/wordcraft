//! Text boxes' text direction, alignment and linked boxes (Shape Format › Text).

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
    text_box_with(d, para, text, w, h, body, Default::default())
}

/// [`text_box`] placed as `float` says.
fn text_box_with(d: &mut Document, para: usize, text: &str, w: f32, h: f32, body: TextBody, float: wordcraft_doc::para::Float) -> u32 {
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
        stroke_width: 0.0,
        float,
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
    crate::test_fonts();
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
    crate::test_fonts();
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
    crate::test_fonts();
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
    crate::test_fonts();
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

/// A floating box that resizes to fit its text takes its text's height too, both where it is
/// drawn and the room the text beside it wraps around (an OpenDocument frame without a height).
#[test]
fn floating_box_that_resizes_to_fit_takes_its_texts_height() {
    use wordcraft_doc::para::{Float, Wrap};
    let rect = |auto_fit: bool, h: f32| {
        let mut d = Document::from_text("Beside the box the text runs on and on and on.");
        let id = text_box(&mut d, 0, "Inside", 144.0, h, TextBody { auto_fit, ..Default::default() });
        if let Some(Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut)
            && let Some(InlineObject::Shape { float, .. }) = p.objects.first_mut()
        {
            *float = Float { wrap: Wrap::Square, ..Default::default() };
        }
        frame_of(&lay(&d), id)
    };
    let fixed = rect(false, 1.0);
    assert!(fixed.h < 2.0, "a box that keeps its height: {fixed:?}");
    let grown = rect(true, 1.0);
    assert!(grown.h > 15.0 && grown.h < 30.0, "one line and the margins: {grown:?}");
    let shrunk = rect(true, 300.0);
    assert!((shrunk.h - grown.h).abs() < 0.5, "{shrunk:?} {grown:?}");
}

/// #4250: on a later page of its paragraph, a floating box that resizes to fit its text takes its
/// text's height too.
#[test]
fn a_floating_box_on_a_later_page_of_its_paragraph_fits_its_text() {
    let mut d = Document::from_text(&"The quick brown fox jumps over the lazy dog. ".repeat(400));
    let float = wordcraft_doc::para::Float { wrap: wordcraft_doc::para::Wrap::Square, ..Default::default() };
    let id = text_box_with(&mut d, 0, "Inside", 144.0, 1.0, TextBody { auto_fit: true, ..Default::default() }, float);
    let l = lay(&d);
    assert!(l.pages.len() >= 3, "pages {}", l.pages.len());
    let hit = l.find_object(0, |o| o.text_box == Some(id)).unwrap();
    assert!(hit.page > 0, "drawn on the page of its anchor");
    assert!(hit.rect.h > 15.0 && hit.rect.h < 30.0, "one line and the margins: {:?}", hit.rect);
}

#[test]
fn a_turned_linked_box_turns_only_its_own_lines() {
    // Linked boxes show the first box's story; turning the second box turns the lines it shows,
    // not the ones the first box shows (on screen and drawn item by item, as PDF export draws).
    let mut d = Document::from_text("one\ntwo");
    let text = "Words that run on well past the first little box. ".repeat(6);
    let a = text_box(&mut d, 0, &text, 144.0, 48.0, TextBody::default());
    let float = wordcraft_doc::para::Float { wrap: wordcraft_doc::para::Wrap::Square, x: 20.0, y: 300.0, rot: 30.0, ..Default::default() };
    let b = text_box_with(&mut d, 1, "", 144.0, 400.0, TextBody::default(), float);
    d.parts.get_mut(&a).unwrap().body.next = Some(b);
    let l = lay(&d);
    // (Words drawn turned, words drawn upright.)
    let turned_texts = |draws: &[display::Draw]| -> (bool, bool) {
        let mut inside = String::new();
        let mut outside = String::new();
        for x in draws {
            match x {
                display::Draw::Rotated { items, .. } => items.iter().for_each(|g| {
                    if let display::Draw::Glyphs { text, .. } = g {
                        inside.push_str(text)
                    }
                }),
                display::Draw::Glyphs { text, .. } => outside.push_str(text),
                _ => {}
            }
        }
        (inside.contains("Words"), outside.contains("Words"))
    };
    let page = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
    assert_eq!(turned_texts(&page), (true, true), "the first box's lines stay upright, the second's turn");
    let shown = display::ItemDisplay::new(&d, &l.pages[0]);
    let items: Vec<display::Draw> = l.pages[0].items.iter().flat_map(|it| shown.draws(it, &Default::default())).collect();
    assert_eq!(turned_texts(&items), (true, true));
}

#[test]
fn a_clipped_line_in_a_turned_box_is_drawn_once_turned_and_clipped() {
    // Lines in a cell cut off at its edge (an exact-height row) inside a turned text box: drawn
    // once, clipped, inside the box's turn (it was drawn upright and clipped, and again turned).
    let mut d = Document::from_text("one");
    let text = "Words in the box.";
    let float = wordcraft_doc::para::Float { wrap: wordcraft_doc::para::Wrap::Square, x: 20.0, y: 100.0, rot: 30.0, ..Default::default() };
    let b = text_box_with(&mut d, 0, text, 144.0, 200.0, TextBody::default(), float);
    let l = lay(&d);
    let mut page = l.pages[0].clone();
    let in_box: Vec<usize> = page
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| matches!(it, Placed::Lines { story: StoryRef::Part(s), .. } if *s == b))
        .map(|(i, _)| i)
        .collect();
    // Its lines taken as ones in a clipped cell (table 5, row 0, cell 0).
    let Some(Placed::Lines { path, .. }) = in_box.first().and_then(|&i| page.items.get_mut(i)) else { panic!("the box has no lines") };
    *path = wordcraft_doc::Path(vec![5, 0, 0, 0]);
    let rect = Rect::new(0.0, 0.0, 1000.0, 1000.0);
    page.items.push(Placed::Cell {
        rect,
        covered: false,
        table: wordcraft_doc::Path(vec![5]),
        row: 0,
        cell: 0,
        story: StoryRef::Part(b),
        clip: true,
    });
    let draws = display::page_display(&d, &page, &display::DisplayOptions::default());
    let upright_clips = draws.iter().filter(|x| matches!(x, display::Draw::Clip { .. })).count();
    let turned_clips: usize = draws
        .iter()
        .map(|x| match x {
            display::Draw::Rotated { items, .. } => items.iter().filter(|g| matches!(g, display::Draw::Clip { .. })).count(),
            _ => 0,
        })
        .sum();
    assert_eq!((upright_clips, turned_clips), (0, 1));
}

/// A `kind` shape `w` × `h` holding `lines` one-word paragraphs, inline in body paragraph 0; its
/// story id. `corner` is a rounded rectangle's (`roundRect`'s `adj`).
fn shape_with_text(
    d: &mut Document,
    kind: wordcraft_doc::para::ShapeKind,
    corner: Option<u32>,
    (w, h): (f32, f32),
    lines: usize,
    body: TextBody,
) -> u32 {
    let id = text_box(d, 0, "", w, h, body);
    d.parts.get_mut(&id).unwrap().blocks = (0..lines).map(|k| para_block(Paragraph::with_text(&format!("Line{k}"), Default::default()))).collect();
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    for o in &mut p.objects {
        if let InlineObject::Shape { kind: k, extra, .. } = o {
            *k = kind;
            extra.corner = corner;
        }
    }
    p.touch();
    id
}

/// Text in a shape is set in the shape's text rectangle, not its whole frame. Measured in Word
/// for Windows (16.0.20625) with a 300 × 100 pt inline shape, no margins, one line of text: in a
/// rounded rectangle it starts 1.46, 4.1, 7.3 and 14.6 pt in from the left and the top for `adj`
/// 5000, 13965, 25000 and 50000 (the corner radius × (1 - cos 45°); 70000 counts as 50000), in an
/// ellipse 43.9 and 14.6 pt, in a diamond 75 and 25 pt; right-aligned and bottom-anchored text
/// keeps the same distance from the right and the bottom, the box's own margins come on top, and
/// lines are that much shorter. A rounded rectangle that resizes to fit its text ends up as tall
/// as its text plus what the rectangle left above and below in the box as saved: saved 100 pt
/// tall with `adj` 50000 around three lines it is their height and 29.3 pt. WordCraft set the
/// text in the whole frame.
#[test]
fn text_in_a_shape_is_set_in_the_shapes_text_rectangle() {
    use wordcraft_doc::para::ShapeKind;
    crate::test_fonts();
    let zero = TextBody { inset: Some([0; 4]), ..Default::default() };
    // Where the first line starts in the frame, the frame, and how many lines show.
    let first = |kind: ShapeKind, corner: Option<u32>, size: (f32, f32), lines: usize, body: TextBody| {
        let mut d = Document::from_text("x");
        let id = shape_with_text(&mut d, kind, corner, size, lines, body);
        let l = lay(&d);
        let r = frame_of(&l, id);
        let all = self::lines(&l, id);
        let (x, y, ..) = *all.first().expect("the shape shows its text");
        ((x - r.x, y - r.y), r, all.iter().map(|(.., l0, l1)| l1 - l0).sum::<usize>())
    };
    let near = |got: (f32, f32), want: (f32, f32)| (got.0 - want.0).abs() < 0.02 && (got.1 - want.1).abs() < 0.02;
    let inset = 0.292_893_2f32;
    for (what, kind, corner, want) in [
        ("a rectangle", ShapeKind::Rectangle, None, (0.0, 0.0)),
        ("a text box", ShapeKind::TextBox, None, (0.0, 0.0)),
        ("a rounded rectangle, adj 5000", ShapeKind::RoundedRectangle, Some(5_000), (5.0 * inset, 5.0 * inset)),
        ("a rounded rectangle, adj 13965", ShapeKind::RoundedRectangle, Some(13_965), (13.965 * inset, 13.965 * inset)),
        ("a rounded rectangle, adj 50000", ShapeKind::RoundedRectangle, Some(50_000), (50.0 * inset, 50.0 * inset)),
        ("a rounded rectangle, adj 70000", ShapeKind::RoundedRectangle, Some(70_000), (50.0 * inset, 50.0 * inset)),
        ("an ellipse", ShapeKind::Ellipse, None, (150.0 * inset, 50.0 * inset)),
        ("a diamond", ShapeKind::Diamond, None, (75.0, 25.0)),
    ] {
        let (at, ..) = first(kind, corner, (300.0, 100.0), 1, zero);
        assert!(near(at, want), "{what}: the text starts {at:?} into the frame, expected {want:?}");
    }
    // The shorter side sets a rounded rectangle's corner, and the box's own margins come on top.
    let (at, ..) = first(ShapeKind::RoundedRectangle, Some(50_000), (60.0, 150.0), 1, zero);
    assert!(near(at, (30.0 * inset, 30.0 * inset)), "60 x 150: {at:?}");
    let margins = TextBody { inset: Some([91_440, 45_720, 91_440, 45_720]), ..Default::default() };
    let (at, ..) = first(ShapeKind::RoundedRectangle, Some(50_000), (300.0, 100.0), 1, margins);
    assert!(near(at, (50.0 * inset + 7.2, 50.0 * inset + 3.6)), "with the default margins: {at:?}");
    // Middle and bottom: aligned in the text rectangle, the same distance from the bottom.
    let (top, ..) = first(ShapeKind::Rectangle, None, (300.0, 100.0), 1, TextBody { anchor: VAlign::Bottom, ..zero });
    let (low, ..) = first(ShapeKind::RoundedRectangle, Some(50_000), (300.0, 100.0), 1, TextBody { anchor: VAlign::Bottom, ..zero });
    assert!((top.1 - low.1 - 50.0 * inset).abs() < 0.02, "bottom-anchored: {low:?} in the rounded rectangle, {top:?} in the rectangle");
    let (mid, ..) = first(ShapeKind::Rectangle, None, (300.0, 100.0), 1, TextBody { anchor: VAlign::Center, ..zero });
    let (round, ..) = first(ShapeKind::RoundedRectangle, Some(50_000), (300.0, 100.0), 1, TextBody { anchor: VAlign::Center, ..zero });
    assert!((mid.1 - round.1).abs() < 0.02, "centred: {round:?} in the rounded rectangle, {mid:?} in the rectangle");
    // Text taller than the text rectangle starts at its top, and no line that showed in the
    // whole frame is hidden: the cut stays as deep as the frame allows.
    let (_, _, plain) = first(ShapeKind::Rectangle, None, (300.0, 100.0), 30, TextBody { anchor: VAlign::Center, ..zero });
    let (at, _, shown) = first(ShapeKind::RoundedRectangle, Some(50_000), (300.0, 100.0), 30, TextBody { anchor: VAlign::Center, ..zero });
    assert!(plain > 2 && plain < 30, "some of the 30 lines fit the rectangle: {plain}");
    assert!(near(at, (50.0 * inset, 50.0 * inset)), "text taller than the text rectangle starts at its top: {at:?}");
    assert_eq!(shown, plain, "as many lines show as in the rectangle");
    // Resize to fit: the text's height and the room the rectangle left in the box as saved.
    let fit = TextBody { auto_fit: true, ..zero };
    let (_, plain, _) = first(ShapeKind::Rectangle, None, (300.0, 100.0), 3, fit);
    for (saved, corner) in [(100.0f32, 50_000u32), (20.0, 50_000), (100.0, 13_965)] {
        let ((_, top), round, shown) = first(ShapeKind::RoundedRectangle, Some(corner), (300.0, saved), 3, fit);
        let room = 2.0 * saved * corner as f32 / 100_000.0 * inset;
        assert_eq!(shown, 3, "saved {saved} pt tall, adj {corner}: all three lines show");
        assert!(
            (round.h - plain.h - room).abs() < 0.02,
            "saved {saved} pt tall, adj {corner}: {} pt tall, the text needs {}, expected {room} more",
            round.h,
            plain.h
        );
        // In the fitted box the text starts where its new size puts the text rectangle.
        let want = round.h.min(300.0) * corner as f32 / 100_000.0 * inset;
        assert!((top - want).abs() < 0.02, "saved {saved} pt tall, adj {corner}: the text starts {top} pt down, expected {want}");
    }
}

/// A shape that resizes to fit its text is as tall as the text plus the room its text rectangle
/// left above and below it in the box as saved, and the text then starts where the fitted box's
/// own text rectangle does: the two are not the same rectangle. Measured in Word for Windows
/// (16.0.20625) with 300 pt wide shapes without margins around three 11.5 pt lines: a diamond
/// saved 20 pt tall comes out 44.5 pt tall (the text and twice 20 / 4) with its text 11.1 pt down
/// (44.5 / 4), saved 100 pt tall 84.5 pt and 21.1 pt; an ellipse saved 20 pt tall 40.4 pt and 5.9
/// pt. Stacked text is set in the whole frame, so its box is fitted without the rectangle too.
#[test]
fn a_fitted_shape_keeps_the_room_its_saved_text_rectangle_left() {
    use wordcraft_doc::para::ShapeKind;
    crate::test_fonts();
    let fit = TextBody { inset: Some([0; 4]), auto_fit: true, ..Default::default() };
    // The fitted frame and how far down it the first line starts.
    let fitted = |kind: ShapeKind, corner: Option<u32>, saved: f32, body: TextBody| {
        let mut d = Document::from_text("x");
        let id = shape_with_text(&mut d, kind, corner, (300.0, saved), 3, body);
        let l = lay(&d);
        let r = frame_of(&l, id);
        let top = self::lines(&l, id).first().map(|(_, y, ..)| y - r.y).expect("the shape shows its text");
        (r.h, top)
    };
    let inset = 0.292_893_2f32;
    let (text, _) = fitted(ShapeKind::Rectangle, None, 100.0, fit);
    for (what, kind, saved, room, share) in [
        ("a diamond saved 20 pt tall", ShapeKind::Diamond, 20.0f32, 2.0 * 20.0 / 4.0, 0.25),
        ("a diamond saved 100 pt tall", ShapeKind::Diamond, 100.0, 2.0 * 100.0 / 4.0, 0.25),
        ("an ellipse saved 20 pt tall", ShapeKind::Ellipse, 20.0, 20.0 * inset, inset / 2.0),
        ("an ellipse saved 100 pt tall", ShapeKind::Ellipse, 100.0, 100.0 * inset, inset / 2.0),
    ] {
        let (h, top) = fitted(kind, None, saved, fit);
        assert!((h - text - room).abs() < 0.02, "{what}: {h} pt tall, the text needs {text}, expected {room} more");
        assert!((top - h * share).abs() < 0.02, "{what}: the text starts {top} pt down the fitted box, expected {}", h * share);
    }
    // Stacked text: no shape's rectangle where it is placed, so none in the fitted height.
    let stacked = TextBody { vert: TextVert::Stacked, ..fit };
    let (plain, _) = fitted(ShapeKind::Rectangle, None, 100.0, stacked);
    let (round, top) = fitted(ShapeKind::RoundedRectangle, Some(50_000), 100.0, stacked);
    let (_, plain_top) = fitted(ShapeKind::Rectangle, None, 100.0, stacked);
    assert!((round - plain).abs() < 0.02, "stacked text: a rounded rectangle is fitted {round} pt tall, a rectangle {plain}");
    assert!((top - plain_top).abs() < 0.02, "stacked text starts {top} pt down the rounded rectangle, {plain_top} down the rectangle");
}

/// Text in a shape keeps clear of half its outline's width on every side, whether the outline is
/// drawn or not. Measured in Word for Windows (16.0.20625) with 300 pt wide rectangles, rounded
/// rectangles and ellipses: with an outline of 0.5, 3 and 6 pt the first line starts 0.25, 1.5
/// and 3 pt further in from the left and further down than with an outline without a width (in
/// the PDF 0.24, 1.44 and 3.0 pt), right-aligned lines end as much short of the right edge and
/// bottom-anchored text sits as much above the bottom; a 3 pt outline whose fill is taken off
/// (`a:noFill` under an `a:ln` that states its width) takes the same 1.5 pt. A shape that resizes
/// to fit three 11.5 pt lines is taller by the whole width: 34.5 pt without an outline, 35.0,
/// 37.5 and 40.5 pt with one of 0.5, 3 and 6 pt. WordCraft set the text against the frame's
/// edge, under the outline's inner half.
#[test]
fn text_in_a_shape_keeps_clear_of_half_its_outline() {
    use wordcraft_doc::para::ShapeKind;
    use wordcraft_doc::props::Rgb;
    crate::test_fonts();
    let zero = TextBody { inset: Some([0; 4]), ..Default::default() };
    // Where the first of three lines starts in the frame, how long a line may be, and the frame.
    let first = |kind: ShapeKind, corner: Option<u32>, outline: (Option<Rgb>, f32), body: TextBody| {
        let mut d = Document::from_text("x");
        let id = shape_with_text(&mut d, kind, corner, (300.0, 100.0), 3, body);
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        for o in &mut p.objects {
            if let InlineObject::Shape { stroke, stroke_width, .. } = o {
                (*stroke, *stroke_width) = outline;
            }
        }
        p.touch();
        let l = lay(&d);
        let r = frame_of(&l, id);
        let (at, len) = l.pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Lines { story: StoryRef::Part(s), para, x, y, l0, .. } if *s == id => {
                    let line = para.lines.get(*l0)?;
                    Some(((x - r.x, y - r.y), line.right - line.left))
                }
                _ => None,
            })
            .expect("the shape shows its text");
        (at, len, r)
    };
    let near = |a: f32, b: f32| (a - b).abs() < 0.02;
    let none = (None, 0.0);
    let drawn = |w: f32| (Some(Rgb::BLACK), w);
    for (what, kind, corner) in [
        ("a rectangle", ShapeKind::Rectangle, None),
        ("a text box", ShapeKind::TextBox, None),
        ("a rounded rectangle", ShapeKind::RoundedRectangle, Some(50_000)),
        ("an ellipse", ShapeKind::Ellipse, None),
    ] {
        let ((x0, y0), len0, _) = first(kind, corner, none, zero);
        for (outline, w) in [(drawn(0.5), 0.5f32), (drawn(3.0), 3.0), (drawn(6.0), 6.0), ((None, 3.0), 3.0)] {
            let ((x, y), len, _) = first(kind, corner, outline, zero);
            assert!(
                near(x - x0, w / 2.0) && near(y - y0, w / 2.0),
                "{what}, outline {outline:?}: the text starts ({x}, {y}) into the frame, without an outline ({x0}, {y0})"
            );
            assert!(near(len0 - len, w), "{what}, outline {outline:?}: lines are {len} pt long, without an outline {len0}");
        }
    }
    // The box's own margins come on top, as they do on the shape's text rectangle.
    let margins = TextBody { inset: Some([91_440, 45_720, 91_440, 45_720]), ..Default::default() };
    let ((x, y), len, _) = first(ShapeKind::Rectangle, None, drawn(3.0), margins);
    assert!(near(x, 7.2 + 1.5) && near(y, 3.6 + 1.5) && near(len, 300.0 - 14.4 - 3.0), "with the default margins: ({x}, {y}), lines {len} pt long");
    // Bottom-anchored text sits half the width above the bottom, centred text stays centred.
    let low = TextBody { anchor: VAlign::Bottom, ..zero };
    let (((_, y0), ..), ((_, y), ..)) = (first(ShapeKind::Rectangle, None, none, low), first(ShapeKind::Rectangle, None, drawn(6.0), low));
    assert!(near(y0 - y, 3.0), "bottom-anchored: {y} pt down with a 6 pt outline, {y0} without");
    let mid = TextBody { anchor: VAlign::Center, ..zero };
    let (((_, y0), ..), ((_, y), ..)) = (first(ShapeKind::Rectangle, None, none, mid), first(ShapeKind::Rectangle, None, drawn(6.0), mid));
    assert!(near(y0, y), "centred: {y} pt down with a 6 pt outline, {y0} without");
    // Resize to fit: taller by the whole width, the text half of it down.
    let fit = TextBody { auto_fit: true, ..zero };
    let (_, _, plain) = first(ShapeKind::Rectangle, None, none, fit);
    for (outline, w) in [(drawn(0.5), 0.5f32), (drawn(3.0), 3.0), (drawn(6.0), 6.0), ((None, 3.0), 3.0)] {
        let ((_, y), _, r) = first(ShapeKind::Rectangle, None, outline, fit);
        assert!(near(r.h - plain.h, w), "fitted, outline {outline:?}: {} pt tall, without an outline {}", r.h, plain.h);
        assert!(near(y, w / 2.0), "fitted, outline {outline:?}: the text starts {y} pt down");
    }
    // A width that is no width takes no room, and one wider than the shape still lays out.
    for w in [f32::NAN, f32::INFINITY, -4.0] {
        let ((x, y), len, _) = first(ShapeKind::Rectangle, None, (Some(Rgb::BLACK), w), zero);
        assert!(near(x, 0.0) && near(y, 0.0) && near(len, 300.0), "an outline {w} pt wide: ({x}, {y}), lines {len} pt long");
    }
    let (_, len, r) = first(ShapeKind::Rectangle, None, drawn(1.0e9), zero);
    assert!(len.is_finite() && r.h.is_finite(), "an outline wider than the shape: lines {len} pt long in {r:?}");
}

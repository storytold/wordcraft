//! Text boxes' text direction, alignment and linked boxes (Shape Format › Text).

#[allow(unused_imports)]
use super::*;
use wordcraft_doc::props::{TextVert, VAlign};
use wordcraft_doc::{PartKind, Pos, TextBody, para_block};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// The bundled font the documents of the boxes fitted to their text are set in, so their text's
/// widths and heights don't depend on the fonts a machine has installed.
const TEST_FONT: &str = "Source Sans 3";

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
        let mut d = super::tests::pagination_fixture_document(&format!("{}\n\n\n{brk}\nSecond page text.", filler.join("\n")));
        // Exact spacing is only for the filler/short text. The object paragraph must retain
        // automatic spacing so its inline box really contributes its physical height.
        d.para_mut(StoryRef::Body, &Path::top(FILLER)).unwrap().props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.15));
        let probe = lay(&d);
        super::tests::assert_pagination_fixture_font(&probe);
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
        super::tests::assert_pagination_fixture_font(&l);
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
        "the box fits its three lines with the last one's space after, its margins and outline: {r:?}{}",
        super::tests::pagination_fixture_diagnostics("inline_autofit/height", Some(&d), &l)
    );
    assert_eq!(
        pages,
        2,
        "the page break stays on the first page; filler={FILLER} box={r:?}{}",
        super::tests::pagination_fixture_diagnostics("inline_autofit/pages", Some(&d), &l)
    );
    let (pages, saved, d, l) = doc(false);
    assert_eq!(
        pages,
        3,
        "a box keeping its 10 cm pushes the page break over; filler={FILLER} box={saved:?}{}",
        super::tests::pagination_fixture_diagnostics("inline_fixed/pages", Some(&d), &l)
    );
    assert!(
        (saved.h - 283.46).abs() < 1.0,
        "without auto-fit the box keeps its saved height: {saved:?}{}",
        super::tests::pagination_fixture_diagnostics("inline_fixed/height", Some(&d), &l)
    );
}

/// Filler paragraphs before the box in [`inline_box_that_resizes_to_fit_takes_its_texts_height`]:
/// enough that a 10 cm box no longer leaves room for the page break on the first page.
const FILLER: usize = 22;

/// A paragraph of exact 17 pt lines with `before` and `after` points of space around it.
fn exact(text: &str, before: f32, after: f32) -> Paragraph {
    let mut p = Paragraph::with_text(text, Default::default());
    p.props = wordcraft_doc::props::ParaProps {
        space_before: Some(before),
        space_after: Some(after),
        line_spacing: Some(wordcraft_doc::props::LineSpacing::Exactly(17.0)),
        ..Default::default()
    };
    p
}

/// Word's margins with a 7.09 pt bottom one (`bIns="90000"`), resizing to fit the text.
const FIT: TextBody = TextBody {
    vert: TextVert::Horz,
    anchor: VAlign::Top,
    next: None,
    inset: Some([91_440, 45_720, 91_440, 90_000]),
    auto_fit: true,
    auto_color: None,
};

/// 20 pt below the top of its paragraph, at the left margin, text only above and below it.
fn below_para() -> wordcraft_doc::para::Float {
    wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::TopAndBottom,
        h_rel: wordcraft_doc::para::Anchor::Margin,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        y: 20.0,
        ..Default::default()
    }
}

/// `ZZZ`, `AAA` and `BBB` (`AAA` as `aaa` says) in exact 17 pt lines, `ZZZ` anchoring a text box
/// `w` × `h` with an outline `stroke` wide showing `paras`, placed as `float`, in [`TEST_FONT`];
/// and its story.
fn anchored(paras: Vec<Paragraph>, (w, h): (f32, f32), stroke: f32, body: TextBody, float: wordcraft_doc::para::Float, aaa: &str) -> (Document, u32) {
    let mut d = Document::new();
    d.styles.default_chr.font = Some(TEST_FONT.into());
    d.body = vec![para_block(exact("ZZZ", 0.0, 0.0)), para_block(exact(aaa, 0.0, 0.0)), para_block(exact("BBB", 0.0, 0.0))];
    let id = d.add_part(PartKind::TextBox, paras.into_iter().map(para_block).collect());
    d.parts.get_mut(&id).unwrap().body = body;
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w,
        h,
        fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(0xC0, 0xC0, 0xC0))),
        stroke: None,
        stroke_width: stroke,
        float,
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 3), shape, &Default::default()).unwrap();
    (d, id)
}

/// Where body paragraph `i`'s first line starts (page, top).
fn para_top(l: &DocLayout, i: u32) -> (usize, f32) {
    l.pages
        .iter()
        .enumerate()
        .find_map(|(pi, p)| {
            p.items.iter().find_map(|it| match it {
                Placed::Lines { story: StoryRef::Body, path, para, l0, y, .. } if *path == Path::top(i as usize) => {
                    para.lines.get(*l0).map(|_| (pi, *y))
                }
                _ => None,
            })
        })
        .unwrap()
}

fn three() -> Vec<Paragraph> {
    vec![exact("BOXA", 0.0, 0.0), exact("BOXB", 0.0, 0.0), exact("BOXC", 0.0, 0.0)]
}

/// Word for Mac, 300 pt wide boxes 20 pt below their paragraph, 3.6 / 7.09 pt margins, 17 pt
/// lines: (paragraphs, stored height, outline width, Word's drawn height). The space before
/// the first paragraph and after the last count, an empty last paragraph too; the outline
/// counts whole, drawn or not; a box stored lower than its margins and outline (11.19 pt)
/// adds only its stored height to the text.
#[test]
fn a_box_that_fits_its_text_is_as_tall_as_its_text_margins_and_outline() {
    let one = || vec![exact("BOXA", 0.0, 0.0)];
    let no_margins = TextBody { inset: Some([0; 4]), ..FIT };
    let cases: Vec<(Vec<Paragraph>, f32, f32, TextBody, f32)> = vec![
        (one(), 100.0, 0.5, FIT, 28.2),
        (vec![exact("BOXA", 0.0, 10.0)], 100.0, 0.5, FIT, 38.2),
        (vec![exact("BOXA", 6.0, 0.0)], 100.0, 0.5, FIT, 34.2),
        (vec![exact("BOXA", 0.0, 0.0), exact("", 0.0, 0.0)], 100.0, 0.5, FIT, 45.2),
        (vec![exact("BOXA", 0.0, 0.0), exact("BOXB", 0.0, 0.0)], 100.0, 0.5, FIT, 45.2),
        (one(), 100.0, 0.0, FIT, 27.7),
        (one(), 100.0, 2.0, FIT, 29.7),
        (one(), 100.0, 0.5, no_margins, 17.5),
        (one(), 100.0, 0.0, no_margins, 17.0),
        (three(), 5.0, 0.5, FIT, 56.0),
        (three(), 10.0, 0.5, FIT, 61.0),
        (three(), 11.0, 0.5, FIT, 62.0),
        (three(), 11.5, 0.5, FIT, 62.2),
        (three(), 20.0, 0.5, FIT, 62.2),
        (three(), 63.0, 0.5, FIT, 62.2),
        (three(), 10.0, 0.0, FIT, 61.0),
        (three(), 20.0, 0.0, FIT, 61.7),
        (three(), 10.0, 0.5, no_margins, 51.5),
        (three(), 40.0, 0.0, no_margins, 51.0),
        (vec![exact("BOXA", 0.0, 0.0), exact("BOXB", 0.0, 0.0), exact("BOXC", 0.0, 10.0)], 10.0, 0.5, FIT, 71.0),
    ];
    for (k, (paras, h, stroke, body, want)) in cases.into_iter().enumerate() {
        for dist_bottom in [0.0, 9.0] {
            let (d, id) = anchored(paras.clone(), (300.0, h), stroke, body, wordcraft_doc::para::Float { dist_bottom, ..below_para() }, "AAA");
            let l = lay(&d);
            let r = frame_of(&l, id);
            let (_, zzz) = para_top(&l, 0);
            assert!((r.h - want).abs() < 0.05, "case {k}: the box is {} high, Word {want}", r.h);
            assert!((r.y - (zzz + 20.0)).abs() < 0.01, "case {k}: its top stays 20 pt below its paragraph: {} / {zzz}", r.y);
            let (_, aaa) = para_top(&l, 1);
            assert!(
                (aaa - (r.bottom() + dist_bottom)).abs() < 0.05,
                "case {k}: the text after it starts {dist_bottom} pt below its bottom {}: {aaa}",
                r.bottom()
            );
            let shown = lines(&l, id).iter().map(|(.., l0, l1)| l1 - l0).sum::<usize>();
            assert_eq!(shown, paras.len(), "case {k}: all its text shows");
        }
    }
}

/// Square wrapping: a box as wide as the text sends the text below its fitted bottom; beside
/// a 200 pt box the text runs on in the strip down to it (Word: two lines, the third at the
/// margin 51 pt below `ZZZ`, not below the stored 100 pt).
#[test]
fn text_wraps_around_the_box_fitted_to_its_text() {
    let square = wordcraft_doc::para::Float { wrap: wordcraft_doc::para::Wrap::Square, dist: 9.0, ..below_para() };
    let one = vec![exact("BOXA", 0.0, 0.0)];
    let (d, id) = anchored(one.clone(), (600.0, 100.0), 0.5, FIT, square.clone(), "AAA");
    let l = lay(&d);
    let r = frame_of(&l, id);
    assert!((r.h - 28.2).abs() < 0.05, "{r:?}");
    assert!((para_top(&l, 1).1 - r.bottom()).abs() < 0.05, "the text starts below the fitted box");
    let long = format!("AAA {}", ["wrd"; 120].join(" "));
    let (d, id) = anchored(one, (200.0, 100.0), 0.5, FIT, square, &long);
    let l = lay(&d);
    let r = frame_of(&l, id);
    let (_, zzz) = para_top(&l, 0);
    let rows: Vec<(f32, f32)> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, .. } if *path == Path::top(1) => {
                let first = para.lines.get(*l0)?;
                Some(
                    (*l0..*l1)
                        .filter_map(|k| para.lines.get(k).and_then(|ln| Some((x + para.x_of(k, ln.start)?, y + ln.top - first.top))))
                        .collect::<Vec<_>>(),
                )
            }
            _ => None,
        })
        .flatten()
        .collect();
    let margin_x = rows.iter().map(|(x, _)| *x).fold(f32::MAX, f32::min);
    let beside = rows.iter().filter(|(x, _)| *x > r.right()).count();
    let first_full = rows.iter().find(|(x, _)| (*x - margin_x).abs() < 1.0).map(|(_, y)| *y).unwrap();
    assert_eq!(beside, 2, "two lines beside the box: {rows:?}");
    assert!((first_full - (zzz + 51.0)).abs() < 0.05, "the third line goes back to the margin below the fitted box: {first_full} ({zzz})");
}

/// Word keeps the stored size without Resize shape to fit text (`a:noAutofit` or no
/// autofit element), and cuts what doesn't fit.
#[test]
fn boxes_that_do_not_fit_their_text_keep_their_size() {
    let keep = TextBody { auto_fit: false, ..FIT };
    for h in [100.0, 10.0] {
        let (d, id) = anchored(three(), (300.0, h), 0.5, keep, below_para(), "AAA");
        let l = lay(&d);
        let r = frame_of(&l, id);
        assert_eq!(r.h, h);
        assert!((para_top(&l, 1).1 - r.bottom()).abs() < 0.05);
    }
    let (mut d, id) = anchored(three(), (300.0, 100.0), 0.5, FIT, below_para(), "AAA");
    let other = text_box(&mut d, 2, "", 300.0, 50.0, TextBody::default());
    d.parts.get_mut(&id).unwrap().body.next = Some(other);
    let l = lay(&d);
    let mut heights: Vec<f32> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { rect, text_box: Some(s), .. } if *s == id => Some(rect.h),
            _ => None,
        })
        .collect();
    heights.sort_by(f32::total_cmp);
    assert_eq!(heights, [50.0, 100.0], "linked boxes keep their sizes");
    let (d, id) = anchored(three(), (300.0, 100.0), 0.5, TextBody { vert: TextVert::Stacked, ..FIT }, below_para(), "AAA");
    assert_eq!(frame_of(&lay(&d), id).h, 100.0, "stacked text keeps the box's size");
}

/// Word: aligned to the bottom of the margin, the fitted box ends there; centred on the page,
/// its middle is the page's; placed at an offset, its top stays, whatever its text's anchor.
#[test]
fn a_box_fitted_to_its_text_is_aligned_and_placed_with_its_new_size() {
    use wordcraft_doc::para::{Anchor, FloatAlign, Wrap};
    let over = |v_rel, v_align, y, anchor| {
        let float = wordcraft_doc::para::Float { wrap: Wrap::InFrontOfText, v_rel, v_align, y, ..below_para() };
        let (d, id) = anchored(vec![exact("BOXA", 0.0, 0.0)], (300.0, 100.0), 0.5, TextBody { anchor, ..FIT }, float, "AAA");
        let s = d.last_section.clone();
        (frame_of(&lay(&d), id), s)
    };
    for anchor in [VAlign::Top, VAlign::Bottom] {
        let (r, s) = over(Anchor::Margin, Some(FloatAlign::End), 0.0, anchor);
        assert!((r.h - 28.2).abs() < 0.05 && (r.bottom() - (s.page_h - s.margin_bottom)).abs() < 0.01, "{r:?}");
    }
    let (r, s) = over(Anchor::Page, Some(FloatAlign::Center), 0.0, VAlign::Top);
    assert!((r.y + r.h / 2.0 - s.page_h / 2.0).abs() < 0.01, "{r:?}");
    for anchor in [VAlign::Top, VAlign::Center, VAlign::Bottom] {
        let (r, _) = over(Anchor::Page, None, 400.0, anchor);
        assert!((r.y - 400.0).abs() < 0.01 && (r.h - 28.2).abs() < 0.05, "{r:?}");
    }
}

/// Word: an inline box stored 100 pt high with one 17 pt line is drawn 28.2 pt high, and its
/// line is as tall as a box of that size makes it; one stored 10 pt high with three is 61 pt.
#[test]
fn an_inline_box_fitted_to_its_text_sets_its_lines_height() {
    let inline = |h: f32, body: TextBody| {
        let (d, id) = anchored(three().into_iter().take(if h < 11.0 { 3 } else { 1 }).collect(), (300.0, h), 0.5, body, Default::default(), "AAA");
        let l = lay(&d);
        (frame_of(&l, id), para_top(&l, 1).1 - para_top(&l, 0).1)
    };
    let (fitted, after_fitted) = inline(100.0, FIT);
    assert!((fitted.h - 28.2).abs() < 0.05, "{fitted:?}");
    let (stored, after_stored) = inline(fitted.h, TextBody { auto_fit: false, ..FIT });
    assert_eq!(stored.h, fitted.h);
    assert!((after_fitted - after_stored).abs() < 0.01, "the line holds the fitted box: {after_fitted} / {after_stored}");
    let (grown, _) = inline(10.0, FIT);
    assert!((grown.h - 61.0).abs() < 0.05, "three lines in a box stored 10 pt high: {grown:?}");
}

/// Word: a box whose text runs down or up the page keeps its height and takes its text's
/// width (17 pt lines, 7.2 pt left and right margins, 0.5 pt outline), from its left edge.
#[test]
fn turned_text_fits_the_boxs_width() {
    for (vert, paras, w, want) in [(TextVert::Vert, 1, 100.0, 31.9), (TextVert::Vert270, 1, 100.0, 31.9), (TextVert::Vert, 3, 10.0, 61.0)] {
        let paras: Vec<Paragraph> = three().into_iter().take(paras).collect();
        let (d, id) = anchored(paras, (w, 100.0), 0.5, TextBody { vert, ..FIT }, below_para(), "AAA");
        let l = lay(&d);
        let r = frame_of(&l, id);
        let (_, zzz) = para_top(&l, 0);
        assert!((r.w - want).abs() < 0.05 && r.h == 100.0, "{vert:?}: {r:?}, Word {want} wide");
        assert!((r.x - d.last_section.margin_left).abs() < 0.01 && (r.y - (zzz + 20.0)).abs() < 0.01, "{r:?}");
    }
}

#[test]
fn hostile_sizes_of_boxes_fitted_to_their_text_still_lay_out() {
    for (w, h, stroke, inset) in [(f32::NAN, f32::NAN, f32::NAN, [u32::MAX; 4]), (1e30, -1e30, 1e30, [0; 4]), (0.0, 0.0, -5.0, [7; 4])] {
        let (d, id) = anchored(three(), (w, h), stroke, TextBody { inset: Some(inset), ..FIT }, below_para(), "AAA");
        let r = frame_of(&lay(&d), id);
        assert!(r.h.is_finite() && r.h >= 1.0 && r.h <= 4000.0, "{r:?}");
        let (d, id) = anchored(three(), (w, h), stroke, TextBody { inset: Some(inset), ..FIT }, Default::default(), "AAA");
        assert!(frame_of(&lay(&d), id).h.is_finite());
    }
}

/// `k` paragraphs of exact 17 pt lines on an A4 page with Word's synthetic margins (28.35 pt top,
/// 42.55 pt bottom: the text ends at 799.35), then `ZZZ` anchoring a text box 480 pt wide
/// stored `h` high, `y` below the paragraph, wrapped as `wrap`, then `AAA` and `BBB`; laid out
/// in compatibility mode `mode`, in [`TEST_FONT`]. The page and frame of the box.
fn near_bottom(k: usize, y: f32, h: f32, wrap: wordcraft_doc::para::Wrap, dist_bottom: f32, body: TextBody, mode: u32) -> (usize, Rect, f32) {
    let mut d = Document::new();
    d.styles.default_chr.font = Some(TEST_FONT.into());
    d.settings.compat_mode = mode;
    d.last_section.page_w = 595.3;
    d.last_section.page_h = 841.9;
    d.last_section.margin_top = 28.35;
    d.last_section.margin_bottom = 42.55;
    d.last_section.header = 0.0;
    d.last_section.footer = 0.0;
    let mut blocks: Vec<Block> = (0..k).map(|i| Block::Para(exact(&format!("L{i}"), 0.0, 0.0))).collect();
    blocks.extend([Block::Para(exact("ZZZ", 0.0, 0.0)), Block::Para(exact("AAA", 8.5, 0.0)), Block::Para(exact("BBB", 0.0, 0.0))]);
    d.body = blocks.into_iter().map(Arc::new).collect();
    let id = d.add_part(PartKind::TextBox, vec![para_block(exact("BOXA", 0.0, 0.0))]);
    d.parts.get_mut(&id).unwrap().body = body;
    let float = wordcraft_doc::para::Float { wrap, y, dist_bottom, ..below_para() };
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 480.0,
        h,
        fill: None,
        stroke: None,
        stroke_width: 0.5,
        float,
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(k, 3), shape, &Default::default()).unwrap();
    let l = lay(&d);
    let (page, rect) = l
        .pages
        .iter()
        .enumerate()
        .find_map(|(pi, pg)| {
            pg.items.iter().find_map(|it| match it {
                Placed::Object { rect, text_box: Some(s), .. } if *s == id => Some((pi, *rect)),
                _ => None,
            })
        })
        .unwrap();
    let line = l.pages[page]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, path, y, .. } if path.0.first() == Some(&(k as u32)) => Some(*y),
            _ => None,
        })
        .unwrap_or(f32::NAN);
    (page, rect, line)
}

/// Word, from its 2013 layout on (compatibility mode 15): a box placed relative to its paragraph
/// with text wrapping around it that would reach below the bottom margin moves up to end there,
/// but not above the bottom of the paragraph's first line; if it still doesn't fit, the paragraph
/// starts the next page with it (at the top of a page the box stays below the line, however
/// tall). Not before Word 2013 layout, not for a box the text doesn't wrap around, and the
/// distance from text below doesn't count.
#[test]
fn a_box_anchored_near_the_page_bottom_stays_above_it_in_word_2013_layout() {
    use wordcraft_doc::para::Wrap;
    let fixed = TextBody { auto_fit: false, ..FIT };
    let at = |k, y, h, wrap, dist, body, mode| {
        let (page, r, _) = near_bottom(k, y, h, wrap, dist, body, mode);
        (page, (r.y * 100.0).round() / 100.0, (r.h * 10.0).round() / 10.0)
    };
    let cases = [
        ((42, 27.15, 26.2, Wrap::Square, 0.0, fixed, 15), (0, 769.5, 26.2)),
        ((42, 30.3, 26.2, Wrap::Square, 0.0, fixed, 15), (0, 772.65, 26.2)),
        ((42, 31.3, 26.2, Wrap::Square, 0.0, fixed, 15), (0, 773.15, 26.2)),
        ((42, 27.15, 26.2, Wrap::Square, 9.0, fixed, 15), (0, 769.5, 26.2)),
        ((43, 27.15, 26.2, Wrap::Square, 0.0, fixed, 15), (1, 55.5, 26.2)),
        ((44, 27.15, 26.2, Wrap::Square, 0.0, fixed, 15), (1, 55.5, 26.2)),
        ((43, 27.15, 26.2, Wrap::TopAndBottom, 0.0, fixed, 15), (1, 55.5, 26.2)),
        ((43, 27.15, 41.2, Wrap::Square, 0.0, FIT, 15), (1, 55.5, 28.2)),
        ((43, 27.15, 26.2, Wrap::InFrontOfText, 0.0, fixed, 15), (0, 786.5, 26.2)),
        ((43, 27.15, 26.2, Wrap::Square, 0.0, fixed, 14), (0, 786.5, 26.2)),
        ((44, 27.15, 26.2, Wrap::Square, 0.0, fixed, 12), (0, 803.5, 26.2)),
        ((0, 27.15, 900.0, Wrap::Square, 0.0, fixed, 15), (0, 45.35, 900.0)),
        ((30, 27.15, 900.0, Wrap::Square, 0.0, fixed, 15), (1, 45.35, 900.0)),
    ];
    for ((k, y, h, wrap, dist, body, mode), want) in cases {
        assert_eq!(at(k, y, h, wrap, dist, body, mode), want, "{k} paragraphs, {y} pt below, {h} high, {wrap:?}, mode {mode}");
    }
    // The tall box lifted to the bottom of its paragraph's line leaves that line at the top.
    for k in [0, 30] {
        let (_, _, line) = near_bottom(k, 27.15, 900.0, Wrap::Square, 0.0, fixed, 15);
        assert!((line - 28.35).abs() < 0.01, "{k} paragraphs: the line at {line}");
    }
}

/// A box shape showing story `id`, `w` × `h`, inline.
fn box_shape(id: u32, w: f32, h: f32) -> InlineObject {
    InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 0.5,
        float: Default::default(),
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    }
}

/// Measuring a box that resizes to fit its text leaves the list numbering as it was, also for a
/// linked box inside it: a numbered paragraph, a box holding a linked box with the next item,
/// and one more item read 1., 2., 3.
#[test]
fn a_linked_box_inside_a_box_fitted_to_its_text_counts_its_list_item_once() {
    let mut d = Document::from_text("one\nbox\nthree");
    let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    let item = wordcraft_doc::props::NumRef { num, level: 0 };
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(2, 5), &|p| p.numbering = Some(item)).unwrap();
    d.format_paragraphs(&Pos::body(1, 0), &Pos::body(1, 3), &|p| p.numbering = None).unwrap();
    let outer = text_box(&mut d, 1, "", 300.0, 100.0, FIT);
    let mut two = Paragraph::with_text("two", Default::default());
    two.props.numbering = Some(item);
    let linked = d.add_part(PartKind::TextBox, vec![para_block(two)]);
    let rest = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::new())]);
    d.parts.get_mut(&linked).unwrap().body.next = Some(rest);
    d.insert_object(&Pos { story: StoryRef::Part(outer), path: Path::top(0), off: 0 }, box_shape(linked, 200.0, 40.0), &Default::default()).unwrap();
    let end = d.para(StoryRef::Body, &Path::top(1)).map_or(0, |p| p.len());
    d.insert_object(&Pos::body(1, end), box_shape(rest, 200.0, 40.0), &Default::default()).unwrap();
    assert_eq!(d.text_box_chains(), vec![vec![linked, rest]]);
    let l = lay(&d);
    let mut labels: Vec<String> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| if let Placed::Lines { para, .. } = it { para.label.as_ref().map(|x| x.text.clone()) } else { None })
        .collect();
    labels.sort();
    assert_eq!(labels, ["1.", "2.", "3."]);
}

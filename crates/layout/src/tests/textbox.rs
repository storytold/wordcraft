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

#[test]
fn an_object_starting_in_the_bottom_margin_stays_out_of_the_box() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    // #5059: only text may run into the margin. An object stays whole, so it would stick out.
    let shown = |y: f32| {
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, "One", 144.0, 72.0, TextBody::default());
        let float = Float { wrap: Wrap::InFrontOfText, v_rel: Anchor::Paragraph, y, ..Default::default() };
        let picture = InlineObject::Shape {
            kind: wordcraft_doc::para::ShapeKind::Rectangle,
            w: 20.0,
            h: 20.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float,
            story: None,
            effects: Default::default(),
            freeform: None,
            extra: Default::default(),
        };
        d.insert_object(&Pos { story: StoryRef::Part(id), path: Path::top(0), off: 3 }, picture, &Default::default()).unwrap();
        let l = lay(&d);
        l.pages[0].items.iter().filter(|it| matches!(it, Placed::Shape { rect, .. } if rect.w == 20.0)).count()
    };
    let inner = 72.0 - 2.0 * BOX_INSET_Y;
    assert_eq!(shown(inner - 1.0), 1, "it starts inside the margins");
    assert_eq!(shown(inner + 2.0), 0, "it starts in the bottom margin");
}

#[test]
fn turned_cell_text_starting_in_the_bottom_margin_stays_out_of_the_box() {
    // Like an object it stays whole, so it would stick out of the box.
    let starts = |h: f32| {
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, "One", 144.0, h, TextBody::default());
        let mut t = wordcraft_doc::Table::new(1, 1, 120.0);
        t.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("Turned", Default::default()))];
        t.rows[0].cells[0].props.text_direction = TextDirection::Down;
        d.parts.get_mut(&id).unwrap().blocks.push(std::sync::Arc::new(wordcraft_doc::Block::Table(t)));
        let l = lay(&d);
        let top = frame_of(&l, id).y + BOX_INSET_Y;
        l.pages[0].items.iter().find_map(Placed::turned_bounds).map(|r| r.y - top)
    };
    let y = starts(400.0).unwrap();
    let height = |inner: f32| inner + 2.0 * BOX_INSET_Y;
    assert!(starts(height(y + 1.0)).is_some(), "it starts inside the margins");
    assert!(starts(height(y - 2.0)).is_none(), "it starts in the bottom margin");
}

#[test]
fn shading_and_side_borders_follow_the_text_into_the_bottom_margin() {
    use wordcraft_doc::props::{Border, Borders, Rgb};
    // (the box's height, the text's end, the shading's and the left border's) relative to its top.
    let ends = |h: f32| {
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, "One", 144.0, h, TextBody::default());
        let mut p = Paragraph::with_text("Two", Default::default());
        p.props.shading = Some(Rgb(0xEE, 0xEE, 0xEE));
        p.props.borders = Some(Borders { left: Some(Border::single(1.0)), ..Borders::default() });
        d.parts.get_mut(&id).unwrap().blocks = vec![para_block(Paragraph::with_text("One", Default::default())), para_block(p)];
        let l = lay(&d);
        let top = frame_of(&l, id).y;
        let text = l.pages[0].items.iter().find_map(|it| match it {
            Placed::Lines { path, para, y, .. } if *path == Path::top(1) => para.lines.first().map(|f| y + f.text_depth()),
            _ => None,
        });
        let shade = l.pages[0].items.iter().find_map(|it| if let Placed::Fill { rect, .. } = it { Some(rect.bottom()) } else { None });
        let side = l.pages[0].items.iter().find_map(|it| if let Placed::Rule { y1, .. } = it { Some(*y1) } else { None });
        (text.map(|y| y - top), shade.map(|y| y - top), side.map(|y| y - top))
    };
    let end = ends(400.0).0.unwrap();
    let h = end + 0.25;
    let (text, shade, side) = ends(h);
    assert!(text.is_some(), "the line stays");
    for reach in [shade.unwrap(), side.unwrap()] {
        assert!(reach >= end - 0.01 && reach <= h + 0.5 + 0.01, "{reach} against {end} in {h}");
    }
}

#[test]
fn a_line_stays_while_its_text_ends_inside_the_box() {
    use wordcraft_doc::props::LineSpacing;
    // #5059: two lines in a box `w` × `h`, how many show and where the second's text ends.
    let two = |spacing: f32, w: f32, body: TextBody| {
        let mut d = Document::from_text("x");
        let (bw, bh) = if body.vert == TextVert::Horz { (144.0, w) } else { (w, 144.0) };
        let id = text_box(&mut d, 0, "One", bw, bh, body);
        let para = |text: &str| {
            let mut p = Paragraph::with_text(text, Default::default());
            p.props.line_spacing = Some(LineSpacing::Multiple(spacing));
            para_block(p)
        };
        d.parts.get_mut(&id).unwrap().blocks = vec![para("One"), para("Two")];
        let l = lay(&d);
        let ends = l.pages[0].items.iter().filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), para, l0, y, .. } if *s == id => {
                let (first, last) = (para.lines.get(*l0)?, para.lines.last()?);
                Some(y + last.top - first.top + last.baseline - last.top + last.text_desc)
            }
            _ => None,
        });
        let end = ends.fold(0.0f32, f32::max) - frame_of(&l, id).y;
        (lines(&l, id).iter().map(|(.., l0, l1)| l1 - l0).sum::<usize>(), end)
    };
    // Tight spacing shrinks the ascent, not the descent, so the text ends below the line box.
    for spacing in [0.5, 1.0, 1.5] {
        let (_, end) = two(spacing, 400.0, TextBody::default());
        assert_eq!(two(spacing, end + 0.25, TextBody::default()).0, 2, "{spacing}: the text ends in the bottom margin");
        assert_eq!(two(spacing, end - 1.0, TextBody::default()).0, 1, "{spacing}: the text would run past the box");
    }
    // Turned text ends at the side its lines stack towards: the left at 90°, the right at 270°.
    let (_, end) = two(1.0, 400.0, TextBody::default());
    let (near, far) = (4 * 12700, 20 * 12700);
    let inset = BOX_INSET_Y as u32 * 12700;
    let deep = 4.0 + end - BOX_INSET_Y;
    for (vert, l, r) in [(TextVert::Vert, far, near), (TextVert::Vert270, near, far)] {
        let body = TextBody { vert, inset: Some([l, inset, r, inset]), ..Default::default() };
        assert_eq!(two(1.0, deep + 0.25, body).0, 2, "{vert:?}: the text ends in the far margin");
        assert_eq!(two(1.0, deep - 1.0, body).0, 1, "{vert:?}: the text would run past the box");
    }
    // A linked box keeps the same lines: the next picks up after them and none shows twice.
    let (_, end) = two(1.5, 400.0, TextBody::default());
    let mut d = Document::from_text("one\ntwo");
    let a = text_box(&mut d, 0, "One", 144.0, end + 0.25, TextBody::default());
    let b = text_box(&mut d, 1, "", 144.0, 400.0, TextBody::default());
    let part = d.parts.get_mut(&a).unwrap();
    part.body.next = Some(b);
    part.blocks = ["One", "Two", "Three"]
        .map(|text| {
            let mut p = Paragraph::with_text(text, Default::default());
            p.props.line_spacing = Some(LineSpacing::Multiple(1.5));
            para_block(p)
        })
        .into();
    let l = lay(&d);
    let boxes = l.pages[0].items.iter().filter_map(|it| match it {
        Placed::Object { rect, text_box: Some(s), .. } if *s == a => Some(rect.y),
        _ => None,
    });
    let second = boxes.fold(0.0f32, f32::max);
    let shown = lines(&l, a);
    assert_eq!(shown.iter().map(|(.., l0, l1)| l1 - l0).sum::<usize>(), 3);
    assert_eq!(shown.iter().filter(|(_, y, ..)| *y < second).count(), 2, "the first box keeps its two lines");
    let paras: Vec<_> = l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(s), path, .. } if *s == a => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(paras, [Path::top(0), Path::top(1), Path::top(2)], "each line shows once, in order");
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

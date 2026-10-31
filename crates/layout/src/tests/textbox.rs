//! Text boxes' text direction, alignment and linked boxes (Shape Format › Text).

#[allow(unused_imports)]
use crate::*;
use std::sync::Arc;
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
fn rotated_text_box_turns_kanji_while_east_asian_vertical_keeps_it_upright() {
    let glyphs_stood_up = |vert| {
        let mut d = Document::from_text("");
        text_box(&mut d, 0, "日本", 144.0, 144.0, TextBody { vert, ..Default::default() });
        let l = lay(&d);
        let draws = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
        draws.iter().any(|draw| match draw {
            display::Draw::Turned { items, .. } => items.iter().any(|item| matches!(item, display::Draw::Rotated { .. })),
            _ => false,
        })
    };
    assert!(!glyphs_stood_up(TextVert::Vert), "DrawingML vert turns the whole line, as on the checked envelope");
    assert!(glyphs_stood_up(TextVert::EaVert), "eaVert stands ideographs up after turning the line");
}

#[test]
fn missing_east_asian_faces_keep_three_rotated_columns_legible() {
    use wordcraft_doc::props::LineSpacing;

    for family in ["MS Mincho", "Missing East Asian Test Face", "Inter"] {
        let requested = wordcraft_fonts::word::resolve(family, false, false);
        if !requested.substituted && requested.face.covers('日') {
            continue;
        }
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, "", 135.0, 300.0, TextBody { vert: TextVert::Vert, ..Default::default() });
        let char_props =
            wordcraft_doc::props::CharProps { font: Some(family.into()), font_ea: Some(family.into()), size: Some(24.0), ..Default::default() };
        d.parts.get_mut(&id).unwrap().blocks = (0..3)
            .map(|_| {
                let mut p = Paragraph::with_text("日本語", char_props.clone());
                p.props.snap_to_grid = Some(false);
                p.props.space_before = Some(0.0);
                p.props.space_after = Some(0.0);
                para_block(p)
            })
            .collect();
        let positions: Vec<_> = lines(&lay(&d), id).into_iter().map(|(x, ..)| x).collect();
        assert_eq!(positions.len(), 3, "all three columns remain visible with {family}: {positions:?}");
        for pair in positions.windows(2) {
            let gap = pair[0] - pair[1];
            assert!((gap - 34.5).abs() < 4.0, "a 24 pt fallback follows the inherited 1.15 line spacing with {family}: {gap}");
        }

        for block in &mut d.parts.get_mut(&id).unwrap().blocks {
            if let Block::Para(p) = Arc::make_mut(block) {
                p.props.line_spacing = Some(LineSpacing::Exactly(18.0));
                p.touch();
            }
        }
        let explicit: Vec<_> = lines(&lay(&d), id).into_iter().map(|(x, ..)| x).collect();
        assert_eq!(explicit.len(), 3);
        for pair in explicit.windows(2) {
            assert!((pair[0] - pair[1] - 18.0).abs() < 3.0, "explicit 18 pt spacing wins with {family}: {explicit:?}");
        }
    }
}

#[test]
fn turned_text_and_fields_share_size_relative_spacing() {
    let advance = |text: &str, family: &str, field: bool, size: f32, automatic: bool| {
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, "", 220.0, 2400.0, TextBody { vert: TextVert::Vert, ..Default::default() });
        let props =
            wordcraft_doc::props::CharProps { font: Some(family.into()), font_ea: Some(family.into()), size: Some(size), ..Default::default() };
        let mut p = if field { Paragraph::new() } else { Paragraph::with_text(text, props.clone()) };
        if field {
            p.insert_object(
                0,
                InlineObject::Field {
                    instr: "MERGEFIELD address".into(),
                    result: text.into(),
                    locked: false,
                    dirty: false,
                    code: false,
                    form: String::new(),
                },
                &props,
            )
            .unwrap();
        }
        p.props.snap_to_grid = Some(false);
        p.props.auto_space_de = Some(automatic);
        p.props.auto_space_dn = Some(automatic);
        d.parts.get_mut(&id).unwrap().blocks = vec![para_block(p)];
        let laid = lay(&d);
        let para = laid.pages[0].items.iter().find_map(|item| match item {
            Placed::Lines { story: StoryRef::Part(story), para, .. } if *story == id => Some(para),
            _ => None,
        });
        let line = para.and_then(|p| p.lines.first()).expect("text-box line");
        let para = para.unwrap();
        let origins: Vec<f32> = para
            .clusters
            .iter()
            .enumerate()
            .flat_map(|(i, c)| {
                let x = line.cl_left(i).unwrap_or(0.0);
                para.glyphs[c.g0 as usize..c.g1 as usize].iter().map(move |g| x + g.dx)
            })
            .collect();
        (line.xs.last().copied().unwrap_or(0.0) - line.xs.first().copied().unwrap_or(0.0), origins)
    };

    for family in ["MS Mincho", "Inter"] {
        for automatic in [false, true] {
            for text in [
                "日2本",
                "日38本",
                "日３８本",
                "会館3階",
                "〒577-0845　日本2丁目5番18号",
                "日  本",
                "日　　本",
                "日  38  本",
                "日　　38　　本",
                "「日本（東京）」、「案内（概要）」",
            ] {
                let (reference_width, reference_origins) = advance(text, family, false, 24.0, automatic);
                for size in [9.0, 10.0, 16.0, 24.0, 36.0, 48.0] {
                    let (plain_width, plain_origins) = advance(text, family, false, size, automatic);
                    let (field_width, field_origins) = advance(text, family, true, size, automatic);
                    assert!((plain_width - field_width).abs() < 0.01, "field width: {family}, {text}, {size}, automatic={automatic}");
                    assert_eq!(plain_origins.len(), field_origins.len());
                    for (plain, field) in plain_origins.iter().zip(&field_origins) {
                        assert!((plain - field).abs() < 0.01, "field glyph origin: {family}, {text}, {size}, automatic={automatic}");
                    }
                    assert!((plain_width / size - reference_width / 24.0).abs() < 0.01, "width scales with size");
                    for (scaled, reference) in plain_origins.iter().zip(&reference_origins) {
                        assert!((scaled / size - reference / 24.0).abs() < 0.01, "glyph origin scales with size");
                    }
                }
            }
        }
    }
}

#[test]
fn small_leading_adjustments_keep_the_last_rotated_column_visible() {
    use wordcraft_doc::props::LineSpacing;
    let r = wordcraft_fonts::word::resolve("MS Mincho", false, false);
    if !r.substituted && r.face.covers('日') {
        return;
    }
    for size in [9.0, 12.0, 24.0, 36.0] {
        let mut d = Document::from_text("");
        d.styles.default_para.line_spacing = Some(LineSpacing::Multiple(1.0));
        let body = TextBody { vert: TextVert::Vert, ..Default::default() };
        let [left, _, right, _] = body.insets_pt();
        let id = text_box(&mut d, 0, "", 3.0 * size * 1.28 + left + right + 0.1, 400.0, body);
        d.parts.get_mut(&id).unwrap().blocks = (0..3)
            .map(|_| {
                let mut p = Paragraph::with_text(
                    "日本語",
                    wordcraft_doc::props::CharProps { font: Some("MS Mincho".into()), size: Some(size), ..Default::default() },
                );
                p.props.snap_to_grid = Some(false);
                p.props.space_before = Some(0.0);
                p.props.space_after = Some(0.0);
                para_block(p)
            })
            .collect();
        assert_eq!(lines(&lay(&d), id).len(), 3, "a small leading adjustment must not clip the last {size} pt column");
    }
}

#[test]
fn turned_line_spacing_preserves_the_typographic_centre() {
    let metrics = |vert, multiple| {
        let mut d = Document::from_text("x");
        let id = text_box(&mut d, 0, "", 220.0, 300.0, TextBody { vert, ..Default::default() });
        let mut p =
            Paragraph::with_text("Text 38", wordcraft_doc::props::CharProps { font: Some("Inter".into()), size: Some(24.0), ..Default::default() });
        p.props.snap_to_grid = Some(false);
        p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(multiple));
        d.parts.get_mut(&id).unwrap().blocks = vec![para_block(p)];
        let laid = lay(&d);
        laid.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                Placed::Lines { story: StoryRef::Part(s), para, .. } if *s == id => para.lines.first().map(|l| (l.baseline - l.top, l.height)),
                _ => None,
            })
            .unwrap()
    };
    let (base, height) = metrics(TextVert::Vert, 1.0);
    let (compressed, short) = metrics(TextVert::Vert, 0.8);
    assert!((base - compressed - (height - short) / 2.0).abs() < 0.01);
    let (expanded, tall) = metrics(TextVert::Vert, 1.2);
    assert!((expanded - base - (tall - height) / 2.0).abs() < 0.01);
    let (horizontal, _) = metrics(TextVert::Horz, 0.8);
    assert!((horizontal - base * 0.8).abs() < 0.01, "horizontal baseline behaviour stays unchanged");
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

//! Text boxes' text direction, alignment and linked boxes (Shape Format › Text).

use super::*;
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
fn inline_box_that_resizes_to_fit_takes_its_texts_height() {
    // #2791: an inline box with Resize shape to fit text, saved 10 cm tall around three short
    // lines, shrinks to them as in Word; the page break after it stays on the first page.
    let brk = wordcraft_doc::para::PAGE_BREAK;
    let doc = |auto_fit: bool| {
        let filler: Vec<String> = (1..=FILLER).map(|i| format!("Filler line {i}")).collect();
        let mut d = Document::from_text(&format!("{}\n\n\n{brk}\nSecond page text.", filler.join("\n")));
        let id = text_box(&mut d, FILLER, "", 453.5, 283.46, TextBody { auto_fit, anchor: VAlign::Center, ..Default::default() });
        d.parts.get_mut(&id).unwrap().blocks =
            (1..=3).map(|i| para_block(Paragraph::with_text(&format!("Text box line {i}"), Default::default()))).collect();
        let l = lay(&d);
        let r = l.pages.iter().find_map(|p| {
            p.items.iter().find_map(|it| match it {
                Placed::Object { rect, text_box: Some(s), .. } if *s == id => Some(*rect),
                _ => None,
            })
        });
        (l.pages.len(), r.unwrap())
    };
    let (pages, r) = doc(true);
    assert!(r.h > 3.0 * 12.0 && r.h < 80.0, "the box fits its three lines: {r:?}");
    assert_eq!(pages, 2, "the page break stays on the first page");
    let (pages, saved) = doc(false);
    assert_eq!(pages, 3, "a box keeping its 10 cm pushes the page break over");
    assert!((saved.h - 283.46).abs() < 1.0, "without auto-fit the box keeps its saved height: {saved:?}");
}

/// Filler paragraphs before the box in [`inline_box_that_resizes_to_fit_takes_its_texts_height`]:
/// enough that a 10 cm box no longer leaves room for the page break on the first page.
const FILLER: usize = 22;

fn auto_fit_case(body: TextBody, wrap: Wrap, h: f32) -> (Document, u32) {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::LineSpacing;
    let mut d = Document::from_text("");
    let id = text_box(&mut d, 0, "First\nSecond\nThird", 180.0, h, body);
    let p = d.para_mut(StoryRef::Part(id), &Path::top(0)).unwrap();
    p.props.line_spacing = Some(LineSpacing::Exactly(16.0));
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    let InlineObject::Shape { float, .. } = &mut p.objects[0] else { panic!("shape") };
    *float = Float { wrap, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 100.0, ..Default::default() };
    (d, id)
}

#[test]
fn auto_fit_grows_shape_frame() {
    let body = TextBody { auto_fit: true, inset: Some([25_400; 4]), ..Default::default() };
    let (d, id) = auto_fit_case(body, Wrap::InFrontOfText, 14.0);
    let l = lay(&d);
    let r = frame_of(&l, id);
    assert!((r.h - 52.0).abs() < 0.01, "three exact16pt lines plus 2pt top/bottom margins: {r:?}");
    assert_eq!((r.x, r.y, r.w), (72.0, 100.0, 180.0));
    let painted = l.pages[0].items.iter().find_map(|it| match it {
        Placed::Shape { rect, .. } if *rect == r => Some(*rect),
        _ => None,
    });
    assert_eq!(painted, Some(r), "shape paint and object hit frame use the same fitted size");
}

#[test]
fn auto_fit_wraps_at_fitted_edge() {
    let body = TextBody { auto_fit: true, inset: Some([25_400; 4]), ..Default::default() };
    let (mut d, id) = auto_fit_case(body, Wrap::TopAndBottom, 14.0);
    d.body.extend(Document::from_text(&"Text below the resized shape. ".repeat(50)).body);
    let l = lay(&d);
    let r = frame_of(&l, id);
    assert!((r.h - 52.0).abs() < 0.01);
    let mut below = false;
    for it in &l.pages[0].items {
        let Placed::Lines { story: StoryRef::Body, para, l0, l1, y, .. } = it else { continue };
        let first = &para.lines[*l0];
        for line in &para.lines[*l0..*l1] {
            let top = y + line.top - first.top;
            assert!(top + line.height <= r.y + 0.01 || top >= r.bottom() - 0.01, "line at {top} overlaps fitted frame {r:?}");
            below |= top >= r.bottom() - 0.01;
        }
    }
    assert!(below);
}

#[test]
fn fixed_shape_keeps_frame_height() {
    for h in [14.0, 80.0] {
        let (d, id) = auto_fit_case(TextBody { inset: Some([25_400; 4]), ..Default::default() }, Wrap::InFrontOfText, h);
        assert_eq!(frame_of(&lay(&d), id).h, h);
    }
}

#[test]
fn auto_fit_shrinks_shape_frame() {
    let body = TextBody { auto_fit: true, inset: Some([25_400; 4]), ..Default::default() };
    let (mut d, id) = auto_fit_case(body, Wrap::InFrontOfText, 80.0);
    let p = d.para_mut(StoryRef::Part(id), &Path::top(0)).unwrap();
    let props = p.props.clone();
    *p = Paragraph::with_text("Short", Default::default());
    p.props = props;
    assert!((frame_of(&lay(&d), id).h - 20.0).abs() < 0.01);
}

#[test]
fn auto_fit_includes_outline() {
    use wordcraft_doc::props::Rgb;
    let body = TextBody { auto_fit: true, inset: Some([25_400; 4]), ..Default::default() };
    for (height, text, expected) in [(14.0, "First\nSecond\nThird", 53.0), (80.0, "Short", 21.0)] {
        let (mut d, id) = auto_fit_case(body, Wrap::InFrontOfText, height);
        let p = d.para_mut(StoryRef::Part(id), &Path::top(0)).unwrap();
        let props = p.props.clone();
        *p = Paragraph::with_text(text, Default::default());
        p.props = props;
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        let InlineObject::Shape { stroke, stroke_width, .. } = &mut p.objects[0] else { panic!("shape") };
        *stroke = Some(Rgb(0, 0, 0));
        *stroke_width = 1.0;
        assert!((frame_of(&lay(&d), id).h - expected).abs() < 0.01);
    }
}

fn probe_shape(id: u32, wrap: Wrap) -> InlineObject {
    InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 180.0,
        h: 2000.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Float { wrap, ..Default::default() },
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    }
}

fn probe_part(d: &mut Document, text: &str) -> u32 {
    let id = d.add_part(PartKind::TextBox, Document::from_text(text).body);
    d.parts.get_mut(&id).unwrap().body.inset = Some([0; 4]);
    id
}

fn probe_child(d: &mut Document, parent: u32, para: usize, child: u32, wrap: Wrap) {
    d.insert_object(&Pos::new(StoryRef::Part(parent), Path::top(para), 0), probe_shape(child, wrap), &Default::default()).unwrap();
}

fn nested_fit_doc(count: usize, wrap: Wrap) -> (Document, Vec<u32>) {
    let mut d = Document::from_text("");
    d.styles.default_chr.font = Some("Source Sans 3".into());
    let outer = probe_part(&mut d, &"\n".repeat(count.saturating_sub(1)));
    let mut children = Vec::new();
    for k in 0..count {
        let child = probe_part(&mut d, &format!("Child{k:03}"));
        d.parts.get_mut(&child).unwrap().body.auto_fit = true;
        probe_child(&mut d, outer, k, child, Wrap::InFrontOfText);
        children.push(child);
    }
    d.insert_object(&Pos::body(0, 0), probe_shape(outer, wrap), &Default::default()).unwrap();
    (d, children)
}

fn probe_ink(d: &Document, l: &DocLayout) -> String {
    l.pages
        .iter()
        .flat_map(|p| display::page_display(d, p, &display::DisplayOptions::default()))
        .filter_map(|item| match item {
            display::Draw::Glyphs { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn fit_nested_children_visible() {
    for count in [32, 33, 64, 65] {
        let (d, children) = nested_fit_doc(count, Wrap::InFrontOfText);
        let l = lay(&d);
        let ink = probe_ink(&d, &l);
        for (k, child) in children.iter().enumerate() {
            let shown = l
                .pages
                .iter()
                .flat_map(|p| &p.items)
                .filter(|it| {
                    matches!(it,
                Placed::Lines { story: StoryRef::Part(id), l0, l1, .. } if id == child && l1 > l0)
                })
                .count();
            assert_eq!(shown, usize::from(k < wordcraft_doc::BoxBudget::MAX_NESTED), "count={count}, child={k}");
            assert_eq!(ink.matches(&format!("Child{k:03}")).count(), shown, "each permitted sentinel is painted once");
        }
    }
}

#[test]
fn fit_inline_probe_keeps_budget() {
    let (mut d, _) = nested_fit_doc(33, Wrap::Inline);
    let InlineObject::Shape { story: Some(outer), .. } = d.para(StoryRef::Body, &Path::top(0)).unwrap().objects[0] else { panic!("outer") };
    // Compare real expansion work, not saved frame height or the number of speculative calls.
    let fixed = lay(&d).text_boxes;
    d.parts.get_mut(&outer).unwrap().body.auto_fit = true;
    let fitted = lay(&d);
    assert_eq!(fitted.text_boxes, fixed);
    assert_eq!(fitted.text_boxes, 34);
}

fn numbered_chain(d: &mut Document, list: u32) -> (u32, u32) {
    let a = probe_part(d, "Chain sentinel");
    let b = probe_part(d, "");
    d.parts.get_mut(&a).unwrap().body.next = Some(b);
    d.para_mut(StoryRef::Part(a), &Path::top(0)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
    (a, b)
}

fn placed_labels(l: &DocLayout, story: StoryRef) -> Vec<String> {
    l.pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|it| match it {
            Placed::Lines { story: s, para, .. } if *s == story => para.label.as_ref().map(|label| label.text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn fit_probe_keeps_link_labels() {
    for wrap in [Wrap::InFrontOfText, Wrap::Inline] {
        let mut d = Document::from_text("\nAfter");
        d.styles.default_chr.font = Some("Source Sans 3".into());
        let list = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        let outer = probe_part(&mut d, "\n");
        d.parts.get_mut(&outer).unwrap().body.auto_fit = true;
        let (a, b) = numbered_chain(&mut d, list);
        probe_child(&mut d, outer, 0, a, Wrap::InFrontOfText);
        probe_child(&mut d, outer, 1, b, Wrap::InFrontOfText);
        d.insert_object(&Pos::body(0, 0), probe_shape(outer, wrap), &Default::default()).unwrap();
        d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
        let l = lay(&d);
        assert_eq!(placed_labels(&l, StoryRef::Part(a)), ["1."]);
        assert_eq!(placed_labels(&l, StoryRef::Body), ["2."]);
        assert!(probe_ink(&d, &l).contains("Chain sentinel"), "the linked sentinel text is painted");
    }
}

#[test]
fn fit_probe_keeps_old_flows() {
    let mut d = Document::from_text("\n\nAfter");
    let list = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    let (a, b) = numbered_chain(&mut d, list);
    d.insert_object(&Pos::body(0, 0), probe_shape(a, Wrap::InFrontOfText), &Default::default()).unwrap();
    d.insert_object(&Pos::body(0, 0), probe_shape(b, Wrap::InFrontOfText), &Default::default()).unwrap();
    let outer = probe_part(&mut d, "\n\n");
    d.parts.get_mut(&outer).unwrap().body.auto_fit = true;
    probe_child(&mut d, outer, 0, a, Wrap::InFrontOfText);
    let (c, e) = numbered_chain(&mut d, list);
    probe_child(&mut d, outer, 1, c, Wrap::InFrontOfText);
    probe_child(&mut d, outer, 2, e, Wrap::InFrontOfText);
    d.insert_object(&Pos::body(1, 0), probe_shape(outer, Wrap::InFrontOfText), &Default::default()).unwrap();
    d.para_mut(StoryRef::Body, &Path::top(2)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
    let l = lay(&d);
    assert_eq!(placed_labels(&l, StoryRef::Part(a)), ["1.", "1."]);
    assert_eq!(placed_labels(&l, StoryRef::Part(c)), ["2."]);
    assert_eq!(placed_labels(&l, StoryRef::Body), ["3."]);
}

#[test]
fn fit_probe_keeps_budget() {
    let (d, _) = nested_fit_doc(3, Wrap::InFrontOfText);
    assert_eq!(lay(&d).text_boxes, 4, "only real outer/child expansions are counted");
}

#[test]
fn fit_probe_keeps_guards() {
    let mut d = Document::from_text("");
    let root = probe_part(&mut d, "Cycle sentinel");
    d.parts.get_mut(&root).unwrap().body.auto_fit = true;
    probe_child(&mut d, root, 0, root, Wrap::InFrontOfText);
    d.insert_object(&Pos::body(0, 0), probe_shape(root, Wrap::InFrontOfText), &Default::default()).unwrap();
    let l = lay(&d);
    assert_eq!(l.text_boxes, 1, "a probe keeps the active ancestor guard");
    assert_eq!(probe_ink(&d, &l).matches("Cycle sentinel").count(), 1);

    let mut d = Document::from_text("");
    let ids: Vec<_> = (0..6).map(|k| probe_part(&mut d, &format!("Depth{k}"))).collect();
    for id in &ids {
        d.parts.get_mut(id).unwrap().body.auto_fit = true;
    }
    for pair in ids.windows(2) {
        probe_child(&mut d, pair[0], 0, pair[1], Wrap::InFrontOfText);
    }
    d.insert_object(&Pos::body(0, 0), probe_shape(ids[0], Wrap::InFrontOfText), &Default::default()).unwrap();
    let l = lay(&d);
    assert_eq!(l.text_boxes, wordcraft_doc::MAX_TEXT_BOX_DEPTH);
    for (k, id) in ids.iter().enumerate() {
        let shown = l.pages.iter().flat_map(|p| &p.items).any(|item| {
            matches!(item,
            Placed::Lines { story: StoryRef::Part(s), .. } if s == id)
        });
        assert_eq!(shown, k < wordcraft_doc::MAX_TEXT_BOX_DEPTH, "depth={k}");
    }
}

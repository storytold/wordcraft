//! Text boxes' text direction, alignment and linked boxes (Shape Format › Text).

use super::*;
use crate::display::Draw;
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

/// A floating text box turned `deg` (flipped as given) showing "Rotated text box", in a body of a
/// few paragraphs; its story id.
fn turned_box(d: &mut Document, deg: f32, flip_h: bool, flip_v: bool) -> u32 {
    let id = text_box(d, 0, "Rotated text box", 180.0, 60.0, TextBody::default());
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    for o in p.objects.iter_mut() {
        if let InlineObject::Shape { float, .. } = o {
            float.wrap = Wrap::Square;
            float.rot = deg;
            float.flip_h = flip_h;
            float.flip_v = flip_v;
        }
    }
    id
}

/// The spins of the turns around the box text's glyphs (empty: drawn upright), and how many glyph
/// runs are drawn upright, outside any turn.
fn glyph_turns(draws: &[Draw]) -> (Vec<Spin>, usize) {
    fn walk(ds: &[Draw], spin: Option<Spin>, turns: &mut Vec<Spin>, plain: &mut usize) {
        for d in ds {
            match d {
                Draw::Glyphs { text, .. } if text.contains("Rotated") || text.contains("box") => match spin {
                    Some(s) => turns.push(s),
                    None => *plain += 1,
                },
                Draw::Rotated { spin: s, items, .. } => walk(items, Some(*s), turns, plain),
                Draw::Clip { items, .. } | Draw::Turned { items, .. } => walk(items, spin, turns, plain),
                _ => {}
            }
        }
    }
    let (mut turns, mut plain) = (Vec::new(), 0);
    walk(draws, None, &mut turns, &mut plain);
    (turns, plain)
}

#[test]
fn text_in_a_rotated_text_box_turns_with_the_box() {
    let mut d = Document::from_text("Body words around it.");
    let id = turned_box(&mut d, 30.0, false, false);
    let l = lay(&d);
    assert!(!lines(&l, id).is_empty(), "the box shows its text");
    let draws = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
    let (turns, plain) = glyph_turns(&draws);
    assert_eq!(plain, 0, "no glyph of the box's text is drawn upright");
    assert!(!turns.is_empty() && turns.iter().all(|s| (s.deg - 30.0).abs() < 0.01), "the text turns 30° with the box: {turns:?}");
}

#[test]
fn a_rotated_text_box_is_drawn_straight_while_its_text_is_edited() {
    let mut d = Document::from_text("Body words around it.");
    let id = turned_box(&mut d, 30.0, false, false);
    let other = text_box(&mut d, 0, "Another one", 120.0, 40.0, TextBody::default());
    if let Some(InlineObject::Shape { float, .. }) = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().objects.last_mut() {
        float.wrap = Wrap::Square;
        float.rot = 30.0;
        float.y = 300.0;
    }
    let l = lay(&d);
    let opts = display::DisplayOptions { straight: Some(id), ..Default::default() };
    let draws = display::page_display(&d, &l.pages[0], &opts);
    let (turns, plain) = glyph_turns(&draws);
    assert!(turns.iter().all(|s| s.is_identity()), "the edited box's text isn't turned: {turns:?}");
    assert!(plain > 0, "it is drawn upright");
    // The other turned box stays turned, frame and all.
    fn turned_text(ds: &[Draw], spin: bool) -> Vec<String> {
        ds.iter()
            .flat_map(|d| match d {
                Draw::Glyphs { text, .. } if spin => vec![text.clone()],
                Draw::Rotated { items, .. } => turned_text(items, true),
                Draw::Clip { items, .. } => turned_text(items, spin),
                _ => Vec::new(),
            })
            .collect()
    }
    let turned = turned_text(&draws, false).concat();
    assert!(turned.contains("Another"), "the other box stays turned: {turned:?}");
    let _ = other;
}

#[test]
fn a_flipped_text_box_turns_its_text_but_does_not_mirror_it() {
    let mut d = Document::from_text("Body words around it.");
    turned_box(&mut d, 30.0, true, false);
    let l = lay(&d);
    let (turns, plain) = glyph_turns(&display::page_display(&d, &l.pages[0], &display::DisplayOptions::default()));
    assert_eq!(plain, 0);
    assert!(!turns.is_empty() && turns.iter().all(|s| (s.deg - 30.0).abs() < 0.01 && !s.flip_h && !s.flip_v), "{turns:?}");
}

#[test]
fn a_click_on_a_rotated_box_lands_on_the_character_drawn_there() {
    let mut d = Document::from_text("Body words around it.");
    let id = turned_box(&mut d, 30.0, false, false);
    let l = lay(&d);
    let o = l.find_object(0, |o| o.text_box == Some(id)).unwrap();
    let story = StoryRef::Part(id);
    let pos = |off| Pos { story, path: Path::top(0), off };
    // A quarter into the "b" of "box" (byte 13), unturned, and where it is drawn: turned about
    // the frame's centre.
    let (a, b) = (l.caret_on(&pos(13), 0).unwrap(), l.caret_on(&pos(14), 0).unwrap());
    let (ux, uy) = (a.x + (b.x - a.x) * 0.25, a.top + a.height / 2.0);
    let (cx, cy) = (o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
    let (x, y) = Spin::new(30.0, false, false).apply(cx, cy, ux, uy);
    assert_eq!(l.story_at_shown(0, x, y, None), Some(story), "the click is in the box");
    assert_eq!(l.hit_shown(0, x, y, story, None).map(|p| p.off), Some(13), "the click starts editing on the character under it");
    assert_ne!(l.hit(0, x, y, story).map(|p| p.off), Some(13), "(plain coordinates miss it)");
    // While the box is edited it is drawn straight: the point is plain page coordinates.
    assert_eq!(l.story_at_shown(0, ux, uy, Some(id)), Some(story));
    assert_eq!(l.hit_shown(0, ux, uy, story, Some(id)).map(|p| p.off), Some(13));
    // Hostile points stay safe.
    let _ = l.hit_shown(0, f32::NAN, f32::INFINITY, story, None);
    assert!(l.story_at_shown(99, 0.0, 0.0, None).is_none());
}

#[test]
fn an_edited_turned_box_is_grabbed_on_the_straight_border_it_is_drawn_with() {
    let mut d = Document::from_text("Body words around it.");
    let id = turned_box(&mut d, 30.0, false, false);
    let l = lay(&d);
    let o = l.find_object(0, |o| o.text_box == Some(id)).unwrap();
    // A point on the straight frame's top edge, 60 pt right of its middle: well off the turned
    // border.
    let (x, y) = (o.rect.x + o.rect.w / 2.0 + 60.0, o.rect.y);
    let grabbed = |straight: Option<u32>| l.object_at_shown(0, x, y, 3.0, StoryRef::Body, straight).map(|o| o.text_box);
    assert_eq!(grabbed(None), None, "not edited, the box is turned: no border there");
    assert_eq!(grabbed(Some(id)), Some(Some(id)), "edited, it is drawn straight: its border is there");
    let hit = l.object_at_shown(0, x, y, 3.0, StoryRef::Body, Some(id)).unwrap();
    assert!((hit.spin.deg - 30.0).abs() < 0.01, "the grab keeps the box's real turn for the move");
    // Inside the straight frame (off the border) is its text, not a grab.
    let (cx, cy) = (o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
    assert!(l.object_at_shown(0, cx, cy, 3.0, StoryRef::Body, Some(id)).is_none());
}

/// The PDF draws an item inside its turned text box from a page of just the box's open marker
/// and the item: that gives the same draws as the whole page on screen (a shadow keeps its page
/// direction in both).
#[test]
fn a_turned_box_shape_draws_the_same_alone_as_on_the_page() {
    let mut d = Document::from_text("Body words around it.");
    turned_box(&mut d, 30.0, false, false);
    if let Some(InlineObject::Shape { effects, fill, stroke, .. }) = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().objects.first_mut() {
        effects.shadow = Some(wordcraft_doc::effects::Shadow { distance: 8.0, angle: 0.0, ..Default::default() });
        *fill = Some(wordcraft_doc::fill::Fill::solid(wordcraft_doc::props::Rgb(0, 0, 255)));
        *stroke = Some(wordcraft_doc::props::Rgb::BLACK);
    }
    let l = lay(&d);
    let page = &l.pages[0];
    let opts = display::DisplayOptions::default();
    // The shadow angle of the shape drawn inside a turn.
    fn shadow(ds: &[Draw], turned: bool) -> Option<f32> {
        ds.iter().find_map(|d| match d {
            Draw::Shape { effects, .. } if turned => effects.shadow.map(|s| s.angle),
            Draw::Rotated { items, .. } => shadow(items, true),
            _ => None,
        })
    }
    let open = page.items.iter().position(|it| matches!(it, Placed::Spun { open: true, .. })).expect("an open marker");
    let shape = page.items.iter().skip(open).find(|it| matches!(it, Placed::Shape { .. })).expect("the box's shape").clone();
    let one = Page { items: vec![page.items[open].clone(), shape], ..page.clone() };
    let alone = shadow(&display::page_display(&d, &one, &opts), false).expect("drawn alone, turned, with its shadow");
    let whole = shadow(&display::page_display(&d, page, &opts), false).expect("drawn on the page, turned, with its shadow");
    assert_eq!(alone, whole, "the PDF's shadow matches the screen's");
    assert!((alone - 330.0).abs() < 0.01, "cast 30° back inside the turn, so it falls to the right on the page: {alone}");
}

/// Whether any glyph run of the box text ("box" in it) in `ds` carries a hyperlink.
fn box_text_links(ds: &[Draw]) -> Vec<String> {
    ds.iter()
        .flat_map(|d| match d {
            Draw::Glyphs { text, link: Some(l), .. } if text.contains("box") => vec![l.clone()],
            Draw::Rotated { items, .. } | Draw::Clip { items, .. } | Draw::Turned { items, .. } | Draw::Figure { draws: items, .. } => {
                box_text_links(items)
            }
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn a_link_in_a_rotated_text_box_gets_no_link_area_but_an_upright_box_keeps_it() {
    for (deg, linked) in [(30.0, false), (0.0, true)] {
        let mut d = Document::from_text("Body words around it.");
        let id = turned_box(&mut d, deg, false, false);
        let story = StoryRef::Part(id);
        let n = d.para(story, &Path::top(0)).unwrap().len();
        let at = |off| Pos { story, path: Path::top(0), off };
        d.format_range(&at(0), &at(n), &|c| c.link = Some("https://a.example/".into())).unwrap();
        let l = lay(&d);
        let links = box_text_links(&display::page_display(&d, &l.pages[0], &display::DisplayOptions::default()));
        assert_eq!(!links.is_empty(), linked, "turn {deg}: {links:?}");
    }
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

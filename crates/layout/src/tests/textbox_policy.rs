use super::*;
use wordcraft_doc::para::{Anchor, Float, InlineObject, ShapeKind, Wrap};
use wordcraft_doc::props::{LineSpacing, Rgb};
use wordcraft_doc::{Document, Paragraph, PartKind, Pos, para_block};

const FRAME_WIDTH: f32 = 180.0;
const SAVED_HEIGHT: f32 = 80.0;
const LINE_HEIGHT: f32 = 16.0;
const CHILD_COLOR: Rgb = Rgb(25, 73, 137);

fn paragraph() -> Paragraph {
    let mut p = Paragraph::with_text("Parent", Default::default());
    p.props.line_spacing = Some(LineSpacing::Exactly(LINE_HEIGHT));
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    p
}

fn rectangle(wrap: Wrap, y: f32) -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 40.0,
        h: 10.0,
        fill: Some(wordcraft_doc::fill::Fill::solid(CHILD_COLOR)),
        stroke: None,
        stroke_width: 0.0,
        float: Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, y, ..Default::default() },
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    }
}

fn policy_doc(child: Option<InlineObject>) -> (Document, u32) {
    let mut d = Document::from_text("");
    d.styles.default_chr.font = Some("Arial".into());
    d.styles.default_chr.size = Some(10.0);
    let root = d.add_part(PartKind::TextBox, vec![para_block(paragraph())]);
    d.parts.get_mut(&root).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
    if let Some(child) = child {
        d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), child, &Default::default()).unwrap();
    }
    let parent = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: FRAME_WIDTH,
        h: SAVED_HEIGHT,
        fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(240, 248, 240))),
        stroke: Some(Rgb(192, 0, 0)),
        stroke_width: 1.0,
        float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 100.0, ..Default::default() },
        story: Some(root),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), parent, &Default::default()).unwrap();
    (d, root)
}

fn lay(d: &Document) -> crate::DocLayout {
    crate::layout(d, &mut crate::LayoutCache::new(), &crate::LayoutOptions::default())
}

fn frame(l: &crate::DocLayout, root: u32) -> Rect {
    l.pages
        .iter()
        .flat_map(|p| &p.items)
        .find_map(|it| match it {
            Placed::Object { rect, text_box: Some(id), .. } if *id == root => Some(*rect),
            _ => None,
        })
        .unwrap()
}

fn paint(l: &crate::DocLayout, owner: &Pos) -> usize {
    l.pages.iter().flat_map(|p| &p.items).filter(|item| matches!(item, Placed::Shape { object: Some(pos), .. } if pos == owner)).count()
}

fn child_draws(draws: &[crate::display::Draw]) -> usize {
    use crate::display::Draw;
    draws
        .iter()
        .map(|draw| match draw {
            Draw::Shape { fill: Some(color), .. } if *color == crate::display::Paint::solid(CHILD_COLOR) => 1,
            Draw::Rotated { items, .. } | Draw::Turned { items, .. } | Draw::Clip { items, .. } => child_draws(items),
            _ => 0,
        })
        .sum()
}

fn draws(d: &Document, l: &crate::DocLayout) -> usize {
    l.pages.iter().map(|p| child_draws(&crate::display::page_display(d, p, &Default::default()))).sum()
}

#[test]
fn floating_rect_height() {
    let mut observed = Vec::new();
    for wrap in [Wrap::InFrontOfText, Wrap::BehindText] {
        for y in [0.0, 12.0, 500.0] {
            let (d, root) = policy_doc(Some(rectangle(wrap, y)));
            let l = lay(&d);
            observed.push((wrap, y, frame(&l, root).h));
        }
    }
    eprintln!("FLOAT_PARENT_HEIGHT {observed:?}");
    assert!(observed.iter().all(|(_, _, h)| (*h - 17.0).abs() < 0.01), "{observed:?}");
}

#[test]
fn floating_rect_paint() {
    let mut observed = Vec::new();
    for wrap in [Wrap::InFrontOfText, Wrap::BehindText] {
        for y in [0.0, 12.0, 500.0] {
            let (d, root) = policy_doc(Some(rectangle(wrap, y)));
            let l = lay(&d);
            let owner = Pos::new(StoryRef::Part(root), Path::top(0), 0);
            let hits = l
                .pages
                .iter()
                .flat_map(|p| &p.items)
                .filter(|item| {
                    matches!(item,
                Placed::Object { story, path, off, .. } if *story == owner.story && *path == owner.path && *off == owner.off)
                })
                .count();
            assert_eq!(hits, 1, "source object remains selectable");
            assert_eq!(d.para(owner.story, &owner.path).unwrap().objects.len(), 1);
            observed.push((wrap, y, paint(&l, &owner), draws(&d, &l)));
        }
    }
    eprintln!("FLOAT_PARENT_PAINT {observed:?}");
    assert!(observed.iter().all(|(_, _, placed, draw)| *placed == 0 && *draw == 0), "{observed:?}");
}

#[test]
fn square_keeps_flow() {
    let mut child = rectangle(Wrap::Square, 0.0);
    if let InlineObject::Shape { w, h, .. } = &mut child {
        *w = 170.0;
        *h = 40.0;
    }
    let (d, root) = policy_doc(Some(child));
    let l = lay(&d);
    let r = frame(&l, root);
    let top = l
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .find_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(id), y, .. } if *id == root => Some(*y),
            _ => None,
        })
        .unwrap();
    eprintln!("SQUARE_FRAME {r:?} text_top={top} paint={}", draws(&d, &l));
    assert!((r.h - 57.0).abs() < 0.01, "{r:?}");
    assert!((top - r.y - 40.0).abs() < 0.01, "wrapping displacement survives");
    assert_eq!(draws(&d, &l), 0);
}

#[test]
fn plain_and_inline_keep_flow() {
    let (d, root) = policy_doc(None);
    assert!((frame(&lay(&d), root).h - 17.0).abs() < 0.01);
    let image = InlineObject::Image {
        media: "original-policy-control".into(),
        w: 40.0,
        h: 100.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    let (mut d, root) = policy_doc(Some(image));
    d.para_mut(StoryRef::Part(root), &Path::top(0)).unwrap().props.line_spacing = Some(LineSpacing::AtLeast(LINE_HEIGHT));
    let l = lay(&d);
    let h = frame(&l, root).h;
    let owner = Pos::new(StoryRef::Part(root), Path::top(0), 0);
    let objects: Vec<_> = l
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|it| match it {
            Placed::Object { story, path, off, rect, .. } if *story == owner.story && *path == owner.path && *off == 0 => Some(*rect),
            _ => None,
        })
        .collect();
    eprintln!("MODEL_TALL_INLINE height={h} hits={objects:?}");
    assert!((101.0..110.0).contains(&h), "{h}");
    assert_eq!(objects.len(), 1);
    assert_eq!((objects[0].w, objects[0].h), (40.0, 100.0));
    let root_lines: Vec<_> = l
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Part(id), para, l0, l1, .. } if *id == root => Some((para, l0, l1)),
            _ => None,
        })
        .collect();
    assert_eq!(root_lines.len(), 1);
    assert!(root_lines[0].0.clusters.iter().any(|c| matches!(c.kind, crate::para::ClKind::Object(0))));
}

#[derive(Clone, Copy, Debug)]
enum OtherChild {
    Ellipse,
    Outline,
    Effect,
    Turn,
    Flip,
    TopBottom,
    Tight,
    PageAnchor,
    TextBox,
}

#[test]
fn other_children_keep_paint() {
    for case in [
        OtherChild::Ellipse,
        OtherChild::Outline,
        OtherChild::Effect,
        OtherChild::Turn,
        OtherChild::Flip,
        OtherChild::TopBottom,
        OtherChild::Tight,
        OtherChild::PageAnchor,
        OtherChild::TextBox,
    ] {
        let (mut d, root) = policy_doc(Some(rectangle(Wrap::InFrontOfText, 500.0)));
        let nested = d.add_part(PartKind::TextBox, vec![para_block(paragraph())]);
        let child = d.para_mut(StoryRef::Part(root), &Path::top(0)).unwrap().object_at_mut(0).unwrap();
        if let InlineObject::Shape { kind, stroke, effects, float, story, .. } = child {
            match case {
                OtherChild::Ellipse => *kind = ShapeKind::Ellipse,
                OtherChild::Outline => *stroke = Some(CHILD_COLOR),
                OtherChild::Effect => effects.soft_edge = Some(1.0),
                OtherChild::Turn => float.rot = 15.0,
                OtherChild::Flip => float.flip_h = true,
                OtherChild::TopBottom => float.wrap = Wrap::TopAndBottom,
                OtherChild::Tight => float.wrap = Wrap::Tight,
                OtherChild::PageAnchor => float.v_rel = Anchor::Page,
                OtherChild::TextBox => {
                    *kind = ShapeKind::TextBox;
                    *story = Some(nested);
                }
            }
        }
        let l = lay(&d);
        assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), 0)), 1, "{case:?}");
        assert!(draws(&d, &l) > 0, "{case:?}");
    }
}

#[derive(Clone, Copy, Debug)]
enum OtherParent {
    Fixed,
    Inline,
    Turned,
    Center,
    Bottom,
    Linked,
}

#[test]
fn other_parents_keep_paint() {
    for case in [OtherParent::Fixed, OtherParent::Inline, OtherParent::Turned, OtherParent::Center, OtherParent::Bottom, OtherParent::Linked] {
        // The child is inside the saved frame, so fixed/linked clipping is not the policy under test.
        let (mut d, root) = policy_doc(Some(rectangle(Wrap::InFrontOfText, 0.0)));
        let tail = d.add_part(PartKind::TextBox, vec![para_block(paragraph())]);
        match case {
            OtherParent::Fixed => d.parts.get_mut(&root).unwrap().body.auto_fit = false,
            OtherParent::Inline => {
                if let InlineObject::Shape { float, .. } = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().object_at_mut(0).unwrap() {
                    float.wrap = Wrap::Inline;
                }
            }
            OtherParent::Turned => d.parts.get_mut(&root).unwrap().body.vert = TextVert::Vert,
            OtherParent::Center => d.parts.get_mut(&root).unwrap().body.anchor = VAlign::Center,
            OtherParent::Bottom => d.parts.get_mut(&root).unwrap().body.anchor = VAlign::Bottom,
            OtherParent::Linked => d.parts.get_mut(&root).unwrap().body.next = Some(tail),
        }
        let l = lay(&d);
        assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), 0)), 1, "{case:?}");
        assert!(draws(&d, &l) > 0, "{case:?}");
    }
}

#[test]
fn same_rect_distinct_owners() {
    let (mut d, root) = policy_doc(Some(rectangle(Wrap::InFrontOfText, 500.0)));
    let mut other = rectangle(Wrap::InFrontOfText, 500.0);
    if let InlineObject::Shape { kind, .. } = &mut other {
        *kind = ShapeKind::Ellipse;
    }
    let off = d.para(StoryRef::Part(root), &Path::top(0)).unwrap().len();
    d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), off), other, &Default::default()).unwrap();
    let l = lay(&d);
    assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), 0)), 0);
    assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), off)), 1);
    assert_eq!(draws(&d, &l), 1);
    assert_eq!(d.para(StoryRef::Part(root), &Path::top(0)).unwrap().objects.len(), 2);
}

#[test]
fn table_child_keeps_paint() {
    let (mut d, root) = policy_doc(None);
    let mut p = paragraph();
    p.text.insert(0, wordcraft_doc::para::OBJ);
    p.runs[0].len += wordcraft_doc::para::OBJ.len_utf8();
    p.objects.push(rectangle(Wrap::InFrontOfText, 0.0));
    let mut t = wordcraft_doc::Table::new(1, 1, FRAME_WIDTH);
    t.rows[0].cells[0].blocks = vec![para_block(p)];
    d.parts.get_mut(&root).unwrap().blocks.push(Arc::new(Block::Table(t)));
    let l = lay(&d);
    let owners: Vec<_> = l
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|it| match it {
            Placed::Shape { object: Some(owner), .. } if owner.story == StoryRef::Part(root) => Some(owner),
            _ => None,
        })
        .collect();
    assert_eq!(owners.len(), 1);
    assert!(owners[0].path.0.len() > 1);
    assert_eq!(draws(&d, &l), 1);
}

#[test]
fn inset_and_color_are_not_keys() {
    let (mut d, root) = policy_doc(Some(rectangle(Wrap::BehindText, 123.0)));
    d.parts.get_mut(&root).unwrap().body.inset = Some([12700; 4]);
    let child = d.para_mut(StoryRef::Part(root), &Path::top(0)).unwrap().object_at_mut(0).unwrap();
    if let InlineObject::Shape { fill, w, h, .. } = child {
        *fill = Some(wordcraft_doc::fill::Fill::solid(Rgb(107, 31, 48)));
        *w = 53.0;
        *h = 19.0;
    }
    let parent = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().object_at_mut(0).unwrap();
    if let InlineObject::Shape { w, h, .. } = parent {
        *w = 230.0;
        *h = 120.0;
    }
    let l = lay(&d);
    assert!((frame(&l, root).h - 19.0).abs() < 0.01);
    assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), 0)), 0);
}

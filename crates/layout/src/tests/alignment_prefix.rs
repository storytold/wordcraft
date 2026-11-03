use super::*;
use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
use wordcraft_doc::props::{Borders, LineSpacing, ParaProps};
use wordcraft_doc::section::PageVAlign;

fn aligned_prefix_doc(count: usize) -> Document {
    let mut doc = Document::from_text("First\nLast");
    doc.styles.default_chr = CharProps { font: Some("Source Sans 3".into()), size: Some(10.0), ..Default::default() };
    doc.styles.default_para =
        ParaProps { line_spacing: Some(LineSpacing::Exactly(20.0)), space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
    doc.last_section.page_borders = Some(Borders::box_(Border::single(1.0)));
    for i in 0..count {
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 40.0,
            h: 20.0,
            fill: Some(wordcraft_doc::fill::Fill::Solid { color: wordcraft_doc::Rgb::BLACK, transparency: 0.0 }),
            stroke: None,
            stroke_width: 1.0,
            float: Float {
                wrap: Wrap::BehindText,
                h_rel: Anchor::Column,
                v_rel: Anchor::Paragraph,
                x: 20.0 + i as f32 * 50.0,
                y: 30.0,
                ..Default::default()
            },
            story: None,
            effects: Default::default(),
            freeform: None,
            extra: Default::default(),
        };
        doc.insert_object(&wordcraft_doc::Pos::body(0, 0), shape, &Default::default()).unwrap();
    }
    doc
}

#[test]
fn centered_behind_prefix() {
    check_aligned_prefix(PageVAlign::Center);
}

#[test]
fn bottom_behind_prefix() {
    check_aligned_prefix(PageVAlign::Bottom);
}

fn check_aligned_prefix(align: PageVAlign) {
    for count in [0, 1, 3] {
        let mut doc = aligned_prefix_doc(count);
        let top = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
        doc.last_section.valign = align;
        let original = doc.clone();
        let mut bare = doc.clone();
        bare.last_section.page_borders = None;
        let reference = layout(&bare, &mut LayoutCache::new(), &LayoutOptions::default());
        let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
        let rules = |laid: &DocLayout| -> Vec<_> {
            laid.pages[0]
                .items
                .iter()
                .filter_map(|item| match item {
                    Placed::Rule { x0, y0, x1, y1, .. } => Some((*x0, *y0, *x1, *y1)),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(rules(&top).len(), 4);
        assert_eq!(rules(&laid), rules(&top), "{align:?}, behind count {count}: page borders must stay fixed");
        for i in 0..count {
            let pos = wordcraft_doc::Pos::body(0, 0);
            assert_eq!(
                laid.object(&pos, i).unwrap().rect,
                reference.object(&pos, i).unwrap().rect,
                "page borders must not change body-anchored behind-object alignment"
            );
        }
        let body = |laid: &DocLayout| -> Vec<f32> {
            laid.pages[0]
                .items
                .iter()
                .filter_map(|item| match item {
                    Placed::Lines { story: StoryRef::Body, y, .. } => Some(*y),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(body(&laid), body(&reference), "page borders must not change body alignment");
        let (a, b) = (body(&top), body(&laid));
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), a.len());
        let shift = b[0] - a[0];
        assert!(shift > 0.0);
        assert!((b[1] - a[1] - shift).abs() < 0.01);
        assert_eq!(doc, original);
    }
}

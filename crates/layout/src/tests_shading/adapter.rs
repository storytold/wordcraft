use super::*;
use wordcraft_doc::para::{InlineObject, Wrap};
use wordcraft_doc::props::VAlign;

const IMAGE_MEDIA: &str = "generated-shading-adapter";
const IMAGE_SIZE: f32 = 12.0;

fn picture_shade_doc() -> Document {
    let mut d = shade_doc(ShadeFlow::Partial, ShadeKind::Paragraph);
    let Block::Table(t) = std::sync::Arc::make_mut(&mut d.body[0]) else { panic!("expected original table") };
    t.rows[0].cells[0].props.valign = Some(VAlign::Center);
    let mut p = Paragraph::with_text("Tag ", wordcraft_doc::CharProps { size: Some(12.0), ..Default::default() });
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    p.props.line_spacing = Some(LineSpacing::Exactly(LINE_HEIGHT));
    p.props.widow_control = Some(false);
    p.insert_object(
        p.text.len(),
        InlineObject::Image {
            media: IMAGE_MEDIA.into(),
            w: IMAGE_SIZE,
            h: IMAGE_SIZE,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        },
        &wordcraft_doc::CharProps::default(),
    )
    .unwrap();
    t.rows[0].cells[0].blocks = vec![para_block(p)];
    let Block::Para(p) = std::sync::Arc::make_mut(&mut t.rows[0].cells[1].blocks[0]) else { panic!("expected original paragraph") };
    p.props.shading = Some(SHADE);
    d
}

fn flow_snapshot(l: &DocLayout) -> Vec<Vec<String>> {
    l.pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    Placed::Fill { .. } => None,
                    Placed::Lines { story, path, para, l0, l1, x, y, turn } => Some(format!(
                        "Lines {story:?} {path:?} {l0}..{l1} {x:?} {y:?} {turn:?} {:?}",
                        para.lines[*l0..*l1].iter().map(|line| (line.top, line.height)).collect::<Vec<_>>()
                    )),
                    other => Some(format!("{other:?}")),
                })
                .collect()
        })
        .collect()
}

#[test]
fn shaded_sibling_keeps_picture() {
    let d = picture_shade_doc();
    let laid = shade_layout(&d);
    assert!(laid.pages.len() > 1);
    assert_paragraph_fill(&laid);
    let picture_path = wordcraft_doc::Path(vec![0, 0, 0, 0]);
    let first = &laid.pages[0];
    let (para, l0, l1) = first
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { path, para, l0, l1, .. } if path == &picture_path => Some((para, *l0, *l1)),
            _ => None,
        })
        .unwrap();
    assert_eq!((l0, l1), (0, para.lines.len()));
    assert_eq!(para.lines.len(), 1);
    let off = "Tag ".len();
    let line = &para.lines[0];
    assert!(para.clusters[line.c0..line.c1].iter().any(|c| c.start == off && matches!(c.kind, crate::para::ClKind::Object(0))));
    let hits: Vec<_> = first
        .items
        .iter()
        .filter_map(|item| match item {
            Placed::Object { path, off: actual, wrap: Wrap::Inline, rect, .. } if path == &picture_path && *actual == off => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].y >= PAGE_MARGIN && hits[0].bottom() <= PAGE_HEIGHT - PAGE_MARGIN);
    for page in laid.pages.iter().skip(1) {
        assert!(!page.items.iter().any(|item| match item {
            Placed::Lines { path, .. } | Placed::Object { path, .. } => path == &picture_path,
            _ => false,
        }));
    }
    let mut next = 0;
    for item in laid.pages.iter().flat_map(|page| &page.items) {
        if let Placed::Lines { path, l0, l1, .. } = item
            && path.0 == [0, 0, 1, 0]
        {
            assert_eq!(*l0, next);
            assert!(l1 > l0);
            next = *l1;
        }
    }
    assert_eq!(next, 20);
    let mut plain = d.clone();
    let Block::Table(t) = std::sync::Arc::make_mut(&mut plain.body[0]) else { panic!("expected original table") };
    let Block::Para(p) = std::sync::Arc::make_mut(&mut t.rows[0].cells[1].blocks[0]) else { panic!("expected original paragraph") };
    p.props.shading = None;
    let plain = shade_layout(&plain);
    assert_eq!(flow_snapshot(&laid), flow_snapshot(&plain), "paint ownership must not alter line/image/cell geometry");
}

#[test]
fn center_shade_keeps_fallback() {
    let mut d = shade_doc(ShadeFlow::Partial, ShadeKind::Paragraph);
    let Block::Table(t) = std::sync::Arc::make_mut(&mut d.body[0]) else { panic!("expected original table") };
    t.rows[0].cells[0].props.valign = Some(VAlign::Center);
    let Block::Para(p) = std::sync::Arc::make_mut(&mut t.rows[0].cells[0].blocks[0]) else { panic!("expected original paragraph") };
    let props = p.props.clone();
    let text_props = p.runs[0].props.clone();
    *p = Paragraph::with_text(&["Shade"; 8].join("\n"), text_props);
    p.props = props;
    let laid = shade_layout(&d);
    assert!(laid.pages.len() > 1);
    let snapshots: Vec<_> = laid.pages.iter().map(|page| page.items.iter().map(shade_snapshot).collect::<Vec<_>>()).collect();
    eprintln!("ADAPTER_CENTER_FALLBACK {}", serde_json::to_string(&snapshots).unwrap());
    let total: usize = laid
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|item| match item {
            Placed::Lines { path, l0, l1, .. } if path.0 == [0, 0, 0, 0] => Some(l1 - l0),
            _ => None,
        })
        .sum();
    assert_eq!(total, 8);
}

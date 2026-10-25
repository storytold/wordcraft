use super::*;
use std::sync::Arc;
use wordcraft_doc::para::{InlineObject, Wrap};
use wordcraft_doc::props::{CharProps, LineSpacing, ParaProps};
use wordcraft_doc::{Document, Path};

const LINE_HEIGHT: f32 = 20.0;
const ROW_HEIGHT: f32 = 124.0;
const CUT: f32 = 44.0;
const SIBLING_TEXT: &str = "First\nSecond\nThird\nFourth\nFifth\nSixth";
const IMAGE_MEDIA: &str = "generated-fragment-control";

fn generated_para(text: &str, height: f32, borders: Option<Borders>) -> Arc<ParaLayout> {
    let mut doc = Document::new();
    doc.body.clear();
    let mut paragraph = Paragraph::with_text(text, Default::default());
    paragraph.props = ParaProps {
        borders,
        space_before: Some(0.0),
        space_after: Some(0.0),
        line_spacing: Some(LineSpacing::Exactly(height)),
        widow_control: Some(false),
        ..Default::default()
    };
    doc.body.push(wordcraft_doc::para_block(paragraph));
    let layout = crate::layout(&doc, &mut crate::LayoutCache::new(), &crate::LayoutOptions::default());
    let para = layout
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Placed::Lines { path, para, .. } if path.0 == [0] => Some(para.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(para.text_len, text.len());
    assert_eq!(para.lines.len(), text.split(wordcraft_doc::para::LINE_BREAK).count());
    para
}

fn make_cell(items: std::ops::Range<usize>, end: f32) -> CellPart {
    CellPart { top: 0.0, center: None, merge: None, items, end: Some(end) }
}

fn make_row(height: f32, borders: Option<Borders>, sibling: &str) -> RowLayout {
    let offset = (ROW_HEIGHT - height) / 2.0;
    let para = generated_para("Marker", height, borders);
    let mut items = Vec::new();
    crate::push_para(&mut items, StoryRef::Body, &[0], &para, 0, 1, 0.0, offset, 100.0, crate::Edges::own(&para, 0, 1));
    items.push(Placed::Image {
        rect: Rect::new(50.0, offset + 2.0, 12.0, height - 4.0),
        media: IMAGE_MEDIA.into(),
        crop: [0.0; 4],
        story: StoryRef::Body,
        path: Path::top(0),
        off: 0,
        spin: Default::default(),
    });
    let group_end = items.len();
    let mut group = make_cell(0..group_end, offset + height);
    group.center = Some(CellCenter { kind: CenterKind::Fragment, offset, height, content: 0..group_end });

    let para = generated_para(sibling, LINE_HEIGHT, None);
    let sibling_height = para.lines.len() as f32 * LINE_HEIGHT;
    crate::push_para(&mut items, StoryRef::Body, &[1], &para, 0, para.lines.len(), 110.0, 0.0, 100.0, crate::Edges::own(&para, 0, para.lines.len()));
    let end = items.len();
    RowLayout { edge: 0.0, height: ROW_HEIGHT, min_height: 0.0, items, bottom: 4.0, cells: vec![group, make_cell(group_end..end, sibling_height)] }
}

fn baseline(row: &RowLayout, cut: f32) -> Option<(RowLayout, RowLayout)> {
    let mut row = row.clone();
    for cell in &mut row.cells {
        if cell.center.as_ref().is_some_and(|center| center.kind == CenterKind::Fragment) {
            cell.center = None;
        }
    }
    split_row_inner(&row, cut)
}

fn line_ranges(row: &RowLayout, path: u32) -> Vec<(usize, usize, f32)> {
    row.items
        .iter()
        .filter_map(|item| match item {
            Placed::Lines { path: owner, l0, l1, y, .. } if owner.0 == [path] => Some((*l0, *l1, *y)),
            _ => None,
        })
        .collect()
}

fn image_rects(row: &RowLayout) -> Vec<Rect> {
    row.items
        .iter()
        .filter_map(|item| match item {
            Placed::Image { media, rect, .. } if media == IMAGE_MEDIA => Some(*rect),
            _ => None,
        })
        .collect()
}

fn assert_same(actual: &RowLayout, expected: &RowLayout) {
    assert_eq!(actual.height, expected.height);
    assert_eq!(actual.min_height, expected.min_height);
    assert_eq!(actual.bottom, expected.bottom);
    assert_eq!(format!("{:?}", actual.items), format!("{:?}", expected.items));
    assert_eq!(actual.cells.len(), expected.cells.len());
    for (actual, expected) in actual.cells.iter().zip(&expected.cells) {
        assert_eq!(actual.items, expected.items);
        assert_eq!(actual.end, expected.end);
        assert_eq!(actual.top, expected.top);
        assert!(actual.center.is_none());
    }
}

fn assert_fallback(row: &RowLayout) {
    let expected = baseline(row, CUT).unwrap();
    let actual = split_row(row, CUT).unwrap();
    assert_same(&actual.0, &expected.0);
    assert_same(&actual.1, &expected.1);
    assert!(image_rects(&actual.0).is_empty());
    assert_eq!(image_rects(&actual.1).len(), 1);
}

#[test]
fn fitting_group_stays_whole() {
    let row = make_row(LINE_HEIGHT, None, SIBLING_TEXT);
    let original_items = format!("{:?}", row.items);
    let ordinary = baseline(&row, CUT).unwrap();
    assert!(line_ranges(&ordinary.0, 0).is_empty());
    assert!(image_rects(&ordinary.0).is_empty());
    assert_eq!(line_ranges(&ordinary.1, 0).len(), 1);
    assert_eq!(image_rects(&ordinary.1).len(), 1);

    let (first, rest) = split_row(&row, CUT).unwrap();
    assert_eq!(first.height, CUT);
    assert_eq!(rest.height, 84.0);
    assert_eq!(line_ranges(&first, 0), vec![(0, 1, 12.0)]);
    assert_eq!(image_rects(&first), vec![Rect::new(50.0, 14.0, 12.0, 16.0)]);
    assert!(line_ranges(&rest, 0).is_empty());
    assert!(image_rects(&rest).is_empty());
    assert_eq!(line_ranges(&first, 1), vec![(0, 2, 0.0)]);
    assert_eq!(line_ranges(&rest, 1), vec![(2, 6, 0.0)]);
    assert_eq!(format!("{:?}", row.items), original_items);
}

fn inline_paragraph() -> Paragraph {
    let props = CharProps { size: Some(10.0), ..Default::default() };
    let mut paragraph = Paragraph::with_text("Tag ", props.clone());
    paragraph.props = ParaProps {
        space_before: Some(0.0),
        space_after: Some(0.0),
        line_spacing: Some(LineSpacing::Exactly(LINE_HEIGHT)),
        widow_control: Some(false),
        ..Default::default()
    };
    let image_offset = paragraph.text.len();
    paragraph
        .insert_object(
            image_offset,
            InlineObject::Image {
                media: IMAGE_MEDIA.into(),
                w: 12.0,
                h: 12.0,
                alt: String::new(),
                float: Default::default(),
                crop: [0.0; 4],
                ole: None,
            },
            &props,
        )
        .unwrap();
    paragraph
}

#[test]
fn generated_inline_stays_whole() {
    let paragraph = inline_paragraph();
    let image_offset = paragraph.object_offsets()[0];
    assert!(matches!(paragraph.object_at(image_offset), Some(InlineObject::Image { media, .. }) if media == IMAGE_MEDIA));
    let mut doc = Document::new();
    doc.body.clear();
    doc.body.push(wordcraft_doc::para_block(paragraph));
    let laid = crate::layout(&doc, &mut crate::LayoutCache::new(), &crate::LayoutOptions::default());
    let mut group: Vec<Placed> = laid
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter(|item| match item {
            Placed::Lines { path, .. } | Placed::Object { path, .. } => path.0 == [0],
            _ => false,
        })
        .cloned()
        .collect();
    assert_eq!(group.len(), 2);
    let (owner, l0, l1, x, y) = group
        .iter()
        .find_map(|item| match item {
            Placed::Lines { para, l0, l1, x, y, .. } => Some((para.clone(), *l0, *l1, *x, *y)),
            _ => None,
        })
        .unwrap();
    assert_eq!((l0, l1), (0, 1));
    assert_eq!(owner.lines.len(), 1);
    let line = &owner.lines[0];
    assert!(owner.clusters[line.c0..line.c1].iter().any(|cluster| cluster.start == image_offset && matches!(cluster.kind, ClKind::Object(0))));
    let is_object = |item: &Placed| {
        matches!(item,
            Placed::Object { story: StoryRef::Body, path, off, wrap: Wrap::Inline, text_box: None, .. }
            if path.0 == [0] && *off == image_offset
        )
    };
    assert_eq!(group.iter().filter(|item| is_object(item)).count(), 1);

    let offset = (ROW_HEIGHT - LINE_HEIGHT) / 2.0;
    for item in &mut group {
        item.translate(-x, offset - y);
    }
    let mut row = make_row(LINE_HEIGHT, None, SIBLING_TEXT);
    // Preserve the emitted cluster and hit area; only move the generated group into the row.
    drop(row.items.splice(0..2, group));
    let ordinary = baseline(&row, CUT).unwrap();
    assert!(line_ranges(&ordinary.0, 0).is_empty());
    assert!(!ordinary.0.items.iter().any(|item| matches!(item, Placed::Object { .. })));
    assert_eq!(line_ranges(&ordinary.1, 0).len(), 1);
    assert_eq!(ordinary.1.items.iter().filter(|item| matches!(item, Placed::Object { .. })).count(), 1);

    let (first, rest) = split_row(&row, CUT).unwrap();
    assert_eq!(line_ranges(&first, 0), vec![(l0, l1, 12.0)]);
    assert_eq!(
        first
            .items
            .iter()
            .filter(|item| matches!(item,
                Placed::Lines { para, l0: a, l1: b, .. } if Arc::ptr_eq(para, &owner) && *a == l0 && *b == l1
            ))
            .count(),
        1
    );
    assert_eq!(first.items.iter().filter(|item| is_object(item)).count(), 1);
    let rect = first
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Object { rect, .. } if is_object(item) => Some(rect),
            _ => None,
        })
        .unwrap();
    assert!(rect.y >= 0.0);
    assert!(rect.bottom() <= first.height - first.bottom);
    assert!(line_ranges(&rest, 0).is_empty());
    assert!(!rest.items.iter().any(|item| matches!(item, Placed::Object { .. })));
    assert_eq!(line_ranges(&first, 1), vec![(0, 2, 0.0)]);
    assert_eq!(line_ranges(&rest, 1), vec![(2, 6, 0.0)]);
}

#[test]
fn rescued_item_ownership() {
    let row = make_row(LINE_HEIGHT, None, SIBLING_TEXT);
    let (first, rest) = split_row(&row, CUT).unwrap();
    assert_eq!(first.items.len(), 3);
    assert_eq!(rest.items.len(), 1);
    assert!(matches!(&first.items[0], Placed::Lines { path, .. } if path.0 == [0]));
    assert!(matches!(&first.items[1], Placed::Image { media, .. } if media == IMAGE_MEDIA));
    assert!(matches!(&first.items[2], Placed::Lines { path, .. } if path.0 == [1]));
    assert_eq!(rest.cells.len(), 2);
    assert_eq!(rest.cells[0].items, 0..0);
    assert_eq!(rest.cells[1].items, 0..1);
    assert!(image_rects(&rest).is_empty());
    assert_eq!(line_ranges(&rest, 1), vec![(2, 6, 0.0)]);
}

#[test]
fn non_fitting_group_falls_back() {
    let row = make_row(60.0, None, SIBLING_TEXT);
    assert!(row.cells[0].center.as_ref().unwrap().height > CUT);
    assert_fallback(&row);
}

#[test]
fn text_only_keeps_baseline() {
    let mut row = make_row(LINE_HEIGHT, None, SIBLING_TEXT);
    row.items.remove(1);
    row.cells[0].items = 0..1;
    row.cells[0].center.as_mut().unwrap().content = 0..1;
    row.cells[1].items = 1..2;
    let expected = baseline(&row, CUT).unwrap();
    let actual = split_row(&row, CUT).unwrap();
    assert_same(&actual.0, &expected.0);
    assert_same(&actual.1, &expected.1);
    assert!(line_ranges(&actual.0, 0).is_empty());
    assert_eq!(line_ranges(&actual.1, 0).len(), 1);
}

#[test]
fn mixed_merge_keeps_baseline() {
    let mut row = make_row(LINE_HEIGHT, None, SIBLING_TEXT);
    row.cells[1].center = Some(CellCenter { kind: CenterKind::Merge, offset: 0.0, height: 120.0, content: 2..3 });
    assert_fallback(&row);
}

#[test]
fn line_overflow_keeps_group() {
    let mut row = make_row(CUT, None, SIBLING_TEXT);
    // The image can fit after translation, but its line crosses the reserved bottom band.
    if let Placed::Image { rect, .. } = &mut row.items[1] {
        rect.h = 36.0;
    }
    assert_fallback(&row);
}

#[test]
fn bordered_group_falls_back() {
    let row = make_row(LINE_HEIGHT, Some(Borders { top: Some(Border::single(1.0)), ..Default::default() }), SIBLING_TEXT);
    assert!(row.items.iter().any(|item| matches!(item, Placed::Rule { .. })));
    assert_fallback(&row);
}

#[test]
fn no_sibling_remainder_fallback() {
    let mut row = make_row(LINE_HEIGHT, None, "First\nSecond");
    // A separate non-text cell sustains a split without extending the text cell's last paragraph.
    let tail_start = row.items.len();
    row.items.push(Placed::Image {
        rect: Rect::new(220.0, 100.0, 12.0, 12.0),
        media: "generated-sibling-tail".into(),
        crop: [0.0; 4],
        story: StoryRef::Body,
        path: Path::top(2),
        off: 0,
        spin: Default::default(),
    });
    row.cells.push(make_cell(tail_start..row.items.len(), 112.0));
    let ordinary = baseline(&row, CUT).unwrap();
    assert_eq!(line_ranges(&ordinary.0, 1), vec![(0, 2, 0.0)]);
    assert!(line_ranges(&ordinary.1, 1).is_empty());
    assert!(ordinary.1.items.iter().any(|item| matches!(item, Placed::Image { media, .. } if media == "generated-sibling-tail")));
    assert_fallback(&row);
}

#[test]
fn no_baseline_split_falls_back() {
    let mut row = make_row(LINE_HEIGHT, None, SIBLING_TEXT);
    // No sibling establishes a first fragment, even though recentering could rescue the group.
    let start = row.cells[1].items.start;
    for item in &mut row.items[start..] {
        item.translate(0.0, 52.0);
    }
    row.cells[1].end = Some(172.0);
    row.height = 176.0;
    for item in &mut row.items[..start] {
        item.translate(0.0, 26.0);
    }
    row.cells[0].end = Some(98.0);
    row.cells[0].center.as_mut().unwrap().offset = 78.0;
    assert!(baseline(&row, CUT).is_none());
    assert!(split_row(&row, CUT).is_none());
}

#[test]
fn inline_table_stays_on_page() {
    const PAGE_MARGIN: f32 = 40.0;
    const PAGE_HEIGHT: f32 = 124.0;

    let mut doc = Document::new();
    doc.last_section.page_w = 300.0;
    doc.last_section.page_h = PAGE_HEIGHT;
    doc.last_section.margin_top = PAGE_MARGIN;
    doc.last_section.margin_bottom = PAGE_MARGIN;
    doc.last_section.margin_left = PAGE_MARGIN;
    doc.last_section.margin_right = PAGE_MARGIN;
    let mut table = Table::new(1, 2, 200.0);
    table.props.style = None;
    table.props.fixed = true;
    table.props.borders = Some(Borders::default());
    table.props.cell_margins = Some([0.0; 4]);
    table.rows[0].cells[0].props.valign = Some(VAlign::Center);
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(inline_paragraph())];
    let mut sibling = Paragraph::with_text(SIBLING_TEXT, Default::default());
    sibling.props = ParaProps {
        space_before: Some(0.0),
        space_after: Some(0.0),
        line_spacing: Some(LineSpacing::Exactly(LINE_HEIGHT)),
        widow_control: Some(false),
        ..Default::default()
    };
    table.rows[0].cells[1].blocks = vec![wordcraft_doc::para_block(sibling)];
    doc.body = vec![Arc::new(Block::Table(table))];
    let laid = crate::layout(&doc, &mut crate::LayoutCache::new(), &crate::LayoutOptions::default());
    assert!(laid.pages.len() > 1);
    let path = Path(vec![0, 0, 0, 0]);
    let first = &laid.pages[0];
    let (para, l0, l1) = first
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { path: owner, para, l0, l1, .. } if owner == &path => Some((para, *l0, *l1)),
            _ => None,
        })
        .unwrap();
    assert_eq!((l0, l1), (0, para.lines.len()));
    assert_eq!(para.lines.len(), 1);
    let off = "Tag ".len();
    let line = &para.lines[0];
    assert!(para.clusters[line.c0..line.c1].iter().any(|c| c.start == off && matches!(c.kind, ClKind::Object(0))));
    assert_eq!(
        first
            .items
            .iter()
            .filter(|item| matches!(item,
                Placed::Object { path: owner, off: offset, wrap: Wrap::Inline, rect, .. }
                if owner == &path && *offset == off && rect.y >= PAGE_MARGIN && rect.bottom() <= PAGE_HEIGHT - PAGE_MARGIN
            ))
            .count(),
        1
    );
    for page in laid.pages.iter().skip(1) {
        assert!(!page.items.iter().any(|item| match item {
            Placed::Lines { path: owner, .. } | Placed::Object { path: owner, .. } => owner == &path,
            _ => false,
        }));
    }
}

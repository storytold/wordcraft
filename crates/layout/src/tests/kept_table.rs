//! Synthetic keep-with-next controls observed in Word for Mac 16.114.3.
use super::*;
use wordcraft_doc::props::{HeightRule, LineSpacing};

fn control(header: f32, minimum: f32, cant_split: bool, count: usize, keep: bool) -> Document {
    let plain = ParaProps {
        space_before: Some(0.0),
        space_after: Some(0.0),
        line_spacing: Some(LineSpacing::Exactly(12.0)),
        widow_control: Some(false),
        ..Default::default()
    };
    let paragraph = |text: &str, props| {
        let mut p = Paragraph::with_text(text, Default::default());
        p.props = props;
        wordcraft_doc::para_block(p)
    };
    let mut doc = pagination_fixture_document("SYNTHETIC FILLER");
    doc.last_section.page_w = 400.0;
    doc.last_section.page_h = 360.0;
    doc.last_section.margin_top = 36.0;
    doc.last_section.margin_bottom = 36.0;
    doc.last_section.margin_left = 36.0;
    doc.last_section.margin_right = 36.0;
    let mut table = Table::new(2, 1, 328.0);
    table.props.style = None;
    table.props.fixed = true;
    table.props.cell_margins = Some([0.0; 4]);
    table.rows[0].props.header = true;
    table.rows[0].props.height = Some(header);
    table.rows[0].props.height_rule = HeightRule::AtLeast;
    table.rows[0].cells[0].blocks = vec![paragraph("SYNTHETIC HEADER", plain.clone())];
    table.rows[1].props.cant_split = cant_split;
    if minimum > 0.0 {
        table.rows[1].props.height = Some(minimum);
        table.rows[1].props.height_rule = HeightRule::AtLeast;
    }
    table.rows[1].cells[0].blocks = (0..count).map(|n| paragraph(&format!("SYNTHETIC ROW {n:03}"), plain.clone())).collect();
    doc.body = vec![
        paragraph("SYNTHETIC FILLER", ParaProps { line_spacing: Some(LineSpacing::Exactly(170.0)), ..plain.clone() }),
        paragraph(
            "SYNTHETIC HEADING",
            ParaProps {
                line_spacing: Some(LineSpacing::Exactly(14.0)),
                space_after: Some(6.0),
                keep_next: Some(keep),
                keep_lines: Some(true),
                ..plain
            },
        ),
        Arc::new(Block::Table(table)),
    ];
    doc
}
fn heading_page(doc: &Document) -> usize {
    let layout = lay(doc);
    assert_pagination_fixture_font(&layout);
    layout.caret(&Pos::body(1, 0)).unwrap().page
}

#[test]
fn kept_heading_follows_header_and_fitting_indivisible_row() {
    for header in [28.0, 72.0] {
        assert_eq!(heading_page(&control(header, 100.0, true, 1, true)), 1, "header {header}");
    }
}
#[test]
fn unkept_heading_stays_before_table_page_break() {
    assert_eq!(heading_page(&control(28.0, 100.0, true, 1, false)), 0);
}
#[test]
fn kept_heading_moves_once_even_before_an_oversized_indivisible_row() {
    assert_eq!(heading_page(&control(28.0, 0.0, true, 100, true)), 1);
}
#[test]
fn kept_heading_stays_when_first_body_fragment_can_start() {
    assert_eq!(heading_page(&control(28.0, 0.0, false, 30, true)), 0);
}
#[test]
fn kept_heading_at_page_top_does_not_create_a_blank_page() {
    let mut doc = control(28.0, 0.0, true, 100, true);
    doc.body.remove(0);
    let layout = lay(&doc);
    assert_eq!(layout.caret(&Pos::body(0, 0)).unwrap().page, 0);
    assert!(layout.pages.len() < 10);
}

#[test]
fn kept_heading_stays_when_the_whole_table_prefix_fits() {
    let mut doc = control(28.0, 100.0, true, 1, true);
    doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.line_spacing = Some(LineSpacing::Exactly(100.0));
    assert_eq!(heading_page(&doc), 0);
}

#[test]
fn kept_heading_checks_a_first_row_without_headers() {
    let mut doc = control(28.0, 100.0, true, 1, true);
    doc.table_mut(StoryRef::Body, &Path::top(2)).unwrap().rows.remove(0);
    assert_eq!(heading_page(&doc), 1);
}

#[test]
fn table_lookahead_preserves_list_counters() {
    let mut doc = control(28.0, 100.0, true, 1, true);
    let num = doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    let numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
    doc.para_mut(StoryRef::Body, &Path(vec![2, 1, 0, 0])).unwrap().props.numbering = numbering;
    let mut next = Paragraph::with_text("SYNTHETIC FOLLOWING ITEM", Default::default());
    next.props.numbering = numbering;
    doc.body.push(wordcraft_doc::para_block(next));
    let layout = lay(&doc);
    let labels: Vec<_> = layout
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|item| match item {
            Placed::Lines { para, l0: 0, .. } => para.label.as_ref().map(|label| label.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["1.", "2."], "probing the table must not number its cells twice");
}

/// A wrapping shape clears the table's full width even when the heading can fit beside it.
#[test]
fn kept_heading_counts_float_clearance_before_the_table() {
    use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
    for (overlap, keep, expected) in [(true, true, 1), (true, false, 0), (false, true, 0)] {
        let mut doc = control(28.0, 40.0, true, 1, keep);
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 20.0,
            h: 70.0,
            fill: None,
            stroke: None,
            stroke_width: 0.0,
            float: Float {
                wrap: Wrap::Square,
                h_rel: Anchor::Page,
                v_rel: Anchor::Page,
                x: if overlap { 300.0 } else { 372.0 },
                y: 210.0,
                dist: 0.0,
                dist_top: 0.0,
                dist_bottom: 0.0,
                ..Default::default()
            },
            story: None,
            effects: Default::default(),
            freeform: None,
            extra: Default::default(),
        };
        doc.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
        let layout = lay(&doc);
        assert_pagination_fixture_font(&layout);
        let heading = layout.caret(&Pos::body(1, 0)).unwrap().page;
        let table = layout.caret(&Pos { story: StoryRef::Body, path: Path(vec![2, 0, 0, 0]), off: 0 }).unwrap().page;
        assert_eq!([heading, table], [expected, usize::from(overlap)], "overlap={overlap}, keep={keep}");
    }
}

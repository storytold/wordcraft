use super::*;
use wordcraft_doc::para::{Anchor, Float, Wrap};
use wordcraft_doc::{PartKind, SectionProps, para_block};

#[test]
fn mixed_font_extra_height_is_not_multiplied_by_line_spacing() {
    crate::test_fonts();
    let height = |multiple, mixed| {
        let mut d = pagination_fixture_document("Alpha probe");
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(multiple));
        d.format_range(&Pos::body(0, 0), &Pos::body(0, 11), &|c| {
            c.font = Some("Inter".into());
            c.size = Some(12.0);
        })
        .unwrap();
        if mixed {
            d.format_range(&Pos::body(0, 0), &Pos::body(0, 5), &|c| {
                c.font = Some("Source Sans 3".into());
                c.size = Some(10.5);
            })
            .unwrap();
        }
        lay(&d).pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Lines { para, .. } => Some(para.lines[0].height),
                _ => None,
            })
            .unwrap()
    };
    let extra = height(1.0, true) - height(1.0, false);
    assert!(extra > 0.01, "the fonts must contribute different ascent/descent maxima");
    for multiple in [1.15, 1.5, 2.0] {
        let spaced_extra = height(multiple, true) - height(multiple, false);
        assert!((spaced_extra - extra).abs() < 0.001, "mixed-font overflow stays constant: {extra}, {spaced_extra}");
    }
}

#[test]
fn filename_field_wraps_inside_header_cell() {
    let mut d = Document::from_text("Body");
    let mut table = Table::new(1, 2, 240.0);
    table.props.style = None;
    table.props.width = Some(240.0);
    table.props.fixed = true;
    let mut p = Paragraph::new();
    p.insert_object(
        0,
        InlineObject::Field {
            instr: "FILENAME".into(),
            result: "A_very_long_report_filename_that_cannot_fit_in_one_cell.docx".into(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        },
        &Default::default(),
    )
    .unwrap();
    table.rows[0].cells[0].blocks = vec![para_block(p)];
    table.rows[0].cells[1].blocks = vec![para_block(Paragraph::with_text("Confidential", Default::default()))];
    let id = d.add_part(PartKind::Header, vec![Arc::new(Block::Table(table))]);
    d.last_section.headers.default = Some(id);
    let l = lay(&d);
    let para = l.pages[0]
        .header
        .iter()
        .find_map(|it| match it {
            Placed::Lines { path, para, .. } if path.depth() > 0 && !para.shown.is_empty() => Some(para),
            _ => None,
        })
        .unwrap();
    assert!(para.lines.len() > 1, "a filename must wrap rather than overlap its neighbour");
    for line in &para.lines {
        let right = line.xs.iter().zip(&para.clusters[line.c0..line.c1]).map(|(x, c)| x + c.adv).fold(0.0, f32::max);
        assert!(right <= line.right + 0.1, "the field must stay within its cell");
    }
}

#[test]
fn page_relative_header_image_does_not_follow_cell_vertical_alignment() {
    let mut d = Document::from_text("Body");
    let mut table = Table::new(1, 1, 240.0);
    table.props.style = None;
    table.rows[0].props.height = Some(90.0);
    table.rows[0].cells[0].props.valign = Some(wordcraft_doc::props::VAlign::Bottom);
    let mut p = Paragraph::new();
    p.insert_object(
        0,
        InlineObject::Image {
            media: "synthetic-logo".into(),
            w: 50.0,
            h: 20.0,
            alt: String::new(),
            float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Page, x: 5.0, y: 12.0, ..Default::default() },
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        },
        &Default::default(),
    )
    .unwrap();
    table.rows[0].cells[0].blocks = vec![para_block(p)];
    let id = d.add_part(PartKind::Header, vec![Arc::new(Block::Table(table))]);
    d.last_section.headers.default = Some(id);
    let image_y = |d: &Document| {
        lay(d).pages[0]
            .header
            .iter()
            .find_map(|it| match it {
                Placed::Image { rect, .. } => Some(rect.y),
                _ => None,
            })
            .unwrap()
    };
    let bottom = image_y(&d);
    let Block::Table(table) = Arc::make_mut(&mut d.parts.get_mut(&id).unwrap().blocks[0]) else { panic!("header table") };
    table.rows[0].cells[0].props.valign = Some(wordcraft_doc::props::VAlign::Top);
    assert!((image_y(&d) - bottom).abs() < 0.01, "row alignment must not move the page-relative image");
}

#[test]
fn trailing_page_break_and_empty_section_mark_share_the_next_page() {
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("Cover\u{c}", Default::default()))];
    let mut end = Paragraph::new();
    end.section = Some(Box::new(SectionProps::default()));
    d.body.push(para_block(end));
    d.body.push(para_block(Paragraph::with_text("Body", Default::default())));
    let l = lay(&d);
    assert_eq!(l.pages.len(), 2, "a page break adjacent to a section break must not leave an empty page");
}

#[test]
fn multiple_spaced_body_uses_the_same_text_fit_above_a_footer_as_above_the_margin() {
    use wordcraft_doc::props::LineSpacing;
    for multiple in [1.15, 1.5, 2.0] {
        let mut d = Document::from_text(&"Words on the last line must fit above the footer. ".repeat(120));
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.props.line_spacing = Some(LineSpacing::Multiple(multiple));
        p.props.widow_control = Some(false);
        let footer = d.add_part(PartKind::Footer, vec![para_block(Paragraph::with_text("Footer", Default::default()))]);
        d.last_section.footers.default = Some(footer);
        d.last_section.footer = d.last_section.margin_bottom + 20.0;
        let l = lay(&d);
        let top = l.pages[0]
            .footer
            .iter()
            .find_map(|it| match it {
                Placed::Lines { y, .. } => Some(*y),
                _ => None,
            })
            .unwrap();
        let mut margin = d.clone();
        margin.last_section.footers.default = None;
        margin.last_section.margin_bottom = margin.last_section.page_h - top;
        let lines = |doc: &Document| {
            lay(doc).pages[0]
                .items
                .iter()
                .filter_map(|it| match it {
                    Placed::Lines { l0, l1, .. } => Some(l1 - l0),
                    _ => None,
                })
                .sum::<usize>()
        };
        assert_eq!(lines(&d), lines(&margin), "{multiple}: footer and margin must use the same last-line fit");
    }
}

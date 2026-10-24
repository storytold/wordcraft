//! #3977: floating tables include a line's bottom edge; shapes keep a strict boundary.
//! The expected distinction is cyril265's Word for Mac 16.114.1 observation in the issue.

use super::*;
use wordcraft_doc::para::{Anchor, Float, InlineObject, ShapeKind, Wrap};
use wordcraft_doc::props::{HeightRule, LineSpacing, ParaProps, TableFloat};
use wordcraft_doc::{CharProps, Paragraph, Pos, Table};

const LINE: f32 = 82.875;
const CONTACT: f32 = 403.5;

fn document(mode: u32) -> Document {
    let mut d = Document::new();
    d.settings.compat_mode = mode;
    let ch = CharProps { font: Some("Source Sans 3".into()), size: Some(62.5), ..Default::default() };
    d.body = (1..=6)
        .map(|n| {
            let mut p = Paragraph::with_text(&format!("P{n}"), ch.clone());
            p.mark = ch.clone();
            p.props = ParaProps {
                space_before: Some(0.0),
                space_after: Some(0.0),
                line_spacing: Some(LineSpacing::Multiple(1.0)),
                widow_control: Some(false),
                ..Default::default()
            };
            wordcraft_doc::para_block(p)
        })
        .collect();
    d
}

fn add_table(d: &mut Document, top: f32, width: f32) {
    let mut t = Table::new(1, 1, width);
    t.props.style = None;
    t.props.width = Some(width);
    t.props.fixed = true;
    t.props.cell_margins = Some([0.0; 4]);
    t.props.float = Some(TableFloat { h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: top, ..Default::default() });
    t.rows[0].props.height = Some(50.0);
    t.rows[0].props.height_rule = HeightRule::Exact;
    d.body.insert(0, Arc::new(Block::Table(t)));
}

fn add_shape(d: &mut Document, top: f32, left: f32, width: f32, height: f32) {
    let shape = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: width,
        h: height,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Page, x: left, y: top, ..Default::default() },
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    let block = d.body.iter().position(|b| matches!(&**b, Block::Para(_))).unwrap();
    d.insert_object(&Pos::body(block, 0), shape, &Default::default()).unwrap();
}

fn line_positions(d: &Document) -> Vec<(f32, f32, f32)> {
    let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
    assert_eq!(l.pages.len(), 1);
    let lines: Vec<_> = l.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Placed::Lines { path, x, y, para, story: StoryRef::Body, .. } if path.0.len() == 1 => {
                assert_eq!(para.lines.len(), 1);
                let line = &para.lines[0];
                // Placed::Lines::y is already the top of its first displayed line.
                Some((x + line.left, *y, line.height))
            }
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 6);
    lines
}

#[test]
fn only_tables_include_exact_line_bottom_contact_in_all_compatibility_modes() {
    for mode in [11, 12, 14, 15] {
        let control = line_positions(&document(mode));
        assert_eq!(control[3].1 + control[3].2, CONTACT);
        assert!(control.iter().enumerate().all(|(n, l)| l.1 == 72.0 + n as f32 * LINE && l.2 == LINE));
        for table in [false, true] {
            for twips in [-1, 0, 1] {
                let top = CONTACT + twips as f32 / 20.0;
                let mut d = document(mode);
                if table {
                    add_table(&mut d, top, 468.0);
                } else {
                    add_shape(&mut d, top, 72.0, 468.0, 50.0);
                }
                let lines = line_positions(&d);
                let moves = twips < 0 || (table && twips == 0);
                let expected = if moves { top + 50.0 } else { control[3].1 };
                assert!((lines[3].1 - expected).abs() < 0.001, "mode={mode}, table={table}, twips={twips}: {lines:?}");
                assert_eq!(&lines[..3], &control[..3]);
                // If P4 stays above, P5 is the first line that meets the object.
                let next = if moves { lines[3].1 + LINE } else { top + 50.0 };
                assert!((lines[4].1 - next).abs() < 0.001, "mode={mode}, table={table}, twips={twips}: {lines:?}");
                assert_eq!(lines[5].1, lines[4].1 + LINE);
            }
        }
    }
}

#[test]
fn a_word_skipping_a_narrow_strip_at_table_contact_resumes_when_the_table_ends() {
    for mode in [11, 12, 14, 15] {
        let mut d = document(mode);
        // Before contact the shape leaves 200 pt on the left. At contact the table takes
        // 168 pt of that, leaving a 32 pt strip, too narrow for P4. The table ends before
        // the shape: P4 must use the newly available left strip, not jump below the shape.
        add_table(&mut d, CONTACT, 168.0);
        add_shape(&mut d, 72.0, 272.0, 268.0, 500.0);
        let lines = line_positions(&d);
        assert!((lines[3].1 - (CONTACT + 50.0)).abs() < 0.001, "mode={mode}: {lines:?}");
        assert_eq!(lines[3].0, 72.0);
        assert!(lines[..3].iter().enumerate().all(|(n, l)| l.1 == 72.0 + n as f32 * LINE));
    }
}

#[test]
fn exact_spacing_does_not_make_mark_height_a_table_contact_boundary() {
    for mode in [11, 12, 14, 15] {
        for table in [false, true] {
            let mut d = Document::new();
            d.settings.compat_mode = mode;
            let mut p = Paragraph::with_text("P1", CharProps { font: Some("Source Sans 3".into()), size: Some(11.0), ..Default::default() });
            p.mark = CharProps { font: Some("Source Sans 3".into()), size: Some(62.5), ..Default::default() };
            p.props =
                ParaProps { line_spacing: Some(LineSpacing::Exactly(12.0)), space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
            d.body = vec![wordcraft_doc::para_block(p)];
            // The mark's natural 82.875 pt estimate ends far below the actual 12 pt line.
            let top = 72.0 + LINE;
            if table {
                add_table(&mut d, top, 468.0);
            } else {
                add_shape(&mut d, top, 72.0, 468.0, 50.0);
            }
            let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
            let (y, height) = l.pages[0]
                .items
                .iter()
                .find_map(|item| match item {
                    Placed::Lines { story: StoryRef::Body, path, y, para, .. } if path.0.len() == 1 => Some((*y, para.lines[0].height)),
                    _ => None,
                })
                .unwrap();
            assert_eq!(height, 12.0);
            assert_eq!(y, 72.0, "mode={mode}, table={table}");
        }
    }
}

fn exact_document(mode: u32, mark_size: f32) -> Document {
    let mut d = Document::new();
    d.settings.compat_mode = mode;
    let mut p = Paragraph::with_text("WWWWWWWWWWWW", CharProps { font: Some("Source Sans 3".into()), size: Some(11.0), ..Default::default() });
    p.mark = CharProps { font: Some("Source Sans 3".into()), size: Some(mark_size), ..Default::default() };
    p.props = ParaProps { line_spacing: Some(LineSpacing::Exactly(12.0)), space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
    d.body = vec![wordcraft_doc::para_block(p)];
    d
}

fn exact_line(d: &Document) -> (f32, f32) {
    let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
    assert_eq!(l.pages.len(), 1);
    l.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { story: StoryRef::Body, path, x, y, para, .. } if path.0 == vec![(d.body.len() - 1) as u32] => {
                assert_eq!(para.lines.len(), 1);
                assert_eq!(para.lines[0].height, 12.0);
                Some((x + para.lines[0].left, *y))
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn exact_table_contact_uses_the_spacing_with_larger_and_smaller_marks() {
    for mode in [11, 12, 14, 15] {
        for mark in [4.0, 62.5] {
            for twips in [-1, 0, 1] {
                let mut d = exact_document(mode, mark);
                let top = 84.0 + twips as f32 / 20.0;
                add_table(&mut d, top, 468.0);
                let (_, y) = exact_line(&d);
                let expected = if twips <= 0 { top + 50.0 } else { 72.0 };
                assert!((y - expected).abs() < 0.001, "mode={mode}, mark={mark}, twips={twips}: {y}");
            }
        }
    }
}

#[test]
fn exact_table_contact_uses_the_spacing_when_a_word_skips_a_strip() {
    for mode in [11, 12, 14, 15] {
        let mut d = exact_document(mode, 4.0);
        // Anchor the shape in an earlier paragraph so the tested word has no inline anchor.
        let Block::Para(p) = &*d.body[0] else { panic!("expected paragraph") };
        let mut anchor = Paragraph::with_text("P0", p.mark.clone());
        anchor.mark = p.mark.clone();
        anchor.props = p.props.clone();
        d.body.insert(0, wordcraft_doc::para_block(anchor));
        add_table(&mut d, 96.0, 168.0);
        add_shape(&mut d, 72.0, 272.0, 268.0, 500.0);
        // The 32 pt strip cannot hold the word. The table ends before the shape,
        // leaving 200 pt on the left, so skipping must stop at the table's bottom.
        assert_eq!(exact_line(&d), (72.0, 146.0), "mode={mode}");
    }
}

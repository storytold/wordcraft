//! Layout invariants for whitespace and tab stops across OOXML half-point sizes.
//! These tests do not claim that the results match Word.

use super::*;
use wordcraft_doc::para::Run;
use wordcraft_doc::para_block;
use wordcraft_doc::props::{TabAlign, TabLeader, TabStop, TextColor};

fn props(size: f32, spacing: f32, scale: f32) -> CharProps {
    CharProps { font: Some("Source Sans 3".into()), size: Some(size), spacing: Some(spacing), scale: Some(scale), ..Default::default() }
}

fn paragraph(text: &str, props: CharProps, split: bool) -> Paragraph {
    let mut p = Paragraph::with_text(text, props.clone());
    p.props.auto_space_de = Some(false);
    p.props.auto_space_dn = Some(false);
    if split {
        p.runs = text
            .chars()
            .enumerate()
            .map(|(i, ch)| {
                let mut part = props.clone();
                part.color = Some(TextColor::Rgb(Rgb((i % 2) as u8, 0, 0)));
                Run { len: ch.len_utf8(), props: part }
            })
            .collect();
        p.touch();
    }
    p
}

fn paragraphs(doc: &Document) -> Vec<Arc<ParaLayout>> {
    lay(doc)
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter_map(|item| match item {
            Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn left_right_decimal_and_default_tabs_reach_their_stops_at_every_half_point() {
    for half_points in 2..=144 {
        let size = half_points as f32 / 2.0;
        for (align, offset, expected) in
            [(None, 1, 36.0), (Some(TabAlign::Left), 1, 200.0), (Some(TabAlign::Right), 5, 200.0), (Some(TabAlign::Decimal), 3, 200.0)]
        {
            let mut doc = Document::new();
            let mut p = paragraph("\t12.3", props(size, 0.0, 100.0), false);
            if let Some(align) = align {
                p.props.tabs = Some(vec![TabStop { pos: 200.0, align, leader: TabLeader::None }]);
            }
            doc.body = vec![para_block(p)];
            let laid = paragraphs(&doc);
            let para = &laid[0];
            let x = para.x_of(0, offset).expect("a tab-aligned caret exists");
            assert!((x - expected).abs() < 0.1, "{size}pt {align:?} offset {offset}: {x}, expected {expected}");
        }
    }
}

#[test]
fn consecutive_and_mixed_tabs_keep_stops_across_run_boundaries() {
    for half_points in 2..=144 {
        let size = half_points as f32 / 2.0;
        for (text, explicit) in [("\t\tZ", false), ("\t \tZ", false), ("A \t\t B", false), ("A \t\t B", true)] {
            let mut doc = Document::new();
            let make = |split| {
                let mut p = paragraph(text, props(size, 0.0, 100.0), split);
                if explicit {
                    p.props.tabs =
                        Some([120.0, 240.0].into_iter().map(|pos| TabStop { pos, align: TabAlign::Left, leader: TabLeader::None }).collect());
                }
                p
            };
            doc.body = vec![para_block(make(false)), para_block(make(true))];
            let laid = paragraphs(&doc);
            assert_eq!(laid.len(), 2);
            for (off, ch) in text.char_indices() {
                let x = laid[0].x_of(0, off).expect("before character");
                let split_x = laid[1].x_of(0, off).expect("before character in split runs");
                assert!((x - split_x).abs() < 0.01, "{size}pt {text:?}, explicit {explicit}, byte {off}: {x} vs {split_x}");
                if ch == '\t' {
                    let after = laid[0].x_of(0, off + 1).expect("after tab");
                    let expected = if explicit { if x < 119.99 { 120.0 } else { 240.0 } } else { ((x + 0.01) / 36.0).floor() * 36.0 + 36.0 };
                    assert!((after - expected).abs() < 0.1, "{size}pt {text:?}, explicit {explicit}: {x} → {after}, expected {expected}");
                }
            }
        }
    }
}

#[test]
fn decimal_tabs_keep_the_separator_aligned_when_the_segment_ends() {
    for (text, aligned_offset) in [
        ("\t12.34", 3),
        ("\t12,34", 3),
        ("\t12.34.56", 3),
        ("\t12,34.56", 3),
        ("\t1234", 5),
        ("\t12.34\tX", 3),
        ("\t12.34\nX", 3),
        ("\t1234\t56.78", 5),
        ("\t1234\n56.78", 5),
    ] {
        for split in [false, true] {
            let mut doc = Document::new();
            let mut p = paragraph(text, props(9.0, 0.0, 100.0), split);
            p.props.tabs = Some(vec![TabStop { pos: 200.0, align: TabAlign::Decimal, leader: TabLeader::None }]);
            doc.body = vec![para_block(p)];
            let laid = paragraphs(&doc);
            let x = laid[0].x_of(0, aligned_offset).expect("tab segment caret");
            assert!((x - 200.0).abs() < 0.1, "{text:?}, split {split}: expected decimal at 200pt, got {x}");
        }
    }
}

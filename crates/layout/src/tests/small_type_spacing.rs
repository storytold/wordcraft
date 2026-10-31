//! Layout invariants for whitespace and tab stops across OOXML half-point sizes.
//! These tests do not claim that the results match Word.

use super::*;
use wordcraft_doc::para::Run;
use wordcraft_doc::para_block;
use wordcraft_doc::props::TextColor;
use wordcraft_doc::section::{DocGrid, DocGridType};

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
fn spaces_keep_advances_when_only_run_color_boundaries_change() {
    for half_points in 2..=144 {
        let size = half_points as f32 / 2.0;
        for grid in [false, true] {
            for balance in [false, true] {
                let mut doc = Document::new();
                doc.body.clear();
                doc.settings.balance_single_byte_double_byte_width = balance;
                if grid {
                    doc.last_section.doc_grid = Some(DocGrid { kind: DocGridType::LinesAndChars, line_pitch: 18.0, char_space: -6030 });
                }
                let mut descriptions = Vec::new();
                for ch in [' ', '\u{a0}', '\u{3000}'] {
                    for count in [1, 2, 4, 8] {
                        let text = format!("A{}B", ch.to_string().repeat(count));
                        let (spacing, scale) = if count % 4 == 0 { (1.0, 80.0) } else { (0.0, 100.0) };
                        let char_props = props(size, spacing, scale);
                        doc.body.push(para_block(paragraph(&text, char_props.clone(), false)));
                        doc.body.push(para_block(paragraph(&text, char_props, true)));
                        descriptions.push((ch, count, spacing, scale));
                    }
                }
                let laid = paragraphs(&doc);
                assert_eq!(laid.len(), descriptions.len() * 2, "all fixture paragraphs at {size}pt");
                for (i, &(ch, count, spacing, scale)) in descriptions.iter().enumerate() {
                    let a = &laid[i * 2];
                    let b = &laid[i * 2 + 1];
                    assert_eq!(a.clusters.len(), b.clusters.len());
                    for (ca, cb) in a.clusters.iter().zip(&b.clusters) {
                        assert!(ca.adv.is_finite() && cb.adv.is_finite());
                        assert!(
                            (ca.adv - cb.adv).abs() < 0.01,
                            "{size}pt {ch:?} x{count}, spacing {spacing}, scale {scale}, grid {grid}, balance {balance}: {} vs {}",
                            ca.adv,
                            cb.adv
                        );
                    }
                }
            }
        }
    }
}

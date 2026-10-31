//! Word's legacy width balancing changes contextual spaces, including across run boundaries.

use super::*;
use wordcraft_doc::para::{Run, ShapeKind};
use wordcraft_doc::props::{FitText, TextColor};
use wordcraft_doc::section::{DocGrid, DocGridType};
use wordcraft_doc::{PartKind, para_block};

fn formatted(text: &str, font: &str, size: f32, spacing: f32, scale: f32) -> Paragraph {
    let chr = CharProps {
        font: Some(font.into()),
        font_ea: Some(font.into()),
        size: Some(size),
        spacing: Some(spacing),
        scale: Some(scale),
        ..Default::default()
    };
    let mut p = Paragraph::with_text(text, chr);
    p.props.auto_space_de = Some(false);
    p.props.auto_space_dn = Some(false);
    p
}

fn first_para(doc: &Document) -> Arc<ParaLayout> {
    lay(doc)
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Placed::Lines { para, .. } => Some(para.clone()),
            _ => None,
        })
        .expect("fixture paragraph is laid out")
}

#[test]
fn ideographic_space_uses_east_asian_font_advance_across_run_boundaries() {
    let db = wordcraft_fonts::FontDb::global();
    if !db.has_family("BIZ UDPGothic") || !db.has_family("BIZ UDMincho") {
        return;
    }
    for east_asian in ["BIZ UDPGothic", "BIZ UDMincho", "MS PGothic"] {
        for size in [9.0, 10.5, 18.0, 36.0] {
            for (spacing, scale) in [(0.0, 100.0), (1.0, 80.0), (-1.0, 120.0)] {
                for split in [false, true] {
                    let make = |latin: &str| {
                        let mut p = formatted("A　　B", latin, size, spacing, scale);
                        p.runs[0].props.font_ea = Some(east_asian.into());
                        if split {
                            p.format(1, 4, &|r| r.color = Some(TextColor::Rgb(Rgb(1, 2, 3)))).unwrap();
                            p.format(4, 7, &|r| r.color = Some(TextColor::Rgb(Rgb(3, 2, 1)))).unwrap();
                        }
                        p.touch();
                        let mut doc = Document::new();
                        doc.body = vec![para_block(p)];
                        first_para(&doc)
                    };
                    let mixed = make("BIZ UDMincho");
                    let reference = make(east_asian);
                    for i in [1, 2] {
                        assert!(
                            (mixed.clusters[i].adv - reference.clusters[i].adv).abs() < 0.01,
                            "U+3000 must use the East Asian face: {east_asian}, {size}pt, spacing={spacing}, scale={scale}, split={split}"
                        );
                    }
                    assert!(mixed.styles[mixed.clusters[0].style as usize].face.named("BIZ UDMincho"), "Latin text keeps its face");
                }
            }
        }
    }
}

fn case_paras(doc: &Document) -> Vec<Arc<ParaLayout>> {
    lay(doc)
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter_map(|item| match item {
            Placed::Lines { story: StoryRef::Body, para, .. } => Some(para.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn contextual_ascii_and_no_break_spaces_balance_at_half_em() {
    // Positions are character indices. The single spaces within Latin words deliberately
    // have no entry; every other listed position is a space Word balances.
    let cases: [(&str, &[usize]); 10] = [
        ("A B", &[]),
        ("A\u{a0}B", &[]),
        ("A  B", &[1, 2]),
        ("A\u{a0} B", &[1, 2]),
        ("  A", &[0, 1]),
        ("A  ", &[1, 2]),
        ("A 国", &[1]),
        ("国 A", &[1]),
        ("A　 B", &[2]),
        ("A \u{a0} B", &[1, 2, 3]),
    ];
    for font in ["Arial", "BIZ UDMincho", "Source Sans 3"] {
        for size in [6.0, 9.0, 12.0, 24.0, 36.0] {
            for spacing in [-1.0, 0.0, 1.0] {
                for scale in [50.0, 100.0, 150.0] {
                    let mut doc = Document::new();
                    doc.body = cases.iter().map(|(text, _)| para_block(formatted(text, font, size, spacing, scale))).collect();
                    let natural = case_paras(&doc);
                    doc.settings.balance_single_byte_double_byte_width = true;
                    let balanced = case_paras(&doc);
                    assert_eq!(natural.len(), cases.len());
                    assert_eq!(balanced.len(), cases.len());
                    // With balancing off, NBSP is shaped as the same native-width space.
                    let ordinary = natural[0].clusters[1].adv;
                    assert!((natural[1].clusters[1].adv - ordinary).abs() < 0.01, "natural NBSP: {font}, {size}, {spacing}, {scale}");
                    for ((text, qualifying), (before, after)) in cases.iter().zip(natural.iter().zip(&balanced)) {
                        assert_eq!(before.clusters.len(), text.chars().count());
                        assert_eq!(after.clusters.len(), before.clusters.len());
                        for (index, ch) in text.chars().enumerate() {
                            if ch != ' ' && ch != '\u{a0}' {
                                continue;
                            }
                            let old = before.clusters[index].adv;
                            let new = after.clusters[index].adv;
                            if qualifying.contains(&index) {
                                let expected = size * scale / 200.0 + 2.0 * spacing;
                                assert!(new.is_finite(), "balanced advance is finite: {text:?}, {font}, {size}, {spacing}, {scale}");
                                if expected >= 0.0 {
                                    assert!(
                                        (new - expected).abs() < 0.01,
                                        "{text:?} cluster {index}, {font}, {size}, {spacing}, {scale}: {new} vs {expected}"
                                    );
                                }
                            } else {
                                assert!(
                                    (new - old).abs() < 0.01,
                                    "single Latin space retains native advance: {text:?}, {font}, {size}, {spacing}, {scale}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn split_runs_do_not_split_a_balanced_space_sequence() {
    let text = "A \u{a0}  B";
    let p = formatted(text, "Arial", 12.0, 1.0, 100.0);
    let mut split = p.clone();
    split.runs = text
        .chars()
        .enumerate()
        .map(|(i, ch)| Run {
            len: ch.len_utf8(),
            props: CharProps {
                font: Some("Arial".into()),
                font_ea: Some("Arial".into()),
                size: Some(12.0),
                spacing: Some(1.0),
                scale: Some(100.0),
                color: Some(TextColor::Rgb(Rgb(i as u8 % 2, 0, 0))),
                ..Default::default()
            },
        })
        .collect();
    split.touch();
    let mut doc = Document::new();
    doc.settings.balance_single_byte_double_byte_width = true;
    doc.body = vec![para_block(p), para_block(split)];
    let laid = case_paras(&doc);
    assert_eq!(laid.len(), 2);
    for (i, (a, b)) in laid[0].clusters.iter().zip(&laid[1].clusters).enumerate() {
        if matches!(text.chars().nth(i), Some(' ' | '\u{a0}')) {
            assert!((a.adv - b.adv).abs() < 0.01, "run boundaries preserve contextual space advances at {i}: {} vs {}", a.adv, b.adv);
        }
    }
    assert_eq!(laid[0].clusters.len(), laid[1].clusters.len());
}

#[test]
fn balanced_no_break_spaces_keep_their_no_break_rule() {
    let mut doc = Document::new();
    doc.settings.balance_single_byte_double_byte_width = true;
    doc.body = vec![para_block(formatted("A  B", "Arial", 12.0, 0.0, 100.0)), para_block(formatted("A\u{a0}\u{a0}B", "Arial", 12.0, 0.0, 100.0))];
    let laid = case_paras(&doc);
    assert_eq!(laid.len(), 2);
    assert!(laid[0].clusters[1].break_after || laid[0].clusters[2].break_after, "ordinary spaces retain a line-break opportunity");
    assert!(!laid[1].clusters[1].break_after && !laid[1].clusters[2].break_after, "balanced NBSPs do not allow a line break");
    for k in 1..=2 {
        assert!((laid[0].clusters[k].adv - laid[1].clusters[k].adv).abs() < 0.01, "balanced NBSP and ordinary space have equal advances");
    }
}

#[test]
fn fitted_space_group_still_occupies_its_requested_width() {
    for balance in [false, true] {
        let mut doc = Document::new();
        doc.settings.balance_single_byte_double_byte_width = balance;
        let mut p = formatted("A   B", "Arial", 12.0, 1.0, 100.0);
        p.runs[0].props.fit_text = Some(FitText { width: 90.0, id: Some(7) });
        p.touch();
        doc.body = vec![para_block(p)];
        let laid = first_para(&doc);
        let width: f32 = laid.clusters.iter().map(|c| c.adv).sum();
        assert!((width - 90.0).abs() < 0.01, "fitText width survives balance={balance}: {width}");
    }
}

#[test]
fn body_table_cell_and_text_box_share_balanced_advances() {
    let para = formatted("A  B　 C", "Arial", 12.0, 1.0, 100.0);
    let mut body = Document::new();
    body.settings.balance_single_byte_double_byte_width = true;
    body.body = vec![para_block(para.clone())];
    let body_advances: Vec<f32> = first_para(&body).clusters.iter().map(|c| c.adv).collect();

    let mut table_doc = body.clone();
    let mut table = Table::new(1, 1, 300.0);
    table.props.style = None;
    table.rows[0].cells[0].blocks = vec![para_block(para.clone())];
    table_doc.body = vec![Arc::new(Block::Table(table))];
    let table_advances: Vec<f32> = first_para(&table_doc).clusters.iter().map(|c| c.adv).collect();

    let mut box_doc = Document::new();
    box_doc.settings.balance_single_byte_double_byte_width = true;
    box_doc.body = vec![para_block(Paragraph::new())];
    let story = box_doc.add_part(PartKind::TextBox, vec![para_block(para)]);
    let shape = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 300.0,
        h: 100.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: Some(story),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    box_doc.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let box_layout = lay(&box_doc);
    let box_advances: Vec<f32> = box_layout
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Placed::Lines { story: StoryRef::Part(id), para, .. } if *id == story => Some(para.clusters.iter().map(|c| c.adv).collect()),
            _ => None,
        })
        .expect("text box story is laid out");

    assert_eq!(body_advances.len(), table_advances.len());
    assert_eq!(body_advances.len(), box_advances.len());
    for (a, (b, c)) in body_advances.iter().zip(table_advances.iter().zip(&box_advances)) {
        assert!((a - b).abs() < 0.01, "table cell has the body space advances");
        assert!((a - c).abs() < 0.01, "text box has the body space advances");
    }
}

#[test]
fn additive_grid_spacing_keeps_the_first_glyph_origin() {
    for text in ["A", "国"] {
        for delta in [-4096, 8192] {
            let mut doc = Document::new();
            doc.body = vec![para_block(formatted(text, "BIZ UDMincho", 12.0, 0.0, 100.0))];
            doc.styles.default_chr.size = Some(12.0);
            doc.last_section.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: 18.0, char_space: delta });
            let plain = first_para(&doc);
            doc.last_section.doc_grid.as_mut().unwrap().kind = DocGridType::LinesAndChars;
            let additive = first_para(&doc);
            doc.last_section.doc_grid.as_mut().unwrap().kind = DocGridType::SnapToChars;
            let cells = first_para(&doc);
            let dx = |p: &ParaLayout| p.glyphs[p.clusters[0].g0 as usize].dx;
            assert!((additive.clusters[0].adv - plain.clusters[0].adv - delta as f32 / 4096.0).abs() < 0.01);
            assert!((dx(&additive) - dx(&plain)).abs() < 0.01, "{text:?}, delta {delta}: additive grid keeps the outline origin");
            let cell_shift = (cells.clusters[0].adv - plain.clusters[0].adv) / 2.0;
            assert!((dx(&cells) - dx(&plain) - cell_shift).abs() < 0.01, "{text:?}, delta {delta}: snapToChars centres the outline");
        }
    }
}

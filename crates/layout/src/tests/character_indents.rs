use super::*;

/// Mac Word fixtures vary document defaults independently of each paragraph style. LeftChars
/// places the first line; hangingChars moves following lines, without consulting stale twips.
#[test]
fn character_hanging_uses_separate_default_and_text_units() {
    for family in ["Source Serif 4", "Carlito", "BIZ UDGothic"] {
        for base in [8.0, 12.0, 24.0] {
            for size in [9.0, 12.0, 18.0] {
                for left in [None, Some(10.0), Some(42.85)] {
                    let mut d = Document::new();
                    let normal = d.styles.get_mut("Normal").unwrap();
                    normal.chr.font = Some(family.into());
                    normal.chr.size = Some(base);
                    d.styles.upsert(wordcraft_doc::Style {
                        id: "Measured".into(),
                        name: "Measured".into(),
                        based_on: Some("Normal".into()),
                        chr: CharProps { size: Some(size), ..Default::default() },
                        ..Default::default()
                    });
                    let mut p = Paragraph::with_text("H one\nH two", CharProps { size: Some(size * 2.0), ..Default::default() });
                    p.mark.size = Some(48.0);
                    p.props = ParaProps {
                        style: Some("Measured".into()),
                        indent_left: left,
                        indent_first: Some(-11.05),
                        indent_left_chars: Some(2.99),
                        indent_first_chars: Some(-1.0),
                        line_spacing: Some(wordcraft_doc::props::LineSpacing::Exactly(48.0)),
                        ..Default::default()
                    };

                    d.body = vec![wordcraft_doc::para_block(p)];
                    let layout = lay(&d);
                    let para = layout.pages[0].items.iter().find_map(|p| if let Placed::Lines { para, .. } = p { Some(para) } else { None }).unwrap();
                    assert_eq!(para.lines.len(), 2);
                    assert!((para.lines[0].left - 2.99 * base).abs() < 0.01, "{family}, {base}, {size}");
                    assert!((para.lines[1].left - (2.99 * base + size * 2.0)).abs() < 0.01);
                }
            }
        }
    }
}

#[test]
fn a_character_hanging_without_left_chars_starts_at_the_margin() {
    for left_chars in [None, Some(0.0)] {
        let mut d = Document::new();
        let mut p = Paragraph::with_text("H one\nH two", Default::default());
        p.props = ParaProps { indent_left: Some(42.85), indent_left_chars: left_chars, indent_first_chars: Some(-1.0), ..Default::default() };
        let size = d.styles.resolve_char(None, &Default::default()).size;
        d.body = vec![wordcraft_doc::para_block(p)];
        let layout = lay(&d);
        let para = layout.pages[0].items.iter().find_map(|p| if let Placed::Lines { para, .. } = p { Some(para) } else { None }).unwrap();
        assert!(para.lines[0].left.abs() < 0.01);
        assert!((para.lines[1].left - size).abs() < 0.01);
    }
}

/// Both the document grid and legacy width balancing contribute to character-unit indents.
/// Direct mark formatting is deliberately unrelated to either the default or text run unit.
#[test]
fn character_indent_units_include_tracking_and_character_grid() {
    use wordcraft_doc::section::{DocGrid, DocGridType};
    for size in [9.0, 12.0, 16.0, 24.0] {
        for tracking in [-0.5, 0.0, 1.0, 1.5] {
            for balance in [false, true] {
                for delta in [-1.5, 0.0, 2.0] {
                    let mut d = Document::new();
                    d.settings.balance_single_byte_double_byte_width = balance;
                    d.last_section.doc_grid =
                        Some(DocGrid { kind: DocGridType::LinesAndChars, line_pitch: 24.0, char_space: (delta * 4096.0) as i32 });
                    let normal = d.styles.get_mut("Normal").unwrap();
                    normal.chr.size = Some(10.5);
                    normal.chr.spacing = Some(0.8);
                    let props = CharProps { size: Some(size), spacing: Some(tracking), ..Default::default() };
                    let mut p = Paragraph::with_text("H text\nH text", props);
                    p.mark.size = Some(48.0);
                    p.mark.spacing = Some(9.0);
                    p.props.indent_first_chars = Some(3.0);
                    let mut hanging = p.clone();
                    hanging.props.indent_left_chars = Some(2.99);
                    hanging.props.indent_first_chars = Some(-1.0);
                    hanging.touch();
                    d.body = vec![wordcraft_doc::para_block(p), wordcraft_doc::para_block(hanging)];
                    let layout = lay(&d);
                    let paras: Vec<_> = layout
                        .pages
                        .iter()
                        .flat_map(|p| &p.items)
                        .filter_map(|p| if let Placed::Lines { para, .. } = p { Some(para) } else { None })
                        .collect();
                    let multiplier = if balance { 2.0 } else { 1.0 };
                    let text_unit = size + tracking * multiplier + delta;
                    let default_unit = 10.5 + 0.8 * multiplier + delta;
                    assert!((paras[0].lines[0].left - 3.0 * text_unit).abs() < 0.01);
                    assert!((paras[0].lines[1].left).abs() < 0.01);
                    assert!((paras[1].lines[0].left - 2.99 * default_unit).abs() < 0.01);
                    assert!((paras[1].lines[1].left - (2.99 * default_unit + text_unit)).abs() < 0.01);
                }
            }
        }
    }
}

/// Native Word controls distinguish the first run from the largest/last run and the mark.
#[test]
fn first_and_hanging_character_indents_follow_the_first_visible_run() {
    use wordcraft_doc::para::Run;
    for first in [9.0, 18.0] {
        for hanging in [false, true] {
            let mut d = Document::new();
            d.styles.get_mut("Normal").unwrap().chr.size = Some(12.0);
            let mut p = Paragraph::with_text("H first\nH next", Default::default());
            p.runs = vec![
                Run { len: 8, props: CharProps { size: Some(first), ..Default::default() } },
                Run { len: 6, props: CharProps { size: Some(27.0 - first), ..Default::default() } },
            ];
            p.mark.size = Some(24.0);
            p.props.indent_left_chars = Some(if hanging { 2.0 } else { 0.0 });
            p.props.indent_first_chars = Some(if hanging { -1.0 } else { 2.0 });
            d.body = vec![wordcraft_doc::para_block(p)];
            let layout = lay(&d);
            let para = layout.pages[0].items.iter().find_map(|item| if let Placed::Lines { para, .. } = item { Some(para) } else { None }).unwrap();
            assert!((para.lines[0].left - if hanging { 24.0 } else { 2.0 * first }).abs() < 0.01);
            assert!((para.lines[1].left - if hanging { 24.0 + first } else { 0.0 }).abs() < 0.01);
        }
    }
}

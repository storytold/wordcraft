//! A run's Latin and East Asian faces have independent substitution provenance.

use super::*;

/// Word's fixed line box has the same baseline for faces with different natural descents.
/// Explicit mark/run formats also cover the empty line after a manual break: it must use the
/// fixed box rather than reverting to the paragraph mark's natural font metrics.
#[test]
fn exact_line_baselines_follow_the_fixed_pitch_across_font_metrics() {
    use wordcraft_doc::props::LineSpacing;
    for family in ["Source Sans 3", "Source Serif 4", "JetBrains Mono"] {
        for size in [9.0, 12.0, 24.0, 36.0] {
            for pitch in [12.0, 16.0, 24.0, 32.0, 48.0] {
                for text in ["", "Agjp\nAgjp\n"] {
                    let chr =
                        wordcraft_doc::CharProps { font: Some(family.into()), font_ea: Some(family.into()), size: Some(size), ..Default::default() };
                    let mut paragraph = wordcraft_doc::Paragraph::with_text(text, chr);
                    paragraph.props = ParaProps {
                        line_spacing: Some(LineSpacing::Exactly(pitch)),
                        snap_to_grid: Some(false),
                        space_before: Some(0.0),
                        space_after: Some(0.0),
                        ..Default::default()
                    };
                    let mut doc = Document::new();
                    doc.body = vec![wordcraft_doc::para_block(paragraph)];
                    let layout = lay(&doc);
                    let para = layout
                        .pages
                        .iter()
                        .flat_map(|p| &p.items)
                        .find_map(|item| if let Placed::Lines { para, .. } = item { Some(para) } else { None })
                        .expect("the fixture paragraph is laid out");
                    assert_eq!(para.lines.len(), if text.is_empty() { 1 } else { 3 });
                    for line in &para.lines {
                        assert!((line.height - pitch).abs() < 0.001, "{family}, {size} pt, pitch {pitch}: {line:?}");
                        assert!((line.baseline - line.top - pitch * 0.8).abs() < 0.001, "{family}, {size} pt, pitch {pitch}: {line:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn exact_line_baselines_ignore_the_grid_after_a_terminal_manual_break() {
    use wordcraft_doc::props::LineSpacing;
    use wordcraft_doc::section::{DocGrid, DocGridType};
    let chr = wordcraft_doc::CharProps { font: Some("Source Sans 3".into()), size: Some(12.0), ..Default::default() };
    let mut paragraph = wordcraft_doc::Paragraph::with_text("Agjp\n", chr);
    paragraph.props.line_spacing = Some(LineSpacing::Exactly(24.0));
    paragraph.props.snap_to_grid = Some(true);
    let mut doc = Document::new();
    doc.body = vec![wordcraft_doc::para_block(paragraph)];
    doc.last_section.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: 18.0, char_space: 0 });
    let layout = lay(&doc);
    let para = layout.pages[0].items.iter().find_map(|item| if let Placed::Lines { para, .. } = item { Some(para) } else { None }).unwrap();
    assert_eq!(para.lines.len(), 2);
    for line in &para.lines {
        assert!((line.height - 24.0).abs() < 0.001);
        assert!((line.baseline - line.top - 19.2).abs() < 0.001, "{line:?}");
    }
}

#[test]
fn east_asian_line_metrics_do_not_depend_on_the_latin_family_being_installed() {
    let missing = wordcraft_fonts::word::resolve("MS Mincho", false, false);
    if !missing.substituted || wordcraft_fonts::word::substitute_cjk_leading(&missing.face, true) == 0.0 {
        eprintln!("skipped: requires an absent MS Mincho and an installed one-em CJK substitute");
        return;
    }
    // Exercise both directions: an absent East Asian family needs its substitute's leading,
    // while an explicitly requested installed one-em face keeps that face's own metrics.
    for east_asian in ["MS Mincho", missing.face.family.as_str()] {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            for size in [9.0, 12.0, 24.0, 36.0] {
                let metrics = |latin: &str| {
                    let mut doc = Document::from_text("日本語");
                    doc.styles.default_chr.font = Some(latin.into());
                    doc.styles.default_chr.font_ea = Some(east_asian.into());
                    doc.styles.default_chr.size = Some(size);
                    doc.styles.default_chr.bold = Some(bold);
                    doc.styles.default_chr.italic = Some(italic);
                    let layout = lay(&doc);
                    let para = layout
                        .pages
                        .iter()
                        .flat_map(|p| &p.items)
                        .find_map(|item| if let Placed::Lines { para, .. } = item { Some(para) } else { None })
                        .expect("a laid-out Japanese paragraph");
                    let cluster = para.clusters.first().expect("Japanese text has a cluster");
                    let style = para.styles.get(cluster.style as usize).expect("cluster style");
                    (style.face.id(), style.ascent, style.descent, style.grid_size)
                };
                assert_eq!(
                    metrics("Source Sans 3"),
                    metrics("WordCraft Missing Latin Family 987"),
                    "East Asian {east_asian} at {size} pt bold={bold} italic={italic} keeps its own metrics"
                );
            }
        }
    }
}

//! Word-measured horizontal Japanese leading, independent of family names and type size.

use super::*;
use crate::para::{Grid, ParaEnv, layout_para};
use wordcraft_doc::props::LineSpacing;

fn measured_paragraph(family: &str, size: f32, spacing: LineSpacing) -> ParaLayout {
    measured_paragraph_with_grid(family, size, spacing, Grid::default(), false)
}

fn measured_paragraph_with_grid(family: &str, size: f32, spacing: LineSpacing, grid: Grid, snap: bool) -> ParaLayout {
    let mut doc = Document::new();
    let fields = FieldCtx::default();
    let chars = CharProps { font: Some(family.into()), font_ea: Some(family.into()), size: Some(size), ..Default::default() };
    doc.styles.default_chr = chars.clone();
    let mut paragraph = Paragraph::with_text("Agjp 日本語\nAgjp 日本語\n", chars);
    paragraph.props =
        ParaProps { line_spacing: Some(spacing), snap_to_grid: Some(snap), space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
    layout_para(
        &paragraph,
        &ParaEnv {
            doc: &doc,
            fields: &fields,
            width: 1000.0,
            label: None,
            show_hidden: false,
            hide_deleted: Default::default(),
            table: None,
            proofing: None,
            typing_word: None,
            field_codes: false,
            grid,
            exclusions: &[],
            eq_number: 0,
            numeral: Default::default(),
            read_line_spacing: None,
            fit_objects: false,
            fitted: &[],
        },
    )
}

#[test]
fn japanese_substitutes_and_zero_gap_faces_share_word_minimum_line_leading() {
    for family in ["明朝体", "MS Mincho", "MS Gothic", "BIZ UDMincho"] {
        let resolved = wordcraft_fonts::word::resolve(family, false, false);
        let expected_substitution = family != "BIZ UDMincho";
        if resolved.substituted != expected_substitution {
            continue;
        }
        let Some((ascent, descent)) = wordcraft_fonts::word::horizontal_japanese_line_metrics(&resolved.face, resolved.substituted) else {
            assert_ne!(family, "BIZ UDMincho", "the installed zero-gap Japanese face uses Word's horizontal metrics");
            continue;
        };
        for size in [9.0, 10.5, 12.0, 16.0, 24.0, 36.0] {
            for minimum in [0.0, 36.0, 48.0, 72.0] {
                let para = measured_paragraph(family, size, LineSpacing::AtLeast(minimum));
                let scale = f64::from(size) / resolved.face.upem;
                let natural = ((ascent + descent) * scale) as f32;
                let desc = (descent * scale) as f32;
                assert!((natural - size * 1.3).abs() < 0.01, "{family}: {natural}");
                assert_eq!(para.lines.len(), 3);
                for line in &para.lines {
                    let expected_height = natural.max(minimum);
                    assert!((line.height - expected_height).abs() < 0.001, "{family}, {size}, {minimum}: {line:?}");
                    assert!((line.baseline - line.top - (expected_height - desc)).abs() < 0.001, "{family}, {size}, {minimum}: {line:?}");
                }
            }
        }
    }
}

#[test]
fn japanese_single_lines_match_zero_minimum_spacing() {
    for family in ["明朝体", "MS Mincho", "MS Gothic", "BIZ UDMincho"] {
        for size in [9.0, 12.0, 24.0, 36.0] {
            let single = measured_paragraph(family, size, LineSpacing::Multiple(1.0));
            let minimum = measured_paragraph(family, size, LineSpacing::AtLeast(0.0));
            assert_eq!(single.lines.len(), minimum.lines.len());
            for (single_line, minimum_line) in single.lines.iter().zip(&minimum.lines) {
                assert!((single_line.height - minimum_line.height).abs() < 0.001, "{family}, {size}");
                assert!((single_line.baseline - minimum_line.baseline).abs() < 0.001, "{family}, {size}");
            }
        }
    }
}

#[test]
fn japanese_exact_line_baselines_keep_the_fixed_pitch() {
    for family in ["明朝体", "MS Mincho", "MS Gothic", "BIZ UDMincho", "Hiragino Mincho ProN"] {
        for size in [9.0, 12.0, 24.0, 36.0] {
            for pitch in [12.0, 36.0, 48.0, 72.0] {
                let para = measured_paragraph(family, size, LineSpacing::Exactly(pitch));
                for line in &para.lines {
                    assert!((line.height - pitch).abs() < 0.001);
                    assert!((line.baseline - line.top - pitch * 0.8).abs() < 0.001, "{family}, {size}, {pitch}: {line:?}");
                }
            }
        }
    }
}

#[test]
fn installed_japanese_native_leading_is_preserved() {
    for family in ["Hiragino Mincho ProN", "Hiragino Kaku Gothic ProN"] {
        let resolved = wordcraft_fonts::word::resolve(family, false, false);
        if resolved.substituted {
            continue;
        }
        let (ascent, descent) = wordcraft_fonts::word::line_metrics(&resolved.face);
        if (ascent + descent) / resolved.face.upem <= 1.01 || wordcraft_fonts::word::horizontal_japanese_line_metrics(&resolved.face, true).is_none()
        {
            continue;
        }
        assert!(wordcraft_fonts::word::horizontal_japanese_line_metrics(&resolved.face, false).is_none());
        for size in [9.0, 12.0, 24.0, 36.0] {
            let para = measured_paragraph(family, size, LineSpacing::AtLeast(0.0));
            let scale = f64::from(size) / resolved.face.upem;
            for style in &para.styles {
                assert!((style.ascent - (ascent * scale) as f32).abs() < 0.001, "{family}, {size}: {style:?}");
                assert!((style.descent - (descent * scale) as f32).abs() < 0.001, "{family}, {size}: {style:?}");
            }
        }
    }
}

#[test]
fn japanese_grid_lines_retain_previous_extents_and_baselines() {
    let grid = Grid { line: Some(14.35), ..Default::default() };
    for family in ["BIZ UDMincho", "Hiragino Mincho ProN", "Hiragino Kaku Gothic ProN"] {
        let resolved = wordcraft_fonts::word::resolve(family, false, false);
        if resolved.substituted {
            continue;
        }
        let (ascent, descent) = wordcraft_fonts::word::line_metrics(&resolved.face);
        for size in [9.0, 12.0, 24.0] {
            for spacing in [LineSpacing::Multiple(1.0), LineSpacing::AtLeast(18.0)] {
                let para = measured_paragraph_with_grid(family, size, spacing, grid, true);
                let scale = f64::from(size) / resolved.face.upem;
                let a = (ascent * scale) as f32;
                let d = (descent * scale) as f32;
                let grid_size = wordcraft_fonts::word::japanese_grid_height_for_style(family, size, false, false);
                let height = grid.snap_height(grid_size).max(if let LineSpacing::AtLeast(v) = spacing { v } else { 0.0 });
                for style in &para.styles {
                    assert!((style.ascent - a).abs() < 0.001, "{family}, {size}: {style:?}");
                    assert!((style.descent - d).abs() < 0.001, "{family}, {size}: {style:?}");
                }
                for (index, line) in para.lines.iter().enumerate() {
                    let baseline = if index + 1 == para.lines.len() { height - d } else { ((height - a - d) / 2.0).max(0.0) + a };
                    assert!((line.height - height).abs() < 0.001, "{family}, {size}, {spacing:?}: {line:?}");
                    assert!((line.baseline - line.top - baseline).abs() < 0.001, "{family}, {size}, {spacing:?}: {line:?}");
                }
            }
        }
    }
}

#[test]
fn zero_minimum_spacing_bypasses_the_grid_with_natural_japanese_metrics() {
    let grid = Grid { line: Some(14.35), ..Default::default() };
    for family in ["明朝体", "BIZ UDMincho"] {
        for size in [9.0, 12.0, 24.0] {
            let natural = measured_paragraph(family, size, LineSpacing::AtLeast(0.0));
            let bypass = measured_paragraph_with_grid(family, size, LineSpacing::AtLeast(0.0), grid, true);
            assert_eq!(natural.lines.len(), bypass.lines.len());
            for (natural_line, bypass_line) in natural.lines.iter().zip(&bypass.lines) {
                assert!((natural_line.height - bypass_line.height).abs() < 0.001, "{family}, {size}");
                assert!((natural_line.baseline - bypass_line.baseline).abs() < 0.001, "{family}, {size}");
            }
        }
    }
}

//! Word-measured synthetic-bold advances and bold character-unit indents.

use super::*;
use wordcraft_doc::para_block;
use wordcraft_doc::props::FitText;

#[allow(clippy::too_many_arguments)]
fn laid_out(font: &str, text: &str, size: f32, scale: f32, bold: bool, balance: bool, spacing: f32, fit: Option<f32>) -> Arc<ParaLayout> {
    let mut doc = Document::new();
    doc.settings.balance_single_byte_double_byte_width = balance;
    let props = CharProps {
        font: Some(font.into()),
        font_ea: Some(font.into()),
        size: Some(size),
        scale: Some(scale),
        bold: Some(bold),
        spacing: Some(spacing),
        fit_text: fit.map(|width| FitText { width, id: Some(17) }),
        ..Default::default()
    };
    doc.body = vec![para_block(Paragraph::with_text(text, props))];
    lay(&doc)
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Placed::Lines { para, .. } => Some(para.clone()),
            _ => None,
        })
        .expect("test paragraph is laid out")
}

#[test]
fn native_bold_uses_its_font_advances_without_simulated_width() {
    let regular_face = wordcraft_fonts::word::resolve("Arial", false, false);
    let bold_face = wordcraft_fonts::word::resolve("Arial", true, false);
    if regular_face.substituted || bold_face.substituted || bold_face.synth_bold || regular_face.face.id() == bold_face.face.id() {
        eprintln!("skipped: needs installed Arial Regular and native Arial Bold");
        return;
    }
    for size in [9.0, 10.5, 12.0, 18.0] {
        for scale in [50.0, 100.0, 150.0] {
            let regular = laid_out("Arial", "A", size, scale, false, false, 0.0, None);
            let bold = laid_out("Arial", "A", size, scale, true, false, 0.0, None);
            let r = &regular.clusters[0];
            let b = &bold.clusters[0];
            let rs = &regular.styles[r.style as usize];
            let bs = &bold.styles[b.style as usize];
            assert!(!bs.synth_bold, "native bold face needs no simulated width");
            let rg = &regular.glyphs[r.g0 as usize];
            let bg = &bold.glyphs[b.g0 as usize];
            let native_difference = (bs.face.advance(bg.gid) / bs.face.upem - rs.face.advance(rg.gid) / rs.face.upem) as f32 * size * scale / 100.0;
            assert!((b.adv - r.adv - native_difference).abs() < 0.01, "native Arial Bold at {size}pt, {scale}% has no extra synthetic advance");
        }
    }
}

#[test]
fn simulated_bold_adds_two_percent_per_positive_glyph_not_per_combining_mark() {
    let regular_face = wordcraft_fonts::word::resolve("BIZ UDMincho", false, false);
    let bold_face = wordcraft_fonts::word::resolve("BIZ UDMincho", true, false);
    if regular_face.substituted || bold_face.substituted || !bold_face.synth_bold || regular_face.face.id() != bold_face.face.id() {
        eprintln!("skipped: needs installed regular-only BIZ UDMincho");
        return;
    }
    for size in [9.0, 10.5, 12.0, 18.0] {
        for scale in [50.0, 100.0, 150.0] {
            let regular = laid_out("BIZ UDMincho", "A国キ\u{3099}", size, scale, false, false, 0.0, None);
            let bold = laid_out("BIZ UDMincho", "A国キ\u{3099}", size, scale, true, false, 0.0, None);
            assert_eq!(regular.clusters.len(), 3, "the combining mark stays with its base");
            assert_eq!(bold.clusters.len(), regular.clusters.len());
            let expected = size * scale / 100.0 * wordcraft_fonts::word::BOLD_WIDTH_EM;
            for (i, (r, b)) in regular.clusters.iter().zip(&bold.clusters).enumerate() {
                assert!((b.adv - r.adv - expected).abs() < 0.01, "BIZ synthetic bold cluster {i} at {size}pt, {scale}% adds one width unit");
            }
        }
    }
}

#[test]
fn simulated_bold_space_balancing_and_fitted_width_remain_independent() {
    let bold_face = wordcraft_fonts::word::resolve("BIZ UDMincho", true, false);
    if bold_face.substituted || !bold_face.synth_bold {
        eprintln!("skipped: needs installed regular-only BIZ UDMincho");
        return;
    }
    for size in [9.0, 12.0, 24.0] {
        for scale in [50.0, 100.0, 150.0] {
            for spacing in [-1.0, 0.0, 1.0] {
                let balanced = laid_out("BIZ UDMincho", "A  B", size, scale, true, true, spacing, None);
                let expected = size * scale / 200.0 + size * scale / 200.0 * wordcraft_fonts::word::BOLD_WIDTH_EM + 2.0 * spacing;
                for k in [1, 2] {
                    assert!((balanced.clusters[k].adv - expected).abs() < 0.01, "bold balanced space {k} at {size}pt, {scale}%, tracking {spacing}");
                }
            }
        }
    }
    for font in ["BIZ UDMincho", "Arial"] {
        for bold in [false, true] {
            let fitted = laid_out(font, "AA国", 12.0, 100.0, bold, false, 1.0, Some(64.0));
            let width: f32 = fitted.clusters.iter().map(|c| c.adv).sum();
            assert!((width - 64.0).abs() < 0.01, "fitText retains the requested width with {font}, bold={bold}: {width}");
        }
    }
}

#[test]
fn simulated_bold_does_not_widen_ideographic_spaces() {
    let bold_face = wordcraft_fonts::word::resolve("BIZ UDMincho", true, false);
    if bold_face.substituted || !bold_face.synth_bold {
        eprintln!("skipped: needs installed regular-only BIZ UDMincho");
        return;
    }
    for size in [9.0, 10.5, 18.0, 24.0] {
        for scale in [50.0, 100.0, 150.0] {
            for spacing in [-1.0, 0.0, 1.0] {
                for balanced in [false, true] {
                    let regular = laid_out("BIZ UDMincho", "A\u{3000}\u{3000}B", size, scale, false, balanced, spacing, None);
                    let bold = laid_out("BIZ UDMincho", "A\u{3000}\u{3000}B", size, scale, true, balanced, spacing, None);
                    for k in [1, 2] {
                        assert!((bold.clusters[k].adv - regular.clusters[k].adv).abs() < 0.01, "space {k} at {size}pt, {scale}%, tracking {spacing}");
                    }
                }
            }
        }
    }
}

#[test]
fn first_line_character_indent_includes_bold_width_for_native_and_simulated_fonts() {
    for font in ["Arial", "BIZ UDMincho"] {
        for size in [9.0, 10.5, 12.0, 18.0] {
            let left = |bold| {
                let mut doc = Document::new();
                let props =
                    CharProps { font: Some(font.into()), font_ea: Some(font.into()), size: Some(size), bold: Some(bold), ..Default::default() };
                let mut p = Paragraph::with_text("A", props);
                p.props.indent_first_chars = Some(10.0);
                doc.body = vec![para_block(p)];
                let layout = lay(&doc);
                layout.pages[0]
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Placed::Lines { para, .. } => Some(para.lines[0].left),
                        _ => None,
                    })
                    .expect("first line is laid out")
            };
            let regular = left(false);
            let bold = left(true);
            let expected = 10.0 * size * wordcraft_fonts::word::BOLD_WIDTH_EM;
            assert!((regular - 10.0 * size).abs() < 0.01, "regular {font} at {size}pt");
            assert!((bold - regular - expected).abs() < 0.01, "bold {font} at {size}pt adds 2% em per character unit");
        }
    }
}

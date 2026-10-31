//! Tab stops when the text after a tab fits, or the tab moves to a new line.
//!
//! Mac Word comparison: `tab-fit-v2-hiragothic-9-{doNotCompress,compressPunctuation}`.
//! The fit control has a shorter prefix than the Word document to ensure the suffix can
//! physically reach the stop; the 21-size sweep is a synthetic invariant, not 21 Word observations.

use super::*;
use wordcraft_doc::{
    CharacterSpacingControl, para_block,
    props::{TabAlign, TabStop},
};

const SIZES: [f32; 21] = [5.0, 5.5, 6.5, 7.5, 8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0];

fn paragraph(size: f32, tab_align: TabAlign, wrapped: bool, alignment: Align, policy: CharacterSpacingControl) -> Arc<ParaLayout> {
    let mut doc = Document::new();
    doc.settings.default_tab = size * 3.0;
    doc.settings.character_spacing_control = policy;
    let text = if wrapped { "漢漢漢漢漢漢漢\t漢、12.3" } else { "漢\t漢、12.3" };
    let props = CharProps {
        font: Some("Hiragino Kaku Gothic ProN".into()),
        font_ea: Some("Hiragino Kaku Gothic ProN".into()),
        size: Some(size),
        spacing: Some(0.0),
        ..Default::default()
    };
    let mut p = Paragraph::with_text(text, props);
    p.props.align = Some(alignment);
    p.props.indent_right = Some(468.0 - size * if wrapped { 7.55 } else { 8.0 });
    p.props.overflow_punct = Some(false);
    p.props.auto_space_de = Some(false);
    p.props.auto_space_dn = Some(false);
    p.props.snap_to_grid = Some(false);
    p.props.tabs =
        Some(vec![TabStop { pos: size * if tab_align == TabAlign::Right { 6.0 } else { 5.0 }, align: tab_align, leader: Default::default() }]);
    doc.body = vec![para_block(p)];
    lay(&doc)
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para.clone()),
            _ => None,
        })
        .expect("body paragraph")
}

#[test]
fn right_and_decimal_tab_stops_survive_a_wrap_before_the_tab() {
    if wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false).substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    for size in SIZES {
        for tab in [TabAlign::Right, TabAlign::Decimal] {
            for alignment in [Align::Left, Align::Justify] {
                for policy in [CharacterSpacingControl::DoNotCompress, CharacterSpacingControl::CompressPunctuation] {
                    let fit = paragraph(size, tab, false, alignment, policy.clone());
                    let wrapped = paragraph(size, tab, true, alignment, policy);
                    let fit_text = "漢\t漢、12.3";
                    let wrapped_text = "漢漢漢漢漢漢漢\t漢、12.3";
                    assert_eq!(fit.lines.len(), 1, "{size} {tab:?} fit");
                    assert_eq!(wrapped.lines.len(), 2, "{size} {tab:?} wrapped");
                    assert_eq!(wrapped.lines[0].stop, "漢漢漢漢漢漢漢".len(), "{size} {tab:?} tab moves to next line");
                    let fit_off = if tab == TabAlign::Right { fit_text.len() } else { fit_text.find('.').expect("decimal") };
                    let wrap_off = if tab == TabAlign::Right { wrapped_text.len() } else { wrapped_text.find('.').expect("decimal") };
                    let fit_x = fit.x_of(0, fit_off).expect("fit anchor");
                    let wrap_x = wrapped.x_of(1, wrap_off).expect("wrapped anchor");
                    // Word's right-stop ink matched; the decimal ink differed by 2–3 device
                    // pixels after wrapping, so its origin uses a looser one-point tolerance.
                    let tolerance = if tab == TabAlign::Right { 0.1 } else { 1.5 };
                    assert!((fit_x - wrap_x).abs() <= tolerance, "{size} {tab:?} {fit_x} vs {wrap_x}");
                }
            }
        }
    }
}

fn pending_tab_paragraph(size: f32, tab: TabAlign, width: f32, policy: CharacterSpacingControl) -> Arc<ParaLayout> {
    let mut doc = Document::new();
    doc.settings.default_tab = 3.0 * size;
    doc.settings.character_spacing_control = policy;
    let text = if tab == TabAlign::Right { "漢、\t漢ぁ漢漢、漢終" } else { "漢、\t漢ぁ12.3、漢終" };
    let props = CharProps {
        font: Some("Hiragino Kaku Gothic ProN".into()),
        font_ea: Some("Hiragino Kaku Gothic ProN".into()),
        size: Some(size),
        spacing: Some(0.0),
        ..Default::default()
    };
    let mut p = Paragraph::with_text(text, props);
    p.props.align = Some(Align::Justify);
    p.props.indent_right = Some(468.0 - width * size);
    p.props.overflow_punct = Some(false);
    p.props.auto_space_de = Some(false);
    p.props.auto_space_dn = Some(false);
    p.props.snap_to_grid = Some(false);
    p.props.tabs = Some(vec![TabStop { pos: size * if tab == TabAlign::Right { 6.0 } else { 5.0 }, align: tab, leader: Default::default() }]);
    doc.body = vec![para_block(p)];
    lay(&doc)
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para.clone()),
            _ => None,
        })
        .expect("body paragraph")
}

/// Word v3 at 9pt: a pending right stop must not suppress punctuation compression.
/// The other sizes test scale invariance and are not independent Word observations.
#[test]
fn pending_right_tab_can_fit_compressed_closing_punctuation() {
    if wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false).substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    for size in SIZES {
        for (width, plain, compressed) in [(6.5, "漢、\t漢ぁ漢", "漢、\t漢ぁ漢漢、"), (7.5, "漢、\t漢ぁ漢漢、", "漢、\t漢ぁ漢漢、漢")]
        {
            for (policy, expected) in [(CharacterSpacingControl::DoNotCompress, plain), (CharacterSpacingControl::CompressPunctuation, compressed)] {
                let para = pending_tab_paragraph(size, TabAlign::Right, width, policy);
                assert_eq!(para.lines[0].stop, expected.len(), "{size}pt, {width}em: expected {expected:?}");
            }
        }
    }
}

/// Same-family decimal fixture whose actual Latin face in Word has not been verified.
#[test]
#[ignore = "Word actual Latin face for the same-family v3 fixture remains unverified"]
fn pending_decimal_tab_same_family_v3_unverified() {
    if wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false).substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    for size in SIZES {
        for (width, plain, compressed) in [(6.5, "漢、\t漢ぁ", "漢、\t漢ぁ12.3、"), (7.5, "漢、\t漢ぁ12.3、", "漢、\t漢ぁ12.3、漢")]
        {
            for (policy, expected) in [(CharacterSpacingControl::DoNotCompress, plain), (CharacterSpacingControl::CompressPunctuation, compressed)] {
                let para = pending_tab_paragraph(size, TabAlign::Decimal, width, policy);
                assert_eq!(para.lines[0].stop, expected.len(), "{size}pt, {width}em: expected {expected:?}");
            }
        }
    }
}

/// Word v3 at 9pt: a pending right or decimal stop must not suppress punctuation compression.
/// The other sizes test scale invariance and are not independent Word observations.
#[test]
fn compressed_pending_tab_keeps_first_glyph_anchor() {
    if wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false).substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    for size in SIZES {
        for tab in [TabAlign::Right, TabAlign::Decimal] {
            let anchor = |policy| pending_tab_paragraph(size, tab, 6.5, policy).x_of(0, "漢、\t".len()).expect("first glyph after tab");
            let plain = anchor(CharacterSpacingControl::DoNotCompress);
            let compressed = anchor(CharacterSpacingControl::CompressPunctuation);
            assert!((plain - compressed).abs() < 0.1, "{tab:?}, {size}pt first glyph moves: {plain} vs {compressed}");
        }
    }
}

/// Word v5: ASCII Arial and Japanese Hiragino at 6.5em, all 21 sizes.
#[test]
fn decimal_tab_mixed_ascii_and_japanese_fits_like_word_v5() {
    if wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false).substituted
        || wordcraft_fonts::word::resolve("Arial", false, false).substituted
    {
        eprintln!("skipped: needs installed Arial and Hiragino Kaku Gothic ProN");
        return;
    }
    for size in SIZES {
        for (policy, expected) in
            [(CharacterSpacingControl::DoNotCompress, "漢、\t漢ぁ"), (CharacterSpacingControl::CompressPunctuation, "漢、\t漢ぁ12.3、")]
        {
            let mut doc = Document::new();
            doc.settings.default_tab = 3.0 * size;
            doc.settings.character_spacing_control = policy;
            let props = CharProps {
                font: Some("Arial".into()),
                font_ea: Some("Hiragino Kaku Gothic ProN".into()),
                size: Some(size),
                spacing: Some(0.0),
                ..Default::default()
            };
            let mut p = Paragraph::with_text("漢、\t漢ぁ12.3、漢終", props);
            p.props.align = Some(Align::Justify);
            p.props.indent_right = Some(468.0 - 6.5 * size);
            p.props.overflow_punct = Some(false);
            p.props.auto_space_de = Some(false);
            p.props.auto_space_dn = Some(false);
            p.props.snap_to_grid = Some(false);
            p.props.tabs = Some(vec![TabStop { pos: 5.0 * size, align: TabAlign::Decimal, leader: Default::default() }]);
            doc.body = vec![para_block(p)];
            let layout = lay(&doc);
            let para = layout
                .pages
                .iter()
                .flat_map(|page| &page.items)
                .find_map(|item| match item {
                    Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para),
                    _ => None,
                })
                .expect("body paragraph");
            assert_eq!(para.lines[0].stop, expected.len(), "{size}pt: expected {expected:?}");
        }
    }
}

/// Synthetic invariant: distributing the last segment must not move earlier tab stops.
/// This fixture has not been compared with Word.
#[test]
fn multiple_tabs_keep_earlier_stops_during_line_justification() {
    if wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false).substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    for size in [9.0, 18.0] {
        for first in [TabAlign::Right, TabAlign::Decimal] {
            for second in [TabAlign::Left, TabAlign::Right, TabAlign::Decimal] {
                let mut doc = Document::new();
                doc.settings.character_spacing_control = CharacterSpacingControl::CompressPunctuation;
                let props = CharProps {
                    font: Some("Hiragino Kaku Gothic ProN".into()),
                    font_ea: Some("Hiragino Kaku Gothic ProN".into()),
                    size: Some(size),
                    spacing: Some(0.0),
                    ..Default::default()
                };
                let mut p = Paragraph::with_text("A\t1.2\t漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢", props);
                p.props.align = Some(Align::Justify);
                p.props.indent_right = Some(468.0 - 11.0 * size);
                p.props.overflow_punct = Some(false);
                p.props.auto_space_de = Some(false);
                p.props.auto_space_dn = Some(false);
                p.props.snap_to_grid = Some(false);
                p.props.tabs = Some(vec![
                    TabStop { pos: 3.0 * size, align: first, leader: Default::default() },
                    TabStop { pos: 8.0 * size, align: second, leader: Default::default() },
                ]);
                doc.body = vec![para_block(p)];
                let layout = lay(&doc);
                let para = layout
                    .pages
                    .iter()
                    .flat_map(|page| &page.items)
                    .find_map(|item| match item {
                        Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para),
                        _ => None,
                    })
                    .expect("body paragraph");
                assert!(para.lines.len() > 1, "{size}pt must wrap");
                let offset = if first == TabAlign::Right { "A\t1.2".len() } else { "A\t1".len() };
                let x = para.x_of(0, offset).expect("first tab anchor");
                assert!((x - 3.0 * size).abs() < 0.1, "{size} {first:?}/{second:?}: anchor {x}");
            }
        }
        for second in [TabAlign::Right, TabAlign::Decimal] {
            let mut doc = Document::new();
            let props = CharProps {
                font: Some("Hiragino Kaku Gothic ProN".into()),
                font_ea: Some("Hiragino Kaku Gothic ProN".into()),
                size: Some(size),
                spacing: Some(0.0),
                ..Default::default()
            };
            let mut p = Paragraph::with_text("A\t1.2\t漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢漢", props);
            p.props.align = Some(Align::Justify);
            p.props.indent_right = Some(468.0 - 11.0 * size);
            p.props.auto_space_de = Some(false);
            p.props.auto_space_dn = Some(false);
            p.props.snap_to_grid = Some(false);
            p.props.tabs = Some(vec![
                TabStop { pos: 3.0 * size, align: TabAlign::Left, leader: Default::default() },
                TabStop { pos: 8.0 * size, align: second, leader: Default::default() },
            ]);
            doc.body = vec![para_block(p)];
            let layout = lay(&doc);
            let para = layout
                .pages
                .iter()
                .flat_map(|page| &page.items)
                .find_map(|item| match item {
                    Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para),
                    _ => None,
                })
                .expect("body paragraph");
            assert!(para.lines.len() > 1, "{size}pt must wrap");
            let x = para.x_of(0, "A\t".len()).expect("first tab anchor");
            assert!((x - 3.0 * size).abs() < 0.1, "{size} Left/{second:?}: anchor {x}");
        }
    }
}

/// Word's omitted-language document and the otherwise identical explicit-ja clone
/// both break before the small kana after a left/default tab.
#[test]
fn left_and_default_tabs_break_before_small_kana_without_explicit_east_asian_language() {
    if wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false).substituted {
        return;
    }
    for size in SIZES {
        for explicit_tab in [false, true] {
            let layout = |language: Option<&str>| {
                let mut doc = Document::new();
                // The source fixture has no language in its document defaults or run.
                doc.styles.default_chr.lang = None;
                doc.styles.default_chr.lang_ea = None;
                doc.settings.default_tab = 27.0;
                doc.settings.character_spacing_control = CharacterSpacingControl::CompressPunctuation;
                let props = CharProps {
                    font: Some("Arial".into()),
                    font_ea: Some("Hiragino Kaku Gothic ProN".into()),
                    lang_ea: language.map(str::to_string),
                    size: Some(size),
                    spacing: Some(0.0),
                    ..Default::default()
                };
                let mut p = Paragraph::with_text("漢、\t漢ぁ12.3、漢終", props);
                p.props.align = Some(Align::Justify);
                p.props.indent_right = Some(468.0 - 6.5 * size);
                p.props.overflow_punct = Some(false);
                p.props.auto_space_de = Some(false);
                p.props.auto_space_dn = Some(false);
                p.props.snap_to_grid = Some(false);
                if explicit_tab {
                    p.props.tabs = Some(vec![TabStop { pos: 5.0 * size, align: TabAlign::Left, leader: Default::default() }]);
                }
                doc.body = vec![para_block(p)];
                lay(&doc)
                    .pages
                    .iter()
                    .flat_map(|page| &page.items)
                    .find_map(|item| match item {
                        Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para.lines.iter().map(|l| l.stop).collect::<Vec<_>>()),
                        _ => None,
                    })
                    .expect("body paragraph")
            };
            let explicit = layout(Some("ja-JP"));
            let absent = layout(None);
            assert_eq!(absent, explicit, "{size}pt explicit_tab={explicit_tab}");
            assert_eq!(absent.len(), 2, "{size}pt explicit_tab={explicit_tab}");
            let expected_first = if explicit_tab || size <= 5.5 { "漢、\t漢" } else { "漢、\t漢ぁ" };
            assert_eq!(absent[0], expected_first.len(), "{size}pt explicit_tab={explicit_tab}");
        }
    }
}

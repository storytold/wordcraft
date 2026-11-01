//! CJK regression foundation: preservation through layout, mixed Latin/CJK, and line-break
//! opportunities justified by UAX #14 / Word kinsoku (not by copying current bugs).
//!
//! See `docs/cjk-typography-audit.md`. Advanced features without layout (warichū, tate-chū-yoko)
//! are documented there; defect demos that would fail CI stay `#[ignore]`.

use crate::para::ClKind;
use crate::*;
use wordcraft_doc::props::EastAsianLayout;
use wordcraft_doc::{Pos, StoryRef};

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn first_para(l: &DocLayout) -> Arc<ParaLayout> {
    l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap()
}

/// Reconstruct visible text from the clusters placed on each line (Markers skipped), so a cluster
/// left out of every line is noticed.
fn cluster_slices<'a>(para_text: &'a str, pl: &ParaLayout) -> Vec<&'a str> {
    pl.lines
        .iter()
        .flat_map(|line| pl.clusters.get(line.c0..line.c1).into_iter().flatten())
        .filter(|c| c.kind != ClKind::Marker && c.start < c.end && c.end <= para_text.len())
        .map(|c| &para_text[c.start..c.end])
        .collect()
}

/// Ordinary Japanese prose survives shaping and line breaking without dropped characters.
#[test]
fn japanese_sentence_is_preserved_through_layout() {
    // Modern Japanese with comma, period, and brackets.
    let text = "日本語の組版では、「禁則処理」が重要です。";
    let d = Document::from_text(text);
    let pl = first_para(&lay(&d));
    let slices = cluster_slices(text, &pl);
    assert_eq!(slices.concat(), text, "{slices:?}");
    assert_eq!(slices.len(), text.chars().count());
}

/// Ordinary Simplified Chinese prose is preserved the same way.
#[test]
fn chinese_sentence_is_preserved_through_layout() {
    let text = "学而时习之，不亦说乎？";
    let d = Document::from_text(text);
    let pl = first_para(&lay(&d));
    assert_eq!(cluster_slices(text, &pl).concat(), text);
}

/// Mixed Latin and CJK keep every character, in order.
#[test]
fn mixed_latin_and_cjk_text_is_preserved_through_layout() {
    let text = "WordCraftで日本語とEnglishを混ぜる。";
    let d = Document::from_text(text);
    let pl = first_para(&lay(&d));
    assert_eq!(cluster_slices(text, &pl).concat(), text);
}

/// Supplementary-plane ideograph + IVS bytes remain addressable clusters after layout.
#[test]
fn supplementary_and_ivs_codepoints_survive_layout() {
    let ivs = "\u{8FBA}\u{E0100}";
    let text = format!("漢\u{20000}{ivs}字");
    let d = Document::from_text(&text);
    let pl = first_para(&lay(&d));
    let slices = cluster_slices(&text, &pl);
    assert_eq!(slices.concat(), text);
    // IVS is one grapheme cluster → one layout cluster when the shaper keeps it together.
    assert!(slices.contains(&ivs), "IVS should stay one cluster: {slices:?}");
    assert!(slices.contains(&"\u{20000}"), "{slices:?}");
}

/// Lay out `text` in a narrow column; return the first character of each line after the first and
/// the text the layout's clusters reproduce.
fn wrap(text: &str, kinsoku: bool) -> (Vec<char>, String) {
    let mut d = Document::from_text(text);
    d.last_section.page_w = 72.0;
    d.last_section.margin_left = 18.0;
    d.last_section.margin_right = 18.0;
    if let Ok(p) = d.para_mut(StoryRef::Body, &Path::top(0)) {
        p.props.kinsoku = Some(kinsoku);
    }
    let pl = first_para(&lay(&d));
    let heads = pl.lines.iter().skip(1).filter_map(|l| text.get(l.start..).and_then(|s| s.chars().next())).collect();
    (heads, cluster_slices(text, &pl).concat())
}

fn line_heads(text: &str, kinsoku: bool) -> Vec<char> {
    wrap(text, kinsoku).0
}

/// How many ideographs fit on the first line of the narrow column.
fn first_line_capacity() -> usize {
    let probe = "春".repeat(40);
    let mut d = Document::from_text(&probe);
    d.last_section.page_w = 72.0;
    d.last_section.margin_left = 18.0;
    d.last_section.margin_right = 18.0;
    let pl = first_para(&lay(&d));
    pl.lines.get(1).and_then(|l| probe.get(..l.start)).map_or(0, |head| head.chars().count())
}

/// A closing full stop that would land first on a line moves down with the character before it
/// when kinsoku is on (Word Asian typography; JLREQ prohibited line-start). The fixture puts 。
/// exactly where the line overflows, and the kinsoku-off layout proves the fixture can start a
/// line with it.
#[test]
fn wrapped_japanese_line_does_not_start_with_full_stop() {
    let n = first_line_capacity();
    assert!(n >= 2, "fixture needs a measure of at least two characters, got {n}");
    let text = format!("{}。続き", "春".repeat(n));
    assert!(line_heads(&text, false).contains(&'。'), "precondition: with kinsoku off the line starts with 。");
    let (heads, laid_out) = wrap(&text, true);
    assert_eq!(laid_out, text, "wrapping must not drop or reorder text");
    assert!(!heads.contains(&'。'), "kinsoku on: no line starts with 。: {heads:?}");
    assert_eq!(heads.first(), Some(&'春'), "the preceding character travels with it: {heads:?}");
}

/// An opening bracket that would end a line moves down to start the next one when kinsoku is on.
#[test]
fn wrapped_japanese_line_does_not_end_with_opening_bracket() {
    let n = first_line_capacity();
    assert!(n >= 2, "fixture needs a measure of at least two characters, got {n}");
    let text = format!("{}「続き」", "春".repeat(n - 1));
    assert_eq!(
        line_heads(&text, false).first(),
        Some(&'続'),
        "precondition: with kinsoku off 「 is the last character on the first line, so 続 starts the next"
    );
    let (heads, laid_out) = wrap(&text, true);
    assert_eq!(laid_out, text, "wrapping must not drop or reorder text");
    assert_eq!(heads.first(), Some(&'「'), "kinsoku on: 「 starts the next line");
}

/// `w:eastAsianLayout` (warichū / tate-chū-yoko) is stored but not applied in layout yet.
/// When layout lands, this test should assert distinct geometry for `combine` / `vert` runs.
#[test]
#[ignore = "verified gap: layout ignores CharProps::east_asian_layout (see docs/cjk-typography-audit.md)"]
fn east_asian_layout_combine_should_alter_line_geometry() {
    let mut d = Document::from_text("本文二行の注続き");
    let start = "本文".len();
    let end = start + "二行の注".len();
    d.format_range(&Pos::body(0, start), &Pos::body(0, end), &|c| {
        c.east_asian_layout = Some(EastAsianLayout { combine: Some(true), ..Default::default() });
    })
    .unwrap();
    let plain = first_para(&lay(&Document::from_text("本文二行の注続き")));
    let warichu = first_para(&lay(&d));
    // Independent expectation once implemented: warichū should not match plain advances.
    assert_ne!(
        plain.clusters.iter().map(|c| c.adv).collect::<Vec<_>>(),
        warichu.clusters.iter().map(|c| c.adv).collect::<Vec<_>>(),
        "warichū layout should change cluster advances"
    );
}

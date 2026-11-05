//! East Asian typography settings: balanced half-width spaces (`w:balanceSingleByteDoubleByteWidth`,
//! ECMA-376 §17.15.3.3) and compressed full-width punctuation (`w:characterSpacingControl`,
//! §17.15.1.18). Assertions compare layouts with a setting on and off, or against advances measured
//! from the same layout, never font metrics (fonts differ between machines).

use crate::*;
use wordcraft_doc::CharacterSpacing;
use wordcraft_doc::props::{Align, CharProps, ParaProps};

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn first_para(l: &DocLayout) -> Arc<ParaLayout> {
    l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap()
}

fn doc_with(text: &str, chr: CharProps, right: f32) -> Document {
    let mut d = Document::new();
    let mut p = wordcraft_doc::Paragraph::with_text(text, chr);
    p.props = ParaProps { align: Some(Align::Left), indent_right: Some(right), ..Default::default() };
    d.body = vec![wordcraft_doc::para_block(p)];
    d
}

/// Spaces in a run whose East Asian font is fixed pitch are half an em with the setting on (Word's
/// print of a form in 標楷體: 6.00 pt at 12 pt, 5.05 pt at 10 pt), whatever the Latin font's space;
/// with it off, or a variable-pitch East Asian font, they keep the font's own width.
#[test]
fn balanced_spaces_are_half_an_em_in_a_fixed_pitch_east_asian_font() {
    let space = |size: f32, balance: bool, fixed: &[&str]| {
        let chr = CharProps { font: Some("Times New Roman".into()), font_ea: Some("Brand Kai".into()), size: Some(size), ..Default::default() };
        let mut d = doc_with("一 二", chr, 0.0);
        d.settings.balance_sbcs_dbcs = balance;
        d.settings.fixed_pitch_fonts = fixed.iter().map(|f| f.to_string()).collect();
        let pl = first_para(&lay(&d));
        let c = pl.clusters.iter().find(|c| c.kind == crate::para::ClKind::Space).expect("a space");
        c.adv
    };
    for size in [10.0, 12.0] {
        let own = space(size, false, &["Brand Kai"]);
        assert!((own - size / 2.0).abs() > 0.5, "the Latin font's space ({own}) isn't half an em already");
        assert_eq!(space(size, true, &["Brand Kai"]), size / 2.0);
        assert_eq!(space(size, true, &["brand kai"]), size / 2.0, "font names compare without case");
        assert_eq!(space(size, true, &["Other Font"]), own, "a variable-pitch East Asian font keeps its spaces");
        assert_eq!(space(size, true, &[]), own);
    }
}

/// Ideographs with a full-width comma after every third, 12 pt, on a line `right` short of the
/// margin; the first line's cluster count and the paragraph's layout.
fn cjk_line(compress: bool, right: f32) -> (usize, Arc<ParaLayout>) {
    let chr = CharProps { size: Some(12.0), ..Default::default() };
    let mut d = doc_with(&"一二三，".repeat(30), chr, right);
    d.settings.character_spacing = if compress { CharacterSpacing::CompressPunctuation } else { CharacterSpacing::DoNotCompress };
    let pl = first_para(&lay(&d));
    let line = &pl.lines[0];
    (line.c1 - line.c0, pl)
}

/// With `compressPunctuation`, a line that overflows by less than its marks can give up (a third
/// of each, WordCraft's bound from Word's print: 3.0–3.7 pt of a 12 pt mark taken, 4.25 pt not)
/// keeps its last character, taking only the overflow from the marks; one that overflows by more
/// wraps as before, and so does every line without the setting. A line that ends with room keeps
/// its marks whole.
#[test]
fn compressed_punctuation_fits_a_line_only_as_far_as_it_must() {
    let text_width = Document::new().last_section.text_width();
    let (_, probe) = cjk_line(false, 0.0);
    // 4k + 2 clusters: the line ends on an ideograph and the next starts with one, so the break
    // is allowed either side.
    let n = 4 * 8 + 2;
    let width: f32 = probe.clusters[..n].iter().map(|c| c.adv).sum();
    let marks: Vec<usize> = (0..n).filter(|k| k % 4 == 3).collect();
    let room: f32 = marks.iter().map(|&k| probe.clusters[k].adv).sum::<f32>() / 3.0;
    assert!(room > 2.0, "the commas have width: {room}");
    let right = |overflow: f32| text_width - (width - overflow);

    let (fits, pl) = cjk_line(true, right(room * 0.5));
    assert_eq!(fits, n, "half the room is enough");
    let line = &pl.lines[0];
    let end = *line.xs.last().unwrap() - line.xs[0];
    assert!((end - (width - room * 0.5)).abs() < 0.05, "only the overflow is taken: the line ends at {end}");
    // The ideographs keep their width; each comma gives up the same share, half on each side.
    let x = |k: usize| line.xs[k - line.c0];
    let share = room * 0.5 / marks.len() as f32;
    for &k in &marks {
        let ideograph = probe.clusters[k - 1].adv;
        assert!((x(k) - x(k - 1) - (ideograph - share / 2.0)).abs() < 0.01, "before mark {k}");
        assert!((x(k + 1) - x(k) - (probe.clusters[k].adv - share / 2.0)).abs() < 0.01, "after mark {k}");
    }

    assert!(cjk_line(false, right(room * 0.5)).0 < n, "without the setting the line wraps");
    assert!(cjk_line(true, right(room + 0.5)).0 < n, "past a third of each mark the line wraps");
    // A short paragraph (Word's print: three marks on a line with room, uncompressed).
    let short = |compress: bool| {
        let mut d = doc_with("「一二三，四五六。」", CharProps { size: Some(12.0), ..Default::default() }, 0.0);
        d.settings.character_spacing = if compress { CharacterSpacing::CompressPunctuation } else { CharacterSpacing::DoNotCompress };
        first_para(&lay(&d)).lines[0].xs.clone()
    };
    assert_eq!(short(true), short(false), "a line with room keeps its marks whole");
}

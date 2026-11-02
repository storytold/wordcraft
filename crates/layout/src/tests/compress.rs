//! Punctuation compression (`w:characterSpacingControl`) as WPS lays it out: a line that would
//! overflow squeezes its full-width marks by up to a tenth of an em each, a closing mark ending a
//! line needs no room for its blank half, and a line that fits is left alone. Assertions compare
//! layouts with compression on and off and are relative to the font size, never font metrics; a
//! machine without a CJK font draws the marks proportionally, and then nothing is compressed.

use crate::*;
use wordcraft_doc::CharSpacing;
use wordcraft_doc::props::Align;
use wordcraft_doc::section::{DocGrid, DocGridType};

const SIZE: f32 = 16.0;

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn first_para(l: &DocLayout) -> Arc<ParaLayout> {
    l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap()
}

/// `text` at 16 pt on a line `width_em` ems wide (by a right indent), with `mode`.
fn doc(text: &str, mode: CharSpacing, width_em: Option<f32>) -> Document {
    let mut d = Document::from_text(text);
    d.settings.char_spacing = mode;
    let probe = first_para(&lay(&Document::from_text("x")));
    let full = probe.lines[0].right - probe.lines[0].left;
    if let Some(Block::Para(p)) = d.body.first_mut().map(Arc::make_mut) {
        p.props.align = Some(Align::Left);
        p.props.indent_left = Some(0.0);
        if let Some(w) = width_em {
            p.props.indent_right = Some(full - w * SIZE);
        }
        for r in &mut p.runs {
            r.props.size = Some(SIZE);
        }
    }
    d
}

/// Whether the machine draws `，` full-width at 16 pt (else compression has nothing to work on).
fn full_width_marks() -> bool {
    let g = first_para(&lay(&doc("中，中", CharSpacing::DoNotCompress, None)));
    g.clusters.get(1).is_some_and(|c| c.adv >= 0.9 * SIZE)
}

fn first_line_chars(text: &str, mode: CharSpacing, width_em: f32) -> (usize, Arc<ParaLayout>) {
    let g = first_para(&lay(&doc(text, mode, Some(width_em))));
    (g.lines[0].c1 - g.lines[0].c0, g)
}

#[test]
fn a_line_that_fits_is_left_alone() {
    // Adjacent marks (。” ”（ ）、 、《 《“) keep their advances: WPS never closes up pairs.
    let text = "他说：“你好。”（再见）、《“书”》";
    let xs = |mode| first_para(&lay(&doc(text, mode, None))).lines[0].xs.clone();
    assert_eq!(xs(CharSpacing::CompressPunctuation), xs(CharSpacing::DoNotCompress));
    assert_eq!(xs(CharSpacing::CompressPunctuationAndJapaneseKana), xs(CharSpacing::DoNotCompress));
}

#[test]
fn an_overflowing_line_squeezes_its_marks_by_up_to_a_tenth_of_an_em() {
    if !full_width_marks() {
        return;
    }
    // Ten 中， pairs are 20 em. At 19 em the last ，'s blank half is free (19.5 em) and the ten
    // marks give up 0.05 em each; without compression the line holds nine pairs.
    let text = "中，".repeat(14);
    let (on, g) = first_line_chars(&text, CharSpacing::CompressPunctuation, 19.0);
    let (off, _) = first_line_chars(&text, CharSpacing::DoNotCompress, 19.0);
    assert_eq!((on, off), (20, 18));
    let line = &g.lines[0];
    // The squeezed line ends at the margin (its last mark's blank half hanging past it).
    let end = line.xs.last().copied().unwrap_or_default();
    assert!((end - SIZE / 2.0 - line.right).abs() < 0.05, "line ends at {end}, right {}", line.right);
    // Each mark gave up the same, a tenth of an em at most.
    for k in (1..19).step_by(2) {
        let adv = line.xs[k + 1] - line.xs[k];
        assert!((adv - (SIZE - 0.05 * SIZE)).abs() < 0.05, "mark {k} advances {adv}");
    }
}

#[test]
fn a_mark_never_gives_up_more_than_a_tenth_of_an_em() {
    if !full_width_marks() {
        return;
    }
    // At 18.4 em ten pairs would need 0.11 em from each mark: the line holds nine pairs.
    let (on, _) = first_line_chars(&"中，".repeat(14), CharSpacing::CompressPunctuation, 18.4);
    assert_eq!(on, 18);
    let (on, _) = first_line_chars(&"中，".repeat(14), CharSpacing::CompressPunctuation, 18.55);
    assert_eq!(on, 20);
}

#[test]
fn a_closing_mark_ending_a_line_needs_no_room_for_its_blank_half() {
    if !full_width_marks() {
        return;
    }
    // Nineteen ideographs and a mark are 20 em; at 19.6 em the mark fits when its blank half is
    // free. Not for a closing quote (WPS), and not without compression: kinsoku then takes the
    // last ideograph down with the mark.
    let text = |m: char| format!("{}{m}中中中中", "中".repeat(19));
    for m in ['。', '，', '）', '」', '！'] {
        assert_eq!(first_line_chars(&text(m), CharSpacing::CompressPunctuation, 19.6).0, 20, "{m}");
        assert_eq!(first_line_chars(&text(m), CharSpacing::DoNotCompress, 19.6).0, 18, "{m}");
    }
    assert_eq!(first_line_chars(&text('”'), CharSpacing::CompressPunctuation, 19.6).0, 18);
}

#[test]
fn proportional_marks_and_other_characters_are_not_compressible() {
    // A Latin “ (proportional, in the run's Latin font), ideographs, ？ and ——.
    let text = "A“书”？——中";
    let g = first_para(&lay(&doc(text, CharSpacing::CompressPunctuation, None)));
    for s in &g.squeeze {
        let ch = g.clusters.get(s.k).and_then(|c| text.get(c.start..)).and_then(|t| t.chars().next());
        assert!(matches!(ch, Some('“' | '”')), "{ch:?} is compressible");
        assert!(g.clusters.get(s.k).is_some_and(|c| c.adv >= 0.9 * SIZE), "a proportional mark is compressible");
    }
    let off = first_para(&lay(&doc("他说：“你好。”", CharSpacing::DoNotCompress, None)));
    assert!(off.squeeze.is_empty());
}

#[test]
fn changing_the_setting_relays_out_cached_paragraphs() {
    let mut d = doc(&"中，".repeat(14), CharSpacing::DoNotCompress, Some(19.0));
    let mut cache = LayoutCache::new();
    let opts = LayoutOptions::default();
    let lines = |l: &DocLayout| first_para(l).lines.iter().map(|l| (l.c0, l.c1)).collect::<Vec<_>>();
    let before = lines(&layout(&d, &mut cache, &opts));
    d.settings.char_spacing = CharSpacing::CompressPunctuation;
    assert_eq!(lines(&layout(&d, &mut cache, &opts)), lines(&lay(&d)), "a reused cache lays out like a fresh one");
    d.settings.char_spacing = CharSpacing::DoNotCompress;
    assert_eq!(lines(&layout(&d, &mut cache, &opts)), before);
}

#[test]
fn compression_applies_on_a_character_grid() {
    // At the default size a mark fills its cell, so it stays full-width and compressible.
    let text = "“你好。”（再见）";
    let mut d = Document::from_text(text);
    d.settings.char_spacing = CharSpacing::CompressPunctuation;
    d.last_section.doc_grid = Some(DocGrid { kind: DocGridType::LinesAndChars, line_pitch: 15.6, char_space: 0 });
    let g = first_para(&lay(&d));
    let size = g.clusters.get(3).and_then(|c| g.styles.get(c.style as usize)).map_or(0.0, |s| s.size);
    if g.clusters.get(3).is_some_and(|c| c.adv >= 0.9 * size) {
        assert!(g.squeeze.iter().any(|s| s.k == 3), "。 on a grid is compressible: {:?}", g.squeeze);
    }
}

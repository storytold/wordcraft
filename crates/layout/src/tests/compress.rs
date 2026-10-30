//! Punctuation compression (`w:characterSpacingControl`): adjacent full-width East Asian marks
//! share one blank half (JIS X 4051, W3C clreq §3.1.6). Assertions compare a layout with
//! compression on to the same layout with it off, never font metrics (fonts differ between
//! machines; a CJK font may be missing, and then the marks are not full-width and stay as they are).

use crate::*;
use wordcraft_doc::CharSpacing;
use wordcraft_doc::section::{DocGrid, DocGridType};

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn first_para(l: &DocLayout) -> Arc<ParaLayout> {
    l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap()
}

/// Each cluster's change in advance with `mode` against `doNotCompress`, rounded to a hundredth,
/// and what compression should take: for each `(k, mark)` of `compressed`, cluster `k` loses half
/// the em of cluster `mark` when that mark is full-width.
fn changes(text: &str, mode: CharSpacing, compressed: &[(usize, usize)]) -> (Vec<f32>, Vec<f32>) {
    let mut on = Document::from_text(text);
    on.settings.char_spacing = mode;
    let off = Document::from_text(text);
    let (on, off) = (first_para(&lay(&on)), first_para(&lay(&off)));
    assert_eq!(on.clusters.len(), off.clusters.len());
    let r = |v: f32| (v * 100.0).round() / 100.0;
    let got = on.clusters.iter().zip(&off.clusters).map(|(a, b)| r(a.adv - b.adv)).collect();
    let want = off
        .clusters
        .iter()
        .enumerate()
        .map(|(k, _)| {
            let Some(&(_, m)) = compressed.iter().find(|c| c.0 == k) else { return 0.0 };
            let c = &off.clusters[m];
            let size = off.styles.get(c.style as usize).map_or(0.0, |s| s.size);
            if c.adv >= 0.9 * size { r(-size / 2.0) } else { 0.0 }
        })
        .collect();
    (got, want)
}

#[test]
fn adjacent_full_width_marks_share_one_blank_half() {
    // 他说：“你好。”（再见）、《“书”》: 。” — ”（ — ）、 — 、《 — 《“ (the second's blank, taken
    // from the first's advance) — ”》.
    let text = "他说：“你好。”（再见）、《“书”》";
    let marks = [(6, 6), (7, 7), (11, 11), (12, 12), (13, 14), (16, 16)];
    let (got, want) = changes(text, CharSpacing::CompressPunctuation, &marks);
    assert_eq!(got, want);
    let (kana, _) = changes(text, CharSpacing::CompressPunctuationAndJapaneseKana, &marks);
    assert_eq!(kana, want, "kana mode compresses punctuation too");
}

#[test]
fn nothing_changes_without_compression_or_between_other_characters() {
    let text = "他说：“你好。”（再见）";
    let (got, _) = changes(text, CharSpacing::DoNotCompress, &[]);
    assert!(got.iter().all(|&v| v == 0.0), "{got:?}");
    // Ideographs beside marks, a mark before a space, opening then closing, Latin quotes.
    for text in ["你“好”吗", "好。 “是", "（）", "\"(a)\"", "say \u{201D}\u{201C} it"] {
        let (got, _) = changes(text, CharSpacing::CompressPunctuation, &[]);
        assert!(got.iter().all(|&v| v == 0.0), "{text}: {got:?}");
    }
}

#[test]
fn text_on_a_character_grid_keeps_its_cells() {
    let text = "“你好。”（再见）";
    let grid = |d: &mut Document| {
        d.last_section.doc_grid = Some(DocGrid { kind: DocGridType::LinesAndChars, line_pitch: 15.6, char_space: 0 });
    };
    let mut on = Document::from_text(text);
    on.settings.char_spacing = CharSpacing::CompressPunctuation;
    grid(&mut on);
    let mut off = Document::from_text(text);
    grid(&mut off);
    let advs = |d: &Document| first_para(&lay(d)).clusters.iter().map(|c| c.adv).collect::<Vec<f32>>();
    assert_eq!(advs(&on), advs(&off));
}

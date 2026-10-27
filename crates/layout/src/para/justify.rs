use super::model::ClKind;
use super::proof::{ProofRun, lang_at};
use super::*;

/// Word's default hyphenation zone (0.25"), points.
pub(super) const HYPHENATION_ZONE: f32 = wordcraft_doc::DEFAULT_HYPHENATION_ZONE;

/// How much of their width the spaces of a justified line may give up to fit more text (Word
/// 2013+, compatibility mode 15).
const MAX_SPACE_SHRINK: f32 = 0.25;

/// How far a word (with the space before it) may overrun a justified line and still be fitted
/// onto it by shrinking the line's spaces, as a share of its own width.
const MAX_WORD_OVERRUN: f32 = 0.355;

/// How far a justified line's text may run past its right edge, its spaces (`spaces` wide in
/// all) shrinking to make room, when the word now ending the line is `word` wide with the space
/// before it. Measured in Word for Mac (compatibility mode 15, synthetic justified paragraphs):
/// at most a quarter of the spaces' width, and at most 0.355 of the word's own (0.351 to 0.360
/// across words of 2.7 to 80 pt and spaces of 3.3 and 6.7 pt).
pub(super) fn space_room(spaces: f32, word: f32) -> f32 {
    (spaces * MAX_SPACE_SHRINK).min(word.max(0.0) * MAX_WORD_OVERRUN)
}

/// Take `-slack` (an overflow) out of the line's inner spaces, in proportion to their widths and
/// never more than [`MAX_SPACE_SHRINK`] of them. Trailing spaces hang and are left alone.
pub(super) fn shrink_line_spaces(pl: &ParaLayout, xs: &mut [f32], c0: usize, c1: usize, content_end: f32, slack: f32) {
    let spaces: Vec<(usize, f32)> = (c0..c1)
        .filter_map(|k| {
            let c = pl.clusters.get(k).filter(|c| c.kind == ClKind::Space)?;
            (xs.get(k - c0).copied().unwrap_or(f32::MAX) < content_end - 0.01).then_some((k, c.adv))
        })
        .collect();
    let total: f32 = spaces.iter().map(|s| s.1).sum();
    if total <= 0.0 || !slack.is_finite() {
        return;
    }
    let ratio = (-slack / total).clamp(0.0, MAX_SPACE_SHRINK);
    let mut take = 0.0;
    let mut next = spaces.iter().peekable();
    for k in c0..c1 {
        if let Some(v) = xs.get_mut(k - c0) {
            *v -= take;
        }
        if let Some((_, adv)) = next.next_if(|s| s.0 == k) {
            take += adv * ratio;
        }
    }
    if let Some(v) = xs.last_mut() {
        *v -= take;
    }
}

/// The hyphen glyph and advance in a style (cached per paragraph).
pub(super) fn hyphen_glyph(pl: &ParaLayout, style: u32, cache: &mut Vec<(u32, u32, f32)>) -> (u32, u32, f32) {
    if let Some(h) = cache.iter().find(|h| h.0 == style) {
        return *h;
    }
    let h = pl
        .styles
        .get(style as usize)
        .and_then(|st| {
            let g = wordcraft_fonts::shape(&st.face, "-", &[], |c| c).into_iter().next()?;
            let k = st.size / st.face.upem.max(1.0) as f32 * st.rc.scale / 100.0;
            Some((style, g.gid, g.x_advance as f32 * k))
        })
        .unwrap_or((style, 0, 0.0));
    cache.push(h);
    h
}

/// Clusters after which a line may end with a hyphen: soft hyphens always, and dictionary or
/// pattern hyphenation points of each word, in the language of its run ([`ProofRun`]), when
/// automatic hyphenation is on (words in capitals only with `caps`). Both are found in `text`, the paragraph as laid out (without what
/// the layout leaves out, [`left_out`]), and `to_para` maps the end of a char there to the
/// paragraph's text.
pub(super) fn hyphenation_points(
    text: &str,
    to_para: impl Fn(usize) -> Option<usize>,
    pl: &ParaLayout,
    auto: bool,
    caps: bool,
    runs: &[ProofRun],
) -> Vec<u32> {
    let mut bytes: Vec<usize> = text.char_indices().filter(|(_, c)| *c == SOFT_HYPHEN).map(|(i, c)| i + c.len_utf8()).collect();
    // Vietnamese words are single syllables and never take a hyphen: no automatic hyphenation in
    // a Vietnamese paragraph, nor in a word written with a Vietnamese-only letter (`Nguyễn`).
    if auto && !wordcraft_proof::vi::looks_vietnamese(text) {
        let lim = wordcraft_proof::hyphen::Limits::default();
        let mut start: Option<usize> = None;
        for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
            let word_char = c.is_alphabetic() || c == '\'' || c == '\u{2019}';
            match (start, word_char) {
                (None, true) => start = Some(i),
                (Some(a), false) => {
                    // The patterns are for Latin-script languages: never hyphenate Persian/Arabic words.
                    // Words in capitals (`NASA`, `README`) only when the document allows it.
                    let all_caps = |w: &str| w.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
                    if let Some(w) = text.get(a..i)
                        && w.chars().count() >= lim.min_word
                        && !w.chars().any(wordcraft_fonts::is_rtl)
                        && !wordcraft_proof::vi::is_vietnamese_word(w)
                        && (caps || !all_caps(w))
                    {
                        // The word's language: its first letter's run, found by its place in `p.text`.
                        let first = w.chars().next().map_or(1, char::len_utf8);
                        let lang = to_para(a + first).map(|e| lang_at(runs, e.saturating_sub(first))).unwrap_or_default();
                        let offs: Vec<usize> = w.char_indices().map(|(o, _)| o).collect();
                        for pt in wordcraft_proof::hyphen::hyphen_points_in(w, &lim, lang) {
                            if let Some(o) = offs.get(pt) {
                                bytes.push(a + o);
                            }
                        }
                    }
                    start = None;
                }
                _ => {}
            }
        }
    }
    let mut bytes: Vec<usize> = bytes.into_iter().filter_map(to_para).collect();
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes.sort_unstable();
    let mut out: Vec<u32> = pl
        .clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.kind, ClKind::Text | ClKind::Marker) && bytes.binary_search(&c.end).is_ok())
        .map(|(i, _)| i as u32)
        .collect();
    out.dedup();
    out
}

/// The gap after each cluster for Thai Distributed and Khmer Justify. A zero-width space is a
/// cluster of its own with no width: the gap after it is the word boundary it marks, and the
/// cluster before it has none.
pub(crate) fn cluster_gaps(text: &str, clusters: &[Cluster]) -> Vec<wordcraft_doc::khmer::Gap> {
    use wordcraft_doc::khmer::{self, Gap};
    let slice = |c: &Cluster| text.get(c.start..c.end).unwrap_or("");
    let mut out = vec![Gap::None; clusters.len()];
    let mut k = 0;
    while let Some(c) = clusters.get(k) {
        k += 1;
        let t = slice(c);
        if c.kind == ClKind::Space {
            if let Some(g) = out.get_mut(k - 1) {
                *g = Gap::Space;
            }
            continue;
        }
        if !matches!(c.kind, ClKind::Text) {
            continue;
        }
        let Some(before) = t.chars().next_back() else { continue };
        // Skip what takes no room (zero-width spaces, bookmark and comment anchors, soft hyphens):
        // a zero-width space turns the gap into a word boundary, the rest leave it as it is.
        let mut j = k;
        let mut zwsp = false;
        while let Some(n) = clusters.get(j).filter(|n| n.kind == ClKind::Marker) {
            let t = slice(n);
            zwsp |= !t.is_empty() && t.chars().all(khmer::is_zero_width_space);
            j += 1;
        }
        let Some(next) = clusters.get(j).filter(|n| n.kind == ClKind::Text) else { continue };
        let Some(after) = slice(next).chars().next() else { continue };
        if khmer::is_zero_width_space(before) {
            continue;
        }
        let gap =
            match (khmer::is_khmer(before) || khmer::is_khmer(after), wordcraft_doc::thai::is_thai(before) || wordcraft_doc::thai::is_thai(after)) {
                (true, _) => khmer::gap_between(before, after, zwsp),
                (false, true) => wordcraft_doc::thai::gap_between(before, after),
                _ if zwsp => Gap::Word,
                _ => Gap::None,
            };
        // The gap sits after the last cluster before `next` (the zero-width space, if any).
        if let Some(g) = out.get_mut(j - 1) {
            *g = gap;
        }
    }
    out
}

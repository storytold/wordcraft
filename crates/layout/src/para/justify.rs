use super::model::ClKind;
use super::proof::{ProofRun, lang_at};
use super::{ParaLayout, SOFT_HYPHEN};

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

/// How much of its width a full-width punctuation mark may give up when the document compresses
/// punctuation. Word's print of one form (zh-TW, 12 pt marks) bounds it: a line that needed 3.0
/// to 3.7 pt from each of its marks was compressed onto the line, one that needed 4.25 pt from
/// each wasn't (its last character went to the next line). A third of the mark lies between;
/// Word's own figure isn't known.
const MAX_PUNCT_SHRINK: f32 = 1.0 / 3.0;

/// How far a line may run past its edge when its full-width punctuation, `punct` wide in all,
/// is compressed (`w:characterSpacingControl`, ECMA-376 §17.15.1.18).
pub(super) fn punct_room(punct: f32) -> f32 {
    punct.max(0.0) * MAX_PUNCT_SHRINK
}

/// Take `-slack` (an overflow) out of the line's compressible punctuation ([`ParaLayout::compressible`]),
/// in proportion to the marks' widths and never more than [`MAX_PUNCT_SHRINK`] of them; each
/// mark gives up room on both sides alike. Returns how much was taken. Only what the line needs
/// is taken, as in Word's print of one form (zh-TW text, `compressPunctuation`): a 552 pt line
/// of 46 full-width characters, 6 of them punctuation, was set in the 549 pt line, each of the
/// marks narrower by 0.4 to 0.6 pt; a line with room keeps its marks whole.
pub(super) fn shrink_line_punct(pl: &ParaLayout, xs: &mut [f32], c0: usize, c1: usize, content_end: f32, slack: f32) -> f32 {
    let lo = pl.compressible.partition_point(|&k| (k as usize) < c0);
    let marks: Vec<(usize, f32)> = pl
        .compressible
        .get(lo..)
        .unwrap_or_default()
        .iter()
        .map(|&k| k as usize)
        .take_while(|&k| k < c1)
        .filter(|&k| xs.get(k - c0).copied().unwrap_or(f32::MAX) < content_end - 0.01)
        .filter_map(|k| pl.clusters.get(k).map(|c| (k, c.adv)))
        .filter(|(_, adv)| *adv > 0.0)
        .collect();
    let total: f32 = marks.iter().map(|m| m.1).sum();
    if total <= 0.0 || !slack.is_finite() || slack >= 0.0 {
        return 0.0;
    }
    let ratio = (-slack / total).clamp(0.0, MAX_PUNCT_SHRINK);
    let mut take = 0.0;
    let mut next = marks.iter().peekable();
    for k in c0..c1 {
        let mark = next.next_if(|m| m.0 == k).map(|m| m.1 * ratio);
        if let Some(v) = xs.get_mut(k - c0) {
            *v -= take + mark.unwrap_or(0.0) / 2.0;
        }
        take += mark.unwrap_or(0.0);
    }
    if let Some(v) = xs.get_mut(c1 - c0) {
        *v -= take;
    }
    take
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

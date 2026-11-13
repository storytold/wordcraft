use super::model::ClKind;
use super::proof::{ProofRun, lang_at};
use super::{ParaLayout, SOFT_HYPHEN};

/// Word's default hyphenation zone (0.25"), points.
pub(super) const HYPHENATION_ZONE: f32 = wordcraft_doc::DEFAULT_HYPHENATION_ZONE;

/// How much of their width the spaces of a justified line may give up to fit more text (Word
/// 2013+, compatibility mode 15).
pub(super) const MAX_SPACE_SHRINK: f32 = 0.25;

/// How much of their width the single spaces of a line may give up in a document opened from
/// RTF: just under half (Word keeps the word at 49.2% and wraps it at 50%).
pub(super) const RTF_MAX_SPACE_SHRINK: f32 = 0.5;

/// Points inside the right edge the space before the overrunning word must start, for widths
/// summed in `f32`: Word wraps the word when that space starts exactly on the edge.
const RTF_EDGE_TOLERANCE: f32 = 0.01;

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

/// How far a line of a document opened from RTF may run past its right edge, its inner single
/// spaces (`spaces` wide in all, up to and including the one before the word) shrinking to make
/// room, when the word ending the line ends `overrun` past the edge and the space before it
/// starts at `space_from`. Measured in Word (synthetic RTF files, Courier New 8 to 20 pt, left,
/// right, centred and justified alike): the word stays when the space before it starts inside the
/// line and the overrun is less than half the width of the line's single spaces, however many
/// there are; the word's own width doesn't limit it. Otherwise nothing (the word wraps). Only
/// single spaces count: a run of two or more keeps its width. The caller squeezes only lines whose
/// single spaces are all in fixed-pitch fonts: in Times New Roman an overrun of 0.125 pt wraps.
///
/// With `\wraptrsp` (`Settings::wrap_trail_spaces`, measured for justified lines only) `first` is
/// how far the word's first character ends past the edge and the width of the space before the
/// word: Word keeps the word only when that character overruns by less than half that space.
pub(super) fn rtf_space_room(spaces: f32, overrun: f32, space_from: f32, right_edge: f32, first: Option<(f32, f32)>) -> f32 {
    let first_ok = first.is_none_or(|(over, space)| over < space * 0.5);
    let starts_inside = space_from < right_edge - RTF_EDGE_TOLERANCE;
    if first_ok && starts_inside && overrun > 0.0 && overrun < spaces * RTF_MAX_SPACE_SHRINK { overrun } else { 0.0 }
}

/// Whether the space cluster `k` stands alone: no space next to it (markers between don't
/// separate them).
fn single_space(pl: &ParaLayout, k: usize) -> bool {
    let is_space = |c: Option<&super::model::Cluster>| c.is_some_and(|c| c.kind == ClKind::Space);
    let before = pl.clusters.get(..k).and_then(|cs| cs.iter().rev().find(|c| c.kind != ClKind::Marker));
    let after = pl.clusters.get(k + 1..).and_then(|cs| cs.iter().find(|c| c.kind != ClKind::Marker));
    !is_space(before) && !is_space(after)
}

/// Take `-slack` (an overflow) out of the line's inner spaces, in proportion to their widths and
/// never more than `cap` of them ([`MAX_SPACE_SHRINK`], or [`RTF_MAX_SPACE_SHRINK`]). Trailing
/// spaces hang and are left alone; with `skip_leading`, so are the spaces before the line's
/// first character. With `singles_only` (RTF) a run of two or more spaces keeps its width and
/// only single spaces shrink.
pub(super) fn shrink_line_spaces(
    pl: &ParaLayout,
    xs: &mut [f32],
    (c0, c1): (usize, usize),
    (content_end, slack): (f32, f32),
    cap: f32,
    (skip_leading, singles_only): (bool, bool),
) {
    let first = if skip_leading {
        (c0..c1).find(|k| pl.clusters.get(*k).is_some_and(|c| c.kind != ClKind::Space && c.kind != ClKind::Marker)).unwrap_or(c1)
    } else {
        c0
    };
    let spaces: Vec<(usize, f32)> = (first..c1)
        .filter_map(|k| {
            let c = pl.clusters.get(k).filter(|c| c.kind == ClKind::Space)?;
            if singles_only && !single_space(pl, k) {
                return None;
            }
            (xs.get(k - c0).copied().unwrap_or(f32::MAX) < content_end - 0.01).then_some((k, c.adv))
        })
        .collect();
    let total: f32 = spaces.iter().map(|s| s.1).sum();
    if total <= 0.0 || !slack.is_finite() {
        return;
    }
    let ratio = (-slack / total).clamp(0.0, cap);
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

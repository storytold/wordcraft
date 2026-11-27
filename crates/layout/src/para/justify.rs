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
) -> (Vec<u32>, Vec<std::ops::Range<usize>>) {
    let mut bytes: Vec<usize> = text.char_indices().filter(|(_, c)| *c == SOFT_HYPHEN).map(|(i, c)| i + c.len_utf8()).collect();
    let mut words = Vec::new();
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
                        // Word's English and Spanish automatic hyphenation
                        // leaves three final letters. Other languages retain
                        // their own limits (Polish, for example, permits two).
                        let before_last = match lang {
                            wordcraft_proof::ProofLang::En
                            | wordcraft_proof::ProofLang::EnGb
                            | wordcraft_proof::ProofLang::EnAu
                            | wordcraft_proof::ProofLang::EnCa
                            | wordcraft_proof::ProofLang::Es => 3,
                            _ => lim.before_last,
                        };
                        let lim = wordcraft_proof::hyphen::Limits { before_last, ..lim };
                        let offs: Vec<usize> = w.char_indices().map(|(o, _)| o).collect();
                        // The proofing API conservatively excludes all-caps
                        // words. Layout has the document's explicit caps option,
                        // so allow their lowercase patterns here when enabled.
                        // Keep one char per input char, as the pattern reader
                        // does, so points still index the original word.
                        let lower;
                        let word = if caps && all_caps(w) {
                            lower = w.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect::<String>();
                            lower.as_str()
                        } else {
                            w
                        };
                        let mut points = wordcraft_proof::hyphen::hyphen_points_in(word, &lim, lang);
                        if lang == wordcraft_proof::ProofLang::Es {
                            points.extend(spanish_extra_points(word, &lim));
                            retain_spanish_vocalic_sides(word, &mut points);
                        }
                        if !points.is_empty()
                            && let (Some(start), Some(end)) = (to_para(a + first), to_para(i))
                        {
                            let start = pl.clusters.partition_point(|c| c.end <= start.saturating_sub(first));
                            let end = pl.clusters.partition_point(|c| c.start < end);
                            words.push(start..end);
                        }
                        for pt in points {
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
        return (Vec::new(), Vec::new());
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
    (out, words)
}

/// The Spanish `y-he` opportunity is independent of case. Keep this supplement
/// separate from dictionary patterns: it does not add arbitrary breaks before h.
fn spanish_extra_points(word: &str, lim: &wordcraft_proof::hyphen::Limits) -> Vec<usize> {
    let chars: Vec<char> = word.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
    chars
        .windows(3)
        .enumerate()
        .filter_map(|(i, c)| {
            let pt = i + 1;
            (c == ['y', 'h', 'e'] && pt >= lim.after_first && chars.len() - pt >= lim.before_last).then_some(pt)
        })
        .collect()
}

#[cfg(test)]
mod spanish_tests {
    #[test]
    fn supplemental_points_only_apply_to_yhe_and_respect_margins() {
        let lim = wordcraft_proof::hyphen::Limits { before_last: 3, ..Default::default() };
        for word in ["copyhere", "CopyHere", "COPYHERE", "macyhetera", "macyHetera", "macyheTera", "macyhero"] {
            assert_eq!(super::spanish_extra_points(word, &lim), vec![word.to_ascii_lowercase().find('h').unwrap()]);
        }
        for word in ["copyhara", "copyhira", "copyhora", "copyhura", "copihere", "yhere", "copyhe"] {
            assert!(super::spanish_extra_points(word, &lim).is_empty(), "{word}");
        }
    }
}

/// Pattern matches inside a consonant cluster do not constitute Spanish
/// syllables. Public layout controls require a vocalic nucleus on either side.
/// In foreign words y can support the prefix (`tyr-fesa`), but is not sufficient
/// in the suffix (`rafe-sytr`, unlike `rafe-sitr`). This is a necessary filter,
/// not a dictionary or a complete syllabifier.
fn retain_spanish_vocalic_sides(word: &str, points: &mut Vec<usize>) {
    let vowel = |c: char| {
        matches!(c.to_lowercase().next(), Some('a' | 'e' | 'i' | 'o' | 'u' | '\u{e1}' | '\u{e9}' | '\u{ed}' | '\u{f3}' | '\u{fa}' | '\u{fc}'))
    };
    let first = word.chars().position(|c| vowel(c) || matches!(c, 'y' | 'Y'));
    let last = word.chars().enumerate().filter_map(|(i, c)| vowel(c).then_some(i)).last();
    points.retain(|&pt| first.is_some_and(|i| pt > i) && last.is_some_and(|i| pt <= i));
}

#[cfg(test)]
mod vocalic_tests {
    #[test]
    fn spanish_patterns_cannot_leave_a_consonant_only_side() {
        for (word, point, keep) in [
            ("bcfesa", 2, false),
            ("trfesa", 2, false),
            ("hrqbfiyc", 4, false),
            ("bacfesa", 3, true),
            ("atrfesa", 3, true),
            ("tyrfesa", 3, true),
            ("rafesytr", 4, false),
            ("rafetry", 4, false),
            ("rafesitr", 4, true),
            ("rafetri", 4, true),
            ("fesabcf", 4, false),
            ("fesatr", 4, false),
            ("fesabcf", 2, true),
        ] {
            let mut points = vec![point];
            super::retain_spanish_vocalic_sides(word, &mut points);
            assert_eq!(!points.is_empty(), keep, "{word} at {point}");
        }
    }
}

//! Text clusters for complex scripts (Myanmar, Khmer, Tai Tham, the Indic scripts): where the
//! caret may stop, what Backspace removes, and where Myanmar / Thai / Lao text may wrap.
//!
//! Extended grapheme clusters (UAX #29) split some syllables that a shaper draws as one unit: a
//! Myanmar consonant stacked under a virama (`မ္ဘ`), the Myanmar vowel signs U+102B/U+102C and
//! the visarga U+1038 (excluded from the grapheme `SpacingMark` class), Khmer subscript
//! consonants after the coeng. A caret between those would sit inside a glyph cluster, so the
//! caret steps and layout clusters both use [`joins_cluster`] on top of the grapheme rules.

use std::sync::OnceLock;

use unicode_segmentation::UnicodeSegmentation;

/// Invisible stackers: the next consonant is drawn under this one (Myanmar virama, Khmer coeng,
/// Tai Tham sakot).
fn is_stacker(c: char) -> bool {
    matches!(u32::from(c), 0x1039 | 0x17D2 | 0x1A60)
}

/// Dependent vowel signs, medials, tone marks and other combining signs of Myanmar, Khmer and
/// Tai Tham: never the start of a cluster.
fn is_dependent(c: char) -> bool {
    matches!(u32::from(c),
        0x102B..=0x103E
        | 0x1056..=0x1059
        | 0x105E..=0x1060
        | 0x1062..=0x1064
        | 0x1067..=0x106D
        | 0x1071..=0x1074
        | 0x1082..=0x108D
        | 0x108F
        | 0x109A..=0x109D
        | 0xA9E5
        | 0xAA7B..=0xAA7D
        | 0x17B4..=0x17D1
        | 0x17D3
        | 0x17DD
        | 0x1A55..=0x1A5E
        | 0x1A61..=0x1A7C
        | 0x1A7F)
}

/// No caret stop between `prev` and `next` although a grapheme boundary falls there: `next` is a
/// sign that belongs to the syllable before it, or `prev` stacks the next consonant.
pub fn joins_cluster(prev: char, next: char) -> bool {
    is_stacker(prev) || is_dependent(next)
}

/// Is `off` a cluster boundary of `text` (0 and `text.len()` always are)?
pub fn is_cluster_boundary(text: &str, off: usize) -> bool {
    if off == 0 || off >= text.len() {
        return true;
    }
    if !text.is_char_boundary(off) {
        return false;
    }
    let mut g = unicode_segmentation::GraphemeCursor::new(off, text.len(), true);
    if !g.is_boundary(text, 0).unwrap_or(true) {
        return false;
    }
    let prev = text.get(..off).and_then(|t| t.chars().next_back());
    let next = text.get(off..).and_then(|t| t.chars().next());
    !matches!((prev, next), (Some(p), Some(n)) if joins_cluster(p, n))
}

/// Byte offsets where clusters start (graphemes, joined per [`joins_cluster`]), in order; the
/// first is 0 for non-empty text.
pub fn cluster_starts(text: &str) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut prev: Option<char> = None;
    for (i, g) in text.grapheme_indices(true) {
        let first = g.chars().next();
        let joined = matches!((prev, first), (Some(p), Some(n)) if joins_cluster(p, n));
        if !joined {
            out.push(i);
        }
        prev = g.chars().next_back();
    }
    out
}

/// `off` clamped to `text` and moved back to a character boundary.
fn char_floor(text: &str, off: usize) -> usize {
    let mut o = off.min(text.len());
    while o > 0 && !text.is_char_boundary(o) {
        o -= 1;
    }
    o
}

/// The previous cluster boundary before `off` (0 at the start).
pub fn prev_cluster(text: &str, off: usize) -> usize {
    let mut off = char_floor(text, off);
    loop {
        let mut c = unicode_segmentation::GraphemeCursor::new(off, text.len(), true);
        off = c.prev_boundary(text, 0).ok().flatten().unwrap_or(0);
        if is_cluster_boundary(text, off) {
            return off;
        }
    }
}

/// The next cluster boundary after `off` (`text.len()` at the end).
pub fn next_cluster(text: &str, off: usize) -> usize {
    let mut off = char_floor(text, off);
    loop {
        let mut c = unicode_segmentation::GraphemeCursor::new(off, text.len(), true);
        off = c.next_boundary(text, 0).ok().flatten().unwrap_or(text.len());
        if is_cluster_boundary(text, off) {
            return off;
        }
    }
}

/// Scripts whose Backspace removes one character at a time (a vowel sign, a medial, a stacked
/// consonant) rather than the whole cluster, as on Windows and macOS: the Brahmic scripts of
/// South and South-East Asia.
pub fn backspaces_by_char(c: char) -> bool {
    matches!(u32::from(c),
        0x0900..=0x0DFF // Devanagari … Sinhala
        | 0x0E00..=0x0EFF // Thai, Lao
        | 0x0F00..=0x0FFF // Tibetan
        | 0x1000..=0x109F // Myanmar
        | 0x1780..=0x17FF // Khmer
        | 0x1A20..=0x1AAF // Tai Tham
        | 0x1B00..=0x1B7F // Balinese
        | 0xA980..=0xA9FF // Javanese, Myanmar Extended-B
        | 0xAA60..=0xAA7F) // Myanmar Extended-A
}

/// A Myanmar character that starts a syllable: a consonant (with the Mon, Shan and Karen ones),
/// an independent vowel, the great sa.
fn myanmar_syllable_start(c: char) -> bool {
    matches!(u32::from(c), 0x1000..=0x102A | 0x103F | 0x1050..=0x1055 | 0x105A..=0x105D | 0x1061 | 0x1065..=0x1066 | 0x106E..=0x1070 | 0x1075..=0x1081 | 0x108E)
}

/// Line-break opportunities inside Myanmar text, which has no spaces between words: before each
/// syllable (UAX #14 leaves class SA to a dictionary; Word breaks Myanmar between syllables). A
/// consonant starts a syllable unless it is stacked (after U+1039) or a final (followed, perhaps
/// after a medial, by the asat U+103A, the virama U+1039 or the dot below U+1037). Byte offsets,
/// sorted; only between two Myanmar characters.
pub fn myanmar_breaks(text: &str) -> Vec<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let myanmar = |c: char| matches!(u32::from(c), 0x1000..=0x109F | 0xA9E0..=0xA9FF | 0xAA60..=0xAA7F);
    let mut out = Vec::new();
    for k in 1..chars.len() {
        let (Some(&(i, c)), Some(&(_, prev))) = (chars.get(k), chars.get(k - 1)) else { continue };
        if !myanmar(prev) || !myanmar_syllable_start(c) || is_stacker(prev) {
            continue;
        }
        // Skip one medial ya/ha, then look for a killer.
        let mut j = k + 1;
        if chars.get(j).is_some_and(|&(_, n)| matches!(n, '\u{103B}' | '\u{103E}')) {
            j += 1;
        }
        if chars.get(j).is_some_and(|&(_, n)| matches!(n, '\u{1037}' | '\u{1039}' | '\u{103A}')) {
            continue;
        }
        out.push(i);
    }
    out
}

/// Whether `c` is a Thai or Lao character (U+0E00–U+0EFF).
pub fn is_thai_lao(c: char) -> bool {
    matches!(u32::from(c), 0x0E00..=0x0EFF)
}

/// Thai combining marks: above/below vowel signs, tone marks, and diacritics that stack on top
/// of or below the base consonant. These are Unicode `Extend` (UAX #29) so UAX #29 grapheme
/// clusters already keep them attached to their base — the caret never stops on one alone.
/// This predicate exists to make the rule explicit and to drive tests.
#[cfg(test)]
fn is_thai_combining(c: char) -> bool {
    matches!(u32::from(c),
        0x0E31              // mai han akat (above short vowel marker)
        | 0x0E34..=0x0E37  // sara i, sara ii, sara ue, sara uee (above)
        | 0x0E38..=0x0E3A  // sara u, sara uu, phinthu (below)
        | 0x0E47..=0x0E4E) // maitaikhu, mai ek–chattawa (tone), thanthakhat, nikhahit, yamakkan
}

fn word_segmenter() -> &'static icu_segmenter::WordSegmenterBorrowed<'static> {
    static S: OnceLock<icu_segmenter::WordSegmenterBorrowed<'static>> = OnceLock::new();
    S.get_or_init(|| icu_segmenter::WordSegmenter::new_auto(icu_segmenter::options::WordBreakInvariantOptions::default()))
}

fn line_segmenter() -> &'static icu_segmenter::LineSegmenterBorrowed<'static> {
    static S: OnceLock<icu_segmenter::LineSegmenterBorrowed<'static>> = OnceLock::new();
    S.get_or_init(|| icu_segmenter::LineSegmenter::new_dictionary(icu_segmenter::options::LineBreakOptions::default()))
}

/// The byte range of the contiguous Thai/Lao script run that contains `off`, plus the
/// char-boundary-snapped position of `off` within the text.
///
/// ICU4X dictionary segmentation only understands Thai and Lao; feeding it mixed Thai/Latin text
/// produces wrong word boundaries.  Every script transition is itself a word boundary, so
/// segmenting the isolated run is both correct and sufficient.
///
/// Returns `(run_start, run_end, snapped_off)`.  When `off` sits on a non-Thai/Lao character
/// (e.g. a trailing space), the run is the adjacent Thai/Lao span to the left, so
/// `run_end == snapped_off`.
fn thai_run_at(text: &str, off: usize) -> (usize, usize, usize) {
    // Snap to the nearest char boundary ≤ off (handles mid-multibyte-char positions).
    let off = text.floor_char_boundary(off.min(text.len()));
    // Scan backward through text[..off]: start is the byte after the last non-Thai/Lao char.
    let mut start = 0;
    for (i, c) in text[..off].char_indices() {
        if !is_thai_lao(c) {
            start = i + c.len_utf8();
        }
    }
    // Scan forward through text[off..]: end is just past the last Thai/Lao char.
    let mut end = off;
    for (i, c) in text[off..].char_indices() {
        if !is_thai_lao(c) {
            break;
        }
        end = off + i + c.len_utf8();
    }
    (start, end, off)
}

/// Start of the Thai/Lao word at or before `off` using ICU4X dictionary word segmentation
/// (Ctrl+Left / Option+Left inside a Thai/Lao run). `off` must be ≤ `text.len()`.
pub fn thai_word_start(text: &str, off: usize) -> usize {
    let (rs, re, off) = thai_run_at(text, off);
    let run = &text[rs..re];
    let rel = off.saturating_sub(rs);
    let mut prev = 0usize;
    for b in word_segmenter().segment_str(run) {
        if b >= rel {
            break;
        }
        prev = b;
    }
    rs + prev
}

/// End of the Thai/Lao word after `off` using ICU4X dictionary word segmentation
/// (Ctrl+Right / Option+Right inside a Thai/Lao run), including any trailing spaces or
/// non-breaking spaces — matching the behaviour of the generic `word_end`. `off` must be
/// ≤ `text.len()`.
pub fn thai_word_end(text: &str, off: usize) -> usize {
    let (rs, re, off) = thai_run_at(text, off);
    let run = &text[rs..re];
    let rel = off.saturating_sub(rs);
    let rel_end = word_segmenter().segment_str(run).find(|&b| b > rel).unwrap_or(run.len());
    let mut end = rs + rel_end;
    while let Some(c) = text.get(end..).and_then(|s| s.chars().next()) {
        if !matches!(c, ' ' | '\u{00A0}') {
            break;
        }
        end += c.len_utf8();
    }
    end
}

/// The Thai/Lao word around `off` for double-click selection: (start, end).
///
/// When `off` is exactly on a word boundary (and not at the run's start), returns the word to
/// the *left* of the boundary.  When `off` is inside a word, returns that word.
pub fn thai_word_at(text: &str, off: usize) -> (usize, usize) {
    let (rs, re, off) = thai_run_at(text, off);
    let run = &text[rs..re];
    let rel = off.saturating_sub(rs);
    let bounds: Vec<usize> = word_segmenter().segment_str(run).collect();
    // An exact boundary hit (other than the very start of the run) selects the word to the left.
    if rel > 0
        && let Ok(i) = bounds.binary_search(&rel)
    {
        let a = if i > 0 { bounds[i - 1] } else { 0 };
        return (rs + a, rs + rel);
    }
    let a = bounds.iter().copied().rev().find(|&b| b <= rel).unwrap_or(0);
    let b = bounds.iter().copied().find(|&b| b > rel).unwrap_or(run.len());
    (rs + a, rs + b)
}

pub fn thai_lao_breaks(text: &str) -> Vec<usize> {
    line_segmenter()
        .segment_str(text)
        .filter(|&i| {
            i < text.len()
                && text.get(..i).and_then(|s| s.chars().next_back()).is_some_and(is_thai_lao)
                && text.get(i..).and_then(|s| s.chars().next()).is_some_and(is_thai_lao)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "မြန်မာစာ" (Myanmar script): မြ | န် | မာ | စာ.
    const MYANMAR: &str = "\u{1019}\u{103C}\u{1014}\u{103A}\u{1019}\u{102C}\u{1005}\u{102C}";

    fn split<'a>(s: &'a str, at: &[usize]) -> Vec<&'a str> {
        let mut v = Vec::new();
        for (n, &a) in at.iter().enumerate() {
            let b = at.get(n + 1).copied().unwrap_or(s.len());
            v.push(&s[a..b]);
        }
        v
    }

    #[test]
    fn myanmar_vowel_signs_stay_in_their_syllable() {
        // Graphemes alone would cut "မာ" before the aa sign U+102C.
        assert_eq!(split(MYANMAR, &cluster_starts(MYANMAR)), vec!["မြ", "န်", "မာ", "စာ"]);
        // The prevowel ေ (U+1031) follows its consonant in memory: one cluster, as is visarga း.
        assert_eq!(split("ကြေး", &cluster_starts("ကြေး")), vec!["ကြေး"]);
        // Stacked consonants (virama U+1039) and kinzi join the syllable they hang from.
        let stacked = "\u{1019}\u{1039}\u{1018}\u{102C}"; // မ္ဘာ
        assert_eq!(cluster_starts(stacked), vec![0]);
        let kinzi = "\u{1021}\u{1004}\u{103A}\u{1039}\u{1002}\u{101C}\u{102D}\u{1015}\u{103A}"; // အင်္ဂလိပ်
        assert_eq!(split(kinzi, &cluster_starts(kinzi)), vec!["အ", "င်္ဂ", "လိ", "ပ်"]);
    }

    #[test]
    fn khmer_subscripts_and_indic_conjuncts_are_one_cluster() {
        // ខ្មែរ: ខ + coeng + ម + ែ, then រ.
        assert_eq!(split("ខ្មែរ", &cluster_starts("ខ្មែរ")), vec!["ខ្មែ", "រ"]);
        // Devanagari conjuncts are grapheme clusters already.
        assert_eq!(split("क्षि हि", &cluster_starts("क्षि हि")), vec!["क्षि", " ", "हि"]);
        // Latin is untouched: one cluster per letter, combining accents included.
        assert_eq!(split("e\u{301}a", &cluster_starts("e\u{301}a")), vec!["e\u{301}", "a"]);
    }

    #[test]
    fn caret_steps_skip_over_whole_clusters() {
        let s = MYANMAR;
        assert_eq!(next_cluster(s, 0), 6);
        assert_eq!(next_cluster(s, 12), 18, "past the aa sign, not before it");
        assert_eq!(prev_cluster(s, 18), 12);
        assert_eq!(prev_cluster(s, 0), 0);
        assert_eq!(next_cluster(s, s.len()), s.len());
        assert!(!is_cluster_boundary(s, 15) && is_cluster_boundary(s, 12));
        // Hostile offsets: past the end, inside a character.
        assert_eq!(next_cluster(s, 1_000), s.len());
        assert!(!is_cluster_boundary(s, 1));
        assert_eq!(next_cluster(s, 1), 6);
        assert_eq!(prev_cluster(s, 13), 6);
        assert_eq!(prev_cluster("", 5), 0);
        assert_eq!(next_cluster("", 0), 0);
    }

    #[test]
    fn myanmar_wraps_between_syllables() {
        // မြန်မာစာ: not before န (န် is a final, killed by the asat), so before မာ and စာ.
        assert_eq!(myanmar_breaks(MYANMAR), vec![12, 18]);
        // Never before a stacked consonant or one followed by medial + asat.
        assert!(myanmar_breaks("\u{1019}\u{1039}\u{1018}\u{102C}").is_empty());
        assert!(myanmar_breaks("ကျွန်").is_empty(), "န် is the final of ကျွန်");
        // Not between Latin and Myanmar (UAX #14 handles that) or inside Latin.
        assert!(myanmar_breaks("abcက").is_empty());
        assert!(myanmar_breaks("").is_empty());
    }

    #[test]
    fn backspace_by_character_in_brahmic_scripts_only() {
        assert!(backspaces_by_char('ေ') && backspaces_by_char('ि') && backspaces_by_char('្'));
        assert!(!backspaces_by_char('\u{301}') && !backspaces_by_char('a') && !backspaces_by_char('語'));
    }

    #[test]
    fn thai_combining_marks_are_one_caret_stop() {
        // กี้ = ก (0E01) + ◌ี sara ii (0E35, Mn/Extend) + ◌้ mai ek (0E48, Mn/Extend).
        // UAX #29 grapheme clusters group all three into one unit; next_cluster must skip over
        // both combining marks in a single step.
        let s = "\u{0E01}\u{0E35}\u{0E48}"; // กี้
        assert_eq!(next_cluster(s, 0), s.len(), "must jump over base + 2 combining marks");
        assert_eq!(prev_cluster(s, s.len()), 0, "must jump back over all three");

        // Verify is_thai_combining covers exactly the above/below marks, not base consonants.
        assert!(is_thai_combining('\u{0E35}')); // sara ii (above)
        assert!(is_thai_combining('\u{0E38}')); // sara u (below)
        assert!(is_thai_combining('\u{0E48}')); // mai ek tone mark
        assert!(!is_thai_combining('\u{0E01}')); // ก consonant — NOT combining
        assert!(!is_thai_combining('a'));

        // A string with no combining marks: one stop per character.
        let abc = "\u{0E01}\u{0E02}\u{0E03}"; // กขค
        assert_eq!(next_cluster(abc, 0), 3);
        assert_eq!(next_cluster(abc, 3), 6);
        assert_eq!(next_cluster(abc, 6), 9);
    }

    #[test]
    fn thai_word_movement_uses_icu4x_segmenter() {
        // "อนุรักษ์ภาษาไทย" = อนุรักษ์ (24 B) + ภาษา (12 B) + ไทย (9 B) = 45 bytes, three words.
        let s = "อนุรักษ์ภาษาไทย";
        let len = s.len(); // 45
        assert_eq!(len, 45);

        // word_end from offset 0 must point to the end of "อนุรักษ์" (byte 24).
        let end0 = thai_word_end(s, 0);
        assert_eq!(end0, 24, "first word end: {end0}");

        // word_start from the boundary between word 1 and word 2 returns the start of word 1.
        assert_eq!(thai_word_start(s, 24), 0, "start of word at boundary 24");

        // word_start from inside word 2 (byte 28) returns the start of word 2 (byte 24).
        assert_eq!(thai_word_start(s, 28), 24, "start of ภาษา");

        // word_end includes a trailing space (matching generic word_end behaviour).
        // "วัฒนธรรม ไทย": วัฒนธรรม (24 B) + space (1 B) + ไทย (9 B) = 34 B.
        let spaced = "วัฒนธรรม ไทย";
        assert_eq!(thai_word_end(spaced, 0), 25, "word_end includes trailing space after วัฒนธรรม");

        // word_end chained must traverse all three words.
        let mut off = 0;
        let mut steps = 0usize;
        while off < len {
            let next = thai_word_end(s, off);
            assert!(next > off, "thai_word_end must advance (off={off})");
            off = next;
            steps += 1;
            assert!(steps <= len, "infinite loop guard");
        }
        assert_eq!(steps, 3, "expected 3 words, got {steps}");

        // word_at: click inside word 2 (byte 28) → (24, 36).
        assert_eq!(thai_word_at(s, 28), (24, 36), "click inside ภาษา");

        // word_at: click at the boundary between word 1 and word 2 (byte 24) → word 1 (0, 24),
        // not word 2.  A boundary click lands the caret at the end of the preceding word.
        assert_eq!(thai_word_at(s, 24), (0, 24), "boundary click at 24 selects อนุรักษ์, not ภาษา");

        // word_at: click at offset 0 → first word.
        let (a0, b0) = thai_word_at(s, 0);
        assert_eq!(a0, 0);
        assert!(b0 > 0 && b0 <= len);

        // word_at: click at end of text → last word.
        assert_eq!(thai_word_at(s, len), (36, 45), "click at end → ไทย");

        // Mixed Thai/Latin: "ไทยtest" — cursor at the 't' (byte 9, first byte of "test") must
        // NOT be routed to thai_word_at; it is handled by the Latin path in Paragraph::word_at.
        // thai_word_at itself still returns the Thai word when given that offset (it sees only
        // the Thai side), so the guard lives in word_at.  Here we just verify thai_word_at's own
        // contract: byte 0 → Thai word (0, 9).
        let mixed = "ไทยtest";
        assert_eq!(thai_word_at(mixed, 0), (0, 9), "thai_word_at: click inside Thai part");
        // byte 9 is inside "test" (Latin); thai_word_at's boundary logic returns the Thai word
        // to its left — confirming the guard in word_at must check at_off first.
        assert_eq!(thai_word_at(mixed, 9), (0, 9), "thai_word_at at script boundary returns Thai word");
    }

    #[test]
    fn thai_wraps_at_word_boundaries() {
        // "ภาคภูมิใจในภาษาไทย" = ภาค (9 B) + ภูมิใจ (18 B) + ใน (6 B) + ภาษาไทย (12 B) + ไทย (9 B) = 54 B.
        // ICU4X dictionary breaks: [9, 27, 33, 45] (before ภูมิ, ใน, ภาษา, ไทย).
        let thai = "ภาคภูมิใจในภาษาไทย";
        assert_eq!(thai.len(), 54);
        let breaks = thai_lao_breaks(thai);
        assert_eq!(breaks, vec![9, 27, 33, 45], "expected breaks at word boundaries: {breaks:?}");
        // Every break must fall on a char boundary.
        for &b in &breaks {
            assert!(thai.is_char_boundary(b), "break at {b} is not a char boundary");
        }
        // No break at position 0 or at the end.
        assert!(!breaks.contains(&0));
        assert!(!breaks.contains(&thai.len()));
        // Breaks must not fall inside a Thai character (sanity: all offsets map to is_thai_lao chars).
        for &b in &breaks {
            let prev = thai.get(..b).and_then(|s| s.chars().next_back()).unwrap();
            let next = thai.get(b..).and_then(|s| s.chars().next()).unwrap();
            assert!(is_thai_lao(prev) && is_thai_lao(next));
        }
        // Latin text: no breaks injected (UAX #14 handles Latin; this function is Thai/Lao only).
        assert!(thai_lao_breaks("Hello world").is_empty());
        // Empty input is fine.
        assert!(thai_lao_breaks("").is_empty());
        // Mixed: breaks are only between Thai characters, not at the Thai–Latin boundary.
        let mixed = "testภาษาไทยtest";
        for &b in &thai_lao_breaks(mixed) {
            let prev = mixed.get(..b).and_then(|s| s.chars().next_back()).unwrap();
            let next = mixed.get(b..).and_then(|s| s.chars().next()).unwrap();
            assert!(is_thai_lao(prev) && is_thai_lao(next), "break at {b} crosses script boundary");
        }
    }

    #[test]
    fn lao_wraps_at_word_boundaries() {
        // "ໂລກໃໝ່" = ໂລກ (world, 9 B) + ໃໝ່ (new, 9 B); ICU4X break at [9].
        let lao = "ໂລກໃໝ່";
        assert_eq!(lao.len(), 18);
        let breaks = thai_lao_breaks(lao);
        assert_eq!(breaks, vec![9], "expected one Lao word break: {breaks:?}");
        for &b in &breaks {
            assert!(lao.is_char_boundary(b));
            let prev = lao.get(..b).and_then(|s| s.chars().next_back()).unwrap();
            let next = lao.get(b..).and_then(|s| s.chars().next()).unwrap();
            assert!(is_thai_lao(prev) && is_thai_lao(next));
        }
        assert!(thai_lao_breaks("").is_empty());
    }
}

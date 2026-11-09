//! Text clusters for complex scripts (Myanmar, Khmer, Tai Tham, the Indic scripts): where the
//! caret may stop, what Backspace removes, and where Myanmar text may wrap.
//!
//! Extended grapheme clusters (UAX #29) split some syllables that a shaper draws as one unit: a
//! Myanmar consonant stacked under a virama (`မ္ဘ`), the Myanmar vowel signs U+102B/U+102C and
//! the visarga U+1038 (excluded from the grapheme `SpacingMark` class), Khmer subscript
//! consonants after the coeng. A caret between those would sit inside a glyph cluster, so the
//! caret steps and layout clusters both use [`joins_cluster`] on top of the grapheme rules.

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
    // ASCII: every character starts a grapheme except LF after CR, and none joins (UAX #29).
    if text.is_ascii() {
        let b = text.as_bytes();
        return (0..b.len()).filter(|&i| !(i > 0 && b.get(i - 1) == Some(&b'\r') && b.get(i) == Some(&b'\n'))).collect();
    }
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

/// Characters Backspace removes individually rather than with their whole cluster: Arabic
/// combining marks and characters of the Brahmic scripts of South and South-East Asia.
pub fn backspaces_by_char(c: char) -> bool {
    matches!(u32::from(c),
        0x0610..=0x061A // Arabic nonspacing marks (Unicode 17.0)
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06DC
        | 0x06DF..=0x06E4
        | 0x06E7..=0x06E8
        | 0x06EA..=0x06ED
        | 0x0897..=0x089F
        | 0x08CA..=0x08E1
        | 0x08E3..=0x08FF
        | 0x10EFA..=0x10EFF
        | 0x0900..=0x0DFF // Devanagari … Sinhala
        | 0x0E00..=0x0EFF // Thai, Lao
        | 0x0F00..=0x0FFF // Tibetan
        | 0x1000..=0x109F // Myanmar
        | 0x1780..=0x17FF // Khmer
        | 0x1A20..=0x1AAF // Tai Tham
        | 0x1B00..=0x1B7F // Balinese
        | 0xA980..=0xA9FF // Javanese, Myanmar Extended-B
        | 0xAA60..=0xAA7F) // Myanmar Extended-A
}

/// Thai or Lao character (U+0E00–U+0E7F / U+0E80–U+0EFF). libthai only handles Thai; Lao is
/// excluded from the libthai path but grouped here because they share the UAX #14 SA class.
pub fn is_thai_lao(c: char) -> bool {
    matches!(u32::from(c), 0x0E00..=0x0EFF)
}

/// Singleton Thai word-breaker (built once per process).
fn thai_brk() -> &'static libthai::wbrk::ThWBrk {
    static BRK: std::sync::OnceLock<libthai::wbrk::ThWBrk> = std::sync::OnceLock::new();
    BRK.get_or_init(libthai::wbrk::ThWBrk::new)
}

/// The contiguous Thai run that contains byte offset `off` in `text`.
/// Returns `(run_start, run_end, offset_within_run)` where the run contains only Thai chars
/// (U+0E00–U+0E7F, the range libthai handles). Clamps and snaps to char boundaries safely.
fn thai_run_at(text: &str, off: usize) -> (usize, usize, usize) {
    // Snap to a char boundary (snap backward like char_floor).
    let mut off = off.min(text.len());
    while off > 0 && !text.is_char_boundary(off) {
        off -= 1;
    }
    // Walk left to find run start.
    let mut start = off;
    for (i, c) in text.get(..off).unwrap_or("").char_indices().rev() {
        if matches!(u32::from(c), 0x0E00..=0x0E7F) {
            start = i;
        } else {
            break;
        }
    }
    // Walk right to find run end.
    let mut end = off;
    for (i, c) in text.get(off..).unwrap_or("").char_indices() {
        if matches!(u32::from(c), 0x0E00..=0x0E7F) {
            end = off + i + c.len_utf8();
        } else {
            break;
        }
    }
    (start, end, off - start)
}

/// Ctrl+Left in Thai text: start of the word at or before `off`.
pub fn thai_word_start(text: &str, off: usize) -> usize {
    let (rs, re, rel) = thai_run_at(text, off);
    let run = match text.get(rs..re) {
        Some(r) => r,
        None => return off,
    };
    let mut breaks = thai_brk().find_breaks(run);
    // Synthesise start (0) and end boundaries so the whole run is covered.
    if breaks.first().copied() != Some(0) {
        breaks.insert(0, 0);
    }
    if breaks.last().copied() != Some(run.len()) {
        breaks.push(run.len());
    }
    // Find the word boundary strictly to the left of rel (i.e., the word-start).
    let idx = breaks.partition_point(|&b| b < rel);
    let b = breaks.get(idx.saturating_sub(1)).copied().unwrap_or(0);
    rs + b
}

/// Ctrl+Right in Thai text: end of the word after `off`, including trailing ASCII spaces.
pub fn thai_word_end(text: &str, off: usize) -> usize {
    let (rs, re, rel) = thai_run_at(text, off);
    let run = match text.get(rs..re) {
        Some(r) => r,
        None => return off,
    };
    let mut breaks = thai_brk().find_breaks(run);
    if breaks.last().copied() != Some(run.len()) {
        breaks.push(run.len());
    }
    // First boundary strictly after rel.
    let b = breaks.iter().copied().find(|&b| b > rel).unwrap_or(run.len());
    let word_end = rs + b;
    // Include trailing ASCII spaces, as Word does.
    let mut end = word_end;
    for (i, c) in text.get(word_end..).unwrap_or("").char_indices() {
        if c == ' ' {
            end = word_end + i + 1;
        } else {
            break;
        }
    }
    end
}

/// Double-click in Thai text: `(start, end)` of the word around `off` (no trailing space).
pub fn thai_word_at(text: &str, off: usize) -> (usize, usize) {
    let (rs, re, rel) = thai_run_at(text, off);
    let run = match text.get(rs..re) {
        Some(r) => r,
        None => return (off, off),
    };
    let mut breaks = thai_brk().find_breaks(run);
    // Synthesise start + end so every position has a containing word.
    if breaks.first().copied() != Some(0) {
        breaks.insert(0, 0);
    }
    if breaks.last().copied() != Some(run.len()) {
        breaks.push(run.len());
    }
    // Find the word (a..b) that contains rel: a is the last boundary <= rel, b is the
    // first boundary > rel. If rel is exactly a boundary, select the word to its left
    // (unless rel is 0, in which case select the word to its right).
    let idx = breaks.partition_point(|&b| b <= rel);
    let (a, b) = if rel > 0 && breaks.get(idx.saturating_sub(1)).copied() == Some(rel) {
        // Exactly on a boundary — select word to the left.
        let a = breaks.get(idx.saturating_sub(2)).copied().unwrap_or(0);
        (a, rel)
    } else {
        // Inside a word.
        let a = breaks.get(idx.saturating_sub(1)).copied().unwrap_or(0);
        let b = breaks.get(idx).copied().unwrap_or(run.len());
        (a, b)
    };
    (rs + a, rs + b)
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

    /// The ASCII shortcut gives the grapheme rules' answer.
    #[test]
    fn ascii_cluster_starts_match_the_graphemes() {
        let graphemes = |t: &str| -> Vec<usize> { t.grapheme_indices(true).map(|(i, _)| i).collect() };
        let all: String = (0u8..128).map(char::from).collect();
        for t in ["", "a", "Hello, world.", "\r\n", "a\r\nb\n\rc\r\r\n\n", "\t \u{7f}x\u{0}", all.as_str()] {
            assert_eq!(cluster_starts(t), graphemes(t), "{t:?}");
        }
        for a in 0u8..128 {
            for b in 0u8..128 {
                let t = format!("{}{}", char::from(a), char::from(b));
                assert_eq!(cluster_starts(&t), graphemes(&t), "{t:?}");
            }
        }
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
    fn backspace_by_character_in_brahmic_scripts_and_arabic_marks() {
        assert!(backspaces_by_char('ေ') && backspaces_by_char('ि') && backspaces_by_char('្'));
        assert!(backspaces_by_char('\u{064E}') && backspaces_by_char('\u{06E7}') && backspaces_by_char('\u{10EFF}'));
        assert!(!backspaces_by_char('\u{301}') && !backspaces_by_char('a') && !backspaces_by_char('語'));
        assert!(!backspaces_by_char('ب') && !backspaces_by_char('\u{06E5}') && !backspaces_by_char('\u{08E2}'));
    }

    /// "สวัสดีครับ" (Sawasdee khrab — Thai greeting): สวัสดี | ครับ.
    const THAI_HELLO: &str = "\u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}\u{0E04}\u{0E23}\u{0E31}\u{0E1A}";

    #[test]
    fn thai_word_caret_movement() {
        // สวัสดี = bytes 0..18, ครับ = bytes 18..30 (each Thai char is 3 UTF-8 bytes).
        let s = THAI_HELLO;
        // Ctrl+Left from end of "ครับ" → start of "ครับ".
        assert_eq!(thai_word_start(s, s.len()), 18, "word_start from end");
        // Ctrl+Left from start of "ครับ" → start of "สวัสดี".
        assert_eq!(thai_word_start(s, 18), 0, "word_start from mid boundary");
        // Ctrl+Right from start → end of "สวัสดี" (= 18).
        assert_eq!(thai_word_end(s, 0), 18, "word_end from start");
        // Ctrl+Right from 18 → end of string.
        assert_eq!(thai_word_end(s, 18), s.len(), "word_end from second word");
        // Double-click in "สวัสดี" → (0, 18).
        assert_eq!(thai_word_at(s, 6), (0, 18), "word_at inside first word");
        // Double-click in "ครับ" → (18, 30).
        assert_eq!(thai_word_at(s, 21), (18, s.len()), "word_at inside second word");
    }

    /// Forward and backward word navigation must visit the same set of boundaries (symmetric).
    #[test]
    fn thai_word_skip_symmetry() {
        let s = THAI_HELLO; // สวัสดี | ครับ — 2 words, boundary at 18.

        // Collect forward positions: 0 →(end)→ 18 →(end)→ 30.
        let mut fwd = vec![0usize];
        loop {
            let next = thai_word_end(s, *fwd.last().unwrap());
            if next == *fwd.last().unwrap() || next > s.len() {
                break;
            }
            fwd.push(next);
            if next >= s.len() {
                break;
            }
        }

        // Collect backward positions: 30 →(start)→ 18 →(start)→ 0.
        let mut bwd = vec![s.len()];
        loop {
            let prev = thai_word_start(s, *bwd.last().unwrap());
            if prev == *bwd.last().unwrap() {
                break;
            }
            bwd.push(prev);
            if prev == 0 {
                break;
            }
        }
        bwd.reverse();

        assert_eq!(fwd, bwd, "forward and backward word skip must visit the same boundaries");
        // Expected: [0, 18, 30].
        assert_eq!(fwd, vec![0, 18, 30]);
    }

    /// word_at at a word boundary should select the word to its left (not an empty range).
    #[test]
    fn thai_word_at_boundary_selects_left_word() {
        let s = THAI_HELLO;
        // At boundary 18: should select "สวัสดี" = (0, 18), not (18, 18).
        let (a, b) = thai_word_at(s, 18);
        assert!(b > a, "word_at(18) must not return an empty range, got ({a}, {b})");
        assert_eq!((a, b), (0, 18), "word_at(boundary) should select the word to the left");
        // At pos 0: select "สวัสดี" to the right.
        let (a, b) = thai_word_at(s, 0);
        assert!(b > a, "word_at(0) must not return an empty range");
        assert_eq!((a, b), (0, 18));
    }

    #[test]
    fn thai_word_caret_hostile_offsets() {
        let s = THAI_HELLO;
        // Off past end: clamp.
        assert!(thai_word_start(s, 1_000) <= s.len());
        assert!(thai_word_end(s, 1_000) <= s.len());
        // Mid-char offsets: must stay at or before a char boundary.
        for off in 0..s.len() {
            let ws = thai_word_start(s, off);
            let we = thai_word_end(s, off);
            assert!(s.is_char_boundary(ws), "word_start({off}) = {ws} not on char boundary");
            assert!(s.is_char_boundary(we), "word_end({off}) = {we} not on char boundary");
            let (wa, wb) = thai_word_at(s, off);
            assert!(s.is_char_boundary(wa) && s.is_char_boundary(wb), "word_at({off}) not on boundaries");
        }
    }
}

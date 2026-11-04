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

    /// CJK caret units follow UAX #29 extended graphemes: one step per ideograph or kana.
    /// (`joins_cluster` does not apply to Han/kana; see module docs.)
    #[test]
    fn japanese_and_chinese_text_steps_one_character_at_a_time() {
        // 「春はあけぼの。」— representative classical Japanese (Pillow Book opening).
        let ja = "春はあけぼの。";
        let starts = cluster_starts(ja);
        assert_eq!(split(ja, &starts), vec!["春", "は", "あ", "け", "ぼ", "の", "。"]);
        let mut off = 0;
        for expected in ["春", "は", "あ", "け", "ぼ", "の", "。"] {
            let next = next_cluster(ja, off);
            assert_eq!(&ja[off..next], expected);
            assert!(is_cluster_boundary(ja, off));
            off = next;
        }
        assert_eq!(off, ja.len());
        assert_eq!(prev_cluster(ja, ja.len()), ja.char_indices().last().unwrap().0);

        // 「学而时习之」— short Classical Chinese phrase.
        let zh = "学而时习之";
        assert_eq!(split(zh, &cluster_starts(zh)), vec!["学", "而", "时", "习", "之"]);
        assert!(!backspaces_by_char('春') && !backspaces_by_char('学') && !backspaces_by_char('。'));
    }

    /// Mixed Latin and CJK: each scalar (or Latin grapheme) is its own caret stop.
    #[test]
    fn mixed_latin_and_cjk_keep_grapheme_boundaries() {
        let t = "Hello、世界";
        assert_eq!(split(t, &cluster_starts(t)), vec!["H", "e", "l", "l", "o", "、", "世", "界"]);
        // From the end of "Hello", left should land before "、", then before "o".
        let after_hello = "Hello".len();
        assert_eq!(prev_cluster(t, after_hello + "、".len()), after_hello);
        assert_eq!(prev_cluster(t, after_hello), after_hello - 1);
    }

    /// Supplementary Ideographic Plane (U+20000..) is one Rust char and one caret cluster.
    #[test]
    fn supplementary_plane_ideograph_is_one_cluster() {
        // U+20000 (𠀀), first character of Extension B.
        let sip = "\u{20000}";
        assert_eq!(sip.len(), 4);
        assert_eq!(cluster_starts(sip), vec![0]);
        assert_eq!(next_cluster(sip, 0), 4);
        assert_eq!(prev_cluster(sip, 4), 0);
        let mixed = format!("漢{sip}字");
        assert_eq!(split(&mixed, &cluster_starts(&mixed)), vec!["漢", sip, "字"]);
    }

    /// Ideographic Variation Sequences: base + VS (U+E0100..) is one UAX #29 grapheme.
    /// This pins cluster boundaries only; variation-glyph selection is not verified (see docs/cjk-typography-audit.md).
    #[test]
    fn ideographic_variation_sequence_is_one_grapheme_cluster() {
        // U+8FBA (辺) + VS17 (U+E0100) — a common IVS example shape.
        let ivs = "\u{8FBA}\u{E0100}";
        assert_eq!(cluster_starts(ivs), vec![0]);
        assert_eq!(next_cluster(ivs, 0), ivs.len());
        assert!(!is_cluster_boundary(ivs, '\u{8FBA}'.len_utf8()));
        let with_neighbors = format!("東{ivs}西");
        assert_eq!(split(&with_neighbors, &cluster_starts(&with_neighbors)), vec!["東", ivs, "西"]);
    }

    /// Caret units (left/right) for sequences where CJK differs from Latin. Expected values come
    /// from UAX #29 (Extend, ZWJ and Hangul L/V/T rules), not from what the code did. Backspace and
    /// Delete are deliberately not asserted here: Word's behaviour for these needs observing.
    fn assert_caret_units(text: &str, expected: &[&str]) {
        assert_eq!(split(text, &cluster_starts(text)), expected, "cluster_starts of {text:?}");
        let mut off = 0;
        for unit in expected {
            let next = next_cluster(text, off);
            assert_eq!(text.get(off..next), Some(*unit), "next_cluster from {off} in {text:?}");
            assert_eq!(prev_cluster(text, next), off, "prev_cluster from {next} in {text:?}");
            off = next;
        }
        assert_eq!(off, text.len());
    }

    /// か + U+3099 (combining voiced sound mark) is one unit, with kana on either side.
    #[test]
    fn kana_with_combining_voiced_mark_is_one_cluster() {
        let ka = "か\u{3099}";
        assert_caret_units(&format!("あ{ka}い"), &["あ", ka, "い"]);
    }

    /// Halfwidth katakana plus its separate sound mark (ﾊ + U+FF9E) is one unit.
    #[test]
    fn halfwidth_katakana_with_sound_mark_is_one_cluster() {
        let ba = "\u{FF8A}\u{FF9E}";
        assert_caret_units(&format!("\u{FF76}{ba}\u{FF72}"), &["\u{FF76}", ba, "\u{FF72}"]);
    }

    /// An emoji ZWJ sequence next to kana is one unit and does not absorb its neighbours.
    #[test]
    fn emoji_zwj_sequence_next_to_kana_is_one_cluster() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_caret_units(&format!("あ{family}い"), &["あ", family, "い"]);
    }

    /// Decomposed Hangul syllables (conjoining jamo L+V and L+V+T) are one unit each.
    #[test]
    fn decomposed_hangul_syllables_are_one_cluster() {
        let han = "\u{1112}\u{1161}\u{11AB}"; // ᄒ ᅡ ᆫ → 한
        let geul = "\u{1100}\u{1173}"; // ᄀ ᅳ → 그 (no final)
        assert_caret_units(&format!("{han}{geul}"), &[han, geul]);
    }
}

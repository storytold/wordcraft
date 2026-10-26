//! Scripts: itemising text into single-script runs for the shaper, and the fonts preferred when
//! a document's font lacks a complex script (Myanmar, Khmer, the Indic scripts, Thai…).
//!
//! A shaper picks its script-specific engine (the Myanmar, Khmer, Indic or Universal Shaping
//! Engine reordering, e.g. U+1031 ေ drawn before its consonant) from the run's script, so text
//! mixing scripts in one face must be shaped one script at a time.

#![allow(clippy::single_range_in_vec_init)] // one run is a list of runs

use unicode_script::{Script, UnicodeScript};

/// The script of `c` for itemisation: `None` for characters that take the script of the text
/// around them (spaces, punctuation, digits, combining marks, joiners).
fn real_script(c: char) -> Option<Script> {
    match c.script() {
        Script::Common | Script::Inherited | Script::Unknown => None,
        // Japanese and Chinese mix these within a word: one run.
        Script::Hiragana | Script::Katakana | Script::Bopomofo => Some(Script::Han),
        s => Some(s),
    }
}

/// Maximal single-script byte ranges of `text`. Characters of no particular script join the run
/// before them (or, at the start, the first run), so the ranges cover `text` exactly.
pub(crate) fn script_runs(text: &str) -> Vec<std::ops::Range<usize>> {
    if text.is_ascii() {
        return vec![0..text.len()];
    }
    let mut runs = Vec::new();
    let mut cur: Option<Script> = None;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let Some(s) = real_script(c) else { continue };
        match cur {
            Some(p) if p != s => {
                runs.push(start..i);
                start = i;
                cur = Some(s);
            }
            None => cur = Some(s),
            _ => {}
        }
    }
    runs.push(start..text.len());
    runs
}

/// Scripts whose shaping reorders, stacks or forms clusters (OpenType script-specific engines),
/// so a fallback font must carry OpenType layout tables to draw them properly.
pub fn needs_layout_tables(c: char) -> bool {
    matches!(
        c.script(),
        Script::Myanmar
            | Script::Khmer
            | Script::Devanagari
            | Script::Bengali
            | Script::Gurmukhi
            | Script::Gujarati
            | Script::Oriya
            | Script::Tamil
            | Script::Telugu
            | Script::Kannada
            | Script::Malayalam
            | Script::Sinhala
            | Script::Thai
            | Script::Lao
            | Script::Tibetan
            | Script::Tai_Tham
            | Script::Balinese
            | Script::Javanese
            | Script::Arabic
            | Script::Syriac
    )
}

/// Families preferred, in order, for a character of `c`'s script when the document's font lacks
/// it: the platforms' own OpenType fonts and the open Noto/SIL families. Apple's older fonts for
/// these scripts carry only AAT tables (which the shaper doesn't read), so they come last.
pub fn script_fallbacks(c: char) -> &'static [&'static str] {
    if is_emoji(c) {
        return COLOR_EMOJI;
    }
    match c.script() {
        Script::Myanmar => &["Myanmar Text", "Noto Sans Myanmar", "Padauk", "Pyidaungsu", "Noto Serif Myanmar", "Myanmar Sangam MN", "Myanmar MN"],
        Script::Khmer => &["Leelawadee UI", "Khmer UI", "Noto Sans Khmer", "Noto Serif Khmer", "Khmer OS", "Khmer Sangam MN", "Khmer MN"],
        Script::Devanagari => &["Nirmala UI", "Mangal", "Kohinoor Devanagari", "Noto Sans Devanagari", "Noto Serif Devanagari", "Lohit Devanagari"],
        Script::Bengali => &["Nirmala UI", "Vrinda", "Kohinoor Bangla", "Noto Sans Bengali", "Noto Serif Bengali", "Lohit Bengali"],
        Script::Thai => &["Leelawadee UI", "Leelawadee", "Tahoma", "Thonburi", "Noto Sans Thai", "Noto Serif Thai"],
        Script::Lao => &["Leelawadee UI", "Lao UI", "Noto Sans Lao", "Noto Serif Lao", "Lao Sangam MN"],
        Script::Tamil | Script::Telugu | Script::Kannada | Script::Malayalam | Script::Gujarati | Script::Gurmukhi | Script::Oriya => &["Nirmala UI"],
        _ => &[],
    }
}

/// The platforms' colour emoji fonts. Emoji prefer them over the monochrome symbol fonts that
/// also cover many emoji (Windows' Segoe UI Symbol has ✅, so it drew in black and white).
pub const COLOR_EMOJI: &[&str] = &["Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji"];

/// Characters a shaper draws no glyph for of its own: variation selectors, the joiners, the tags
/// and the other default-ignorables. A face that lacks one is not missing a glyph, so they never
/// choose a face and never disqualify one ([`FontFace::covers`](crate::FontFace::covers)).
pub fn is_default_ignorable(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00AD // soft hyphen
            | 0x034F // combining grapheme joiner
            | 0x061C // Arabic letter mark
            | 0x115F..=0x1160 // Hangul fillers
            | 0x17B4..=0x17B5 // Khmer inherent vowels
            | 0x180B..=0x180F // Mongolian free variation selectors, vowel separator
            | 0x200B..=0x200F // zero-width space, joiners, direction marks
            | 0x202A..=0x202E // bidi embedding and override controls
            | 0x2060..=0x206F // word joiner, invisible operators, bidi isolates
            | 0x3164 // Hangul filler
            | 0xFE00..=0xFE0F // variation selectors 1–16
            | 0xFEFF // zero-width no-break space
            | 0xFFA0 // halfwidth Hangul filler
            | 0x1BCA0..=0x1BCA3 // shorthand format controls
            | 0x1D173..=0x1D17A // musical format controls
            | 0xE0000..=0xE0FFF // tags and variation selectors supplement
    )
}

/// Characters drawn as emoji by default (Unicode `Emoji_Presentation`): the BMP ones listed in
/// emoji-data.txt, and the pictograph blocks from U+1F000.
// ponytail: all of U+1F000..U+1FAFF counts as emoji (a few text-default pictographs too); switch to the exact emoji-data ranges if one must draw monochrome
pub fn is_emoji(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1F000..=0x1FAFF
            | 0x231A..=0x231B
            | 0x23E9..=0x23EC
            | 0x23F0
            | 0x23F3
            | 0x25FD..=0x25FE
            | 0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267F
            | 0x2693
            | 0x26A1
            | 0x26AA..=0x26AB
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26CE
            | 0x26D4
            | 0x26EA
            | 0x26F2..=0x26F3
            | 0x26F5
            | 0x26FA
            | 0x26FD
            | 0x2705
            | 0x270A..=0x270B
            | 0x2728
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2795..=0x2797
            | 0x27B0
            | 0x27BF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
    )
}

/// Whether these characters, one grapheme cluster, are an emoji **presentation sequence**: the
/// text asks for an emoji drawing of characters that are not emoji by themselves — a variation
/// selector-16 (`⚖️`), a keycap (`1️⃣`), a ZWJ sequence, a flag or a skin-toned emoji. Such a
/// cluster is shaped as one emoji in a colour emoji font, whose ligature draws the whole thing;
/// the text font would draw the base and leave the keycap or the joiner without a glyph.
///
/// `false` for text presentation (`❤︎`, U+FE0E) and for a lone emoji, which
/// [`is_emoji`] already sends to a colour emoji face character by character.
pub fn emoji_sequence(chars: &[char]) -> bool {
    let mut selectors = 0u8;
    let mut emoji = false;
    let mut regional = 0usize;
    for c in chars.iter().copied() {
        emoji |= is_emoji(c);
        match u32::from(c) {
            0xFE0F => selectors |= 1,             // VARIATION SELECTOR-16: emoji presentation
            0x20E3 => selectors |= 2,             // COMBINING ENCLOSING KEYCAP
            0x200D => selectors |= 4,             // ZERO WIDTH JOINER
            0x1F3FB..=0x1F3FF => selectors |= 8,  // emoji modifier (skin tone)
            0xE0020..=0xE007F => selectors |= 16, // tag characters (subdivision flags)
            0x1F1E6..=0x1F1FF => regional += 1,   // regional indicators (flags)
            _ => {}
        }
    }
    selectors & 1 != 0
        || selectors & 2 != 0
        || (selectors & 4 != 0 && emoji)
        || regional >= 2
        || (selectors & 8 != 0 && emoji)
        || (selectors & 16 != 0 && emoji)
}

/// How well `family` suits `c` as a fallback: 0 for the first preferred family, then the others
/// in order; then families named after the script ("Noto Sans Tamil"); then everything else.
pub fn fallback_rank(c: char, family: &str) -> usize {
    let list = script_fallbacks(c);
    if let Some(i) = list.iter().position(|n| family.eq_ignore_ascii_case(n)) {
        return i;
    }
    if needs_layout_tables(c) {
        let name = c.script().full_name().replace('_', " ");
        if family.to_ascii_lowercase().contains(&name.to_ascii_lowercase()) {
            return list.len();
        }
    }
    list.len() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_split_by_script_and_neutrals_join_their_neighbours() {
        assert_eq!(script_runs("Hello"), vec![0..5]);
        // "Hi မြန်မာ!" → Latin "Hi ", Myanmar "မြန်မာ!".
        let s = "Hi မြန်မာ!";
        let runs = script_runs(s);
        assert_eq!(runs, vec![0..3, 3..s.len()]);
        // A leading neutral joins the first run; marks never start a run.
        assert_eq!(script_runs("(ကေ)"), vec![0.."(ကေ)".len()]);
        // Japanese kana and kanji stay together.
        assert_eq!(script_runs("日本語のテキスト").len(), 1);
        assert_eq!(script_runs(""), vec![0..0]);
    }

    #[test]
    fn complex_scripts_prefer_their_own_fonts() {
        assert!(needs_layout_tables('မ') && needs_layout_tables('ក') && needs_layout_tables('क'));
        assert!(!needs_layout_tables('a') && !needs_layout_tables('語'));
        assert_eq!(fallback_rank('မ', "Myanmar Text"), 0);
        assert!(fallback_rank('မ', "Noto Sans Myanmar") < fallback_rank('မ', "Myanmar Sangam MN"));
        assert!(fallback_rank('த', "Noto Sans Tamil") < fallback_rank('த', "Arial"));
        assert_eq!(fallback_rank('a', "Arial"), fallback_rank('a', "Noto Sans"));
    }

    #[test]
    fn emoji_prefer_colour_emoji_fonts() {
        for c in ['✅', '🧠', '⭐', '⌚', '🫶'] {
            assert!(is_emoji(c), "{c}");
            assert!(fallback_rank(c, "Segoe UI Emoji") < fallback_rank(c, "Segoe UI Symbol"), "{c}");
            assert!(fallback_rank(c, "Noto Color Emoji") < fallback_rank(c, "Noto Emoji"), "{c}");
        }
        // Text-style symbols keep the symbol fonts.
        for c in ['✔', '☺', '→', 'a', '1'] {
            assert!(!is_emoji(c), "{c}");
        }
    }

    #[test]
    fn emoji_presentation_sequences_are_recognised() {
        let seq = |s: &str| emoji_sequence(&s.chars().collect::<Vec<_>>());
        // Keycaps (with and without the variation selector), emoji presentation of a text-default
        // symbol, a ZWJ sequence, a flag, a skin tone and a tag sequence.
        assert!(seq("1\u{FE0F}\u{20E3}"));
        assert!(seq("1\u{20E3}"));
        assert!(seq("\u{2696}\u{FE0F}"));
        assert!(seq("\u{1F469}\u{200D}\u{1F4BB}"));
        assert!(seq("\u{1F1E7}\u{1F1F7}"));
        assert!(seq("\u{1F44D}\u{1F3FD}"));
        assert!(seq("\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}"));
        // Text presentation, plain text and a lone joiner are not sequences.
        assert!(!seq("\u{2764}\u{FE0E}"));
        assert!(!seq("1"));
        assert!(!seq("Tema central"));
        assert!(!seq("\u{1F4D8}"));
        assert!(!seq("\u{200D}"));
    }

    #[test]
    fn default_ignorables_never_choose_a_face() {
        assert!(is_default_ignorable('\u{FE0F}') && is_default_ignorable('\u{FE0E}'));
        assert!(is_default_ignorable('\u{200D}'));
        // The enclosing keycap is drawn by the emoji's ligature: it does need a face.
        assert!(!is_default_ignorable('\u{20E3}'));
        assert!(!is_default_ignorable('1') && !is_default_ignorable('\u{1F4D8}'));
    }
}

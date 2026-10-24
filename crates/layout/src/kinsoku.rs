//! Asian typography line-breaking rules (the paragraph's `kinsoku` and `word_wrap` flags), applied
//! on top of the Unicode line-break opportunities (UAX #14).
//!
//! - Kinsoku on (the default): no line starts with closing East Asian punctuation (、。，）」…)
//!   and none ends with opening punctuation (（「【…). UAX #14 already forbids most of these
//!   breaks; the lists make sure of it for the standard Japanese/Chinese sets.
//! - Kinsoku off: a line may break between any two East Asian characters, punctuation included.
//! - Word wrap off: Latin words may break at any character.
//! - Automatic spacing (`autoSpaceDE`, `autoSpaceDN`): which neighbouring characters get room
//!   between Asian text and Latin text or digits ([`auto_space`]).
//!
//! Only East Asian characters are in the lists, and the extra breaks need letters on both sides,
//! so text with no East Asian characters breaks exactly as before unless word wrap is off.

use unicode_segmentation::UnicodeSegmentation;

/// East Asian characters a line may not start with: closing brackets and quotes, the
/// ideographic comma and full stop, fullwidth and halfwidth punctuation, the middle dot,
/// iteration and sound marks.
const NO_START: &str = "、。，．・：；？！゛゜ヽヾゝゞ々〻〉》」』】〕〗〙〛〞〟）］｝｠｡｣､･ﾞﾟ％￠";

/// East Asian characters a line may not end with: opening brackets and quotes, currency signs.
const NO_END: &str = "〈《「『【〔〖〘〚〝（［｛｟｢＄￡￥";

/// Whether `c` is East Asian text: CJK ideographs, kana, bopomofo, CJK punctuation, and
/// fullwidth/halfwidth forms (not spaces, not combining sound marks).
pub fn is_east_asian(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FFF          // radicals, ideographic description
        | 0x3001..=0x303F        // CJK symbols and punctuation (not the ideographic space)
        | 0x3040..=0x3098        // hiragana
        | 0x309B..=0x30FF        // sound marks, katakana
        | 0x3100..=0x312F        // bopomofo
        | 0x3190..=0x31FF        // kanbun, CJK strokes, katakana extensions
        | 0x3400..=0x4DBF        // CJK extension A
        | 0x4E00..=0x9FFF        // CJK unified ideographs
        | 0xF900..=0xFAFF        // compatibility ideographs
        | 0xFE30..=0xFE4F        // CJK compatibility forms
        | 0xFF01..=0xFFEF        // fullwidth and halfwidth forms
        | 0x20000..=0x3FFFF) // supplementary ideographs
}

/// Adjust the break opportunities `opps` (byte offsets into `text`: a line may start there) for
/// the paragraph's kinsoku and word-wrap settings. The result is sorted and has no duplicates.
///
/// `force_char_break` is set only for vertically stacked text (every character on its own line);
/// it extends the `!word_wrap` breaks to Thai and Lao characters as well. For ordinary paragraphs
/// with `word_wrap = false`, Thai/Lao text keeps its ICU4X dictionary boundaries instead.
pub fn apply(text: &str, opps: &mut Vec<usize>, kinsoku: bool, word_wrap: bool, force_char_break: bool) {
    let next_char = |i: usize| text.get(i..).and_then(|t| t.chars().next());
    let prev_char = |i: usize| text.get(..i).and_then(|t| t.chars().next_back());
    if kinsoku {
        opps.retain(|&i| !next_char(i).is_some_and(|c| NO_START.contains(c)) && !prev_char(i).is_some_and(|c| NO_END.contains(c)));
    } else {
        // Between any two East Asian characters.
        for (i, c) in text.char_indices().skip(1) {
            if is_east_asian(c) && prev_char(i).is_some_and(is_east_asian) {
                opps.push(i);
            }
        }
    }
    if !word_wrap || force_char_break {
        // Between any two letters or digits that are not East Asian (whole graphemes, so accents
        // stay with their letters). Thai/Lao are excluded from the ordinary word_wrap=false path
        // so ICU4X dictionary boundaries are preserved; force_char_break (stacked text) includes
        // them to get one character per line.
        let latin = |g: &str| {
            g.chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() && !is_east_asian(c) && (force_char_break || !wordcraft_doc::cluster::is_thai_lao(c)))
        };
        let mut prev: Option<&str> = None;
        for (i, g) in text.grapheme_indices(true) {
            if prev.is_some_and(latin) && latin(g) {
                opps.push(i);
            }
            prev = Some(g);
        }
    }
    opps.sort_unstable();
    opps.dedup();
}

/// Which of two adjacent characters is the Asian one, when the paragraph's automatic spacing puts
/// room between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsianSide {
    Left,
    Right,
}

/// Whether the paragraph's `autoSpaceDE` (Asian and Latin text) or `autoSpaceDN` (Asian text and
/// digits) spaces `a` from the `b` right after it, and which side is Asian. Asian text is
/// ideographs, kana and bopomofo, not their punctuation: Word leaves "5、" and "、8" alone. Hangul
/// is neither Asian nor Latin here, and Latin punctuation such as `%` or `(` is not Latin.
pub fn auto_space(a: char, b: char, de: bool, dn: bool) -> Option<AsianSide> {
    let spaced = |other: char| (de && is_latin_letter(other)) || (dn && other.is_ascii_digit());
    if is_asian_text(a) && spaced(b) {
        Some(AsianSide::Left)
    } else if is_asian_text(b) && spaced(a) {
        Some(AsianSide::Right)
    } else {
        None
    }
}

fn is_asian_text(c: char) -> bool {
    is_east_asian(c) && c.is_alphanumeric() && !is_hangul(c)
}

fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && !is_east_asian(c) && !is_hangul(c)
}

/// Hangul jamo, compatibility jamo, extended jamo, syllables and halfwidth Hangul.
fn is_hangul(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xFFA0..=0xFFDC)
}

#[cfg(test)]
mod tests {
    use super::{AsianSide, apply, auto_space};

    #[test]
    fn auto_space_pairs() {
        assert_eq!(auto_space('院', '9', true, true), Some(AsianSide::Left));
        assert_eq!(auto_space('A', 'カ', true, true), Some(AsianSide::Right));
        assert_eq!(auto_space('院', '9', true, false), None);
        assert_eq!(auto_space('A', '字', false, true), None);
        for (a, b) in [('5', '、'), ('、', '8'), ('漢', '%'), ('漢', '한'), ('한', 'A'), ('a', 'b'), ('漢', '字'), ('漢', '１')] {
            assert_eq!(auto_space(a, b, true, true), None, "{a}{b}");
        }
    }

    /// Where a line may start, as the text from there to the next opportunity.
    fn breaks(text: &str, kinsoku: bool, word_wrap: bool) -> Vec<String> {
        let mut opps: Vec<usize> = unicode_linebreak::linebreaks(text).map(|(i, _)| i).filter(|&i| i < text.len()).collect();
        apply(text, &mut opps, kinsoku, word_wrap, false);
        opps.iter().map(|&i| text.get(i..).and_then(|t| t.chars().next()).map(String::from).unwrap_or_default()).collect()
    }

    #[test]
    fn kinsoku_and_word_wrap_break_opportunities() {
        let ja = "日本語、「漢字」。テスト";
        let on = breaks(ja, true, true);
        for c in ["、", "」", "。", "漢"] {
            assert!(!on.iter().any(|s| s == c), "kinsoku: no line starts with {c}: {on:?}");
        }
        assert!(on.iter().any(|s| s == "「"), "a line may start at an opening bracket: {on:?}");
        assert!(on.iter().any(|s| s == "テ"), "{on:?}");
        let off = breaks(ja, false, true);
        for c in ["、", "」", "。", "漢"] {
            assert!(off.iter().any(|s| s == c), "kinsoku off: a line may start with {c}: {off:?}");
        }
        // Latin text: unchanged by kinsoku, broken anywhere without word wrap.
        let en = "Hello wonderful (world), 50%.";
        assert_eq!(breaks(en, true, true), breaks(en, false, true));
        assert_eq!(breaks(en, true, true), ["w", "(", "5"]);
        let any = breaks("Hello café", true, false);
        assert_eq!(any, ["e", "l", "l", "o", "c", "a", "f", "é"]);
    }

    #[test]
    fn word_wrap_false_preserves_thai_icu4x_boundaries() {
        // word_wrap=false (the OOXML `w:wordWrap` property) means Latin words may break
        // mid-character.  Thai/Lao characters are alphanumeric and non-East-Asian, so they would
        // fall into the Latin break category — incorrectly inserting a break between every Thai
        // character and discarding the ICU4X dictionary boundaries.  The fix: Thai/Lao chars are
        // excluded from the !word_wrap path; their breaks come from thai_lao_breaks() only.
        let thai = "อนุรักษ์ภาษาไทย"; // ICU4X dictionary breaks at bytes 24 and 36
        let mut opps: Vec<usize> = unicode_linebreak::linebreaks(thai).map(|(i, _)| i).filter(|&i| i < thai.len()).collect();
        apply(thai, &mut opps, true, false, false); // word_wrap=false, NOT stacked
        // Every opportunity must be an ICU4X word boundary, not a grapheme or char boundary.
        for &o in &opps {
            let prev = thai.get(..o).and_then(|s| s.chars().next_back());
            let next = thai.get(o..).and_then(|s| s.chars().next());
            let both_thai = matches!((prev, next), (Some(p), Some(n))
                if wordcraft_doc::cluster::is_thai_lao(p) && wordcraft_doc::cluster::is_thai_lao(n));
            assert!(!both_thai || matches!(o, 24 | 36), "Thai break at {o} is not an ICU4X word boundary (expected 24 or 36): {opps:?}");
        }
    }

    #[test]
    fn force_char_break_breaks_between_every_thai_char() {
        // force_char_break is set by textbox::lay for stacked (vertical CJK-style) text:
        // every character must be on its own line.  For Thai, this means a break between
        // every adjacent Thai grapheme, overriding the ICU4X-only restriction.
        let thai = "ภาษา"; // 4 Thai chars, 12 bytes, 3 inter-char positions
        let mut opps: Vec<usize> = unicode_linebreak::linebreaks(thai).map(|(i, _)| i).filter(|&i| i < thai.len()).collect();
        apply(thai, &mut opps, true, false, true); // force_char_break=true
        // All three inter-character positions must be present.
        assert!(opps.contains(&3), "break after ภ: {opps:?}");
        assert!(opps.contains(&6), "break after า: {opps:?}");
        assert!(opps.contains(&9), "break after ษ: {opps:?}");
    }
}

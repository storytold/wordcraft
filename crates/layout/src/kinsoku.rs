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
pub fn apply(text: &str, opps: &mut Vec<usize>, kinsoku: bool, word_wrap: bool) {
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
    if word_wrap {
        // Word's default `<w:wordWrap/>`: Korean wraps by space-delimited 어절 (like Latin words),
        // not between syllables as UAX #14's `H2`/`H3`/`JL`/`JV`/`JT` classes allow. Keep breaks
        // after spaces, hyphens and CJK ideographs (`漢字|한글`, `서울-|부산`), while keeping mixed
        // Latin/digit+Hangul words (`macOS에서`, `Word를`, `2026년에`) and attached brackets/percent
        // (`(한국어)를`, `한국어(Korean)`, `50%가`) intact.
        opps.retain(|&i| {
            let (Some(p), Some(n)) = (prev_char(i), next_char(i)) else { return true };
            if is_non_ea_word_char(p) && is_non_ea_word_char(n) && (is_hangul(p) || is_hangul(n)) {
                return false;
            }
            if is_hangul(p) && matches!(n, '.' | ',' | '!' | '?' | ':' | ';' | '…' | ')' | ']' | '}' | '”' | '’' | '(' | '[' | '{') {
                return false;
            }
            if matches!(p, ')' | ']' | '}' | '%' | '‰' | '」' | '』' | '）' | '〉' | '》' | '】' | '〕' | '〗' | '〙' | '〛' | '］' | '｝')
                && is_hangul(n)
            {
                return false;
            }
            true
        });
    } else {
        // Between any two letters or digits that are not East Asian (whole graphemes, so accents
        // and NFD conjoining Hangul Jamo stay with their syllables).
        let breakable = |g: &str| g.chars().next().is_some_and(|c| is_hangul(c) || (c.is_alphanumeric() && !is_east_asian(c)));
        let mut prev: Option<&str> = None;
        for (i, g) in text.grapheme_indices(true) {
            if prev.is_some_and(breakable) && breakable(g) {
                opps.push(i);
            }
            prev = Some(g);
        }
    }
    // Never split a conjoining Hangul Jamo syllable across lines, regardless of kinsoku/word_wrap.
    opps.retain(|&i| !prev_char(i).zip(next_char(i)).is_some_and(|(p, n)| splits_conjoining_jamo(p, n)));
    opps.sort_unstable();
    opps.dedup();
}

/// A Hangul or non-East-Asian word character (letters, digits and combining marks/joiners).
fn is_non_ea_word_char(c: char) -> bool {
    is_hangul(c)
        || (!is_east_asian(c)
            && (c.is_alphanumeric()
                || (!c.is_ascii()
                    && !c.is_control()
                    && matches!(unicode_bidi::bidi_class(c), unicode_bidi::BidiClass::NSM | unicode_bidi::BidiClass::BN))))
}

/// Whether breaking between `prev` and `next` splits a conjoining Hangul Jamo syllable (`L+V+T`).
fn splits_conjoining_jamo(prev: char, next: char) -> bool {
    let (p, n) = (u32::from(prev), u32::from(next));
    let is_l = |u: u32| matches!(u, 0x1100..=0x115F | 0xA960..=0xA97F);
    let is_v = |u: u32| matches!(u, 0x1160..=0x11A7 | 0xD7B0..=0xD7C6);
    let is_t = |u: u32| matches!(u, 0x11A8..=0x11FF | 0xD7CB..=0xD7FF);
    let is_s = |u: u32| matches!(u, 0xAC00..=0xD7A3);
    (is_l(p) && (is_l(n) || is_v(n))) || ((is_v(p) || is_s(p)) && (is_v(n) || is_t(n))) || (is_t(p) && is_t(n))
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
        apply(text, &mut opps, kinsoku, word_wrap);
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
    fn korean_word_wrap_breaks_by_eojeol_and_preserves_syllables_when_off() {
        // Default word_wrap == true: Korean wraps at spaces (어절), never between syllables or before trailing punctuation.
        assert_eq!(breaks("한국어 문서입니다.", true, true), ["문"]);
        assert_eq!(breaks("한국어 문서입니다.", false, true), ["문"]);
        // Mixed English/digit stems + Korean particles stay together as single 어절 units.
        assert_eq!(breaks("macOS에서 Word를 2026년에", true, true), ["W", "2"]);
        // CJK ideographs still break before Hangul, while Hangul syllables stay joined.
        assert_eq!(breaks("漢字한글", true, true), ["字", "한"]);
        // Hyphens allow breaking before the post-hyphen Hangul word.
        assert_eq!(breaks("서울-부산", true, true), ["부"]);
        // Opening/closing brackets and quotes stay attached to the enclosed Korean word.
        assert!(breaks("(한국어)", true, true).is_empty());
        assert!(breaks("“한국어”", true, true).is_empty());
        assert!(breaks("(한국어)를", true, true).is_empty());
        assert_eq!(breaks("안녕 (한국어) 문서", true, true), ["(", "문"]);
        assert_eq!(breaks("안녕 “한국어” 문서", true, true), ["“", "문"]);
        // NFD decomposed Hangul ("한글 문서") also wraps only at spaces when word_wrap == true.
        let nfd = "\u{1112}\u{1161}\u{11AB}\u{1100}\u{1173}\u{11AF} \u{1106}\u{116E}\u{11AB}\u{1109}\u{1165}";
        assert_eq!(breaks(nfd, true, true), ["\u{1106}"]);
        // word_wrap == false (`<w:wordWrap w:val="0"/>`): syllable breaks are permitted, while
        // kinsoku still protects punctuation and NFD conjoining Jamo never split mid-syllable.
        assert_eq!(breaks("한국어 문서입니다.", true, false), ["국", "어", "문", "서", "입", "니", "다"]);
        assert_eq!(breaks("한국어。", true, false), ["국", "어"]);
        assert_eq!(breaks(nfd, true, false), ["\u{1100}", "\u{1106}", "\u{1109}"]);
        assert_eq!(breaks(nfd, false, false), ["\u{1100}", "\u{1106}", "\u{1109}"]);
    }

    #[test]
    fn korean_word_wrap_keeps_braces_and_cjk_closing_brackets_attached() {
        // ASCII opening brace `{` after Hangul stays attached just like `(` and `[`.
        assert!(breaks("한글{중괄호}", true, true).is_empty());
        assert!(breaks("한글{중괄호}를", true, true).is_empty());
        assert_eq!(breaks("안녕 한글{중괄호} 문서", true, true), ["한", "문"]);

        // Korean particles after CJK closing brackets and corner quotes stay attached.
        for token in ["「한글」을", "『한국어』가", "《문서》를", "（한글）을", "【제목】이", "〈항목〉은", "〔부록〕에"]
        {
            assert!(breaks(token, true, true).is_empty(), "{token}: {:?}", breaks(token, true, true));
        }
        assert_eq!(breaks("안녕 「한글」을 읽다", true, true), ["「", "읽"]);
        assert_eq!(breaks("안녕 『한국어』가 좋다", true, true), ["『", "좋"]);
    }
}

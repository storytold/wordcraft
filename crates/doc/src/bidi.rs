//! Script classification for bidirectional and complex-script text (Persian, Arabic, Hebrew…).
//!
//! The full Unicode Bidirectional Algorithm (UAX #9) runs in layout; this module only answers
//! the cheap questions the model and the engine need: does a character belong to a
//! right-to-left script, and is it formatted with the complex-script properties?

/// A character of a right-to-left script: Hebrew, Arabic (Persian, Urdu…), Syriac, Thaana, N'Ko,
/// Samaritan, Mandaic, and their presentation forms. Includes the Arabic-Indic and Persian
/// (Extended Arabic-Indic) digits and the Arabic combining marks.
pub fn is_rtl_script(c: char) -> bool {
    matches!(u32::from(c),
        0x0590..=0x08FF
        | 0xFB1D..=0xFDFF
        | 0xFE70..=0xFEFF
        | 0x10800..=0x10FFF
        | 0x1E800..=0x1EFFF)
}

/// A character Word formats with the complex-script properties (`w:cs` font, `w:szCs`, `w:bCs`,
/// `w:iCs`): right-to-left scripts, the Indic scripts, Thai and Lao.
pub fn is_complex_script(c: char) -> bool {
    is_rtl_script(c) || matches!(u32::from(c), 0x0900..=0x0EFF)
}

/// A character that belongs to a word for word selection and Ctrl+arrow movement: letters,
/// digits, `_` and `'`, plus what sits inside Persian and Arabic words: the zero-width non-joiner
/// (نیم‌فاصله, as in «می‌خواهم»), the zero-width joiner, tatweel and the Arabic combining marks;
/// and the signs of the Indic scripts ([`is_indic_word_sign`]).
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
        || c == '_'
        || c == '\''
        || matches!(u32::from(c), 0x200C | 0x200D | 0x0640 | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E8 | 0x06EA..=0x06ED | 0x08D3..=0x08FF | 0x0300..=0x036F)
        || is_indic_word_sign(c)
}

/// A character of the Indic blocks (Devanagari … Sinhala, U+0900–U+0DFF) that Unicode doesn't
/// class as a letter or digit but that sits inside a word: the viramas that form conjuncts
/// (क्ष), the nuktas (ज़), the Vedic accents, and the scripts' few signs and symbols. Only their
/// punctuation (the danda ।, the double danda ॥, the abbreviation signs) separates words.
fn is_indic_word_sign(c: char) -> bool {
    matches!(u32::from(c), 0x0900..=0x0DFF) && !matches!(u32::from(c), 0x0964 | 0x0965 | 0x0970 | 0x09FD | 0x0A76 | 0x0AF0 | 0x0C77 | 0x0C84 | 0x0DF4)
}

/// Does `text` contain a right-to-left character?
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_script)
}

/// Does the first strong directional character in `text` belong to a right-to-left script?
///
/// Follows UAX #9 rules P2 and P3: characters with strong bidirectional class AL or R
/// (Arabic, Hebrew, Syriac, Thaana…) give `true`; strong L characters (Latin, Greek,
/// Cyrillic, CJK…) give `false`. Neutral, weak and whitespace characters are skipped.
pub fn first_strong_is_rtl(text: &str) -> bool {
    for c in text.chars() {
        if c == '\u{200F}' {
            return true;
        }
        if c == '\u{200E}' {
            return false;
        }
        if c.is_alphabetic() {
            return is_rtl_script(c);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_persian_and_latin() {
        // Persian letters, the Persian-only letters (پ چ ژ گ ک ی), ZWNJ-free word, digits.
        assert!("سلام".chars().all(is_rtl_script));
        assert!("پچژگکی".chars().all(is_rtl_script));
        assert!("۰۱۲۳۴۵۶۷۸۹".chars().all(is_rtl_script));
        assert!(is_rtl_script('\u{064E}'), "fatha (a combining mark) is Arabic script");
        assert!("שלום".chars().all(is_rtl_script));
        assert!(!"Hello 123".chars().any(is_rtl_script));
        assert!(!is_rtl_script('\u{200C}'), "ZWNJ is a format character, not a letter");
        assert!(is_complex_script('क') && !is_rtl_script('क'));
        assert!(has_rtl("Word ورد 2024") && !has_rtl("Word 2024"));
        assert!("می‌خواهم".chars().all(is_word_char), "ZWNJ is inside the word");
        assert!("کِتاب".chars().all(is_word_char), "so are harakat");
        assert!(!is_word_char(' ') && !is_word_char('،'));
        assert!(first_strong_is_rtl("سلام"));
        assert!(first_strong_is_rtl("من ناوم هێڤاره"));
        assert!(first_strong_is_rtl("  123. سلام"));
        assert!(!first_strong_is_rtl("Hello سلام"));
        assert!(!first_strong_is_rtl("123 Hello"));
        assert!(!first_strong_is_rtl(""));
    }

    #[test]
    fn indic_signs_are_inside_the_word() {
        // The virama (्) and nukta (़) aren't letters to Unicode, but a Nepali or Hindi word holds
        // them: क्षेत्र, ज़मीन and the half form क्‍ (ZWJ) are each one word.
        for w in ["क्षेत्र", "त्रिभुवन", "ज़मीन", "कर्म", "क्\u{200D}ष", "ক্ষমা", "ಕ್ಷೇತ್ರ", "ශ්‍රී"]
        {
            assert!(w.chars().all(is_word_char), "{w}");
        }
        // The danda and double danda end a sentence, the abbreviation sign follows one.
        for c in ['।', '॥', '॰', '\u{0DF4}'] {
            assert!(!is_word_char(c), "{c}");
        }
        assert!("१२३".chars().all(is_word_char), "Devanagari digits");
    }
}

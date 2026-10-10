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
/// (نیم‌فاصله, as in «می‌خواهم»), the zero-width joiner, tatweel and the Arabic combining marks.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
        || c == '_'
        || c == '\''
        || matches!(u32::from(c), 0x200C | 0x200D | 0x0640 | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E8 | 0x06EA..=0x06ED | 0x08D3..=0x08FF | 0x0300..=0x036F)
}

/// Does `text` contain a right-to-left character?
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_script)
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
    }
}

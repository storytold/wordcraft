//! Khmer script structure for line layout and justification.
//!
//! Khmer is written without spaces between words: spaces (and the optional zero-width space
//! U+200B) separate phrases. The unit of the script is the *orthographic syllable*: a base
//! consonant or independent vowel, any number of stacked subscript consonants (each introduced by
//! the invisible coeng U+17D2), dependent vowels and signs. A stack must never be pulled apart,
//! so justification adds space only *between* syllables, and prefers the gaps between phrases
//! and words to those between syllables.
//!
//! Source of the rules: Unicode chapter 16 (Khmer), the W3C Khmer Layout Requirements and the
//! shaping behaviour of the Khmer fonts commonly installed (see `docs/khmer-justify.md`).

/// Is `c` in the Khmer block (U+1780–17FF) or the Khmer Symbols block (U+19E0–19FF)?
pub fn is_khmer(c: char) -> bool {
    matches!(u32::from(c), 0x1780..=0x17FF | 0x19E0..=0x19FF)
}

/// The role of a Khmer character in a syllable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Consonant (ក–អ, U+1780–17A2).
    Consonant,
    /// Independent vowel (U+17A3–17B3).
    IndependentVowel,
    /// Dependent vowel, including the inherent vowels U+17B4/17B5 and ា–ៅ (U+17B6–17C5).
    DependentVowel,
    /// Invisible subscript marker U+17D2.
    Coeng,
    /// Other combining sign: nikahit, reahmuk, register shifters, bantoc, robat, toandakhiat…
    Sign,
    /// Digit (U+17E0–17E9, lunar date digits U+17F0–17F9).
    Digit,
    /// Phrase and sentence punctuation: khan ។, bariyosan ៕, camnuc pii kuuh ៖, beyyal ៘, phnaek muan ៙, koomuut ៚.
    Punct,
    /// Lek too ៗ, repeating the word before it: it belongs to that word.
    Repeat,
    /// Currency sign riel ៛ and the Khmer symbols block.
    Symbol,
    /// Zero-width joiner or non-joiner: shaping controls that stay with the previous character.
    Joiner,
    /// Any other character.
    Other,
}

/// The Khmer class of `c`.
pub fn class(c: char) -> Class {
    match u32::from(c) {
        0x1780..=0x17A2 => Class::Consonant,
        0x17A3..=0x17B3 | 0x17DC => Class::IndependentVowel,
        0x17B4..=0x17C5 => Class::DependentVowel,
        0x17C6..=0x17D1 | 0x17D3 | 0x17DD => Class::Sign,
        0x17D2 => Class::Coeng,
        0x17D4..=0x17D6 | 0x17D8..=0x17DA => Class::Punct,
        0x17D7 => Class::Repeat,
        0x17DB | 0x19E0..=0x19FF => Class::Symbol,
        0x17E0..=0x17E9 | 0x17F0..=0x17F9 => Class::Digit,
        0x200C | 0x200D => Class::Joiner,
        _ => Class::Other,
    }
}

/// Is `c` a zero-width space or a word joiner: invisible, takes no room, may mark a word boundary?
pub fn is_zero_width_space(c: char) -> bool {
    c == '\u{200B}'
}

/// May a syllable start at `c`? Consonants and independent vowels do, and so do digits and
/// symbols (each stands alone).
fn starts_syllable(c: char) -> bool {
    matches!(class(c), Class::Consonant | Class::IndependentVowel | Class::Digit | Class::Symbol)
}

/// How much a gap between two clusters may stretch in justification, from most to least eager.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Gap {
    /// Not a place to add space (inside a syllable, or inside a Latin word).
    None = 0,
    /// Between orthographic syllables of one word.
    Syllable = 1,
    /// Between words: at a zero-width space, after phrase punctuation, or between Khmer and
    /// another script or a digit.
    Word = 2,
    /// A space: between phrases.
    Space = 3,
}

/// The kind of gap between the character `before` and the character `after` (the last of the
/// cluster that ends here and the first of the next). `after_zwsp` says a zero-width space sits
/// between them (it is a cluster of its own with no width).
pub fn gap_between(before: char, after: char, zwsp_between: bool) -> Gap {
    if before == ' ' || before == '\u{3000}' {
        return Gap::Space;
    }
    if zwsp_between {
        return Gap::Word;
    }
    let (kb, ka) = (is_khmer(before), is_khmer(after));
    if !kb && !ka {
        return Gap::None;
    }
    // Nothing is inserted before a sign that belongs to the syllable or the word before it.
    if matches!(class(after), Class::DependentVowel | Class::Sign | Class::Coeng | Class::Joiner | Class::Repeat) {
        return Gap::None;
    }
    // The coeng and a joiner pull the next consonant into the stack.
    if matches!(class(before), Class::Coeng | Class::Joiner) {
        return Gap::None;
    }
    match (kb, ka) {
        (true, true) => {
            if class(before) == Class::Punct || class(before) == Class::Repeat {
                Gap::Word
            } else if starts_syllable(after) {
                Gap::Syllable
            } else {
                Gap::None
            }
        }
        // Khmer next to a Latin letter or a digit is a word boundary; next to a full stop, a
        // bracket or a quote it is not (those stay attached to the word).
        (true, false) | (false, true) => {
            if (after.is_alphanumeric() && before.is_alphanumeric()) || class(before) == Class::Punct && after.is_alphanumeric() {
                Gap::Word
            } else {
                Gap::None
            }
        }
        _ => Gap::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_follow_the_unicode_chart() {
        assert_eq!(class('ក'), Class::Consonant);
        assert_eq!(class('ឣ'), Class::IndependentVowel);
        assert_eq!(class('ា'), Class::DependentVowel);
        assert_eq!(class('្'), Class::Coeng);
        assert_eq!(class('៉'), Class::Sign, "register shifter muusikatoan");
        assert_eq!(class('។'), Class::Punct);
        assert_eq!(class('ៗ'), Class::Repeat);
        assert_eq!(class('៣'), Class::Digit);
        assert_eq!(class('៛'), Class::Symbol);
        assert_eq!(class('a'), Class::Other);
        assert!(is_khmer('ក') && is_khmer('᧠') && !is_khmer('a'));
    }

    #[test]
    fn gaps_rank_spaces_over_words_over_syllables() {
        assert_eq!(gap_between(' ', 'ក', false), Gap::Space);
        assert_eq!(gap_between('ក', 'ខ', true), Gap::Word, "zero-width space");
        assert_eq!(gap_between('។', 'ក', false), Gap::Word, "after khan");
        assert_eq!(gap_between('ក', 'ខ', false), Gap::Syllable);
        assert_eq!(gap_between('ក', 'a', false), Gap::Word);
        assert!(Gap::Space > Gap::Word && Gap::Word > Gap::Syllable && Gap::Syllable > Gap::None);
    }

    #[test]
    fn nothing_is_inserted_inside_a_syllable() {
        assert_eq!(gap_between('ក', 'ា', false), Gap::None, "vowel sign");
        assert_eq!(gap_between('ក', '្', false), Gap::None, "coeng");
        assert_eq!(gap_between('្', 'រ', false), Gap::None, "subscript consonant");
        assert_eq!(gap_between('ក', '៉', false), Gap::None, "register shifter");
        assert_eq!(gap_between('ក', 'ៗ', false), Gap::None, "lek too stays with its word");
        assert_eq!(gap_between('a', 'b', false), Gap::None, "Latin letters are not spaced");
    }

    #[test]
    fn punctuation_and_brackets_stay_attached_to_khmer_words() {
        assert_eq!(gap_between('ម', '.', false), Gap::None, "full stop");
        assert_eq!(gap_between('ម', ')', false), Gap::None, "close bracket");
        assert_eq!(gap_between('(', 'ក', false), Gap::None, "open bracket");
        assert_eq!(gap_between('ម', '"', false), Gap::None, "quote");
        assert_eq!(gap_between('ម', 'a', false), Gap::Word, "Latin word after Khmer");
        assert_eq!(gap_between('1', 'ក', false), Gap::Word, "digit before Khmer");
        assert_eq!(gap_between('។', 'a', false), Gap::Word, "after khan");
    }
}

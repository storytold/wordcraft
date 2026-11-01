//! Croatian spelling and conservative grammar support.
//! Uses an installed hr_HR Hunspell .aff/.dic pair (not bundled).
//! The engine decodes ISO-8859-2 dictionaries at load time.
//! Suggestions prioritize č/ć, đ, š, ž and common one-character mistakes.
//! Hyphenation remains the host engine's fallback until Croatian patterns are supplied.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of a Croatian Hunspell dictionary.
const NAMES: &[&str] = &["hr_HR", "hr-HR", "hr"];

/// The Croatian dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Croatian");

/// The Croatian dictionary: the installed Hunspell `hr_HR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Croatian spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Croatian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Croatian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Is `word` spelled correctly in Croatian? Numbers, single letters, short words in capitals
/// (acronyms: `HRT`, `RH`), words with digits and words in other scripts are accepted, and so
/// is every word when no Croatian dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words in UPPERCASE off,
/// short words in capitals are checked against the dictionary too, and with Ignore words that
/// contain numbers off, so are words with digits.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    let Some(w) = crate::spelling::word_to_check(d, word, opts) else {
        return true;
    };
    if w.contains('-') {
        return w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
    }
    known(d, w)
}

/// Is one word (no hyphens) in the dictionary or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dictionary_loads() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.len() > 15);
        assert!(d.rule_count() > 0);
        assert!(!d.map().is_empty() && !d.rep().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn correct_words_and_affixes() {
        test_dict::install();
        for w in ["kuća", "kuće", "računalo", "čovjek", "riječ", "dijete", "Hrvatska", "čokolada", "škola", "škole", "HRT", "2026"] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["kuca", "racunalo", "covjek", "rijec", "dijeteh", "skolla"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn missing_dictionary_is_silent() {
        assert!(is_correct_with(&AffixDict::default(), "nepostojećariječ"));
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Croatian checks
    /// those words against the dictionary instead of accepting them.
    #[test]
    fn proofing_options_reach_the_croatian_check() {
        test_dict::install();
        let lang = crate::ProofLang::Hr;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("ŠKOLLA", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("ŠKOLLA", lang, &check_caps));
        assert!(crate::is_correct_in_with("ŠKOLA", lang, &check_caps));
        assert!(crate::is_correct_in_with("školla2", lang, &defaults));
        assert!(!crate::is_correct_in_with("školla2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("ŠKOLLA"), "direct checks keep the defaults");
    }
}

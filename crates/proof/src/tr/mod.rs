//! Turkish proofing.
//!
//! - **Spelling** against the Hunspell `tr_TR` dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `tr_TR.aff`/`tr_TR.dic` from the OS, a
//!   package manager or LibreOffice; it is not bundled). Without it Turkish text gets no spelling
//!   marks. Capitalized stems are proper nouns (`Ankara`); foreign names inflected with an apostrophe
//!   are accepted (`WordCraft'ta`), and the user dictionary applies as in English.
//! - **Suggestions** tuned for Turkish ([`suggest`]): ASCII-diacritic restoration (`turkce` -> `Türkçe`),
//!   common confusions (`yalnış` -> `yanlış`), detached particles (`geldimi` -> `geldi mi`), and typos.
//! - **Grammar** ([`grammar`]): high-precision rules (reduplications, comma around conjunctions,
//!   sentence start with abbreviations).
//! - **Hyphenation** ([`hyphen`]): syllable division by Turkish Language Association (TDK) rules.

pub mod dict;
pub mod grammar;
pub mod hyphen;
pub mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::slot::DictSlot;

pub use dict::TrDict;
pub use suggest::suggest;

/// File names of a Turkish Hunspell dictionary.
const NAMES: &[&str] = &["tr_TR", "tr-TR", "tr"];

/// The Turkish dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<TrDict> = DictSlot::new(NAMES, "Turkish");

/// The Turkish dictionary: the installed Hunspell `tr_TR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available.
pub fn dictionary() -> Arc<TrDict> {
    DICT.get()
}

/// Use this Turkish dictionary from now on (Hunspell `.aff` and `.dic` text). On error the
/// current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Turkish dictionary available?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Is `word` spelled correctly in Turkish?
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words that contain
/// numbers off, words with digits are checked against the dictionary too.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct`] against a given dictionary.
pub fn is_correct_with(d: &TrDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &TrDict, word: &str, opts: &crate::Options) -> bool {
    if d.is_empty() {
        return true; // No Turkish dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1 || (opts.ignore_numbers && w.chars().any(|c| c.is_ascii_digit())) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    // Check user dictionary first
    if crate::in_user_dictionary(&suggest::turkish_to_lower(w)) || crate::in_user_dictionary(&w.to_lowercase()) {
        return true;
    }

    // Direct lookup or lowercase lookup
    if d.contains(w) {
        return true;
    }
    let lower = suggest::turkish_to_lower(w);
    if d.contains(&lower) {
        return true;
    }

    // Handle apostrophe for proper nouns (e.g., `Ankara'da`, `WordCraft'ta`)
    if let Some((stem, _suffix)) = w.split_once(['\'', '’'])
        && (d.contains(stem) || d.contains(&suggest::turkish_to_lower(stem)))
    {
        return true;
    }

    // Compound words separated by hyphens (e.g., `afyon-karahisar`)
    if w.contains('-') && w.split('-').all(|part| part.is_empty() || check(d, part, opts)) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_dictionary_everything_is_correct() {
        let empty = TrDict::default();
        assert!(is_correct_with(&empty, "herhangi"));
        assert!(is_correct_with(&empty, "xyzabc"));
    }

    #[test]
    fn apostrophe_and_compounds_are_handled() {
        let empty = TrDict::default();
        assert!(is_correct_with(&empty, "Ankara'da"));
        assert!(is_correct_with(&empty, "açık-mavi"));
    }

    #[test]
    fn installed_dictionary_checks_words() {
        test_dict::install();
        assert!(is_correct("kitap"));
        assert!(is_correct("kitaplar"));
        assert!(is_correct("evde"));
        assert!(is_correct("olacaklardan"));
        assert!(is_correct("sonra"));
        assert!(!is_correct("sorna"));
        assert!(!is_correct("kitapppp"));
        assert!(!is_correct("evdekişşş"));

        // Test user dictionary
        assert!(!is_correct("benimbilmedigimkelime"));
        crate::add_word("benimbilmedigimkelime");
        assert!(is_correct("benimbilmedigimkelime"));
    }

    /// With Ignore words that contain numbers off, Turkish checks words with digits against the
    /// dictionary instead of accepting them, hyphen parts included.
    #[test]
    fn proofing_options_reach_the_turkish_check() {
        test_dict::install();
        let lang = crate::ProofLang::Tr;
        let defaults = crate::Options::default();
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("kitapp2", lang, &defaults));
        assert!(!crate::is_correct_in_with("kitapp2", lang, &check_digits));
        assert!(!crate::is_correct_in_with("ev-kitapp2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("kitapp2"), "direct checks keep the defaults");
    }
}

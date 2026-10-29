//! Polish proofing.
//!
//! - **Spelling** against the SJP.PL Hunspell dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `pl_PL.aff`/`pl_PL.dic` from the OS, a
//!   package manager or LibreOffice; it isn't bundled). Without it Polish text gets no spelling
//!   marks. Capitalized stems are names (`Warszawa`); a sentence may start with any word and
//!   headings may be in capitals. Hyphenated compounds are checked part by part
//!   (`biało-czerwony`), foreign names inflected with an apostrophe are accepted
//!   (`Kennedy'ego`), and the user dictionary applies as in English.
//! - **Suggestions** tuned for Polish ([`suggest()`]): missing diacritics first (`zolw` → `żółw`), then
//!   the common spelling confusions (`ż`/`rz`, `u`/`ó`, `h`/`ch`…), a missing space (`napewno` →
//!   `na pewno`), one-letter typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Polish messages.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Polish dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of a Polish Hunspell dictionary.
const NAMES: &[&str] = &["pl_PL", "pl-PL", "pl"];

/// The Polish dictionary: the installed Hunspell `pl_PL` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Polish spelling is off then;
/// see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    if let Some(d) = DICT.read().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    slot.get_or_insert(loaded).clone()
}

fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        crate::hunspell::clear_load_error(NAMES[0]);
        log::info!("Polish spelling: no Hunspell pl_PL dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Polish spelling: {e}");
        AffixDict::default()
    })
}

/// Use this Polish dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to UTF-8),
/// for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(d));
    Ok(())
}

/// Forget the loaded dictionary, so the next check looks for an installed one again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// The `.dic` file of the installed Polish dictionary, searching the dictionaries folder and
/// the system locations now (File › Options › Proofing shows it, #1063); `None` when there is
/// none. A dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is a Polish dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Endings Polish adds after an apostrophe to foreign names whose last letter isn't pronounced
/// (`Kennedy'ego`, `Wilde'a`, `Shakespeare'owi`).
const APOSTROPHE_ENDINGS: &[&str] = &["a", "ach", "ami", "e", "ego", "em", "emu", "i", "m", "om", "owi", "owie", "u", "y", "ów", "ą", "ę"];

/// Is `c` a letter of the Latin script (the only one the Polish dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Polish? Numbers, single letters, short words in capitals
/// (acronyms: `PKO`, `ZUS`), words with digits and words in other scripts are accepted, and so
/// is every word when no Polish dictionary is installed.
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
    if d.is_empty() {
        return true; // no Polish dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1 || (opts.ignore_numbers && w.chars().any(|c| c.is_ascii_digit())) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if opts.ignore_uppercase && w.chars().all(|c| !c.is_lowercase()) && n <= 6 {
        return true;
    }
    if !w.chars().any(is_latin_letter) {
        return true; // other scripts: no dictionary
    }
    let w = w.replace('’', "'");
    if w.contains('-') {
        return w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
    }
    known(d, &w)
}

/// Is one word (no hyphens) in the dictionary, the user dictionary, or a foreign name with a
/// Polish ending after an apostrophe?
fn known(d: &AffixDict, w: &str) -> bool {
    if d.check(w) || crate::in_user_dictionary(&w.to_lowercase()) {
        return true;
    }
    if let Some((stem, ending)) = w.split_once('\'') {
        return stem.chars().next().is_some_and(char::is_uppercase) && stem.chars().count() >= 2 && APOSTROPHE_ENDINGS.contains(&ending);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dictionary_loads() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert!(d.len() > 50, "{}", d.len());
        assert_eq!(d.rule_count(), 5, "the test dictionary's affix rules");
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged_and_nothing_suggested() {
        let none = AffixDict::default();
        for w in ["żeka", "kturzy", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(suggest::suggest_in(&none, "żeka", 5).is_empty());
        assert!(install_dictionary("FLAG long\n", "1\nkot\n").is_err(), "unsupported files are refused");
    }

    #[test]
    fn polish_words_are_correct() {
        test_dict::install();
        for w in [
            "źdźbło",
            "gżegżółka",
            "chrząszcz",
            "pięćdziesięciu",
            "rzeka",
            "którzy",
            "żółw",
            "Zażółć",
            "gęślą",
            "jaźń",
            "konstytucja",
            "konstytucyjnego",
            "niebieskiego",
            "nieładnie",
            "przyszliśmy",
            "zrobilibyśmy",
            "Warszawa",
            "Warszawie",
            "WARSZAWA",
            "Kot",
            "biało-czerwony",
            "polsko-niemieckiej",
            "Kennedy'ego",
            "Shakespeare’a",
            "PKO",
            "2026",
            "e-mail",
            "w",
            "się",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["żeka", "kturzy", "napewno", "wogóle", "zolw", "chuśtawka", "bżoza", "warszawa", "niewiem", "pszyjaciel", "Kennedy'xyz"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zxqpolskisłowo"));
        crate::add_word("Zxqpolskisłowo");
        assert!(is_correct("zxqpolskisłowo"));
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Polish checks
    /// those words against the dictionary instead of accepting them.
    #[test]
    fn proofing_options_reach_the_polish_check() {
        test_dict::install();
        let lang = crate::ProofLang::Pl;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("ŻEKA", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("ŻEKA", lang, &check_caps));
        assert!(crate::is_correct_in_with("RZEKA", lang, &check_caps));
        assert!(crate::is_correct_in_with("żeka2", lang, &defaults));
        assert!(!crate::is_correct_in_with("żeka2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("ŻEKA"), "direct checks keep the defaults");
    }
}

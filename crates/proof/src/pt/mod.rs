//! Portuguese proofing.
//!
//! - **Spelling** against the Hunspell `pt_BR` dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `pt_BR.aff`/`pt_BR.dic` — VERO and its
//!   derivatives — from the OS, a package manager or LibreOffice; it isn't bundled). Without it
//!   Portuguese text gets no spelling marks. Capitalized stems are names (`Brasília`); a sentence
//!   may start with any word and headings may be in capitals. Hyphenated compounds are checked
//!   part by part (`guarda-roupa`, `ex-marido`), and the user dictionary applies as in English.
//! - **Suggestions** tuned for Portuguese ([`suggest`]): missing accents first (`coraçao` →
//!   `coração`, `saude` → `saúde`), then the dictionary's `REP` confusions (`seje` → `seja`), a
//!   missing space, one-letter typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Portuguese messages.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of a Portuguese Hunspell dictionary.
const NAMES: &[&str] = &["pt_BR", "pt-BR", "pt"];

/// The Portuguese dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Portuguese");

/// The Portuguese dictionary: the installed Hunspell `pt_BR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Portuguese spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Portuguese dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Portuguese dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Is `word` spelled correctly in Portuguese? Numbers, single letters, short words in capitals
/// (acronyms: `USP`, `PT`), words with digits and words in other scripts are accepted, and so
/// is every word when no Portuguese dictionary is installed.
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
        assert!(d.len() > 50, "{}", d.len());
        assert!(d.rule_count() >= 5, "the test dictionary's affix rules");
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged() {
        let none = AffixDict::default();
        for w in ["coraçao", "saude", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(install_dictionary("COMPOUNDFLAG X\n", "1\nkot\n").is_err(), "unsupported files are refused");
    }

    #[test]
    fn portuguese_words_are_correct() {
        test_dict::install();
        for w in [
            "coração",
            "saudade",
            "pássaro",
            "saúde",
            "árvore",
            "escrever",
            "computador",
            "praia",
            "conhecer",
            "Brasília",
            "São",
            "autoestima",
            "frequência",
            "exceção",
            "amanhã",
            "vocês",
            "coragem",
            "parabéns",
            "cantava",
            "cantando",
            "provável",
            "prováveis",
            "descobrir",
            "trabalho",
            "CORAGEM",
            "USP",
            "2026",
            "e-mail",
            "guarda-roupa",
            "ex-marido",
            "vice-presidente",
            "e",
            "não",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["coraçao", "saude", "trbalho", "excrever", "prsidente", "frequencia", "amnha", "cantaando"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn forbidden_words_are_rejected() {
        test_dict::install();
        // `frequencia/F` is listed with FORBIDDENWORD: the old spelling, never correct.
        assert!(!is_correct("frequencia") && !is_correct("Frequencia") && !is_correct("FREQUENCIA"));
        assert!(is_correct("frequência"));
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zxqportuguespalavra"));
        crate::add_word("Zxqportuguespalavra");
        assert!(is_correct("zxqportuguespalavra"));
    }

    #[test]
    fn user_dictionary_accepts_a_hyphenated_word_whole() {
        test_dict::install();
        let text = "Ele disse que está na Unigranrio-RJ.";
        assert_eq!(crate::check_spelling_in(text, crate::ProofLang::Pt).len(), 1, "Unigranrio isn't in the dictionary");
        // Added as the context menu adds it, hyphen and all (#2082).
        crate::add_word("Unigranrio-RJ");
        assert!(crate::check_spelling_in(text, crate::ProofLang::Pt).is_empty());
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Portuguese checks
    /// those words against the dictionary instead of accepting them.
    #[test]
    fn proofing_options_reach_the_portuguese_check() {
        test_dict::install();
        let lang = crate::ProofLang::Pt;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("PRAYA", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("PRAYA", lang, &check_caps));
        assert!(crate::is_correct_in_with("PRAIA", lang, &check_caps));
        assert!(crate::is_correct_in_with("praya2", lang, &defaults));
        assert!(!crate::is_correct_in_with("praya2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("PRAYA"), "direct checks keep the defaults");
    }
}

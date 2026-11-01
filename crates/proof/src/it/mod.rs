//! Italian proofing.
//!
//! - **Spelling** against the Hunspell `it_IT` dictionary installed on the system (found by
//!   [`crate::hunspell`]: `it_IT.aff`/`it_IT.dic` from the OS, a package manager or
//!   LibreOffice; it isn't bundled — the upstream file is GPL-licensed). Without it Italian
//!   text gets no spelling marks. Elided forms check the word after the apostrophe
//!   (`l'amico`, `un'idea`, `c'è`, `dov'è`, `senz'altro`); truncations keep their apostrophe
//!   (`po'`, `mo'`); hyphenated compounds are checked part by part; the user dictionary applies
//!   as in English. Forms a grammar rule explains better (`pò`, `qual'è`) are left to it.
//! - **Suggestions** tuned for Italian ([`suggest`]): missing accents first (`caffe` → `caffè`,
//!   `perche` → `perché`), then the dictionary's `REP` confusions, a missing space, one-letter
//!   typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Italian messages, which work with
//!   no dictionary installed.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of an Italian Hunspell dictionary (Italy first, then Switzerland).
const NAMES: &[&str] = &["it_IT", "it_CH", "it-IT", "it"];

/// The Italian dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Italian");

/// The Italian dictionary: the installed Hunspell `it_IT` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Italian spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Italian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is an Italian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Proclitics (and verb clippings) that elide before a vowel or `h`: the word after the
/// apostrophe is what gets checked (`l'amico`, `un'idea`, `c'è`, `m'hai`, `dov'è`, `anch'io`).
const PROCLITICS: &[&str] = &[
    "l", "un", "una", "uno", "all", "dall", "dell", "nell", "sull", "coll", "pell", "c", "m", "t", "s", "v", "d", "j", "n", "ch", "gl", "quest",
    "quell", "bell", "buon", "sant", "grand", "poc", "nessun", "ciascun", "qualcun", "alcun", "dov", "cos", "com", "quand", "anch", "senz", "tutt",
    "nient", "mezz", "sott", "tant", "quant", "dev",
];

/// Truncated words written with an apostrophe (`un po'`, `a mo' di`, `be'`): [`crate::words`]
/// trims the apostrophe, so spelling sees the bare word; [`grammar`] checks the apostrophe.
const TRUNCATIONS: &[&str] = &["po", "mo", "be"];

/// Misspellings a grammar rule reports with a clearer message and fix (`pò` → `po'`, `qual'è`
/// → `qual è`), so spelling leaves them alone rather than marking the word twice.
fn left_to_grammar(lower: &str) -> bool {
    lower == "pò" || grammar::qual_elided(lower).is_some()
}

/// Is `word` spelled correctly in Italian? Numbers, single letters, short words in capitals
/// (acronyms: `RAI`, `TG`), words with digits and words in other scripts are accepted, and so
/// is every word when no Italian dictionary is installed.
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
    let lower = w.to_lowercase();
    if TRUNCATIONS.contains(&lower.as_str()) || left_to_grammar(&lower) {
        return true;
    }
    if w.contains('-') {
        return w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
    }
    if let Some((head, tail)) = w.split_once(['\'', '’'])
        && PROCLITICS.contains(&head.to_lowercase().as_str())
        && !tail.is_empty()
    {
        return known(d, tail);
    }
    known(d, w)
}

/// Is one word (no hyphens, no apostrophe) in the dictionary or the user dictionary?
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
        assert!(d.rule_count() >= 4, "the test dictionary's affix rules");
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn swiss_dictionary_files_are_found() {
        // it_CH is searched too, so Swiss users with only it_CH installed get Italian spelling.
        let dir = std::env::temp_dir().join(format!("wc-it-ch-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("it_CH.aff"), test_dict::AFF).expect("aff");
        std::fs::write(dir.join("it_CH.dic"), test_dict::DIC).expect("dic");
        let found = crate::hunspell::find_in(&dir, NAMES);
        let ok = found.as_ref().is_some_and(|(a, d)| a.ends_with("it_CH.aff") && d.ends_with("it_CH.dic"));
        assert!(ok, "{found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged() {
        let none = AffixDict::default();
        for w in ["caffè", "perché", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(install_dictionary("FLAG long\n", "1\nkot\n").is_err(), "unsupported files are refused");
    }

    #[test]
    fn italian_words_are_correct() {
        test_dict::install();
        for w in [
            "casa", "case", "libro", "libri", "città", "caffè", "perché", "più", "può", "parlare", "parla", "parlano", "rifare", "bello", "belli",
            "nuovo", "bianco", "Roma", "Milano", "l'amico", "un'idea", "L'amico", "RAI", "2026", "e", "non",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn elisions_and_truncations_are_correct() {
        test_dict::install();
        for w in ["dov'è", "Com'è", "cos'è", "anch'io", "senz'acqua", "tutt'uno", "quand'è", "nessun'amica", "n'ha", "po'", "po", "mo'"] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["caffé", "perche", "lbiro", "parlre", "amicaa", "citta", "quelo", "dov'lbiro", "xyz'casa"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn grammar_owned_forms_are_not_marked_twice() {
        test_dict::install();
        // `pò` and `qual'è` are reported by the grammar rules, with the fix; spelling stays quiet.
        for w in ["pò", "Pò", "qual'è", "Qual’era"] {
            assert!(is_correct(w), "{w}");
            assert!(!grammar::check(w).is_empty(), "{w}: grammar reports it");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zxqitalianaparola"));
        crate::add_word("Zxqitalianaparola");
        assert!(is_correct("zxqitalianaparola"));
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Italian checks
    /// those words against the dictionary instead of accepting them.
    #[test]
    fn proofing_options_reach_the_italian_check() {
        test_dict::install();
        let lang = crate::ProofLang::It;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("LIBRRO", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("LIBRRO", lang, &check_caps));
        assert!(crate::is_correct_in_with("LIBRO", lang, &check_caps));
        assert!(crate::is_correct_in_with("librro2", lang, &defaults));
        assert!(!crate::is_correct_in_with("librro2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("LIBRRO"), "direct checks keep the defaults");
    }
}

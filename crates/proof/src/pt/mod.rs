//! Portuguese proofing.
//!
//! - **Spelling** against the Hunspell `pt_BR` dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `pt_BR.aff`/`pt_BR.dic` — VERO and its
//!   derivatives — from the OS, a package manager or LibreOffice; it isn't bundled). Without it
//!   Portuguese text gets no spelling marks. Capitalized stems are names (`Brasília`); a sentence
//!   may start with any word and headings may be in capitals. Hyphenated compounds are checked
//!   part by part (`guarda-roupa`, `ex-marido`), and the user dictionary applies as in English.
//! - **Two spellings** ([`Variant`]): European Portuguese (`pt-PT`, [`crate::ProofLang::PtPt`])
//!   is checked against the Hunspell `pt_PT` dictionary (`pt_PT.aff`/`pt_PT.dic`, found the same
//!   way; LibreOffice ships it as `dict-pt-PT`), so `facto`, `equipa` and `económico` are words
//!   there and `econômico` isn't. Without a `pt_PT` dictionary European text falls back to the
//!   Brazilian one, as before the two were told apart.
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
/// For European Portuguese: its own, then the Brazilian one.
const EUROPEAN_NAMES: &[&str] = &["pt_PT", "pt-PT", "pt_BR", "pt-BR", "pt"];

/// The Portuguese dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Portuguese");
/// The European Portuguese dictionary, found the same way.
pub(crate) static EUROPEAN_DICT: DictSlot<AffixDict> = DictSlot::new(EUROPEAN_NAMES, "Portuguese (Portugal)");

/// Which of the two Portuguese spellings a run of text follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Brazilian Portuguese, and Portuguese with no region ([`crate::ProofLang::Pt`]): the
    /// Hunspell `pt_BR` dictionary.
    Br,
    /// European Portuguese ([`crate::ProofLang::PtPt`]): the Hunspell `pt_PT` dictionary.
    Pt,
}

impl Variant {
    /// This spelling's dictionary.
    fn slot(self) -> &'static DictSlot<AffixDict> {
        match self {
            Variant::Br => &DICT,
            Variant::Pt => &EUROPEAN_DICT,
        }
    }
}

/// The Portuguese dictionary: the installed Hunspell `pt_BR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Portuguese spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// [`dictionary`] for one spelling. European Portuguese without a `pt_PT` dictionary reads the
/// Brazilian one.
pub fn dictionary_for(variant: Variant) -> Arc<AffixDict> {
    variant.slot().get()
}

/// Use this Portuguese dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// [`install_dictionary`] for one spelling.
pub fn install_dictionary_for(variant: Variant, aff: &str, dic: &str) -> Result<(), String> {
    variant.slot().install(aff, dic)
}

/// Is a Portuguese dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// [`has_dictionary`] for one spelling.
pub fn has_dictionary_for(variant: Variant) -> bool {
    variant.slot().has_dictionary()
}

/// Is `word` spelled correctly in Portuguese? Numbers, single letters, short words in capitals
/// (acronyms: `USP`, `PT`), words with digits and words in other scripts are accepted, and so
/// is every word when no Portuguese dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] in one spelling (`facto` is a word in European Portuguese only).
pub fn is_correct_for(variant: Variant, word: &str) -> bool {
    is_correct_with(&dictionary_for(variant), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words in UPPERCASE off,
/// short words in capitals are checked against the dictionary too, and with Ignore words that
/// contain numbers off, so are words with digits.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct_with_options`] in one spelling.
pub fn is_correct_with_options_for(variant: Variant, word: &str, opts: &crate::Options) -> bool {
    check(&dictionary_for(variant), word, opts)
}

/// Spelling suggestions for a word misspelled in one spelling ([`suggest`] is the Brazilian one).
pub fn suggest_for(variant: Variant, word: &str, max: usize) -> Vec<String> {
    suggest::suggest_in(&dictionary_for(variant), word, max)
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
    fn the_two_spellings_have_their_own_dictionaries() {
        test_dict::install();
        test_dict::install_european();
        assert!(has_dictionary_for(Variant::Br) && has_dictionary_for(Variant::Pt));
        for w in ["facto", "equipa", "autocarro", "económico", "Económico", "FACTO"] {
            assert!(is_correct_for(Variant::Pt, w), "{w}");
        }
        for w in ["facto", "equipa", "autocarro", "económico"] {
            assert!(!is_correct(w) && !is_correct_for(Variant::Br, w), "{w}");
        }
        for w in ["equipe", "ônibus", "econômico"] {
            assert!(is_correct(w) && !is_correct_for(Variant::Pt, w), "{w}");
        }
        // Words both write the same way are in both.
        assert!(is_correct("coração") && is_correct_for(Variant::Pt, "coração"));
        assert_eq!(suggest_for(Variant::Pt, "economico", 3).first().map(String::as_str), Some("económico"));
        assert_eq!(suggest_for(Variant::Br, "economico", 3).first().map(String::as_str), Some("econômico"));
        assert_eq!((Variant::Br.slot().name(), Variant::Pt.slot().name()), ("pt_BR", "pt_PT"));
    }

    /// Each spelling finds its own file first; European Portuguese falls back to the Brazilian
    /// files, and Brazilian Portuguese never reads a European one.
    #[test]
    fn each_spelling_looks_for_its_own_dictionary_first() {
        let dir = std::env::temp_dir().join(format!("wordcraft-pt-variants-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str| {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
        };
        let (br, pt) = (Variant::Br.slot().names(), Variant::Pt.slot().names());
        for (name, in_br) in [("pt_BR", true), ("pt-BR", true), ("pt", true), ("pt_PT", false), ("pt-PT", false)] {
            write(name);
            assert_eq!(crate::hunspell::find_in(&dir, br).is_some(), in_br, "{name}");
            assert!(crate::hunspell::find_in(&dir, pt).is_some(), "{name}");
            std::fs::remove_file(dir.join(format!("{name}.aff"))).unwrap();
            std::fs::remove_file(dir.join(format!("{name}.dic"))).unwrap();
        }
        write("pt_BR");
        assert_eq!(crate::hunspell::find_in(&dir, pt).map(|(_, dic)| dic), Some(dir.join("pt_BR.dic")), "the fallback");
        write("pt_PT");
        assert_eq!(crate::hunspell::find_in(&dir, pt).map(|(_, dic)| dic), Some(dir.join("pt_PT.dic")));
        assert_eq!(crate::hunspell::find_in(&dir, br).map(|(_, dic)| dic), Some(dir.join("pt_BR.dic")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged() {
        let none = AffixDict::default();
        for w in ["coraçao", "saude", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(install_dictionary("FLAG long\n", "1\nkot\n").is_err(), "unsupported files are refused");
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

    /// European Portuguese takes the File › Options › Proofing choices as Brazilian does.
    #[test]
    fn proofing_options_reach_the_european_portuguese_check() {
        test_dict::install();
        test_dict::install_european();
        let lang = crate::ProofLang::PtPt;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        assert!(crate::is_correct_in_with("FACTU", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("FACTU", lang, &check_caps));
        assert!(crate::is_correct_in_with("FACTO", lang, &check_caps));
        assert!(!crate::is_correct_in_with("FACTO", crate::ProofLang::Pt, &check_caps), "not a Brazilian word");
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

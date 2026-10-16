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

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

pub use suggest::suggest;

/// The Portuguese dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

/// File names of a Portuguese Hunspell dictionary.
const NAMES: &[&str] = &["pt_BR", "pt-BR", "pt"];

/// The Portuguese dictionary: the installed Hunspell `pt_BR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Portuguese spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    if let Some(d) = DICT.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(|e| e.into_inner());
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    slot.get_or_insert(loaded).clone()
}

fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        log::info!("Portuguese spelling: no Hunspell pt_BR dictionary installed");
        return AffixDict::default();
    };
    crate::hunspell::load(&aff, &dic).unwrap_or_else(|e| {
        log::warn!("Portuguese spelling: {e}");
        AffixDict::default()
    })
}

/// Use this Portuguese dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    Ok(())
}

/// Forget the loaded dictionary, so the next check looks for an installed one again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The `.dic` file of the installed Portuguese dictionary, searching the dictionaries folder and
/// the system locations now (File › Options › Proofing shows it, #1063); `None` when there is
/// none. A dictionary given with [`install_dictionary`] has no file and isn't reported.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Is a Portuguese dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Portuguese dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Portuguese? Numbers, single letters, short words in capitals
/// (acronyms: `USP`, `PT`), words with digits and words in other scripts are accepted, and so
/// is every word when no Portuguese dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    if d.is_empty() {
        return true; // no Portuguese dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1 || w.chars().any(|c| c.is_ascii_digit()) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if w.chars().all(|c| !c.is_lowercase()) && n <= 6 {
        return true;
    }
    if !w.chars().any(is_latin_letter) {
        return true; // other scripts: no dictionary
    }
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
}

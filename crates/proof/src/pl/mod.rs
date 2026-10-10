//! Polish proofing.
//!
//! - **Spelling** against the SJP.PL dictionary ([`dictionary`]): ~350k stems with Hunspell-style
//!   affix rules (~4.5 million forms), bundled as `assets/proofing/pl.dic` (under 1 MB, decoded on
//!   first use, ~10 MB in memory). Capitalized stems are names (`Warszawa`); a sentence may start
//!   with any word and headings may be in capitals. Hyphenated compounds are checked part by part
//!   (`biało-czerwony`), foreign names inflected with an apostrophe are accepted
//!   (`Kennedy'ego`), and the user dictionary applies as in English.
//! - **Suggestions** tuned for Polish ([`suggest()`]): missing diacritics first (`zolw` → `żółw`), then
//!   the common spelling confusions (`ż`/`rz`, `u`/`ó`, `h`/`ch`…), a missing space (`napewno` →
//!   `na pewno`), one-letter typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Polish messages.

pub mod grammar;
mod suggest;

use std::sync::OnceLock;

use crate::affix::AffixDict;

pub use suggest::suggest;

static PL_DIC: &[u8] = include_bytes!("../../../../assets/proofing/pl.dic");

/// The bundled SJP.PL dictionary (decoded on first use; empty, with an error logged, if the
/// bundled data were unreadable).
pub fn dictionary() -> &'static AffixDict {
    static D: OnceLock<AffixDict> = OnceLock::new();
    D.get_or_init(|| {
        AffixDict::from_bytes(PL_DIC).unwrap_or_else(|e| {
            log::error!("Polish dictionary: {e}");
            AffixDict::default()
        })
    })
}

/// Endings Polish adds after an apostrophe to foreign names whose last letter isn't pronounced
/// (`Kennedy'ego`, `Wilde'a`, `Shakespeare'owi`).
const APOSTROPHE_ENDINGS: &[&str] = &["a", "ach", "ami", "e", "ego", "em", "emu", "i", "m", "om", "owi", "owie", "u", "y", "ów", "ą", "ę"];

/// Is `c` a letter of the Latin script (the only one the Polish dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Is `word` spelled correctly in Polish? Numbers, single letters, short words in capitals
/// (acronyms: `PKO`, `ZUS`), words with digits and words in other scripts are accepted.
pub fn is_correct(word: &str) -> bool {
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
    let w = w.replace('’', "'");
    if w.contains('-') {
        return w.split('-').all(|p| p.chars().count() <= 1 || known(p));
    }
    known(&w)
}

/// Is one word (no hyphens) in the dictionary, the user dictionary, or a foreign name with a
/// Polish ending after an apostrophe?
fn known(w: &str) -> bool {
    if dictionary().check(w) || crate::in_user_dictionary(&w.to_lowercase()) {
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
    fn dictionary_loads() {
        let d = dictionary();
        assert!(d.len() > 300_000, "{}", d.len());
        assert!(d.rule_count() > 7_000, "{}", d.rule_count());
        assert!(d.comments().iter().any(|c| c.contains("Apache")), "{:?}", d.comments());
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn polish_words_are_correct() {
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
        for w in ["żeka", "kturzy", "napewno", "wogóle", "zolw", "chuśtawka", "bżoza", "warszawa", "niewiem", "pszyjaciel", "Kennedy'xyz"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        assert!(!is_correct("zxqpolskisłowo"));
        crate::add_word("Zxqpolskisłowo");
        assert!(is_correct("zxqpolskisłowo"));
    }
}

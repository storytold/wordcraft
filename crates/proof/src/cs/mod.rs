//! Czech proofing using the installed Hunspell `cs_CZ` dictionary (never bundled).
//! Suggestions use its runtime MAP/REP/TRY/KEY tables. Generic grammar checks have Czech
//! messages; hyphenation keeps the existing fallback, without Czech pattern data.

pub mod grammar;
pub(crate) mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of a Czech Hunspell dictionary.
const NAMES: &[&str] = &["cs_CZ", "cs-CZ", "cs"];

/// The Czech dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Czech");

/// The Czech dictionary: the installed Hunspell `cs_CZ` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Czech spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Czech dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Czech dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Is `word` spelled correctly in Czech? Numbers, single letters, short words in capitals
/// (acronyms: `ČR`, `EU`), words with digits and words in other scripts are accepted, and so
/// is every word when no Czech dictionary is installed.
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
    fn dictionary_lookup_accepts_czech_file_names() {
        let dir = std::env::temp_dir().join(format!("wordcraft-cs-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["cs_CZ", "cs-CZ", "cs"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, NAMES).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "městě"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn czech_words_affixes_chains_case_and_diacritics() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert_eq!(d.len(), 20);
        assert_eq!(d.rule_count(), 8);
        for w in [
            "město",
            "města",
            "městě",
            "neměsta",
            "rychle",
            "rychlejší",
            "učitel",
            "učitelé",
            "Praha",
            "Praze",
            "Město",
            "POČÍTAČ",
            "příliš",
            "žluťoučký",
            "kůň",
            "úpěl",
            "ďábelské",
            "ódy",
            "děkujeme",
            "zpráva",
            "zprava",
            "škola-počítač",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["městu", "městoě", "rychleejší", "učiteléé", "praha", "mĚsto", "pocitac", "dekujem", "škola-pocitac"] {
            assert!(!is_correct(w), "{w}");
        }
        for c in "ěščřžýáíéůú".chars() {
            assert!(test_dict::DIC.contains(c), "UTF-8 fixture covers {c}");
        }
    }

    #[test]
    fn forbidden_words_are_rejected_in_every_case() {
        test_dict::install();
        for w in ["zqzakázané", "Zqzakázané", "ZQZAKÁZANÉ"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn missing_dictionary_and_ignored_inputs_are_quiet() {
        let empty = AffixDict::default();
        assert!(is_correct_with(&empty, "pocitac"));
        assert!(install_dictionary("COMPOUNDFLAG X\n", "1\nstrom\n").is_err());
        let d = AffixDict::from_hunspell(test_dict::AFF, test_dict::DIC, &[]).unwrap();
        for w in ["", "a", "2026", "ČR", "článek2", "日本語"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zqčeskévlastníslovo"));
        crate::add_word("Zqčeskévlastníslovo");
        assert!(is_correct("zqčeskévlastníslovo"));
    }

    #[test]
    fn czech_dispatch_messages_options_and_mixed_spans() {
        test_dict::install();
        assert!(crate::has_dictionary(Some("cs-CZ")));
        assert!(crate::is_correct_for("městě", Some("cze")));
        let text = "Město pocitac";
        let issues = crate::check_text(text, &[(0..text.len(), crate::ProofLang::Cs)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "pocitac");
        assert_eq!(issues[0].message, "Možná pravopisná chyba");
        assert_eq!(issues[0].lang, crate::ProofLang::Cs);
        assert!(crate::suggest_in("pocitac", 8, crate::ProofLang::Cs).contains(&"počítač".into()));
        let text = "Město this is is wrong";
        let issues = crate::check_text(text, &[(0..6, crate::ProofLang::Cs), (7..text.len(), crate::ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == crate::ProofLang::En && i.message.starts_with("Repeated word")));
        assert!(issues.iter().all(|i| i.start >= 7));
        let opts = crate::Options { ignore_uppercase: false, ..Default::default() };
        assert!(!crate::is_correct_in_with("ZQZAKÁZANÉ", crate::ProofLang::Cs, &opts));
    }

    /// With Ignore words in UPPERCASE or Ignore words that contain numbers off, Czech checks
    /// those words against the dictionary instead of accepting them.
    #[test]
    fn proofing_options_reach_the_czech_check() {
        test_dict::install();
        let lang = crate::ProofLang::Cs;
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        assert!(crate::is_correct_in_with("ŠKULA", lang, &defaults), "short capitals are acronyms by default");
        assert!(!crate::is_correct_in_with("ŠKULA", lang, &check_caps));
        assert!(crate::is_correct_in_with("ŠKOLA", lang, &check_caps));
        assert!(crate::is_correct_in_with("škula2", lang, &defaults));
        assert!(!crate::is_correct_in_with("škula2", lang, &check_digits));
        assert!(crate::is_correct_in_with("2026", lang, &check_digits), "numbers alone stay correct");
        assert!(is_correct("ŠKULA"), "direct checks keep the defaults");
    }
}

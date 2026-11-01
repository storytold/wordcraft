//! French proofing.
//!
//! - **Spelling** against the Hunspell French dictionary installed on the system (found by
//!   [`crate::hunspell`]: `fr_FR.aff`/`fr_FR.dic`, else `fr` or a regional one, from the OS, a
//!   package manager or LibreOffice; it isn't bundled — the upstream files are MPL 2.0).
//!   Without it French text gets no spelling marks. The dictionary itself knows elisions
//!   (`l'arbre`, `qu'elle`, `jusqu'à`) and both apostrophes (`’` is read as `'`); hyphenated
//!   compounds it doesn't list are checked part by part (`va-t-il`); the user dictionary applies
//!   as in English, after an elision too.
//! - **Suggestions** tuned for French ([`suggest`]): missing accents first (`ecole` → `école`),
//!   then the dictionary's `REP` confusions (`soeur` → `sœur`), a missing space, one-letter
//!   typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): French typography and a few reliable rules, with French messages.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::{suggest, suggest_in};

/// File names of a French Hunspell dictionary: France first, then the generic and regional ones.
const NAMES: &[&str] = &["fr_FR", "fr", "fr-FR", "fr_BE", "fr_CA", "fr_CH", "fr_LU", "fr_MC"];

/// Both French regions use the same installed dictionary and reload generation.
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "French");

/// The installed Hunspell dictionary, one given with [install_dictionary], or an empty one.
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this French dictionary from now on. An invalid replacement preserves the current one
/// and its diagnostic; a successful replacement clears the previous load error.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)?;
    crate::hunspell::clear_load_error(DICT.name());
    Ok(())
}

/// Forget the loaded dictionary so the next check searches the configured folder again.
pub fn reload_dictionary() {
    DICT.reload();
}

/// The installed dictionary's file, or None for an in-memory replacement or no installed file.
pub fn installed_path() -> Option<std::path::PathBuf> {
    DICT.installed_path()
}

/// Is a French dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Is `c` a letter of the Latin script (the only one the French dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// Words that elide before a vowel or a mute `h` (`l'`, `qu'`, `jusqu'`…), lower case, without
/// the apostrophe. After one, a word of the user dictionary is accepted (`l'Wordcraft`).
const ELISIONS: &[&str] = &["l", "d", "j", "m", "t", "s", "n", "c", "qu", "jusqu", "lorsqu", "puisqu", "quoiqu", "presqu", "quelqu"];

/// Is `word` spelled correctly in French? Numbers, single letters, short words in capitals
/// (acronyms: `SNCF`, `ONU`), words with digits and words in other scripts are accepted, and so
/// is every word when no French dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices. Disabling either ignore
/// option checks the corresponding words against the French dictionary.
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
        return true; // no French dictionary installed: no spelling marks
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && !crate::is_combining_mark(c) && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']).replace('’', "'");
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
    if known(d, &w) {
        return true;
    }
    if let Some((head, tail)) = w.split_once('\'')
        && ELISIONS.contains(&head.to_lowercase().as_str())
        && !tail.is_empty()
        && !tail.contains('\'')
        && crate::in_user_dictionary(&tail.to_lowercase())
    {
        return true;
    }
    // Compounds the dictionary doesn't list (`va-t-il`, `maison-école`): every part.
    w.contains('-') && w.split('-').all(|p| !p.is_empty() && (p.chars().count() <= 1 || known(d, p)))
}

/// Is `w` in the dictionary (any elision included) or the user dictionary?
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
        assert!(d.len() > 100, "{}", d.len());
        assert!(d.rule_count() >= 20, "the test dictionary's affix rules");
        assert!(!d.rep().is_empty() && !d.map().is_empty() && !d.try_chars().is_empty());
    }

    #[test]
    fn regional_dictionary_files_are_found() {
        // Without fr_FR, a Belgian, Canadian or Swiss dictionary is used.
        let dir = std::env::temp_dir().join(format!("wc-fr-ca-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("fr_CA.aff"), test_dict::AFF).expect("aff");
        std::fs::write(dir.join("fr_CA.dic"), test_dict::DIC).expect("dic");
        let found = crate::hunspell::find_in(&dir, NAMES);
        let ok = found.as_ref().is_some_and(|(a, d)| a.ends_with("fr_CA.aff") && d.ends_with("fr_CA.dic"));
        assert!(ok, "{found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_dictionary_nothing_is_flagged() {
        let none = AffixDict::default();
        for w in ["école", "l'arbre", "xqzv", ""] {
            assert!(is_correct_with(&none, w), "{w}");
        }
        assert!(install_dictionary("COMPOUNDFLAG X\n", "1\nchat\n").is_err(), "unsupported files are refused");
    }

    #[test]
    fn french_words_are_correct() {
        test_dict::install();
        for w in [
            "maison",
            "maisons",
            "école",
            "écoles",
            "grand",
            "grande",
            "grands",
            "grandes",
            "aimer",
            "aime",
            "aimons",
            "aiment",
            "suis",
            "est",
            "sommes",
            "sont",
            "a",
            "avons",
            "ont",
            "Paris",
            "Montréal",
            "œuvre",
            "sœur",
            "aujourd'hui",
            "aujourd’hui",
            "peut-être",
            "arc-en-ciel",
            "km",
            "Maison",
            "MAISON",
            "SNCF",
            "2026",
            "très",
            "rivière",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn elisions_follow_the_dictionary() {
        test_dict::install();
        for w in ["l'arbre", "l’arbre", "L'arbre", "d'enfants", "l'homme", "qu'elle", "qu’il", "j'aime", "j’ai", "d'avoir", "l'école", "jusqu'à"]
        {
            assert!(is_correct(w), "{w}");
        }
        // The dictionary decides: no elision before an aspirated h or a consonant, none on a plural.
        for w in ["l'hasard", "l'chat", "l'arbres", "l'arbrre", "qu'maison"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    /// File › Options must reach both French regions and the actual spelling issues.
    #[test]
    fn proofing_options_reach_the_french_check() {
        test_dict::install();
        let defaults = crate::Options::default();
        let check_caps = crate::Options { ignore_uppercase: false, ..defaults };
        let check_digits = crate::Options { ignore_numbers: false, ..defaults };
        let strict = crate::Options { ignore_uppercase: false, ignore_numbers: false, ..defaults };
        for lang in [crate::ProofLang::Fr, crate::ProofLang::FrCa] {
            assert!(crate::is_correct_in_with("SNCF", lang, &defaults));
            assert!(!crate::is_correct_in_with("SNCF", lang, &check_caps));
            assert!(crate::is_correct_in_with("MAISON", lang, &check_caps));
            assert!(crate::is_correct_in_with("mp3x", lang, &defaults));
            assert!(!crate::is_correct_in_with("mp3x", lang, &check_digits));
            assert!(crate::is_correct_in_with("2026", lang, &strict), "numbers alone stay correct");
            assert!(crate::is_correct_in_with("SNCF", lang, &check_digits), "the uppercase option is independent");
            assert!(crate::is_correct_in_with("mp3x", lang, &check_caps), "the numbers option is independent");
            let text = "école SNCF mp3x";
            let spans = [(0..text.len(), lang)];
            assert!(crate::check_text_with(text, &spans, &defaults, false, false).is_empty());
            let issues = crate::check_text_with(text, &spans, &strict, false, false);
            let flagged: Vec<_> = issues.iter().map(|i| (&text[i.start..i.end], i.lang)).collect();
            assert_eq!(flagged, [("SNCF", lang), ("mp3x", lang)]);
        }
        assert!(!is_correct_with_options("SNCF", &check_caps));
        assert!(!is_correct_with_options("mp3x", &check_digits));
        assert!(is_correct("SNCF") && is_correct("mp3x"), "direct checks keep the defaults");
        assert!(check(&AffixDict::default(), "SNCF", &strict), "missing tools produce no spelling marks");
    }

    #[test]
    fn misspellings_are_flagged() {
        test_dict::install();
        for w in ["maisonn", "ecole", "arbrre", "beaucoups", "aimmer", "soeur", "Km", "grandd", "aim"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn compounds_are_checked_part_by_part() {
        test_dict::install();
        assert!(is_correct("maison-école") && is_correct("est-il") && is_correct("a-t-il"));
        assert!(!is_correct("maison-ecole"));
    }

    #[test]
    fn user_dictionary_applies_after_an_elision_too() {
        test_dict::install();
        assert!(!is_correct("zxqmotfrancais") && !is_correct("l'zxqmotfrancais"));
        crate::add_word("Zxqmotfrancais");
        assert!(is_correct("zxqmotfrancais") && is_correct("l'zxqmotfrancais") && is_correct("d’zxqmotfrancais"));
    }

    /// The French dictionary installed on this machine, when there is one (Dicollecte `fr_FR`, as
    /// Debian's `hunspell-fr` installs it); the test passes without checking anything otherwise.
    fn system_dictionary() -> Option<AffixDict> {
        let dir = std::path::Path::new("/usr/share/hunspell");
        let (aff, dic) = crate::hunspell::find_in(dir, &["fr_FR"])?;
        Some(crate::hunspell::load(&aff, &dic).expect("the installed fr_FR dictionary loads"))
    }

    #[test]
    fn installed_french_dictionary_checks_real_text() {
        let Some(d) = system_dictionary() else { return };
        assert!(d.len() > 50_000, "{}", d.len());
        for w in [
            "maison",
            "maisons",
            "Paris",
            "mangeons",
            "finissaient",
            "étaient",
            "l'arbre",
            "l’arbre",
            "d'enfants",
            "qu'elles",
            "j'aime",
            "s'il",
            "aujourd'hui",
            "suis",
            "est",
            "sommes",
            "allons",
            "vais",
            "irons",
            "belle",
            "beaux",
            "heureuse",
            "Allemagne",
            "l'Allemagne",
            "chevaux",
            "œuvre",
            "écrivit",
            "c'est",
            "n'est",
            "jusqu'à",
            "lorsqu'il",
            "Maison",
            "MAISON",
            "m'appelle",
            "t'aime",
            "l'homme",
            "d'une",
            "qu'on",
            "prends",
            "voudrions",
            "peut-être",
            "va-t-il",
        ] {
            assert!(is_correct_with(&d, w), "{w}");
        }
        for w in [
            "mangons",
            "enfans",
            "arbrre",
            "beaucoups",
            "l'arbrre",
            "écrivut",
            "maisonn",
            "chevals",
            "heureusse",
            "étaitent",
            "jaime",
            "sommmes",
            "Parris",
            "finisaient",
        ] {
            assert!(!is_correct_with(&d, w), "{w}");
        }
        let first = |w: &str| suggest::suggest_in(&d, w, 5);
        assert!(first("maisonn").contains(&"maison".to_string()), "{:?}", first("maisonn"));
        assert!(first("ecole").contains(&"école".to_string()), "{:?}", first("ecole"));
    }

    #[test]
    fn suggestions_on_the_installed_dictionary_are_fast_enough() {
        let Some(d) = system_dictionary() else { return };
        let words = ["maisonn", "ecole", "beaucoups", "l'arbrre", "anticonstitutionelement", "xqzvwemotlong", "finisaient", "Parris"];
        let _ = suggest::suggest_in(&d, "bien", 1);
        let t = std::time::Instant::now();
        for w in words {
            let _ = suggest::suggest_in(&d, w, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("fr_FR: {} suggestion calls in {ms:.0} ms ({:.0} ms each)", words.len(), ms / words.len() as f64);
        assert!(ms < 30_000.0, "{ms} ms");
        // NOSUGGEST entries of the real dictionary (unit symbols) are correct but stay out.
        for (typo, unit) in [("dastt", "dast"), ("cgrr", "cgr")] {
            assert!(is_correct_with(&d, unit), "{unit}");
            let s = suggest::suggest_in(&d, typo, 8);
            assert!(!s.iter().any(|x| x == unit), "{typo} → {s:?}");
        }
        // `bar` is also an ordinary noun (only its unit entry is NOSUGGEST): still suggested.
        let s = suggest::suggest_in(&d, "barr", 8);
        assert!(s.iter().any(|x| x == "bar"), "barr → {s:?}");
    }
}

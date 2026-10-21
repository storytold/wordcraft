//! Catalan spelling and suggestions from an installed Hunspell dictionary. No dictionary data,
//! grammar rules or Catalan hyphenation patterns are bundled.

#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::affix::AffixDict;

static DICT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);
const NAMES: &[&str] = &["ca_ES", "ca-ES", "ca"];

/// The installed dictionary, cached on first use; empty when absent or unreadable.
pub fn dictionary() -> Arc<AffixDict> {
    if let Some(d) = DICT.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(|e| e.into_inner());
    slot.get_or_insert(loaded).clone()
}

fn load_installed() -> AffixDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        crate::hunspell::clear_load_error(NAMES[0]);
        return AffixDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, crate::hunspell::load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Catalan spelling: {e}");
        AffixDict::default()
    })
}

/// Replace the in-memory dictionary with decoded Hunspell text. Invalid input preserves it.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    Ok(())
}

/// Search again on the next check after the dictionaries folder changes.
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The installed `.dic` path for File › Options › Proofing, if present.
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Whether Catalan spelling is available.
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Check complete apostrophized and middle-dot words against the dictionary's rules. Hyphens
/// also allow known components, as Hunspell's default break rules do; forbidden forms stay out.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with an explicit dictionary; absent dictionaries and words without Latin
/// letters produce no spelling marks, matching the other installed-dictionary languages.
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    let has_latin = word.chars().any(|c| c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c)));
    if d.is_empty() || !has_latin {
        return true;
    }
    if word.len() > crate::affix::MAX_WORD_BYTES {
        return known(d, word);
    }
    let mut remaining = 128;
    check_parts(d, word, &mut remaining)
}

/// Bound the entire search, including failed splits. Complete listed components win before
/// subdivision (`nord-est-prova`). Boundary hyphens never bypass a forbidden core, even where
/// Hunspell could accept a different segmentation of a trailing-hyphen token.
fn check_parts(d: &AffixDict, word: &str, remaining: &mut usize) -> bool {
    if *remaining == 0 {
        return false;
    }
    *remaining -= 1;
    if known(d, word) {
        return true;
    }
    if !word.contains('-') || d.forbids(word) {
        return false;
    }
    let core = word.trim_matches('-');
    if core != word {
        return check_parts(d, core, remaining);
    }
    for (i, _) in word.match_indices('-') {
        let (Some(left), Some(right)) = (word.get(..i), word.get(i + 1..)) else { continue };
        if check_parts(d, left, remaining) && check_parts(d, right, remaining) {
            return true;
        }
    }
    false
}

fn known(d: &AffixDict, word: &str) -> bool {
    d.check(word) || crate::in_user_dictionary(&word.to_lowercase())
}

/// Dictionary-driven suggestions, using its input conversion and MAP/REP/KEY/TRY tables.
/// The shared ranker handles single-character MAP groups; Hunspell's multi-character MAP
/// alternatives are not expanded. Suggestions are checked against the dictionary before use.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_with(&dictionary(), word, max)
}

/// [`suggest`] with an explicit dictionary.
pub fn suggest_with(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::cs::suggest::suggest_in(d, &d.iconv(word), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelling_suggestions_and_failed_install_preserve_the_dictionary() {
        test_dict::install();
        for word in [
            "català",
            "Català",
            "col·legi",
            "il·lusió",
            "qüestió",
            "abraçar",
            "l'home",
            "l’home",
            "dóna'm",
            "dóna’m",
            "nord-est",
            "prova-home",
            "prova--home",
            "nord-est-prova",
            "nord-est-",
            "xy",
        ] {
            assert!(is_correct(word), "{word}");
        }
        for word in
            ["cataln", "col·legii", "qüestio", "l'qzx", "home-home", "home-homes", "-home-home", "home-home-", "Home-home", "HOME-HOME", "zqprohibit"]
        {
            assert!(!is_correct(word), "{word}");
        }
        for (word, want) in [("cataln", "català"), ("qüestio", "qüestió"), ("col·legii", "col·legi"), ("l'hmoe", "l'home")] {
            assert!(suggest(word, 8).iter().any(|s| s == want), "{word}: {:?}", suggest(word, 8));
        }
        assert!(install_dictionary("FLAG num\n", "1\nbad\n").is_err());
        assert!(is_correct("català"));
        assert!(suggest("", 4).is_empty());
        assert!(suggest("cataln", 0).is_empty());
        assert!(suggest(&"à".repeat(41), 4).is_empty());
        assert!(is_correct_with(&AffixDict::default(), "qzx"));
        assert!(suggest_with(&AffixDict::default(), "qzx", 8).is_empty());
        assert!(!suggest("zqocultt", 8).iter().any(|s| s == "zqocult"));
    }

    #[test]
    fn other_scripts_are_unmarked_but_latin_typos_are_checked() {
        let d = AffixDict::from_hunspell(test_dict::AFF, test_dict::DIC, &[]).unwrap();
        for word in ["Москва", "привіт", "Ελλάδα", "日本語"] {
            assert!(is_correct_with(&d, word), "{word}");
        }
        for word in ["cataln", "qzxМосква", "ḍqzx"] {
            assert!(!is_correct_with(&d, word), "{word}");
        }
        assert!(is_correct_with(&d, "català"));
        test_dict::install();
        let text = "català Москва Ελλάδα cataln";
        let issues = crate::check_spelling_in(text, crate::ProofLang::Ca);
        assert_eq!(issues.iter().map(|issue| &text[issue.start..issue.end]).collect::<Vec<_>>(), ["cataln"]);
    }

    #[test]
    fn dispatch_preserves_whole_middle_dot_words_and_other_languages() {
        test_dict::install();
        let text = "Col·legi il·lusió l’home qüestio";
        let issues = crate::check_text(text, &[(0..text.len(), crate::ProofLang::Ca)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "qüestio");
        let text = "col·legii";
        let issues = crate::check_spelling_in(text, crate::ProofLang::Ca);
        assert_eq!(issues.len(), 1);
        assert_eq!(&text[issues[0].start..issues[0].end], text);
        assert_eq!(crate::words("col·legi"), [(0, 3), (5, 9)], "other languages keep the existing tokenizer");
        assert!(crate::check_grammar_in("català català  català", crate::ProofLang::Ca).is_empty());
        let text = "català this is is wrong";
        let issues = crate::check_text(text, &[(0..7, crate::ProofLang::Ca), (8..text.len(), crate::ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == crate::ProofLang::En && i.message.starts_with("Repeated word")));
        assert!(issues.iter().all(|i| i.start >= 8));
        assert!(!crate::is_correct_for("cataln", Some("ca-ES")));
    }

    #[test]
    fn dictionary_aliases_load_the_same_original_fixture() {
        let dir = std::env::temp_dir().join(format!("wordcraft-ca-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in NAMES {
            let folder = dir.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(folder.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&folder, NAMES).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "col·legi"));
        }
    }
}

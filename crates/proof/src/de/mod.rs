//! German spelling against installed Hunspell dictionaries, preserving the regional standard.
//!
//! Germany, Austria and Switzerland load independently. No dictionary data is bundled; an
//! unavailable dictionary leaves its region unmarked. Swiss spelling uses `ss`, never `ß`.
//! Grammar and dedicated German hyphenation patterns are future work.

mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::{Arc, RwLock};

use crate::{ProofLang, affix::AffixDict};

pub use suggest::{suggest, suggest_with};

static DE: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);
static AT: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);
static CH: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

fn slot(lang: ProofLang) -> &'static RwLock<Option<Arc<AffixDict>>> {
    match lang {
        ProofLang::DeAt => &AT,
        ProofLang::DeCh => &CH,
        _ => &DE,
    }
}

/// Regional filenames, including the frami packaging names. Never cross regional boundaries.
fn names(lang: ProofLang) -> &'static [&'static str] {
    match lang {
        ProofLang::DeAt => &["de_AT", "de_AT_frami", "de-AT"],
        ProofLang::DeCh => &["de_CH", "de_CH_frami", "de-CH"],
        _ => &["de_DE", "de_DE_frami", "de-DE", "de"],
    }
}

/// The installed dictionary for `lang` (loaded once), or an empty dictionary if unavailable.
pub fn dictionary(lang: ProofLang) -> Arc<AffixDict> {
    if let Some(d) = slot(lang).read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return d.clone();
    }
    let loaded = crate::hunspell::find(names(lang)).and_then(|(aff, dic)| match crate::hunspell::load(&aff, &dic) {
        Ok(d) => Some(d),
        Err(e) => {
            log::warn!("German spelling ({}): {e}", lang.code());
            None
        }
    });
    let loaded = Arc::new(loaded.unwrap_or_default());
    slot(lang).write().unwrap_or_else(|e| e.into_inner()).get_or_insert(loaded).clone()
}

/// Install UTF-8-decoded Hunspell text for one German region. A bad file preserves the old one.
pub fn install_dictionary(lang: ProofLang, aff: &str, dic: &str) -> Result<(), String> {
    if !matches!(lang, ProofLang::De | ProofLang::DeAt | ProofLang::DeCh) {
        return Err("a German proofing language is required".into());
    }
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *slot(lang).write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    Ok(())
}

/// Reload this region on its next use, after the dictionaries folder or installed files change.
pub fn reload_dictionary(lang: ProofLang) {
    *slot(lang).write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Is spelling available for this German region?
pub fn has_dictionary(lang: ProofLang) -> bool {
    !dictionary(lang).is_empty()
}

/// Check a German word, accepting foreign scripts, numbers and an unavailable dictionary.
pub fn is_correct(word: &str, lang: ProofLang) -> bool {
    is_correct_with(&dictionary(lang), word, lang)
}

/// Check against a supplied regional dictionary, retaining noun capitalization and KEEPCASE.
pub fn is_correct_with(d: &AffixDict, word: &str, lang: ProofLang) -> bool {
    if d.is_empty() {
        return true;
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric());
    if lang == ProofLang::DeCh && w.contains(['ß', 'ẞ']) {
        return false;
    }
    if w.chars().count() <= 1 || w.chars().any(|c| c.is_ascii_digit()) || !w.chars().any(|c| c.is_alphabetic() && c < '\u{0250}') {
        return true;
    }
    // A common Swiss noun requested in #2087 but absent in some frami releases. This is our
    // own small supplement, not dictionary data copied from another product.
    let swiss_extra = lang == ProofLang::DeCh && matches!(w, "Rüebli" | "RÜEBLI");
    swiss_extra || d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_find_only_their_own_dictionary() {
        let dir = std::env::temp_dir().join(format!("wc-de-regions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("de_DE.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(dir.join("de_DE.dic"), "1\nStraße\n").unwrap();
        assert!(crate::hunspell::find_in(&dir, names(ProofLang::De)).is_some());
        assert!(crate::hunspell::find_in(&dir, names(ProofLang::DeCh)).is_none());
        assert!(crate::hunspell::find_in(&dir, names(ProofLang::DeAt)).is_none());
        for (lang, name) in [(ProofLang::DeCh, "de_CH_frami"), (ProofLang::DeAt, "de_AT_frami")] {
            std::fs::write(dir.join(format!("{name}.aff")), "SET UTF-8\n").unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), "1\nregional\n").unwrap();
            let (aff, _) = crate::hunspell::find_in(&dir, names(lang)).unwrap();
            assert!(aff.ends_with(format!("{name}.aff")));
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn regional_spelling_and_compounds() {
        let de = test_dict::dictionary(ProofLang::De);
        let ch = test_dict::dictionary(ProofLang::DeCh);
        for w in ["Strasse", "gross", "Fussball", "Bundesverwaltungsgericht", "Velofahrer", "Rüebli"] {
            assert!(is_correct_with(&ch, w, ProofLang::DeCh), "Swiss: {w}");
        }
        for w in ["Straße", "groß", "Haustürschlüssel"] {
            assert!(is_correct_with(&de, w, ProofLang::De), "German: {w}");
        }
        for w in ["Straße", "groß", "STRASSEẞ", "strasse", "rüebli", "Velofahrre"] {
            assert!(!is_correct_with(&ch, w, ProofLang::DeCh), "Swiss typo: {w}");
        }
        assert!(!is_correct_with(&de, "haus", ProofLang::De));
        assert!(is_correct_with(&de, "Groß", ProofLang::De));
        assert!(is_correct_with(&ch, "Gross", ProofLang::DeCh));
        assert!(is_correct_with(&AffixDict::default(), "groß", ProofLang::DeCh), "missing tools leave text unmarked");
    }

    proptest::proptest! {
        #[test]
        fn german_input_never_panics(word in "\\PC{0,80}") {
            let ch = test_dict::dictionary(ProofLang::DeCh);
            let _ = is_correct_with(&ch, &word, ProofLang::DeCh);
            let suggestions = suggest_with(&ch, &word, 5, ProofLang::DeCh);
            proptest::prop_assert!(suggestions.len() <= 5);
            proptest::prop_assert!(suggestions.iter().all(|s| !s.contains(['ß', 'ẞ'])));
        }
    }
}

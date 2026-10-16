//! Norwegian spelling from installed Hunspell dictionaries; no dictionary data is bundled.
//! Bokmål and Nynorsk have independent lazy caches. Missing dictionaries disable spelling.

use crate::{
    ProofLang,
    affix::{AffixDict, Casing, capitalize, casing},
};
use std::sync::{Arc, RwLock};

static NB: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);
static NN: RwLock<Option<Arc<AffixDict>>> = RwLock::new(None);

fn slot(lang: ProofLang) -> Option<&'static RwLock<Option<Arc<AffixDict>>>> {
    match lang {
        ProofLang::Nb => Some(&NB),
        ProofLang::Nn => Some(&NN),
        _ => None,
    }
}

fn names(lang: ProofLang) -> Option<&'static [&'static str]> {
    match lang {
        ProofLang::Nb => Some(&["nb_NO", "nb-NO", "nb", "no_NO", "no"]),
        ProofLang::Nn => Some(&["nn_NO", "nn-NO", "nn"]),
        _ => None,
    }
}

/// The installed file shown by Options > Proofing > Dictionaries.
pub fn installed_path(lang: ProofLang) -> Option<std::path::PathBuf> {
    names(lang).and_then(crate::hunspell::find).map(|(_, dic)| dic)
}

pub fn has_dictionary(lang: ProofLang) -> bool {
    !dictionary(lang).is_empty()
}

/// Load nb_NO or nn_NO on first use, including LibreOffice's shared dict-no extension.
pub fn dictionary(lang: ProofLang) -> Arc<AffixDict> {
    let Some(slot) = slot(lang) else { return Arc::new(AffixDict::default()) };
    if let Some(d) = slot.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return d.clone();
    }
    let names = names(lang).unwrap_or_default();
    let d = crate::hunspell::find(names)
        .and_then(|(aff, dic)| match crate::hunspell::load(&aff, &dic) {
            Ok(d) => Some(d),
            Err(e) => {
                log::warn!("Norwegian spelling: {e}");
                None
            }
        })
        .unwrap_or_default();
    slot.write().unwrap_or_else(|e| e.into_inner()).get_or_insert(Arc::new(d)).clone()
}

/// Install a user-selected dictionary. An error preserves the previous dictionary.
pub fn install_dictionary(lang: ProofLang, aff: &str, dic: &str) -> Result<(), String> {
    let slot = slot(lang).ok_or("not a Norwegian language")?;
    let d = AffixDict::from_hunspell(aff, dic, &[])?;
    *slot.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    Ok(())
}

/// Forget only this language's cache, so the next check searches the system again.
pub fn reload_dictionary(lang: ProofLang) {
    if let Some(slot) = slot(lang) {
        *slot.write().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

pub fn is_correct(word: &str, lang: ProofLang) -> bool {
    is_correct_with(&dictionary(lang), word)
}

pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    if d.is_empty() {
        return true;
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'' && c != '’');
    if w.chars().count() <= 1 || w.chars().any(char::is_numeric) {
        return true;
    }
    let known = |p: &str| d.check(p) || crate::in_user_dictionary(&p.to_lowercase());
    known(w) || (w.contains('-') && w.split('-').all(known))
}

/// Bounded one-edit suggestions, checked against stems, inflections and compounds.
/// The installed dictionary's TRY alphabet includes æ, ø and å. Preserve input case.
pub fn suggest(word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    suggest_with(&dictionary(lang), word, max)
}

pub fn suggest_with(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    let lower = word.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    if d.is_empty() || chars.is_empty() || chars.len() > 40 || max == 0 {
        return Vec::new();
    }
    let mut alphabet: Vec<char> = d.try_chars().chars().chain("abcdefghijklmnopqrstuvwxyzæøå".chars()).collect();
    alphabet.sort_unstable();
    alphabet.dedup();
    alphabet.truncate(128);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut offer = |v: Vec<char>| {
        let candidate: String = v.into_iter().collect();
        if candidate != lower && seen.insert(candidate.clone()) && d.can_suggest(&candidate) {
            out.push(match casing(word) {
                Casing::Initial => capitalize(&candidate),
                Casing::Upper => candidate.to_uppercase(),
                _ => candidate,
            });
        }
    };
    for i in 0..chars.len() {
        let mut v = chars.clone();
        v.remove(i);
        offer(v);
        if i + 1 < chars.len() {
            let mut v = chars.clone();
            v.swap(i, i + 1);
            offer(v);
        }
        for &c in &alphabet {
            let mut v = chars.clone();
            if let Some(x) = v.get_mut(i) {
                *x = c;
            }
            offer(v);
        }
    }
    for i in 0..=chars.len() {
        for &c in &alphabet {
            let mut v = chars.clone();
            v.insert(i, c);
            offer(v);
        }
    }
    out.truncate(max.min(32));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    const AFF: &str = "SET UTF-8\nCOMPOUNDFLAG z\nCOMPOUNDMIN 4\nSFX A Y 1\nSFX A 0 ene .\n";
    #[test]
    fn suggestions_respect_installed_dictionary_exclusions() {
        let d = AffixDict::from_hunspell("SET UTF-8\nNOSUGGEST X\n", "1\nstavekontroll/X\n", &[]).unwrap();
        assert!(is_correct_with(&d, "stavekontroll"));
        assert!(suggest_with(&d, "stavekontrol", 4).is_empty());
    }

    #[test]
    fn inflections_compounds_case_and_suggestions() {
        let d = AffixDict::from_hunspell(AFF, "6\nbok/Az\nbøker/z\nhylle/z\nstavekontroll\nærleg\nopne\n", &[]).unwrap();
        for w in ["Bok", "bokene", "bøkerhylle", "ærleg", "OPNE"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
        assert!(!is_correct_with(&d, "skriver"));
        assert_eq!(suggest_with(&d, "Stavekontrol", 4), ["Stavekontroll"]);
        assert!(is_correct_with(&AffixDict::default(), "hvasomhelst"));
        assert!(suggest_with(&AffixDict::default(), "feil", 4).is_empty());
    }
    #[test]
    fn language_tags_are_distinct() {
        for tag in ["nb", "nb_NO", "NB-no", "no_NO", "nob", "nor"] {
            assert_eq!(ProofLang::from_tag(Some(tag)), ProofLang::Nb);
        }
        for tag in ["nn", "nn_NO", "NN-no", "no-nn", "nno"] {
            assert_eq!(ProofLang::from_tag(Some(tag)), ProofLang::Nn);
        }
    }
}

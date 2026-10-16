//! Bounded German spelling candidates, with regional sharp-s and umlaut corrections first.

use std::collections::HashSet;

use crate::{
    ProofLang,
    affix::{AffixDict, capitalize},
};

const MAX_LETTERS: usize = 40;
const MAX_CANDIDATES: usize = 8192;

/// Suggestions from the installed regional dictionary, best first.
pub fn suggest(word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    suggest_with(&super::dictionary(lang), word, max, lang)
}

/// Suggestions against a given dictionary; every candidate is valid in the selected region.
pub fn suggest_with(d: &AffixDict, word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    let word = word.trim();
    if max == 0 || d.is_empty() || word.is_empty() || word.chars().count() > MAX_LETTERS || word.contains(char::is_whitespace) {
        return Vec::new();
    }
    let limit = max.min(64);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut offer = |candidate: String| {
        for candidate in [candidate.clone(), capitalize(&candidate), candidate.to_lowercase()] {
            if out.len() >= limit || seen.len() >= MAX_CANDIDATES {
                return;
            }
            if candidate != word && seen.insert(candidate.clone()) && super::is_correct_with(d, &candidate, lang) {
                out.push(candidate);
            }
        }
    };
    offer(word.to_string());
    // Multiple-character equivalents are not representable by the engine's character MAPs.
    for (from, to) in [("ß", "ss"), ("ẞ", "SS"), ("ss", "ß"), ("SS", "ẞ"), ("ae", "ä"), ("oe", "ö"), ("ue", "ü")] {
        if word.contains(from) {
            offer(word.replace(from, to));
        }
    }
    for (from, to) in d.rep().iter().take(256) {
        if !from.is_empty() && word.contains(from) && to.len() <= crate::affix::MAX_WORD_BYTES {
            offer(word.replacen(from, to, 1));
        }
    }
    let chars: Vec<char> = word.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        // Suggestion tables are external input too. Limit both dimensions independently;
        // reaching the candidate limit must not leave a huge MAP table to walk.
        for group in d.map().iter().take(128) {
            if group.contains(&c) {
                for &replacement in group.iter().take(16) {
                    let mut candidate = chars.clone();
                    if let Some(at) = candidate.get_mut(i) {
                        *at = replacement;
                    }
                    offer(candidate.iter().collect());
                }
            }
        }
    }
    let letters: Vec<char> =
        if d.try_chars().is_empty() { "abcdefghijklmnopqrstuvwxyzäöüß".chars().collect() } else { d.try_chars().chars().take(80).collect() };
    for i in 0..chars.len() {
        let mut dropped = chars.clone();
        dropped.remove(i);
        offer(dropped.iter().collect());
        if i + 1 < chars.len() {
            let mut swapped = chars.clone();
            swapped.swap(i, i + 1);
            offer(swapped.iter().collect());
        }
        for &c in &letters {
            let mut replaced = chars.clone();
            if let Some(at) = replaced.get_mut(i) {
                *at = c;
            }
            offer(replaced.iter().collect());
        }
    }
    for i in 0..=chars.len() {
        for &c in &letters {
            let mut inserted = chars.clone();
            inserted.insert(i, c);
            offer(inserted.iter().collect());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regional_sharp_s_noun_case_and_typo_suggestions() {
        let ch = super::super::test_dict::dictionary(ProofLang::DeCh);
        let de = super::super::test_dict::dictionary(ProofLang::De);
        assert_eq!(suggest_with(&ch, "Straße", 1, ProofLang::DeCh), ["Strasse"]);
        assert_eq!(suggest_with(&ch, "groß", 1, ProofLang::DeCh), ["gross"]);
        assert!(suggest_with(&de, "Strasse", 5, ProofLang::De).contains(&"Straße".into()));
        assert!(suggest_with(&de, "haus", 5, ProofLang::De).contains(&"Haus".into()));
        assert!(suggest_with(&ch, "Velofahrre", 5, ProofLang::DeCh).contains(&"Velofahrer".into()));
        assert!(suggest_with(&ch, "Straße", 64, ProofLang::DeCh).iter().all(|s| !s.contains(['ß', 'ẞ'])));
        assert!(suggest_with(&ch, &"ß".repeat(100), usize::MAX, ProofLang::DeCh).is_empty());
        assert!(suggest_with(&ch, "groß", 0, ProofLang::DeCh).is_empty());
    }
}

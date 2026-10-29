//! Spanish suggestions. The ranking is the dictionary-driven one Czech uses
//! ([`crate::cs::suggest::suggest_in`]: case, MAP — the Spanish dictionaries map each vowel to
//! its accented forms and `n` to `ñ`, so `tambien` → `también` comes first —, REP, missing spaces,
//! KEY, TRY edits and a stem scan); every table comes from the variety's installed dictionary.
//! In Uruguayan Spanish a `vos` form typed without its accent is offered with it first
//! (`veni` → `vení`, `tenes` → `tenés`; [`super::voseo`]).

use crate::ProofLang;
use crate::affix::{AffixDict, Casing, capitalize, casing};
use crate::cs::suggest::MAX_LETTERS;

/// Spelling suggestions for a word in `lang`'s Spanish, best first, at most `max`, in the word's
/// case.
pub fn suggest(word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    suggest_in(&super::dictionary(lang), word, max, lang)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    let mut out = Vec::new();
    let word = word.trim();
    if lang == ProofLang::EsUy
        && !d.is_empty()
        && word.chars().count() <= MAX_LETTERS
        && let Some(v) = super::voseo::accented(&word.to_lowercase(), |inf| d.check(inf))
    {
        out.push(match casing(word) {
            Casing::Upper => v.to_uppercase(),
            Casing::Initial => capitalize(&v),
            _ => v,
        });
    }
    for s in crate::cs::suggest::suggest_in(d, word, max) {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out.truncate(max);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanish_suggestions_use_runtime_tables_and_case() {
        crate::es::test_dict::install();
        for lang in [ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy] {
            for (word, want) in [
                ("tambien", "también"),
                ("cancion", "canción"),
                ("Cancion", "Canción"),
                ("mexico", "México"),
                ("nino", "niño"),
                ("pinguino", "pingüino"),
                ("prueva", "prueba"),
                ("corector", "corrector"),
                ("estoes", "esto es"),
            ] {
                assert!(suggest(word, 8, lang).contains(&want.into()), "{lang:?} {word}: {:?}", suggest(word, 8, lang));
            }
        }
        // The first suggestion for a missing accent is the accented word.
        assert_eq!(suggest("tambien", 4, ProofLang::Es).first().map(String::as_str), Some("también"));
    }

    #[test]
    fn uruguayan_vos_forms_get_their_accent_back_first() {
        crate::es::test_dict::install();
        for (word, want) in [("veni", "vení"), ("tenes", "tenés"), ("Veni", "Vení"), ("TENES", "TENÉS"), ("queres", "querés")] {
            assert_eq!(suggest(word, 4, ProofLang::EsUy).first().map(String::as_str), Some(want), "{word}");
            assert!(!suggest(word, 8, ProofLang::Es419).contains(&want.into()), "{word}: no vos forms outside Uruguay");
        }
    }

    #[test]
    fn suggestions_stay_bounded_and_skip_forbidden_words() {
        crate::es::test_dict::install();
        for lang in [ProofLang::Es, ProofLang::EsUy] {
            for w in ["", "dos palabras", &"a".repeat(41)] {
                assert!(suggest(w, 4, lang).is_empty(), "{w}");
            }
            assert!(suggest("prueva", 0, lang).is_empty());
            assert!(suggest("veni", 0, lang).is_empty());
            assert!(suggest_in(&AffixDict::default(), "veni", 4, lang).is_empty());
            assert!(suggest("prueba", 4, lang).len() <= 4);
            assert!(!suggest("prueba", 4, lang).contains(&"prueba".into()));
            assert!(!suggest("zqprohibda", 8, lang).contains(&"zqprohibida".into()));
        }
    }

    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        crate::affix::tests::assert_nosuggest_is_honoured(|d, w, max| suggest_in(d, w, max, ProofLang::Es));
    }
}

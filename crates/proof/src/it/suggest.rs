//! Italian spelling suggestions, best first: the shared ranking ([`crate::affix_suggest`]) with the
//! words Italian runs into their neighbours (`perme` → `per me`, `amilano` → `a milano`) and
//! letters to try for a dictionary without a TRY line.

use crate::affix::AffixDict;
use crate::affix_suggest::{Split, Tuning};

/// Short words often run into the next one (`per me`, `a Milano`, `di Roma`).
const LEADING_WORDS: &[&str] = &[
    "a", "e", "è", "di", "da", "in", "con", "su", "per", "tra", "fra", "che", "chi", "cui", "non", "si", "ci", "vi", "ne", "più", "meno", "molto",
    "tanto", "come", "dove", "quando", "mai", "qui", "qua", "lì", "là", "ogni", "altro", "tutto", "questo", "quello", "bene", "male", "anche",
    "ancora", "sempre", "già", "poi", "ed", "od", "ad", "un", "uno", "una", "il", "lo", "la", "i", "gli", "le", "del", "al", "dal", "nel", "sul",
    "col", "pel", "allo", "dello", "nello", "sullo", "della", "alla", "dalla", "nella", "sulla", "degli", "agli", "dagli", "negli", "sugli", "delle",
    "alle", "dalle", "nelle", "sulle", "dei", "ai", "dai", "nei", "sui", "coi", "dei",
];
/// Short words often run into the one before (enclitic pronouns: `dimmelo` in writing without
/// the doubling, or a dropped space: `perme` → `per me`).
const TRAILING_WORDS: &[&str] = &[
    "mi", "ti", "si", "ci", "vi", "ne", "lo", "la", "li", "le", "gli", "me", "te", "lui", "lei", "noi", "voi", "loro", "che", "cosa", "come", "dove",
    "è",
];

/// What Italian adds to its dictionary's tables.
const TUNING: Tuning =
    Tuning {
        split: Split::Near { leading: LEADING_WORDS, trailing: TRAILING_WORDS }, alphabet: "aeiortnslcdupmghfbqvàèéìòóùz", ..Tuning::DEFAULT
    };

/// Spelling suggestions for an Italian word, best first, at most `max`, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &TUNING)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::affix_suggest::{fold, weighted_distance};

    fn has(word: &str, want: &str) {
        let s = suggest(word, 8);
        assert!(s.iter().any(|x| x == want), "{word} → {s:?}, want {want}");
    }

    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        crate::affix::tests::assert_nosuggest_is_honoured(suggest_in);
    }

    #[test]
    fn italian_misspellings_get_the_right_suggestion() {
        crate::it::test_dict::install();
        has("caffe", "caffè");
        has("perche", "perché");
        has("cioe", "cioè");
        has("citta", "città");
        has("lbiro", "libro");
        has("parlre", "parlare");
        has("quelo", "quello");
        has("perme", "per me");
        has("amilano", "a Milano");
        has("roma", "Roma");
        has("Caffe", "Caffè");
    }

    #[test]
    fn the_likeliest_fix_comes_first() {
        crate::it::test_dict::install();
        assert_eq!(suggest("caffe", 3).first().map(String::as_str), Some("caffè"));
        assert_eq!(suggest("perche", 3).first().map(String::as_str), Some("perché"));
    }

    #[test]
    fn edge_cases_stay_empty_or_bounded() {
        crate::it::test_dict::install();
        assert!(suggest("", 5).is_empty());
        assert!(suggest("bene", 0).is_empty());
        assert!(suggest(&"a".repeat(41), 5).is_empty());
        assert!(suggest("due parole", 5).is_empty());
        assert!(suggest_in(&crate::affix::AffixDict::default(), "caffe", 5).is_empty());
        assert!(suggest("caffè", 5).len() <= 5);
        assert!(!suggest("caffè", 5).contains(&"caffè".to_string()));
    }

    #[test]
    fn distance_weighs_accents_half() {
        let map = vec![vec!['a', 'à'], vec!['e', 'é', 'è']];
        let related = |a: char, b: char| fold(a, &map) == fold(b, &map);
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(weighted_distance(&c("caffe"), &c("caffè"), &related, 9), 1);
        assert_eq!(weighted_distance(&c("citta"), &c("città"), &related, 9), 1);
        assert_eq!(weighted_distance(&c("bene"), &c("bene"), &related, 9), 0);
        assert_eq!(weighted_distance(&c("bene"), &c("male"), &related, 2), 3);
        assert_eq!(weighted_distance(&c("bene"), &c("benissimo"), &related, 2), 3);
        assert_eq!(weighted_distance(&c(""), &c("ab"), &related, 9), 4);
    }

    #[test]
    fn suggestions_are_fast_enough() {
        crate::it::test_dict::install();
        // Warm the dictionary, then time a worst case (a long word nothing near): the scan round.
        let _ = suggest("bene", 1);
        let t = std::time::Instant::now();
        for w in ["xqzvweitalianaparola", "prsidenti", "parallellepipedo"] {
            let _ = suggest(w, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("3 suggestion calls: {ms:.0} ms");
        assert!(ms < 20_000.0, "{ms} ms");
    }
}

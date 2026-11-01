//! Portuguese spelling suggestions, best first: the shared ranking ([`crate::affix_suggest`]) with
//! the words Portuguese runs into their neighbours (`nãoé` → `não é`, `paraela` → `para ela`) and
//! letters to try for a dictionary without a TRY line.

use crate::affix::AffixDict;
use crate::affix_suggest::{Split, Tuning};

/// Short words often run into the next one (`não é`, `da água`, `para isso`).
const LEADING_WORDS: &[&str] = &[
    "a", "o", "e", "é", "de", "do", "da", "dos", "das", "em", "no", "na", "nos", "nas", "por", "para", "com", "sem", "sob", "sobre", "ao", "aos",
    "à", "às", "que", "se", "não", "como", "mas", "mais", "já", "até", "entre", "contra", "perante", "desde", "após", "ante", "durante", "mediante",
    "segundo", "conforme", "salvo", "fora", "onde", "quando", "bem", "mal", "pois", "logo", "portanto", "porém", "todavia", "contudo", "assim",
    "tão", "tanto", "tudo", "nada", "algo", "cada", "outro", "mesmo", "próprio", "ainda", "tampouco", "quase", "talvez", "nunca", "sempre",
];
/// Short words often run into the one before (`amo-te`, `deu-lhe`; in writing without the
/// hyphen: `amote`, `deulhe`).
const TRAILING_WORDS: &[&str] = &[
    "me", "te", "se", "lhe", "nos", "vos", "lhes", "o", "a", "os", "as", "ele", "ela", "eles", "elas", "isso", "isto", "aquilo", "mim", "ti", "si",
    "eu", "tu", "que", "de", "em", "é",
];

/// What Portuguese adds to its dictionary's tables.
const TUNING: Tuning = Tuning {
    split: Split::Near { leading: LEADING_WORDS, trailing: TRAILING_WORDS },
    alphabet: "aeiousrntmdlcpvgqbzfhjxçãõáéíóúâêôkw",
    ..Tuning::DEFAULT
};

/// Spelling suggestions for a Portuguese word, best first, at most `max`, in the word's letter case.
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
    fn portuguese_misspellings_get_the_right_suggestion() {
        crate::pt::test_dict::install();
        has("coraçao", "coração");
        has("saude", "saúde");
        has("seje", "seja");
        has("excessão", "exceção");
        has("previlégio", "privilégio");
        has("prsidente", "presidente");
        has("provavel", "provável");
        has("trbalho", "trabalho");
        has("nãoé", "não é");
        has("paraela", "para ela");
        has("brasília", "Brasília");
        has("Coraçao", "Coração");
    }

    #[test]
    fn the_likeliest_fix_comes_first() {
        crate::pt::test_dict::install();
        assert_eq!(suggest("coraçao", 3).first().map(String::as_str), Some("coração"));
        assert_eq!(
            suggest("saude", 3).first().map(String::as_str),
            Some("saúde"),
            "an accent at the first difference beats one further right (saudê is a word too)"
        );
        assert_eq!(suggest("seje", 3).first().map(String::as_str), Some("seja"));
    }

    #[test]
    fn edge_cases_stay_empty_or_bounded() {
        crate::pt::test_dict::install();
        assert!(suggest("", 5).is_empty());
        assert!(suggest("bem", 0).is_empty());
        assert!(suggest(&"a".repeat(41), 5).is_empty());
        assert!(suggest("duas palavras", 5).is_empty());
        assert!(suggest_in(&crate::affix::AffixDict::default(), "coraçao", 5).is_empty());
        assert!(suggest("coração", 5).len() <= 5);
        assert!(!suggest("coração", 5).contains(&"coração".to_string()));
    }

    #[test]
    fn distance_weighs_accents_half() {
        let map = vec![vec!['a', 'á', 'à', 'â', 'ã'], vec!['c', 'ç']];
        let related = |a: char, b: char| fold(a, &map) == fold(b, &map);
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(weighted_distance(&c("coraçao"), &c("coração"), &related, 9), 1);
        assert_eq!(weighted_distance(&c("coracao"), &c("coração"), &related, 9), 2, "ç and ã are two accent slips");
        assert_eq!(weighted_distance(&c("coro"), &c("coro"), &related, 9), 0);
        assert_eq!(weighted_distance(&c("bem"), &c("bom"), &related, 9), 2);
        assert_eq!(weighted_distance(&c("bem"), &c("bemte"), &related, 9), 4);
        assert_eq!(weighted_distance(&c("bem"), &c("ver"), &related, 2), 3);
        assert_eq!(weighted_distance(&c(""), &c("ab"), &related, 9), 4);
    }

    #[test]
    fn suggestions_are_fast_enough() {
        crate::pt::test_dict::install();
        // Warm the dictionary, then time a worst case (a long word nothing near): the scan round.
        let _ = suggest("bem", 1);
        let t = std::time::Instant::now();
        for w in ["xqzvweportuguespalavra", "prsidentes", "paralelepípedo"] {
            let _ = suggest(w, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("3 suggestion calls: {ms:.0} ms");
        assert!(ms < 20_000.0, "{ms} ms");
    }
}

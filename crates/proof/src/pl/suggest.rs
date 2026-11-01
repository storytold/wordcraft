//! Polish spelling suggestions, best first: the shared ranking ([`crate::affix_suggest`]) with the
//! words Polish runs into their neighbours (`napewno` → `na pewno`, `kochamcię` → `kocham cię`),
//! letters to try for a dictionary without a TRY line, and final devoicing (`chlep` → `chleb`).

use crate::affix::AffixDict;
use crate::affix_suggest::{COST_ADD_DIACRITIC, COST_MISTYPE, Split, Tuning};

/// Short words often run into the next one (`na pewno`, `w ogóle`, `nie wiem`, `przede
/// wszystkim`).
const LEADING_WORDS: &[&str] = &[
    "a", "i", "o", "u", "w", "z", "we", "ze", "na", "po", "do", "od", "za", "co", "to", "nie", "jak", "tak", "przy", "bez", "pod", "nad", "dla",
    "że", "by", "czy", "już", "ani", "przed", "przez", "przede", "ku", "aż", "bo", "ten", "ta", "te", "tym", "tej", "też", "jest", "ode", "beze",
    "nade",
];
/// Short words often run into the one before (`kocham cię`, `daj mi`, `bałem się`).
const TRAILING_WORDS: &[&str] =
    &["się", "cię", "mi", "mu", "ci", "go", "ją", "je", "nam", "wam", "im", "by", "że", "no", "mnie", "tobie", "jej", "ich"];

/// What Polish adds to its dictionary's tables.
const TUNING: Tuning = Tuning {
    split: Split::Near { leading: LEADING_WORDS, trailing: TRAILING_WORDS },
    alphabet: "aioeznrwcysptkmdłuljągbhęśćóżfńźvqx",
    mistype_cost,
    ..Tuning::DEFAULT
};

/// Final devoicing makes a voiced consonant at the end of a word sound voiceless, so writing it
/// voiceless (`chlep` for `chleb`) is a likelier slip than any other typo.
fn mistype_cost(chars: &[char], i: usize, c: char) -> u32 {
    let last = i + 1 == chars.len();
    if last && matches!((chars.get(i), c), (Some('p'), 'b') | (Some('t'), 'd') | (Some('k'), 'g')) { COST_ADD_DIACRITIC - 1 } else { COST_MISTYPE }
}

/// Spelling suggestions for a Polish word, best first, at most `max`, in the word's letter case.
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
    fn polish_misspellings_get_the_right_suggestion() {
        crate::pl::test_dict::install();
        has("żeka", "rzeka");
        has("kturzy", "którzy");
        has("napewno", "na pewno");
        has("wogóle", "w ogóle");
        has("niewiem", "nie wiem");
        has("zolw", "żółw");
        has("gzegzolka", "gżegżółka");
        has("chuśtawka", "huśtawka");
        has("bżoza", "brzoza");
        has("pszyjaciel", "przyjaciel");
        has("warszawa", "Warszawa");
        has("Żeka", "Rzeka");
        has("ZOLW", "ŻÓŁW");
        has("konstytucija", "konstytucja");
        has("wziąść", "wziąć");
        has("dziewczyan", "dziewczyna");
    }

    #[test]
    fn voiced_consonant_suggestion_ranks_first() {
        crate::pl::test_dict::install();
        let result = suggest("chlep", 6);
        assert_eq!(result.first().map(String::as_str), Some("chleb"), "{result:?}");
    }

    #[test]
    fn the_likeliest_fix_comes_first() {
        crate::pl::test_dict::install();
        assert_eq!(suggest("kturzy", 3).first().map(String::as_str), Some("którzy"));
        assert_eq!(suggest("zolw", 3).first().map(String::as_str), Some("żółw"));
        assert_eq!(suggest("napewno", 3).first().map(String::as_str), Some("na pewno"));
        assert_eq!(suggest("żeka", 3).first().map(String::as_str), Some("rzeka"));
    }

    #[test]
    fn edge_cases_stay_empty_or_bounded() {
        crate::pl::test_dict::install();
        assert!(suggest("", 5).is_empty());
        assert!(suggest("kot", 0).is_empty());
        assert!(suggest(&"a".repeat(41), 5).is_empty());
        assert!(suggest("dwa słowa", 5).is_empty());
        assert!(suggest("żółw", 5).len() <= 5);
        assert!(!suggest("żółw", 5).contains(&"żółw".to_string()));
    }

    #[test]
    fn distance_weighs_diacritics_half() {
        let map = vec![vec!['a', 'ą'], vec!['o', 'ó', 'u']];
        let related = |a: char, b: char| fold(a, &map) == fold(b, &map);
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(weighted_distance(&c("ktury"), &c("który"), &related, 9), 1);
        assert_eq!(weighted_distance(&c("kot"), &c("kto"), &related, 9), 2);
        assert_eq!(weighted_distance(&c("kot"), &c("kotek"), &related, 9), 4);
        assert_eq!(weighted_distance(&c("kot"), &c("pies"), &related, 2), 3);
        assert_eq!(weighted_distance(&c(""), &c("ab"), &related, 9), 4);
    }

    #[test]
    fn suggestions_are_fast_enough() {
        crate::pl::test_dict::install();
        // Warm the dictionary, then time a worst case (a long word nothing near): the scan round.
        let _ = suggest("kot", 1);
        let t = std::time::Instant::now();
        for w in ["xqzvwpolskiegoslowa", "pszyjacielowi", "zażółćgęśląjaźń"] {
            let _ = suggest(w, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("3 suggestion calls: {ms:.0} ms");
        assert!(ms < 20_000.0, "{ms} ms");
    }
}

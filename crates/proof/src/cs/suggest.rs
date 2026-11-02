//! Czech suggestions: the shared ranking ([`crate::affix_suggest`]) with the installed
//! dictionary's own MAP/REP/TRY/KEY tables and nothing Czech hard-coded.

use crate::affix::AffixDict;
use crate::affix_suggest::Tuning;

/// Spelling suggestions for a Czech word, best first, at most `max`, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &Tuning::DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_are_fast_enough() {
        crate::cs::test_dict::install();
        let _ = suggest("město", 1);
        let t = std::time::Instant::now();
        for word in ["zqčeskévelmidlouhéslovo", "pocitacz", "nejrychllejší"] {
            let _ = suggest(word, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("3 Czech suggestion calls: {ms:.0} ms");
        assert!(ms < 20_000.0, "{ms} ms");
    }

    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        crate::affix::tests::assert_nosuggest_is_honoured(suggest_in);
    }

    #[test]
    fn czech_suggestions_use_runtime_tables_and_case() {
        crate::cs::test_dict::install();
        for (word, want) in [
            ("mesto", "město"),
            ("mňe", "mě"),
            ("xůž", "nůž"),
            ("škloa", "škola"),
            ("pocitac", "počítač"),
            ("Mesto", "Město"),
            ("praha", "Praha"),
            ("toškola", "to škola"),
        ] {
            assert!(suggest(word, 8).contains(&want.into()), "{word}: {:?}", suggest(word, 8));
        }
        // Each table changes the answer without a hard-coded Czech alphabet or confusion list.
        for (table, wrong, right) in
            [("MAP 1\nMAP cč\n", "caj", "čaj"), ("REP 1\nREP xy město\n", "xy", "město"), ("TRY š\n", "kola", "škola"), ("KEY xn\n", "xůž", "nůž")]
        {
            let dic = format!("1\n{right}\n");
            let d = AffixDict::from_hunspell(table, &dic, &[]).unwrap();
            assert!(suggest_in(&d, wrong, 4).contains(&right.into()), "{table}");
            let without = AffixDict::from_hunspell("", &dic, &[]).unwrap();
            assert!(!suggest_in(&without, wrong, 4).contains(&right.into()), "must depend on {table}");
        }
    }

    #[test]
    fn suggestions_stay_bounded_and_skip_forbidden_words() {
        crate::cs::test_dict::install();
        for w in ["", "dvě slova", &"ě".repeat(41)] {
            assert!(suggest(w, 4).is_empty());
        }
        assert!(suggest("mesto", 0).is_empty());
        assert!(suggest_in(&AffixDict::default(), "mesto", 4).is_empty());
        assert!(suggest("město", 4).len() <= 4);
        assert!(!suggest("město", 4).contains(&"město".into()));
        assert!(!suggest("zqzakázane", 8).contains(&"zqzakázané".into()));
    }
}

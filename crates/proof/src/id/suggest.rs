//! Indonesian suggestions. The ranking is the dictionary-driven one Czech uses
//! ([`crate::cs::suggest::suggest_in`]: case, MAP, REP, missing spaces, KEY, TRY edits and a stem
//! scan); every table comes from the installed `id_ID` dictionary, so nothing Indonesian is
//! hard-coded here.

use crate::affix::AffixDict;

/// Spelling suggestions for an Indonesian word, best first, at most `max`, in the word's case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::cs::suggest::suggest_in(d, word, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indonesian_suggestions_use_runtime_tables_and_case() {
        crate::id::test_dict::install();
        for (word, want) in [
            ("rumha", "rumah"),
            ("sekolha", "sekolah"),
            ("Sekola", "Sekolah"),
            ("jakarta", "Jakarta"),
            ("makanna", "makanan"),
            ("djalan", "jalan"),
            ("dsn", "dan"),
            ("sayadan", "saya dan"),
            ("membcaa", "membaca"),
        ] {
            assert!(suggest(word, 8).contains(&want.into()), "{word}: {:?}", suggest(word, 8));
        }
    }

    #[test]
    fn suggestions_stay_bounded_and_skip_forbidden_words() {
        crate::id::test_dict::install();
        for w in ["", "dua kata", &"a".repeat(41)] {
            assert!(suggest(w, 4).is_empty());
        }
        assert!(suggest("rumha", 0).is_empty());
        assert!(suggest_in(&AffixDict::default(), "rumha", 4).is_empty());
        assert!(suggest("rumah", 4).len() <= 4);
        assert!(!suggest("rumah", 4).contains(&"rumah".into()));
        assert!(!suggest("zqterlarag", 8).contains(&"zqterlarang".into()));
    }

    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        crate::affix::tests::assert_nosuggest_is_honoured(suggest_in);
    }
}

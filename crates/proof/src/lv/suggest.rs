//! Latvian suggestions. The ranking is the dictionary-driven one Czech uses
//! ([`crate::cs::suggest::suggest_in`]: case, MAP, REP, missing spaces, KEY, TRY edits and a stem
//! scan); every table comes from the installed `lv_LV` dictionary, so nothing Latvian is
//! hard-coded here.

use crate::affix::AffixDict;

/// Spelling suggestions for an Latvian word, best first, at most `max`, in the word's case.
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
    fn latvian_suggestions_use_runtime_tables_and_case() {
        crate::lv::test_dict::install();
        for (word, want) in [("suuns", "suns"), ("sunss", "suns"), ("mājs", "māja"), ("maja", "māja"), ("Suna", "Suns")] {
            assert!(suggest(word, 8).contains(&want.into()), "{word}: {:?}", suggest(word, 8));
        }
    }

    #[test]
    fn suggestions_stay_bounded_and_skip_forbidden_words() {
        crate::lv::test_dict::install();
        for w in ["", "divi vārdi", &"a".repeat(41)] {
            assert!(suggest(w, 4).is_empty());
        }
        assert!(suggest("suuns", 0).is_empty());
        assert!(suggest_in(&AffixDict::default(), "suuns", 4).is_empty());
        assert!(suggest("suns", 4).len() <= 4);
        assert!(!suggest("suns", 4).contains(&"suns".into()));
        assert!(!suggest("zqterlarang", 8).contains(&"zqterlarangs".into()));
    }

    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        crate::affix::tests::assert_nosuggest_is_honoured(suggest_in);
    }
}

//! Slovak suggestions. The ranking is the dictionary-driven one Czech uses
//! ([`crate::cs::suggest::suggest_in`]: case, MAP, REP, missing spaces, KEY, TRY edits and a stem
//! scan); every table comes from the installed `sk_SK` dictionary, so nothing Slovak is
//! hard-coded here.

use crate::affix::AffixDict;

/// Spelling suggestions for a Slovak word, best first, at most `max`, in the word's case.
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
    fn slovak_suggestions_use_runtime_tables_and_case() {
        crate::sk::test_dict::install();
        for (word, want) in [
            // Diacritics left out, one or several (MAP).
            ("rychly", "rýchly"),
            ("pocitac", "počítač"),
            ("dakujeme", "ďakujeme"),
            ("kon", "kôň"),
            ("maso", "mäso"),
            ("dlzka", "dĺžka"),
            ("vrba", "vŕba"),
            ("lubit", "ľúbiť"),
            // A known confusion (REP).
            ("pjatok", "piatok"),
            // Letter case.
            ("Pocitac", "Počítač"),
            ("bratislava", "Bratislava"),
            // A missing space: the common `nieje` for `nie je`.
            ("nieje", "nie je"),
            // Swapped letters and a keyboard neighbour (KEY).
            ("škloa", "škola"),
            ("xie", "nie"),
        ] {
            assert!(suggest(word, 8).contains(&want.into()), "{word}: {:?}", suggest(word, 8));
        }
    }

    /// The common `sk_SK` has no `MAP` table, only single-letter `REP` pairs: without the Slovak
    /// diacritic groups a word missing two or more diacritics got no suggestion at all.
    #[test]
    fn several_missing_diacritics_without_a_map_table() {
        let aff = "SET UTF-8\nREP 2\nREP c č\nREP i í\n";
        let dic = "4\npočítač\nkôň\ndĺžka\nľúbiť\n";
        let d = AffixDict::from_hunspell(aff, dic, &[]).unwrap().with_default_map(crate::sk::DIACRITICS);
        assert_eq!(d.map().len(), 14, "the Slovak groups are filled in");
        for (word, want) in [("pocitac", "počítač"), ("kon", "kôň"), ("dlzka", "dĺžka"), ("lubit", "ľúbiť")] {
            assert!(suggest_in(&d, word, 8).contains(&want.into()), "{word}: {:?}", suggest_in(&d, word, 8));
        }
        // Without them, as before: REP restores one diacritic, never the three `počítač` needs.
        let plain = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
        assert!(!suggest_in(&plain, "pocitac", 8).contains(&"počítač".into()));
        // A dictionary's own MAP table is kept.
        let own = AffixDict::from_hunspell("MAP 1\nMAP cč\n", "1\nčaj\n", &[]).unwrap().with_default_map(&["aá"]);
        assert_eq!(own.map(), [vec!['c', 'č']]);
    }

    #[test]
    fn suggestions_stay_bounded_and_skip_forbidden_words() {
        crate::sk::test_dict::install();
        for w in ["", "dve slová", &"ô".repeat(41)] {
            assert!(suggest(w, 4).is_empty());
        }
        assert!(suggest("pocitac", 0).is_empty());
        assert!(suggest_in(&AffixDict::default(), "pocitac", 4).is_empty());
        assert!(suggest("mesto", 4).len() <= 4);
        assert!(!suggest("mesto", 4).contains(&"mesto".into()));
        assert!(!suggest("zqzakázane", 8).contains(&"zqzakázané".into()));
    }

    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        crate::affix::tests::assert_nosuggest_is_honoured(suggest_in);
    }
}

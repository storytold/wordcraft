//! Dutch suggestions. The ranking is the dictionary-driven one Czech uses
//! ([`crate::cs::suggest::suggest_in`]: case, MAP, REP, missing spaces, KEY, TRY edits and a stem
//! scan) with every table from the installed `nl_NL` dictionary. The word goes through the
//! dictionary's input conversion first (its stems and REP table spell `ij` as `ĳ`), and the
//! suggestions through its output conversion, so they show `ij` as typed.

use crate::affix::AffixDict;

/// Spelling suggestions for a Dutch word, best first, at most `max`, in the word's case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    let input = d.iconv(word);
    let mut out: Vec<String> = Vec::new();
    // A few spare candidates make up for the ones dropped below.
    for s in crate::cs::suggest::suggest_in(d, &input, max.saturating_add(4)) {
        if out.len() >= max {
            break;
        }
        let s = d.oconv(&s).into_owned();
        if s != word && !out.contains(&s) && !stray(&s) {
            out.push(s);
        }
    }
    out
}

/// Is `s` a split with a lone letter (`werkzaamheden n`) or a dangling compound hyphen
/// (`werkzaamheden-`, which the dictionary allows only before another part)?
fn stray(s: &str) -> bool {
    s.split(' ').any(|part| part.chars().count() == 1) || s.starts_with('-') || s.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dutch_suggestions_use_runtime_tables_case_and_ij() {
        crate::nl::test_dict::install();
        for (word, want) in [
            ("zwembat", "zwembad"),
            ("Zwembat", "Zwembad"),
            ("zwemmbad", "zwembad"),
            ("zjin", "zijn"),
            ("zein", "zijn"),
            ("tuinhius", "tuinhuis"),
            ("ijsland", "IJsland"),
            ("hetzwembad", "het zwembad"),
            ("nml", "namelijk"),
        ] {
            assert!(suggest(word, 8).contains(&want.into()), "{word}: {:?}", suggest(word, 8));
        }
        assert!(suggest("zwembat", 8).iter().all(|s| !s.contains('ĳ')), "suggestions show ij as two letters");
        assert!(!suggest("xnml", 8).contains(&"xnamelijk".into()), "an anchored REP applies to the whole word only");
        assert!(suggest("zwembadd", 8).iter().all(|s| !stray(s)), "{:?}", suggest("zwembadd", 8));
    }

    #[test]
    fn suggestions_stay_bounded_and_skip_forbidden_words() {
        crate::nl::test_dict::install();
        for w in ["", "twee woorden", &"a".repeat(41)] {
            assert!(suggest(w, 4).is_empty());
        }
        assert!(suggest("zwembat", 0).is_empty());
        assert!(suggest_in(&AffixDict::default(), "zwembat", 4).is_empty());
        assert!(suggest("zwembad", 4).len() <= 4);
        assert!(!suggest("zwembad", 4).contains(&"zwembad".into()));
        assert!(!suggest("fietsenhk", 8).contains(&"fietsenhok".into()));
    }

    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        crate::affix::tests::assert_nosuggest_is_honoured(suggest_in);
    }
}

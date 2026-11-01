//! Hebrew spelling suggestions driven by the installed `he_IL` Hunspell dictionary tables.

use crate::affix::AffixDict;

/// Spelling suggestions for a Hebrew word, best first, at most `max`.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// Spelling suggestions for `word` against a given [`AffixDict`].
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::cs::suggest::suggest_in(d, word, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that Hebrew suggestions use the dictionary's TRY/REP/MAP tables and skip forbidden words.
    #[test]
    fn hebrew_suggestions_use_runtime_tables() {
        crate::he::test_dict::install();
        assert!(suggest("שלומ", 8).contains(&"שלום".into()));
        assert!(!suggest("מילהאסורה", 8).contains(&"מילהאסורה".into()));
    }
}

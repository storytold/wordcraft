//! The start of every Latin-script dictionary language's spelling check (Czech, Croatian,
//! Indonesian, Italian, Polish, Portuguese, Spanish): the words that need no lookup, under the
//! File › Options › Proofing choices. Each language goes on from there with its own rules.

use crate::Options;
use crate::affix::AffixDict;

/// Is `c` a letter of the Latin script (the only one these dictionaries cover)?
fn latin(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// `word` trimmed of quotes and apostrophes, when `d` must say whether it is right; `None` when it
/// is right without a lookup: there is no dictionary (no spelling marks), the word is a single
/// letter or has no letters, it has digits or is a short word in capitals (an acronym) and the
/// options skip those, or it is in another script.
pub(crate) fn word_to_check<'a>(d: &AffixDict, word: &'a str, opts: &Options) -> Option<&'a str> {
    if d.is_empty() {
        return None;
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1 || (opts.ignore_numbers && w.chars().any(|c| c.is_ascii_digit())) || !w.chars().any(char::is_alphabetic) {
        return None;
    }
    if opts.ignore_uppercase && w.chars().all(|c| !c.is_lowercase()) && n <= 6 {
        return None;
    }
    w.chars().any(latin).then_some(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_needing_no_lookup() {
        let d = AffixDict::from_hunspell("", "1\nkot\n", &[]).unwrap();
        let defaults = Options::default();
        assert_eq!(word_to_check(&AffixDict::default(), "kot", &defaults), None, "no dictionary");
        for w in ["", "a", "2026", "PKO", "B2B", "日本語", "--"] {
            assert_eq!(word_to_check(&d, w, &defaults), None, "{w}");
        }
        assert_eq!(word_to_check(&d, "„kot”", &defaults), Some("kot"));
        assert_eq!(word_to_check(&d, "'kot'", &defaults), Some("kot"));
        let strict = Options { ignore_uppercase: false, ignore_numbers: false, ..defaults };
        assert_eq!(word_to_check(&d, "PKO", &strict), Some("PKO"));
        assert_eq!(word_to_check(&d, "B2B", &strict), Some("B2B"));
        assert_eq!(word_to_check(&d, "PRZEDSIĘBIORSTWO", &defaults), Some("PRZEDSIĘBIORSTWO"), "only short capitals are acronyms");
    }
}

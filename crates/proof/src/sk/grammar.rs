//! Generic repeated-word and spacing checks with Slovak messages: the checks Czech runs
//! ([`crate::cs::grammar`]). No Slovak-specific grammar, comma or sentence-capital rules are
//! inferred.

use crate::cs::grammar::{Messages, check_with_messages};
use crate::{Issue, ProofLang};

const SLOVAK: Messages = Messages {
    lang: ProofLang::Sk,
    repeated: "Opakované slovo: „{w}“",
    space_before_punctuation: "Medzera pred interpunkciou",
    double_space: "Dvojitá medzera",
};

pub fn check(text: &str) -> Vec<Issue> {
    check_with_messages(text, &SLOVAK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IssueKind;

    #[test]
    fn repeated_words_and_spaces_have_slovak_messages() {
        let text = "Mesto mesto ,  škola.";
        let issues = check(text);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues[0].message, "Opakované slovo: „mesto“");
        assert_eq!(issues[0].suggestions, ["Mesto"]);
        assert!(issues.iter().any(|i| i.message == "Medzera pred interpunkciou" && &text[i.start..i.end] == " ,"));
        assert!(issues.iter().any(|i| i.message == "Dvojitá medzera"));
        assert!(issues.iter().all(|i| i.lang == ProofLang::Sk && i.kind == IssueKind::Grammar));
        for text in ["mesto. škola", "Dr. Novák", "A apple", "Mesto, mesto!", "Úsmev :) Číslo .5 a bodky ..."] {
            assert!(check(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn ratios_and_scores_keep_their_spaces() {
        for text in ["3 : 1", "Skóre 12 : 10.", "Pomer 1 : 100."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        assert!(check("Slovo : ďalšie").iter().any(|i| i.message == "Medzera pred interpunkciou"));
    }

    proptest::proptest! {
        #[test]
        fn never_panics_and_ranges_are_valid(s in "\\PC{0,80}") {
            for issue in check(&s) {
                proptest::prop_assert!(issue.start <= issue.end && issue.end <= s.len());
                proptest::prop_assert!(s.is_char_boundary(issue.start) && s.is_char_boundary(issue.end));
            }
        }
    }
}

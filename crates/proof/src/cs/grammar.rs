//! Generic repeated-word and spacing checks with Czech messages.
//! No Czech-specific grammar, comma or sentence-capital rules are inferred. Slovak
//! ([`crate::sk::grammar`]) runs the same checks with its own messages ([`Messages`]).

use crate::{Issue, IssueKind, ProofLang, words};

/// The messages of the generic checks, in one language.
pub(crate) struct Messages {
    /// The language the issues are reported in.
    pub lang: ProofLang,
    /// A word written twice; `{w}` stands for the word.
    pub repeated: &'static str,
    /// A space before `,` `;` `.` `:` `!` `?`.
    pub space_before_punctuation: &'static str,
    /// Two spaces in a row.
    pub double_space: &'static str,
}

const CZECH: Messages = Messages {
    lang: ProofLang::Cs,
    repeated: "Opakované slovo: „{w}“",
    space_before_punctuation: "Mezera před interpunkcí",
    double_space: "Dvojitá mezera",
};

fn issue(m: &Messages, start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: m.lang }
}

pub fn check(text: &str) -> Vec<Issue> {
    check_with_messages(text, &CZECH)
}

/// [`check`] with the messages (and language) of `m`.
pub(crate) fn check_with_messages(text: &str, m: &Messages) -> Vec<Issue> {
    let mut issues = Vec::new();
    repeated_words(text, &words(text), m, &mut issues);
    spaces(text, m, &mut issues);
    issues.sort_by_key(|i| (i.start, i.end));
    issues
}

fn repeated_words(text: &str, ws: &[(usize, usize)], m: &Messages, v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty() && gap.chars().all(char::is_whitespace) && w0.chars().all(char::is_alphabetic) && w0.to_lowercase() == w1.to_lowercase() {
            v.push(issue(m, *a0, *b1, m.repeated.replace("{w}", w1), vec![w0.to_string()]));
        }
    }
}

fn spaces(text: &str, m: &Messages, v: &mut Vec<Issue>) {
    for (i, c) in text.char_indices() {
        if c != ' ' {
            continue;
        }
        let rest = text.get(i + 1..).unwrap_or("");
        let mut it = rest.chars();
        let Some(p) = it.next() else { continue };
        let after = it.next();
        // A ratio/score keeps the spaces in digit-space-colon-space-digit.
        let ratio = p == ':'
            && text.get(..i).and_then(|before| before.chars().next_back()).is_some_and(|c| c.is_ascii_digit())
            && after == Some(' ')
            && it.next().is_some_and(|c| c.is_ascii_digit());
        let flag = !ratio
            && match p {
                ',' => true,
                // `;)` and `;-)` are emoticons.
                ';' => !matches!(after, Some(')' | '(' | '-' | 'P' | 'D')),
                // ` ...` is an ellipsis, ` .5` a number.
                '.' => !matches!(after, Some('.')) && !after.is_some_and(|a| a.is_ascii_digit()),
                // ` :)`, ` :-)`, ` :D` and similar are emoticons; ` ?!` is still a stray space.
                ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '’' | '!' | '?')),
                _ => false,
            };
        if flag {
            v.push(issue(m, i, i + 1 + p.len_utf8(), m.space_before_punctuation.into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(m, i, i + 2, m.double_space.into(), vec![" ".into()]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_words_and_spaces_have_czech_messages() {
        let text = "Město město ,  škola.";
        let issues = check(text);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues[0].message, "Opakované slovo: „město“");
        assert_eq!(issues[0].suggestions, ["Město"]);
        assert!(issues.iter().any(|i| i.message == "Mezera před interpunkcí" && &text[i.start..i.end] == " ,"));
        assert!(issues.iter().any(|i| i.message == "Dvojitá mezera"));
        assert!(issues.iter().all(|i| i.lang == ProofLang::Cs && i.kind == IssueKind::Grammar));
        for text in ["město. škola", "Dr. Novák", "A apple", "Město, město!", "Úsměv :) Číslo .5 a tečky ..."] {
            assert!(check(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn ratios_and_scores_keep_their_spaces() {
        for text in ["3 : 1", "1 : 2", "Skóre 12 : 10.", "Poměr 1 : 100."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        for text in ["Slovo : další", "3 : slovo", "slovo : 1", "3 :"] {
            assert!(check(text).iter().any(|i| i.message == "Mezera před interpunkcí"), "{text}");
        }
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

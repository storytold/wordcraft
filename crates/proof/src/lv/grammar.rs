//! Generic repeated-word and spacing checks with Latvian messages. No Latvian-specific grammar
//! rules are inferred: a word written twice in a row is flagged (the second one is offered as
//! the fix), as are a space before punctuation and doubled spaces.

use crate::{Issue, IssueKind, ProofLang, words};

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Lv }
}

pub fn check(text: &str) -> Vec<Issue> {
    let mut issues = Vec::new();
    repeated_words(text, &words(text), &mut issues);
    spaces(text, &mut issues);
    issues.sort_by_key(|i| (i.start, i.end));
    issues
}

fn repeated_words(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty() && gap.chars().all(char::is_whitespace) && w0.chars().all(char::is_alphabetic) && w0.to_lowercase() == w1.to_lowercase() {
            v.push(issue(*a0, *b1, format!("Atkārtots vārds: “{w1}”"), vec![w0.to_string()]));
        }
    }
}

fn spaces(text: &str, v: &mut Vec<Issue>) {
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
            v.push(issue(i, i + 1 + p.len_utf8(), "Atstarpe pirms pieturzīmes".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Dubulta atstarpe".into(), vec![" ".into()]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_words_and_spaces_have_latvian_messages() {
        let text = "Suns suns ,  māja.";
        let issues = check(text);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues[0].message, "Atkārtots vārds: “suns”");
        assert_eq!(issues[0].suggestions, ["Suns"]);
        assert!(issues.iter().any(|i| i.message == "Atstarpe pirms pieturzīmes" && &text[i.start..i.end] == " ,"));
        assert!(issues.iter().any(|i| i.message == "Dubulta atstarpe"));
        assert!(issues.iter().all(|i| i.lang == ProofLang::Lv && i.kind == IssueKind::Grammar));
        for text in ["suns skrien", "māja. Skola", "Dr. Jānis", "Suns, suns!", "Smaids :) Skaitlis .5 un punkti ...", "Rīga-Jūrmala"] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
    }

    #[test]
    fn ratios_and_scores_keep_their_spaces() {
        for text in ["3 : 1", "Rezultāts 2 : 1.", "Attiecība 1 : 100."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        for text in ["Vārds : cits", "3 : vārds", "3 :"] {
            assert!(check(text).iter().any(|i| i.message == "Atstarpe pirms pieturzīmes"), "{text}");
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

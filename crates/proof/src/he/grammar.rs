//! Generic repeated-word and spacing checks with Hebrew messages.

use crate::{Issue, IssueKind, ProofLang, words};

/// Builds a Hebrew grammar [`Issue`].
fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::He }
}

/// Checks `text` for repeated words and spacing errors with Hebrew diagnostic messages.
pub fn check(text: &str) -> Vec<Issue> {
    let mut issues = Vec::new();
    repeated_words(text, &words(text), &mut issues);
    spaces(text, &mut issues);
    issues.sort_by_key(|i| (i.start, i.end));
    issues
}

/// Flags adjacent identical words separated only by whitespace.
fn repeated_words(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty() && gap.chars().all(char::is_whitespace) && w0.chars().all(char::is_alphabetic) && w0.to_lowercase() == w1.to_lowercase() {
            v.push(issue(*a0, *b1, format!("מילה כפולה: ״{w1}״"), vec![w0.to_string()]));
        }
    }
}

/// Flags stray spaces before punctuation and consecutive spaces.
fn spaces(text: &str, v: &mut Vec<Issue>) {
    for (i, c) in text.char_indices() {
        if c != ' ' {
            continue;
        }
        let rest = text.get(i + 1..).unwrap_or("");
        let mut it = rest.chars();
        let Some(p) = it.next() else { continue };
        let after = it.next();
        let ratio = p == ':'
            && text.get(..i).and_then(|before| before.chars().next_back()).is_some_and(|c| c.is_ascii_digit())
            && after == Some(' ')
            && it.next().is_some_and(|c| c.is_ascii_digit());
        let flag = !ratio
            && match p {
                ',' => true,
                ';' => !matches!(after, Some(')' | '(' | '-' | 'P' | 'D')),
                '.' => !matches!(after, Some('.')) && !after.is_some_and(|a| a.is_ascii_digit()),
                ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '’' | '!' | '?')),
                _ => false,
            };
        if flag {
            v.push(issue(i, i + 1 + p.len_utf8(), "רווח לפני סימן פיסוק".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "רווח כפול".into(), vec![" ".into()]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that repeated words, spaces before punctuation, and double spaces produce Hebrew messages.
    #[test]
    fn repeated_words_and_spaces_have_hebrew_messages() {
        let text = "שלום שלום ,  עולם.";
        let issues = check(text);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues[0].message, "מילה כפולה: ״שלום״");
        assert_eq!(issues[0].suggestions, ["שלום"]);
        assert!(issues.iter().any(|i| i.message == "רווח לפני סימן פיסוק" && &text[i.start..i.end] == " ,"));
        assert!(issues.iter().any(|i| i.message == "רווח כפול"));
        assert!(issues.iter().all(|i| i.lang == ProofLang::He && i.kind == IssueKind::Grammar));
    }
}

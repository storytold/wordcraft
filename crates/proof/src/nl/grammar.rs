//! Generic repeated-word and spacing checks with Dutch messages. No Dutch-specific grammar rules
//! are inferred. A word written twice with a space is flagged, except the doubles Dutch grammar
//! produces on its own (`dat dat`, `die die`, `haar haar`…), which aren't mistakes.

use crate::{Issue, IssueKind, ProofLang, words};

/// Words Dutch legitimately writes twice in a row: `Ik weet dat dat klopt`, `de man die die auto
/// kocht`, `Zij heeft haar haar geknipt`, `Zij zijn zijn sleutels kwijt`, `Hij had had moeten…`.
const DOUBLES: &[&str] = &["dat", "die", "haar", "zijn", "had"];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Nl }
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
        let lower = w0.to_lowercase();
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(char::is_alphabetic)
            && lower == w1.to_lowercase()
            && !DOUBLES.contains(&lower.as_str())
        {
            v.push(issue(*a0, *b1, format!("Herhaald woord: ‘{w1}’"), vec![w0.to_string()]));
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
                // ` ,5` is a decimal number written with a comma.
                ',' => !after.is_some_and(|a| a.is_ascii_digit()),
                // `;)` and `;-)` are emoticons.
                ';' => !matches!(after, Some(')' | '(' | '-' | 'P' | 'D')),
                // ` ...` is an ellipsis, ` .5` a number.
                '.' => !matches!(after, Some('.')) && !after.is_some_and(|a| a.is_ascii_digit()),
                // ` :)`, ` :-)`, ` :D` and similar are emoticons; ` ?!` is still a stray space.
                ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '’' | '!' | '?')),
                _ => false,
            };
        if flag {
            v.push(issue(i, i + 1 + p.len_utf8(), "Spatie voor leesteken".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Dubbele spatie".into(), vec![" ".into()]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_words_and_spaces_have_dutch_messages() {
        let text = "Het het zwembad ,  klaar.";
        let issues = check(text);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues[0].message, "Herhaald woord: ‘het’");
        assert_eq!(issues[0].suggestions, ["Het"]);
        assert!(issues.iter().any(|i| i.message == "Spatie voor leesteken" && &text[i.start..i.end] == " ,"));
        assert!(issues.iter().any(|i| i.message == "Dubbele spatie"));
        assert!(issues.iter().all(|i| i.lang == ProofLang::Nl && i.kind == IssueKind::Grammar));
        for text in ["het zwembad", "klaar. Het", "Dr. Jansen", "Ja, ja!", "Lach :) Getal ,5 en puntjes ...", "zee-egel"] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
    }

    #[test]
    fn grammatical_doubles_are_left_alone() {
        for text in ["Ik weet dat dat klopt.", "De man die die auto kocht.", "Zij heeft haar haar geknipt.", "Zij zijn zijn sleutels kwijt."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        assert_eq!(check("Het is is goed.").len(), 1);
    }

    #[test]
    fn ratios_and_scores_keep_their_spaces() {
        for text in ["3 : 1", "Uitslag 2 : 1.", "Verhouding 1 : 100."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        for text in ["Woord : ander", "3 : woord", "3 :"] {
            assert!(check(text).iter().any(|i| i.message == "Spatie voor leesteken"), "{text}");
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

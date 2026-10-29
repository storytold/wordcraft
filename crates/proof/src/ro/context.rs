//! Narrow lexical cues for ambiguous ASCII forms. A dictionary also accepts `instanta` and
//! `parat` as other forms, so only these adjacent court phrases offer a manual diacritic fix.
//! No sentence-wide keywords, changes to the dictionary, or automatic replacement.

use crate::{Issue, IssueKind, ProofLang};

fn fold(word: &str) -> String {
    crate::composed(word)
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'ă' | 'â' => 'a',
            'î' => 'i',
            'ș' | 'ş' => 's',
            'ț' | 'ţ' => 't',
            c => c,
        })
        .collect()
}

/// Inline spaces only: punctuation, identifiers and hard line/paragraph boundaries stop a cue.
fn joined(text: &str, left: &(usize, usize), right: &(usize, usize)) -> bool {
    text.get(left.1..right.0).is_some_and(|gap| {
        !gap.is_empty() && gap.chars().all(|c| c == '\t' || (c.is_whitespace() && !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}')))
    })
}

fn phrase_at(text: &str, ws: &[(usize, usize)], start: usize, phrase: &[&str]) -> bool {
    let Some(end) = start.checked_add(phrase.len()) else { return false };
    let Some(window) = ws.get(start..end) else { return false };
    window.iter().zip(phrase).all(|(&(a, b), expected)| text.get(a..b).is_some_and(|w| w.len() <= 64 && fold(w) == *expected))
        && window.windows(2).all(|p| match p {
            [left, right] => joined(text, left, right),
            _ => false,
        })
}

fn before(text: &str, ws: &[(usize, usize)], at: usize, phrase: &[&str]) -> bool {
    let Some(start) = at.checked_sub(phrase.len()) else { return false };
    let (Some(left), Some(target)) = (at.checked_sub(1).and_then(|i| ws.get(i)), ws.get(at)) else { return false };
    phrase_at(text, ws, start, phrase) && joined(text, left, target)
}

fn after(text: &str, ws: &[(usize, usize)], at: usize, phrase: &[&str]) -> bool {
    let Some(start) = at.checked_add(1) else { return false };
    let (Some(target), Some(right)) = (ws.get(at), ws.get(start)) else { return false };
    joined(text, target, right) && phrase_at(text, ws, start, phrase)
}

fn literal_edge(c: char) -> bool {
    matches!(c, '_' | '/' | '\\' | '@' | '`' | '"' | '\'' | '‘' | '’' | '“' | '”' | '„' | '«' | '»')
}

pub(super) fn check(text: &str, ws: &[(usize, usize)], issues: &mut Vec<Issue>) {
    for (at, &(a, b)) in ws.iter().enumerate() {
        let Some(word) = text.get(a..b) else { continue };
        if word.len() > 9 {
            continue; // All target forms are short ASCII words; don't allocate for long input.
        }
        let lower = word.to_lowercase();
        let fixed = match lower.as_str() {
            "instanta" => "instanța",
            "instantei" => "instanței",
            "parat" => "pârât",
            "paratul" => "pârâtul",
            "paratului" => "pârâtului",
            _ => continue,
        };
        if (word.chars().skip(1).any(char::is_uppercase) && !word.chars().all(char::is_uppercase))
            || text.get(..a).and_then(|s| s.chars().next_back()).is_some_and(literal_edge)
            || text.get(b..).and_then(|s| s.chars().next()).is_some_and(literal_edge)
            || crate::in_user_dictionary(&lower)
        {
            continue;
        }
        let court = match lower.as_str() {
            "instanta" | "instantei" => {
                after(text, ws, at, &["de", "judecata"])
                    || after(text, ws, at, &["de", "apel"])
                    || after(text, ws, at, &["de", "recurs"])
                    || (lower == "instantei" && (before(text, ws, at, &["pe", "rolul"]) || before(text, ws, at, &["in", "fata"])))
            }
            "parat" => before(text, ws, at, &["calitate", "de"]) || before(text, ws, at, &["calitatea", "de"]),
            "paratul" => {
                before(text, ws, at, &["contradictoriu", "cu"])
                    || before(text, ws, at, &["reclamantul", "si"])
                    || after(text, ws, at, &["a", "depus", "intampinare"])
            }
            "paratului" => ["citarea", "obligarea", "audierea"].iter().any(|cue| before(text, ws, at, &[cue])),
            _ => false,
        };
        if court {
            issues.push(Issue {
                start: a,
                end: b,
                kind: IssueKind::Grammar,
                message: "Diacritice în context juridic: verificați forma cuvântului.".into(),
                suggestions: vec![crate::match_case(word, fixed.into())],
                lang: ProofLang::Ro,
            });
        }
    }
}

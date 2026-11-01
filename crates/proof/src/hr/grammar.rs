//! Conservative Croatian grammar and punctuation checks.
//! Handles repeated words, whitespace, capitals and duplicated punctuation;
//! intentionally avoids speculative agreement and comma rules.

use crate::{Issue, IssueKind, ProofLang, words};

/// Words Croatian repeats on purpose: interjections, onomatopoeia and emphatic doubling.
const ALLOWED_REPEATS: &[&str] =
    &["da", "ne", "no", "ha", "hej", "joj", "aj", "oh", "uh", "hop", "kuc", "tik", "tak", "bum", "malo", "brzo", "polako", "više", "dobro"];
const ABBREVIATIONS: &[&str] = &[
    "npr", "itd", "itp", "tj", "tzv", "odn", "sl", "str", "br", "god", "st", "dr", "mr", "prof", "doc", "ing", "dipl", "mag", "univ", "g", "god",
    "mj", "min", "sek", "čl", "st", "č", "toč", "tab", "sl", "sv", "ul", "trg", "pr", "izd", "ur", "svez", "vol", "vid", "usp", "prim", "engl",
    "njem", "hrv", "lat", "tel", "mob", "e", "p", "o", "d", "m", "v", "z", "r", "pon", "uto", "sri", "čet", "pet", "sub", "ned", "sij", "velj",
    "ožu", "tra", "svi", "lip", "srp", "kol", "ruj", "lis", "stu", "pro",
];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Hr }
}

fn capitalize(w: &str) -> String {
    crate::affix::capitalize(w)
}

/// Is `prev`, before a period, an abbreviation, an initial, a number or a Roman numeral (after
/// which a lower-case word doesn't start a sentence)?
fn abbreviation_or_number(prev: &str) -> bool {
    prev.chars().count() <= 1
        || prev.chars().any(|c| c.is_ascii_digit())
        || (prev.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')))
        || ABBREVIATIONS.contains(&prev.to_lowercase().as_str())
}

const CLOSING: [char; 8] = ['"', '”', '’', '\'', ')', '»', ']', '\u{1}'];
const OPENING: [char; 8] = ['"', '“', '«', '(', '[', '\'', '‚', '—'];

/// Does the gap between two words end a sentence: a `.`, `!` or `?` (not an ellipsis, not after
/// an abbreviation), then space, with nothing but quotes and brackets around?
fn ends_sentence(gap: &str, prev: &str) -> bool {
    let t = gap.trim_start_matches(CLOSING);
    let Some(end) = t.chars().next() else { return false };
    if !matches!(end, '.' | '!' | '?') || t.starts_with("..") {
        return false;
    }
    if end == '.' && abbreviation_or_number(prev) {
        return false;
    }
    let rest = t.trim_start_matches(['.', '!', '?']).trim_start_matches(CLOSING);
    rest.starts_with(char::is_whitespace) && rest.trim_start().chars().all(|c| OPENING.contains(&c))
}

/// Grammar issues in a paragraph of Croatian text, in order.
pub fn check(text: &str) -> Vec<Issue> {
    check_with(text, true)
}

/// [`check`] where `capital_first` says whether the text's first word must start a sentence with a capital
/// (false for a list item, which may be a fragment).
pub fn check_with(text: &str, capital_first: bool) -> Vec<Issue> {
    let ws: Vec<(usize, usize)> = words(text);
    let mut v = Vec::new();
    repeated_words(text, &ws, &mut v);
    spaces(text, &mut v);
    sentence_capitals(text, &ws, capital_first, &mut v);
    doubled_punctuation(text, &mut v);
    v.sort_by_key(|i| (i.start, i.end));
    v
}

fn repeated_words(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(char::is_alphabetic)
            && w0.to_lowercase() == w1.to_lowercase()
            && !ALLOWED_REPEATS.contains(&w0.to_lowercase().as_str())
        {
            v.push(issue(*a0, *b1, format!("Ponovljena riječ: \"{w1}\""), vec![w0.to_string()]));
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
        let flag = match p {
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
            v.push(issue(i, i + 1 + p.len_utf8(), "Razmak ispred interpunkcijskog znaka".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Dvostruki razmak".into(), vec![" ".into()]));
    }
}

fn sentence_capitals(text: &str, ws: &[(usize, usize)], capital_first: bool, v: &mut Vec<Issue>) {
    // List items (`– maçãs,`) may start in lower case.
    let list_item = text.trim_end().ends_with([',', ';']);
    for (k, &(a, b)) in ws.iter().enumerate() {
        let Some(w) = text.get(a..b) else { continue };
        let starts = if k == 0 {
            let before = text.get(..a).unwrap_or("");
            capital_first && !list_item && before.chars().all(|c| c.is_whitespace() || OPENING.contains(&c))
        } else {
            let Some(&(pa, pb)) = ws.get(k - 1) else { continue };
            ends_sentence(text.get(pb..a).unwrap_or(""), text.get(pa..pb).unwrap_or(""))
        };
        if starts && w.chars().next().is_some_and(char::is_lowercase) && w.chars().all(char::is_alphabetic) {
            v.push(issue(a, b, "Rečenica treba započeti velikim slovom".into(), vec![capitalize(w)]));
        }
    }
}

fn doubled_punctuation(text: &str, v: &mut Vec<Issue>) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if !matches!(b, b',' | b'.' | b';') {
            i += 1;
            continue;
        }
        let run = bytes.get(i..).map_or(0, |r| r.iter().take_while(|&&x| x == b).count());
        let flag = match b {
            b'.' => run == 2,
            _ => run >= 2,
        };
        if flag {
            let end = i + run;
            let suggestions: Vec<String> = match b {
                b'.' => vec![".".into(), "…".into()],
                _ => vec![char::from(b).to_string()],
            };
            v.push(issue(i, end, "Ponovljen interpunkcijski znak".into(), suggestions));
        }
        i += run.max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checks_croatian_punctuation() {
        let t = "Ovo ovo je primjer , s dva  razmaka..";
        let issues = check(t);
        assert!(issues.iter().any(|i| i.message.starts_with("Ponovljena riječ")));
        assert!(issues.iter().any(|i| i.message == "Dvostruki razmak"));
        assert!(issues.iter().all(|i| i.lang == ProofLang::Hr));
    }
    #[test]
    fn abbreviations_do_not_trigger_capitals() {
        let issues = check("To je npr. sasvim dobar primjer.");
        assert!(!issues.iter().any(|i| i.message == "Rečenica treba započeti velikim slovom"));
    }
}

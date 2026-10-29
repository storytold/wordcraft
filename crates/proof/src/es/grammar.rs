//! Spanish grammar checks with Spanish messages: the generic repeated-word and spacing checks,
//! and the opening question and exclamation marks Spanish writes before a question or an
//! exclamation (`¿Vienes?`, `¡Qué bien!`): a closing `?` or `!` whose sentence has no opening mark
//! is flagged, with the opening mark offered at the sentence's start, and so is a space after an
//! opening mark (`¿ Vienes?`). Either opening mark may pair with either closing one (`¡Qué
//! dices?`), and `(?)` and `(!)`, which mark doubt or surprise, stand alone. No other
//! Spanish-specific grammar, comma or sentence-capital rules are inferred.

use unicode_normalization::UnicodeNormalization;

use crate::{Issue, IssueKind, ProofLang, words};

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>, lang: ProofLang) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang }
}

/// Grammar issues in a paragraph of Spanish text; `lang` is the variety (they explain the same in
/// every one).
pub fn check(text: &str, lang: ProofLang) -> Vec<Issue> {
    let mut issues = Vec::new();
    repeated_words(text, &words(text), lang, &mut issues);
    spaces(text, lang, &mut issues);
    opening_marks(text, lang, &mut issues);
    issues.sort_by_key(|i| (i.start, i.end));
    issues
}

fn repeated_words(text: &str, ws: &[(usize, usize)], lang: ProofLang, v: &mut Vec<Issue>) {
    // Letters and the combining marks of a decomposed word (`tambie\u{301}n`); a composed and a
    // decomposed spelling of one word are the same word.
    let letters = |w: &str| w.chars().all(|c| c.is_alphabetic() || unicode_normalization::char::is_combining_mark(c));
    let folded = |w: &str| w.nfc().collect::<String>().to_lowercase();
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty() && gap.chars().all(char::is_whitespace) && letters(w0) && folded(w0) == folded(w1) {
            v.push(issue(*a0, *b1, format!("Palabra repetida: «{w1}»"), vec![w0.to_string()], lang));
        }
    }
}

fn spaces(text: &str, lang: ProofLang, v: &mut Vec<Issue>) {
    for (i, c) in text.char_indices() {
        // A space after an opening mark: `¿ Vienes?`.
        if matches!(c, '¿' | '¡')
            && text.get(i + c.len_utf8()..).is_some_and(|r| r.starts_with(' ') && r.trim_start().chars().next().is_some_and(char::is_alphanumeric))
        {
            v.push(issue(i, i + c.len_utf8() + 1, "Espacio después de un signo de apertura".into(), vec![c.to_string()], lang));
            continue;
        }
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
                ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '’' | '»' | '!' | '?')),
                _ => false,
            };
        if flag {
            v.push(issue(i, i + 1 + p.len_utf8(), "Espacio antes de un signo de puntuación".into(), vec![p.to_string()], lang));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Espacio doble".into(), vec![" ".into()], lang));
    }
}

/// Does the character `b` at byte `j` end what comes before it, so a question or exclamation after
/// it starts there? Sentence ends, clause breaks, line breaks and the start of a quotation or
/// parenthesis do, but not punctuation inside a word (`3.5`, `10:30`) or inside an address
/// (`example.com`, `https:`), except a mark ending an address (`…/? Vienes mañana?`).
fn ends_clause(text: &str, addresses: &[(usize, usize)], j: usize, b: char) -> bool {
    let after = j + b.len_utf8();
    if let Some((_, end)) = address_at(addresses, j) {
        return matches!(b, '?' | '!' | '.') && end == after;
    }
    let inside_word = matches!(b, '.' | ':' | ';') && text.get(after..).and_then(|r| r.chars().next()).is_some_and(char::is_alphanumeric);
    !inside_word && matches!(b, '.' | '?' | '!' | '…' | ':' | ';' | '\n' | '\r' | '\t' | '«' | '“' | '"' | '(' | '[' | '—')
}

/// The one of the `addresses` ([`address_ranges`]) that byte `j` is inside, if any. The ranges come
/// in text order and never overlap, so this is a binary search, O(log m) for m addresses: a paragraph
/// of thousands of addresses costs O(n log m) when every character before a question mark asks
/// (#3562 review), not the O(n m) of a scan per character.
fn address_at(addresses: &[(usize, usize)], j: usize) -> Option<(usize, usize)> {
    let after_start = addresses.partition_point(|&(s, _)| s <= j);
    let (s, e) = *addresses.get(after_start.checked_sub(1)?)?;
    (s <= j && j < e).then_some((s, e))
}

/// The byte ranges of the whitespace-delimited tokens of `text` that are web or e-mail addresses
/// (`https://example.com/?`, `www.example.com`, `ana@example.com`), as spelling skips them.
fn address_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    // A final space ends the last token.
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        if !c.is_whitespace() {
            start = start.or(Some(i));
        } else if let Some(s) = start.take()
            && text.get(s..i).is_some_and(crate::is_address)
        {
            out.push((s, i));
        }
    }
    out
}

/// A closing `?` or `!` with no opening `¿` or `¡` before it. Openers are counted left to right, like
/// brackets, and each closing mark takes one back (either opener pairs with either closer:
/// `¡Qué dices?`), so what sits between them doesn't matter: quotes, parentheses, dashes, an
/// ellipsis or an abbreviation's full stop (`¿Viene el Sr. García?`, `¿Leíste «Rayuela»?`) all belong
/// to the question. Only a closing mark with nothing open is flagged, and then the sentence it
/// closes starts after the last clause break ([`ends_clause`]). Marks in web and e-mail addresses
/// aren't sentence punctuation, and neither is punctuation inside an address or a word
/// (`example.com`, `3.5`, `10:30`) a sentence's start.
fn opening_marks(text: &str, lang: ProofLang, v: &mut Vec<Issue>) {
    let addresses = address_ranges(text);
    // Openers not closed yet: a run of them (`¡¡`) counts once, like a run of closers (`!!`).
    let mut open = 0usize;
    for (i, c) in text.char_indices() {
        let before = text.get(..i).unwrap_or("");
        let prev = before.chars().next_back();
        let (opener, message) = match c {
            '¿' | '¡' => {
                if !matches!(prev, Some('¿' | '¡')) {
                    open += 1;
                }
                continue;
            }
            '?' => ('¿', "Falta el signo de interrogación de apertura (¿)"),
            '!' => ('¡', "Falta el signo de exclamación de apertura (¡)"),
            _ => continue,
        };
        let next = text.get(i + 1..).and_then(|r| r.chars().next());
        // `?!` and `!!!` are handled at their first mark; `(?)` stands alone; `?q=`, `a!b` and a
        // mark ending an address (`https://example.com/?`) aren't sentence ends.
        if matches!(prev, Some('?' | '!'))
            || (prev == Some('(') && next == Some(')'))
            || next.is_some_and(char::is_alphanumeric)
            || address_at(&addresses, i).is_some()
        {
            continue;
        }
        if open > 0 {
            open -= 1;
            continue;
        }
        let from = before.char_indices().rev().find(|&(j, b)| matches!(b, '¿' | '¡') || ends_clause(text, &addresses, j, b));
        let start = from.map_or(0, |(j, b)| j + b.len_utf8());
        let Some(sentence) = text.get(start..i) else { continue };
        let lead = sentence.len() - sentence.trim_start().len();
        let start = start + lead;
        let Some(sentence) = text.get(start..i + 1) else { continue };
        if !sentence.chars().any(char::is_alphabetic) {
            continue;
        }
        v.push(issue(start, i + 1, message.into(), vec![format!("{opener}{sentence}")], lang));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(text: &str) -> Vec<(String, String)> {
        check(text, ProofLang::Es).into_iter().map(|i| (text[i.start..i.end].to_string(), i.message)).collect()
    }

    /// The missing-opening-mark issues of `text`: the flagged text and the suggestions offered.
    fn missing(text: &str) -> Vec<(String, Vec<String>)> {
        check(text, ProofLang::Es)
            .into_iter()
            .filter(|i| i.message.starts_with("Falta"))
            .map(|i| (text[i.start..i.end].to_string(), i.suggestions))
            .collect()
    }

    #[test]
    fn repeated_words_and_spaces_have_spanish_messages() {
        let text = "Es es la casa ,  dijo.";
        let issues = check(text, ProofLang::EsMx);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues[0].message, "Palabra repetida: «es»");
        assert_eq!(issues[0].suggestions, ["Es"]);
        assert!(issues.iter().any(|i| i.message == "Espacio antes de un signo de puntuación" && &text[i.start..i.end] == " ,"));
        assert!(issues.iter().any(|i| i.message == "Espacio doble"));
        assert!(issues.iter().all(|i| i.lang == ProofLang::EsMx && i.kind == IssueKind::Grammar));
        for text in ["La casa. Es grande", "Sr. García", "No, no lo sé", "Hola :) número .5 y puntos ...", "3 : 1", "teórico-práctico"] {
            assert!(check(text, ProofLang::Es).is_empty(), "{text}: {:?}", check(text, ProofLang::Es));
        }
    }

    #[test]
    fn questions_and_exclamations_need_their_opening_mark() {
        assert_eq!(messages("Vienes mañana?"), [("Vienes mañana?".into(), "Falta el signo de interrogación de apertura (¿)".into())]);
        assert_eq!(check("Vienes mañana?", ProofLang::Es)[0].suggestions, ["¿Vienes mañana?"]);
        assert_eq!(messages("Llegó. Qué bien!"), [("Qué bien!".into(), "Falta el signo de exclamación de apertura (¡)".into())]);
        assert_eq!(messages("Le preguntó: vienes?")[0].0, "vienes?");
        assert_eq!(messages("Dijo «vienes?»")[0].0, "vienes?");
        assert_eq!(messages("Hola!!!").len(), 1);
        assert_eq!(messages("Qué?!").len(), 1);
        for text in [
            "¿Vienes mañana?",
            "¡Qué bien!",
            "Si vienes, ¿me llamas?",
            "¡Qué dices?",
            "¿Qué?! ¡No!",
            "Nació en 1950 (?) en Montevideo.",
            "Lo dijo (!) en público.",
            "Ver https://example.com/?q=1 y a!b",
            "¿Qué? ¿Cómo?",
            "2 + 2 = ?",
            "",
            "?",
        ] {
            assert!(messages(text).iter().all(|(_, m)| !m.starts_with("Falta")), "{text}: {:?}", messages(text));
        }
    }

    /// Maintainer review on #3562: a URL ending in `/?` isn't a question, and the dots and colons
    /// of an address or a word (`example.com`, `https:`, `10:30`) don't start the sentence the
    /// missing `¿` is offered for; ordinary questions are still checked.
    #[test]
    fn addresses_are_not_questions_or_sentence_starts() {
        for text in [
            "Ver https://example.com/?",
            "Ver https://example.com/? ¿Vienes?",
            "Ver http://example.com/buscar/? y www.example.com/?",
            "Escribí a ana@example.com!",
            "¿Viste https://example.com/?",
        ] {
            assert!(missing(text).is_empty(), "{text}: {:?}", missing(text));
        }
        assert_eq!(
            missing("Mirá https://example.com/ y decime si vienes?"),
            [("Mirá https://example.com/ y decime si vienes?".to_string(), vec!["¿Mirá https://example.com/ y decime si vienes?".to_string()])]
        );
        assert_eq!(missing("Mirá example.com y decime si vienes?")[0].0, "Mirá example.com y decime si vienes?");
        assert_eq!(missing("Son las 10:30, vienes?")[0].0, "Son las 10:30, vienes?");
        assert_eq!(missing("Ver https://example.com/? Vienes mañana?"), [("Vienes mañana?".to_string(), vec!["¿Vienes mañana?".to_string()])]);
        assert_eq!(missing("Llegó. Vienes?")[0].0, "Vienes?", "a full stop before a space still ends a sentence");
    }

    /// Maintainer review on #3562: a correctly opened question or exclamation is never flagged,
    /// whatever sits between its marks: quotes, an abbreviation's full stop, an ellipsis, a dash
    /// aside, parentheses, nested questions, a line break.
    #[test]
    fn correctly_opened_questions_are_not_flagged() {
        for text in [
            "¿Leíste «Rayuela»?",
            "¿Viene el Sr. García?",
            "¿Y si... no viene?",
            "¿Vendrás —preguntó Juan— mañana?",
            "¿Qué significa (aquí) la palabra?",
            "¿Has leído \"Rayuela\" de Cortázar?",
            "¿Leíste “Rayuela”?",
            "¿Es 'Rayuela' o 'Los premios'?",
            "¿Y tú, qué opinas… vienes?",
            "¿Dijo «¿qué?» o no?",
            "¿Viene la Dra. López, es decir, la jefa (la de 2.5 años)?",
            "¿Dónde: aquí o allí?",
            "¿Cuánto cuesta: 3,50 o a las 10:30?",
            "¿Vienes a las 10:30; o mejor mañana?",
            "¡Qué dices, Dr. Pérez!",
            "¡Qué dices, Dr. Pérez? ¡Basta!",
            "¡¡Hola!! ¿Qué tal?",
            "¡¡¡Basta!!!",
            "¿Vienes?!",
            "¿Qué?! ¡No!",
            "¿Vienes?\n¿O no?",
            "¿Vienes\nmañana?",
            "Ana dijo: «¿Vienes?» y yo: «Sí».",
            "—¿Vienes? —preguntó Juan.",
            "¿Viste https://example.com/? Dime.",
            "¿Es verdad (!) o no?",
            "Nació en 1950 (?) en Montevideo.",
        ] {
            assert!(missing(text).is_empty(), "{text:?}: {:?}", missing(text));
        }
    }

    /// A closing mark with nothing open is still flagged, after a sentence or a clause that closed
    /// an earlier question, and with the sentence it closes offered with its opening mark.
    #[test]
    fn closing_marks_with_nothing_open_are_still_flagged() {
        for (text, flagged, offered) in [
            ("Vienes mañana?", "Vienes mañana?", "¿Vienes mañana?"),
            ("Llegó. Vienes?", "Vienes?", "¿Vienes?"),
            ("Le preguntó: vienes?", "vienes?", "¿vienes?"),
            ("Dijo «vienes?»", "vienes?", "¿vienes?"),
            ("Qué bien!", "Qué bien!", "¡Qué bien!"),
            ("¿Vienes? No sé qué hacer?", "No sé qué hacer?", "¿No sé qué hacer?"),
            ("¿Viene el Sr. García? Llegó tarde!", "Llegó tarde!", "¡Llegó tarde!"),
            ("¡¡Hola!! Qué bien!", "Qué bien!", "¡Qué bien!"),
            ("¿Qué?! No sé qué decir!", "No sé qué decir!", "¡No sé qué decir!"),
        ] {
            let found = missing(text);
            assert_eq!(found.len(), 1, "{text:?}: {found:?}");
            assert_eq!((found[0].0.as_str(), found[0].1.first().map(String::as_str)), (flagged, Some(offered)), "{text:?}");
        }
    }

    #[test]
    fn address_lookup_finds_the_range_a_byte_is_in() {
        let ranges = [(2, 5), (7, 8), (10, 20)];
        let inside = |j| address_at(&ranges, j);
        assert_eq!([0, 1].map(inside), [None, None], "before the first");
        assert_eq!([2, 3, 4].map(inside), [Some((2, 5)); 3], "the start is inside, the end is not");
        assert_eq!([5, 6].map(inside), [None, None], "between ranges");
        assert_eq!([7, 10, 19].map(inside), [Some((7, 8)), Some((10, 20)), Some((10, 20))]);
        assert_eq!([8, 9, 20, 99].map(inside), [None; 4], "after a range, and after the last");
        assert_eq!(address_at(&[], 3), None);
    }

    /// Maintainer review on #3562: a paragraph of thousands of addresses followed by a question was
    /// checked in quadratic time (every character before the question scanned every address).
    /// About 99 KB (under the 100,000-byte paragraph cap) took hundreds of milliseconds; it is now
    /// O(n log m): about 1 to 5 ms in a debug build, against about 300 ms for the scan, so the
    /// bound below leaves room on a loaded machine and still catches a return to the quadratic
    /// lookup.
    #[test]
    fn many_addresses_before_a_question_are_checked_quickly() {
        for (unit, ending, whole_paragraph) in
            [("https://x/ ", "Vienes?", true), ("https://x/? ", "Vienes?", false), ("www.x.com ", "Qué bien!", true)]
        {
            // As many as fit in about 99 KB: 9,000 of the first, as in the review.
            let text = format!("{}{ending}", unit.repeat(99_000 / unit.len()));
            assert!(text.len() < 100_000, "{}", text.len());
            let started = std::time::Instant::now();
            let issues = check(&text, ProofLang::Es);
            let took = started.elapsed();
            assert!(took < std::time::Duration::from_millis(75), "{unit:?} took {took:?}");
            let missing: Vec<&Issue> = issues.iter().filter(|i| i.message.starts_with("Falta")).collect();
            assert_eq!(missing.len(), 1, "{unit:?}: {missing:?}");
            // The mark's sentence is the whole paragraph when the addresses don't end with a mark
            // of their own, and only the last question when they do.
            let want_start = if whole_paragraph { 0 } else { text.len() - ending.len() };
            assert_eq!((missing[0].start, missing[0].end), (want_start, text.len()), "{unit:?}");
        }
    }

    /// A word repeated once composed and once decomposed is still repeated.
    #[test]
    fn repeated_words_match_across_normalization_forms() {
        let text = "también tambie\u{301}n";
        let issues = check(text, ProofLang::Es);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!((issues[0].start, issues[0].end), (0, text.len()));
        assert!(issues[0].message.starts_with("Palabra repetida"));
    }

    #[test]
    fn a_space_after_an_opening_mark_is_flagged() {
        let text = "¿ Vienes? ¡ Hola!";
        let issues = check(text, ProofLang::EsUy);
        let spaced: Vec<&str> =
            issues.iter().filter(|i| i.message == "Espacio después de un signo de apertura").map(|i| &text[i.start..i.end]).collect();
        assert_eq!(spaced, ["¿ ", "¡ "], "{issues:?}");
        assert!(issues.iter().all(|i| !i.message.starts_with("Falta")), "{issues:?}");
        assert!(check("¿ ", ProofLang::Es).is_empty());
    }

    proptest::proptest! {
        #[test]
        fn never_panics_and_ranges_are_valid(s in "\\PC{0,80}|[¿¡?!a-z .,(«»):]{0,40}") {
            for issue in check(&s, ProofLang::Es) {
                proptest::prop_assert!(issue.start <= issue.end && issue.end <= s.len());
                proptest::prop_assert!(s.is_char_boundary(issue.start) && s.is_char_boundary(issue.end));
            }
        }
    }
}

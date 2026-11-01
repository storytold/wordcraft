//! Portuguese grammar and punctuation checks: cheap, rule-based and tuned to stay quiet when
//! unsure (a missed mistake costs less than a wrong mark on correct text). Messages are in
//! Portuguese.
//!
//! - **Repeated word** (`ele ele`), except interjections and words Portuguese doubles on purpose
//!   (`sim sim`, `não não`, `ora ora`).
//! - **Space before punctuation** (`bem ,`) and **double spaces**; emoticons (` :)`) and
//!   ellipses are left alone.
//! - **Capital letter at the start of a sentence**, knowing Portuguese abbreviations (`etc.`,
//!   `pág.`, `Prof.`, `Dr.`), initials, ordinal numbers (`5. colocado`), ellipses and dialogue
//!   dashes (`— Vai — disse`), after which lower case is correct.
//! - **Doubled punctuation** (`,,`, `..`, `;;`).
//!
//! No comma rule (`que` opens clauses only sometimes) and no English rule (such as `a`/`an`)
//! applies to Portuguese text.

use crate::{Issue, IssueKind, ProofLang, words};

/// Words Portuguese repeats on purpose: interjections, onomatopoeia and emphatic doubling.
const ALLOWED_REPEATS: &[&str] = &[
    "sim", "não", "ora", "olha", "já", "bem", "muito", "vai", "lá", "cá", "aí", "ha", "he", "hi", "oh", "ah", "ai", "ui", "uf", "uxa", "oba", "viva",
    "psiu", "olá", "tchau", "xau", "adeus", "devagar",
];

/// Abbreviations written with a final period after which a sentence goes on in lower case
/// (lower case, without the period).
const ABBREVIATIONS: &[&str] = &[
    "etc", "ex", "pág", "págs", "séc", "sécs", "fl", "av", "vol", "p", "pp", "ed", "tel", "min", "max", "art", "par", "prof", "profa", "dr", "dra",
    "sr", "sra", "srta", "eng", "arq", "op", "cit", "cf", "ibid", "id", "vide", "seg", "ter", "qua", "qui", "sex", "sáb", "dom", "jan", "fev", "mar",
    "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez", "s.a", "d.c", "a.c", "pg", "ref", "cod", "dept", "univ", "inst", "lat", "ing",
    "esp", "fr", "it", "al", "obs", "att", "obs",
];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Pt }
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

/// Grammar issues in a paragraph of Portuguese text, in order.
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
            v.push(issue(*a0, *b1, format!("Palavra repetida: \"{w1}\""), vec![w0.to_string()]));
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
            v.push(issue(i, i + 1 + p.len_utf8(), "Espaço antes da pontuação".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Espaço duplo".into(), vec![" ".into()]));
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
            v.push(issue(a, b, "Letra maiúscula no início da frase".into(), vec![capitalize(w)]));
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
            v.push(issue(i, end, "Sinal repetido".into(), suggestions));
        }
        i += run.max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(t: &str) -> Vec<String> {
        check(t).into_iter().map(|i| format!("{}@{}", i.message, &t[i.start..i.end])).collect()
    }

    fn clean(t: &str) {
        let m = messages(t);
        assert!(m.is_empty(), "{t:?}: {m:?}");
    }

    #[test]
    fn repeated_words() {
        let v = check("Eu eu vi isso e ele ele saiu.");
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!(v[0].message, "Palavra repetida: \"eu\"");
        assert_eq!(v[0].suggestions, ["Eu"]);
        assert!(v.iter().all(|i| i.lang == ProofLang::Pt && i.kind == IssueKind::Grammar));
        clean("Sim, sim! Não, não! Ora ora, que surpresa.");
    }

    #[test]
    fn spaces_and_punctuation() {
        let m = messages("Olá , mundo .  Tudo bem ?");
        assert!(m.contains(&"Espaço antes da pontuação@ ,".to_string()), "{m:?}");
        assert!(m.contains(&"Espaço antes da pontuação@ .".to_string()), "{m:?}");
        assert!(m.contains(&"Espaço antes da pontuação@ ?".to_string()), "{m:?}");
        assert!(m.contains(&"Espaço duplo@  ".to_string()), "{m:?}");
        clean("Legal :) Muito bom :-) Ou :D Valor .5 e reticências ...");
        let m = messages("Falou,, e saiu.. depois;; fim.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Sinal repetido")).count(), 3, "{m:?}");
        clean("Esperei… e esperei... até ele voltar.");
    }

    #[test]
    fn sentence_start_capitals() {
        let m = messages("isso é um teste. a segunda frase! terceira? ok.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Letra maiúscula")).count(), 4, "{m:?}");
        assert_eq!(check("ele tem razão.")[0].suggestions, ["Ele"]);
        // Abbreviations, initials, ordinals, ellipses and dialogue dashes.
        clean("Comprei frutas, etc. e depois saímos.");
        clean("Conheci o Dr. Silva na Av. Paulista.");
        clean("Zhou ficou em 5. lugar no séc. XX.");
        clean("Esperei... e ele apareceu.");
        clean("— Vai — disse ele. — Amanhã eu volto.");
        clean("Ele disse: \"Volto amanhã\". E saiu.");
        // A list item may start in lower case.
        clean("maçãs,");
    }

    #[test]
    fn correct_portuguese_text_is_clean() {
        clean(
            "O rato roeu a roupa do rei de Roma. Quem não tem cão caça com gato, \
             e quem tem paciência colhe o que plantou.",
        );
        clean("Brasília, 12 de outubro de 2026 — uma data memorável.");
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = check(&s);
        }

        #[test]
        fn never_panics_on_portuguese_like_text(s in "[a-záàâãéêíóôõúüçA-Z ,.;:!?\"…\\-—]{0,60}") {
            for i in check(&s) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
                proptest::prop_assert!(s.is_char_boundary(i.start) && s.is_char_boundary(i.end));
            }
        }
    }
}

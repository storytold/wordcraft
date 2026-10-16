//! Spanish grammar and punctuation checks: cheap, rule-based and tuned to stay quiet when
//! unsure (a missed mistake costs less than a wrong mark on correct text). Messages are in
//! Spanish.
//!
//! - **Repeated word** (`él él`), except interjections and words Spanish doubles on purpose
//!   (`sí sí`, `no no`, `ya ya`).
//! - **Space before punctuation** (`bien ,`) and **double spaces**; emoticons (` :)`) and
//!   ellipses are left alone.
//! - **Capital letter at the start of a sentence**, knowing Spanish abbreviations (`etc.`,
//!   `pág.`, `Prof.`, `Dr.`), initials, ordinal numbers (`5. colocado`), ellipses and dialogue
//!   dashes (`— Vamos — dijo`), after which lower case is correct.
//! - **Doubled punctuation** (`,,`, `..`, `;;`).
//!
//! No comma rule (`que` opens clauses only sometimes) and no English rule (such as `a`/`an`)
//! applies to Spanish text.

use crate::{Issue, IssueKind, ProofLang, words};

/// Words Spanish repeats on purpose: interjections, onomatopoeia and emphatic doubling.
const ALLOWED_REPEATS: &[&str] = &[
    "sí", "no", "ya", "bien", "muy", "casi", "más", "va", "vamos", "dale", "mira", "oye", "ja", "je", "ji", "jo", "ju", "oh", "ah", "ay", "uy", "uf",
    "epa", "adiós", "chau", "chao", "poco", "luego", "nunca", "jamás", "claro", "bueno", "taca", "zas", "pum", "clic",
];

/// Abbreviations written with a final period after which a sentence goes on in lower case
/// (lower case, without the period).
const ABBREVIATIONS: &[&str] = &[
    "etc", "ej", "pág", "págs", "sig", "sigs", "cap", "caps", "av", "avda", "vol", "vols", "p", "pp", "ed", "tel", "mín", "máx", "art", "arts",
    "prof", "profa", "dr", "dra", "sr", "sra", "srta", "lic", "ing", "arq", "gral", "ud", "uds", "vd", "vds", "núm", "nro", "fig", "figs", "op",
    "cit", "cf", "ibíd", "id", "lun", "mar", "mié", "jue", "vie", "sáb", "dom", "ene", "feb", "abr", "may", "jun", "jul", "ago", "sep", "set", "oct",
    "nov", "dic", "s.a", "d.c", "a.c", "ref", "cód", "depto", "dpto", "univ", "inst", "lat", "esp", "fr", "it", "al", "obs", "atte", "aprox",
];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Es }
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
const OPENING: [char; 10] = ['"', '“', '«', '(', '[', '\'', '‚', '—', '¿', '¡'];

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

/// Grammar issues in a paragraph of Spanish text, in order.
pub fn check(text: &str) -> Vec<Issue> {
    let ws: Vec<(usize, usize)> = words(text);
    let mut v = Vec::new();
    repeated_words(text, &ws, &mut v);
    spaces(text, &mut v);
    sentence_capitals(text, &ws, &mut v);
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
            v.push(issue(*a0, *b1, format!("Palabra repetida: \"{w1}\""), vec![w0.to_string()]));
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
            v.push(issue(i, i + 1 + p.len_utf8(), "Espacio antes de la puntuación".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Espacio doble".into(), vec![" ".into()]));
    }
}

fn sentence_capitals(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    // List items (`– manzanas,`) may start in lower case.
    let list_item = text.trim_end().ends_with([',', ';']);
    for (k, &(a, b)) in ws.iter().enumerate() {
        let Some(w) = text.get(a..b) else { continue };
        let starts = if k == 0 {
            let before = text.get(..a).unwrap_or("");
            !list_item && before.chars().all(|c| c.is_whitespace() || OPENING.contains(&c))
        } else {
            let Some(&(pa, pb)) = ws.get(k - 1) else { continue };
            ends_sentence(text.get(pb..a).unwrap_or(""), text.get(pa..pb).unwrap_or(""))
        };
        if starts && w.chars().next().is_some_and(char::is_lowercase) && w.chars().all(char::is_alphabetic) {
            v.push(issue(a, b, "Letra mayúscula al inicio de la oración".into(), vec![capitalize(w)]));
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
            v.push(issue(i, end, "Signo repetido".into(), suggestions));
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
        let v = check("Yo yo vi eso y él él salió.");
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!(v[0].message, "Palabra repetida: \"yo\"");
        assert_eq!(v[0].suggestions, ["Yo"]);
        assert!(v.iter().all(|i| i.lang == ProofLang::Es && i.kind == IssueKind::Grammar));
        clean("¡Sí, sí! ¡No, no! Ya ya, qué sorpresa.");
    }

    #[test]
    fn spaces_and_punctuation() {
        let m = messages("Hola , mundo .  ¿Todo bien ?");
        assert!(m.contains(&"Espacio antes de la puntuación@ ,".to_string()), "{m:?}");
        assert!(m.contains(&"Espacio antes de la puntuación@ .".to_string()), "{m:?}");
        assert!(m.contains(&"Espacio antes de la puntuación@ ?".to_string()), "{m:?}");
        assert!(m.contains(&"Espacio doble@  ".to_string()), "{m:?}");
        clean("Genial :) Muy bien :-) O :D Valor .5 y puntos suspensivos ...");
        let m = messages("Habló,, y salió.. después;; fin.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Signo repetido")).count(), 3, "{m:?}");
        clean("Esperé… y esperé... hasta que volvió.");
    }

    #[test]
    fn sentence_start_capitals() {
        let m = messages("esto es una prueba. ¡la segunda oración! ¿tercera? bien.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Letra mayúscula")).count(), 4, "{m:?}");
        assert_eq!(check("él tiene razón.")[0].suggestions, ["Él"]);
        assert_eq!(check("¿hola?")[0].suggestions, ["Hola"]);
        assert_eq!(check("¡hola!")[0].suggestions, ["Hola"]);
        assert_eq!(check("Terminó. ¿hola?")[0].suggestions, ["Hola"]);
        clean("¿Hola? ¡Hola! Terminó. ¿Hola?");
        // Abbreviations, initials, ordinals, ellipses and dialogue dashes.
        clean("Compré frutas, etc. y después salimos.");
        clean("Conocí al Dr. Gómez en la Av. Libertador.");
        clean("Zhou quedó en 5. lugar en el cap. XX.");
        clean("Esperé... y él apareció.");
        clean("— Vamos — dijo ele. — Amanhã eu volto.");
        clean("Ele disse: \"Volto amanhã\". E saiu.");
        // A list item may start in lower case.
        clean("manzanas,");
    }

    #[test]
    fn correct_spanish_text_is_clean() {
        clean(
            "El niño tocó la canción en español con mucha pasión. Quien madruga encuentra \
             el camino despejado, y quien tiene paciencia cosecha lo que sembró.",
        );
        clean("Ciudad de México, 12 de octubre de 2026 — una fecha memorable.");
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = check(&s);
        }

        #[test]
        fn never_panics_on_spanish_like_text(s in "[a-záàâãéêíóôõúüçA-Z ,.;:!?\"…\\-—]{0,60}") {
            for i in check(&s) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
                proptest::prop_assert!(s.is_char_boundary(i.start) && s.is_char_boundary(i.end));
            }
        }
    }
}

//! Romanian grammar and punctuation checks: cheap, rule-based and tuned to stay quiet when
//! unsure (a missed mistake costs less than a wrong mark on correct text). Messages are in
//! Romanian.
//!
//! - **Repeated word** (`la la`), except words Romanian doubles on purpose (`încet încet`,
//!   `da da`, interjections).
//! - **Space before punctuation** (`bine ,`) and **double spaces**; emoticons (` :)`) and
//!   ellipses are left alone.
//! - **Capital letter at the start of a sentence**, knowing Romanian abbreviations (`etc.`,
//!   `dl.`, `prof.`, `nr.`, `str.`), initials, ordinal numbers and ellipses, and the low opening
//!   quote `„`.
//! - **Doubled punctuation** (`,,`, `..`, `;;`).
//! - **`nici un` / `nici o`** written apart where the norm (DOOM, 2005) joins them: `niciun`,
//!   `nicio`, `niciunul`, `niciuna`… — not in the correlative `nici …, nici …` (`nici unul,
//!   nici altul`, `nici un frate, nici mai mulți`), where the words stay apart.
//! - **Missing comma before `ci` and `dar`** joining two clauses (`Nu vine azi ci mâine`), not
//!   at a sentence start and not where `dar` is the noun (`un dar`, `de dar`).
//!
//! No English rule (such as `a`/`an`) applies to Romanian text.

use crate::{Issue, IssueKind, ProofLang, words};

/// Words Romanian repeats on purpose: emphatic doubling, interjections, onomatopoeia.
const ALLOWED_REPEATS: &[&str] = &[
    "da", "nu", "încet", "repede", "bine", "mereu", "tare", "hai", "ha", "he", "hi", "ah", "oh", "ei", "aoleu", "vai", "na", "uite", "gata", "pas",
    "puțin", "câte", "unul", "una",
];

/// Abbreviations written with a final period after which a sentence goes on in lower case
/// (lower case, without the period).
const ABBREVIATIONS: &[&str] = &[
    "etc", "dl", "dna", "dra", "dlui", "dnei", "dnul", "dvs", "dv", "prof", "conf", "lect", "asist", "dr", "ing", "ec", "av", "arh", "acad", "pr",
    "sf", "st", "gen", "col", "lt", "mr", "cpt", "sg", "nr", "pag", "pg", "p", "pp", "vol", "cap", "art", "alin", "lit", "pct", "par", "fig", "tab",
    "str", "bd", "bdul", "bl", "sc", "ap", "et", "jud", "mun", "or", "com", "sat", "loc", "tel", "fax", "ex", "cf", "cca", "aprox", "max", "min",
    "sec", "mil", "mld", "lei", "ian", "feb", "febr", "mar", "apr", "iun", "iul", "aug", "sept", "oct", "nov", "dec", "lun", "mie", "joi", "vin",
    "sâm", "dum", "ed", "red", "trad", "coord", "op", "cit", "ibid", "id", "v", "vs", "obs", "n", "s", "a", "î", "d", "hr",
];

/// Words before `dar` that make it the noun (`un dar`, `al darului`), not the conjunction.
const DAR_NOUN_BEFORE: &[&str] = &[
    "un",
    "unui",
    "al",
    "acest",
    "acel",
    "acestui",
    "acelui",
    "cel",
    "de",
    "ca",
    "cu",
    "drept",
    "în",
    "pe",
    "la",
    "din",
    "prin",
    "fără",
    "vreun",
    "niciun",
    "orice",
    "ce",
    "mare",
    "frumos",
    "mic",
    "primul",
    "singurul",
    "adevărat",
    "minunat",
    "divin",
    "și",
    "sau",
    "ori",
    "iar",
    "dar",
];

/// `nici` + this word → the joined form the norm asks for.
const NICI_JOINED: &[(&str, &str)] = &[
    ("un", "niciun"),
    ("o", "nicio"),
    ("unul", "niciunul"),
    ("una", "niciuna"),
    ("unui", "niciunui"),
    ("unei", "niciunei"),
    ("unuia", "niciunuia"),
    ("uneia", "niciuneia"),
];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Ro }
}

/// Is `prev`, before a period, an abbreviation, an initial, a number or a Roman numeral (after
/// which a lower-case word doesn't start a sentence)?
fn abbreviation_or_number(prev: &str) -> bool {
    prev.chars().count() <= 1
        || prev.chars().any(|c| c.is_ascii_digit())
        || (prev.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')))
        || ABBREVIATIONS.contains(&super::comma_below(&prev.to_lowercase()).as_str())
}

/// Does the gap between two words end a sentence: a `.`, `!` or `?` (not an ellipsis, not after
/// an abbreviation), then space, with nothing but quotes and brackets around?
fn ends_sentence(gap: &str, prev: &str) -> bool {
    let t = gap.trim_start_matches(['"', '”', '’', '\'', ')', '»', ']', '\u{1}']);
    let Some(end) = t.chars().next() else { return false };
    if !matches!(end, '.' | '!' | '?') || t.starts_with("..") {
        return false;
    }
    if end == '.' && abbreviation_or_number(prev) {
        return false;
    }
    let rest = t.trim_start_matches(['.', '!', '?']).trim_start_matches(['"', '”', '’', '\'', ')', '»', ']']);
    rest.starts_with(char::is_whitespace) && rest.trim_start().chars().all(|c| matches!(c, '"' | '„' | '“' | '«' | '(' | '[' | '\'' | '—' | '–'))
}

fn repeated_words(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(char::is_alphabetic)
            && super::comma_below(&w0.to_lowercase()) == super::comma_below(&w1.to_lowercase())
            && !ALLOWED_REPEATS.contains(&super::comma_below(&w0.to_lowercase()).as_str())
        {
            v.push(issue(*a0, *b1, format!("Cuvânt repetat: „{w1}”"), vec![w0.to_string()]));
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
            v.push(issue(i, i + 1 + p.len_utf8(), "Spațiu înaintea semnului de punctuație".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Spațiu dublu".into(), vec![" ".into()]));
    }
}

fn sentence_capitals(text: &str, ws: &[(usize, usize)], capital_first: bool, v: &mut Vec<Issue>) {
    let mut after_end = capital_first;
    for (k, (a, b)) in ws.iter().enumerate() {
        let Some(w) = text.get(*a..*b) else { continue };
        if after_end && w.chars().next().is_some_and(char::is_lowercase) && w.chars().all(char::is_alphabetic) {
            let fixed: String = w.chars().next().map(|f| f.to_uppercase().chain(w.chars().skip(1)).collect()).unwrap_or_default();
            v.push(issue(*a, *b, "Majusculă la început de propoziție".into(), vec![fixed]));
        }
        // Only the gap up to the next word decides (`Salut. apoi`: `. ` ends the sentence).
        let next = ws.get(k + 1).map_or(text.len(), |n| n.0);
        after_end = ends_sentence(text.get(*b..next).unwrap_or(""), w);
    }
}

fn doubled_punctuation(text: &str, v: &mut Vec<Issue>) {
    for pat in [",,", "..", ";;", "::", "??", "!!"] {
        for (i, _) in text.match_indices(pat) {
            if pat == ".." && text.get(i + 2..).is_some_and(|r| r.starts_with('.')) {
                continue; // ellipsis
            }
            if pat == ".." && text.get(..i).is_some_and(|t| t.ends_with('.')) {
                continue; // the end of an ellipsis
            }
            v.push(issue(i, i + pat.len(), "Punctuație dublă".into(), vec![pat[..1].into()]));
        }
    }
}

/// Does the gap between two words hold a sentence end or a clause break (`.`, `!`, `?`, `;`,
/// `:`, `,`, a dash)?
fn breaks_clause(gap: &str) -> bool {
    gap.chars().any(|c| matches!(c, '.' | '!' | '?' | ';' | ':' | ',' | '—' | '–'))
}

/// Is `w` the word `want`, in any case (cedilla letters read as comma-below)?
fn is_word(w: &str, want: &str) -> bool {
    super::comma_below(&w.to_lowercase()) == want
}

/// `nici un`, `nici o` … written apart, except in the correlative `nici …, nici …`.
fn nici_apart(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for (k, pair) in ws.windows(2).enumerate() {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !is_word(w0, "nici") || gap != " " {
            continue;
        }
        let Some((_, joined)) = NICI_JOINED.iter().find(|(apart, _)| is_word(w1, apart)) else { continue };
        // Correlative `nici … , nici …` (`nici unul, nici altul`, `nici un frate, nici mai mulți`):
        // `nici` is an adverb there and the words stay apart (DOOM). A later `nici` after a comma
        // in the same sentence counts.
        let mut correlative = false;
        for (j, (a, b)) in ws.iter().enumerate().skip(k + 2) {
            let prev = ws.get(j.saturating_sub(1)).copied().unwrap_or((*a, *a));
            let gap = text.get(prev.1..*a).unwrap_or("");
            // A sentence end stops the search; the period of an abbreviation or number (`nr.`,
            // `5.`) doesn't.
            let prev_word = text.get(prev.0..prev.1).unwrap_or("");
            let abbreviation = gap.trim_start().starts_with('.') && !gap.contains("..") && abbreviation_or_number(prev_word);
            if gap.contains(['!', '?']) || (gap.contains('.') && !abbreviation) {
                break;
            }
            if gap.contains(',') && text.get(*a..*b).is_some_and(|w| is_word(w, "nici")) {
                correlative = true;
                break;
            }
        }
        if correlative {
            continue;
        }
        let fixed = if w0.chars().all(|c| !c.is_lowercase()) && w1.chars().all(|c| !c.is_lowercase()) {
            joined.to_uppercase()
        } else if w0.starts_with(char::is_uppercase) {
            crate::affix::capitalize(joined)
        } else {
            (*joined).to_string()
        };
        v.push(issue(*a0, *b1, format!("Se scrie legat: „{joined}”"), vec![fixed]));
    }
}

/// A missing comma before the adversative `ci` or `dar` between two words of one clause.
fn comma_before_adversative(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    // `Dar…` / `Ci…` opening a sentence has no word before it in the same clause: only pairs of
    // words with nothing but space between them are looked at.
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if breaks_clause(gap) || !gap.chars().all(char::is_whitespace) || gap.is_empty() {
            continue;
        }
        let is_ci = is_word(w1, "ci");
        let is_dar = is_word(w1, "dar") && !DAR_NOUN_BEFORE.contains(&super::comma_below(&w0.to_lowercase()).as_str());
        if !(is_ci || is_dar) || !w0.chars().all(char::is_alphabetic) {
            continue;
        }
        v.push(issue(*b0, *a1, format!("Virgulă lipsă înainte de „{}”", w1.to_lowercase()), vec![", ".into()]));
    }
}

/// Grammar issues in a paragraph of Romanian text, in order.
pub fn check(text: &str) -> Vec<Issue> {
    check_with(text, true)
}

/// [`check`] where `capital_first` says whether the text's first word must start a sentence with
/// a capital (false for a list item, which may be a fragment).
pub fn check_with(text: &str, capital_first: bool) -> Vec<Issue> {
    let ws: Vec<(usize, usize)> = words(text);
    let mut v = Vec::new();
    repeated_words(text, &ws, &mut v);
    spaces(text, &mut v);
    sentence_capitals(text, &ws, capital_first, &mut v);
    doubled_punctuation(text, &mut v);
    nici_apart(text, &ws, &mut v);
    comma_before_adversative(text, &ws, &mut v);
    v.sort_by_key(|i| (i.start, i.end));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(t: &str) -> Vec<String> {
        check(t).into_iter().map(|i| i.message).collect()
    }

    #[test]
    fn romanian_grammar_flags() {
        let t = "acesta acesta este un test ,  făcut prost.. Vezi dl. Popescu";
        let v = check(t);
        let msgs: Vec<&str> = v.iter().map(|i| i.message.as_str()).collect();
        assert!(msgs.contains(&"Cuvânt repetat: „acesta”"), "{msgs:?}");
        assert!(msgs.contains(&"Spațiu înaintea semnului de punctuație"), "{msgs:?}");
        assert!(msgs.contains(&"Spațiu dublu"), "{msgs:?}");
        assert!(msgs.contains(&"Punctuație dublă"), "{msgs:?}");
        assert!(msgs.contains(&"Majusculă la început de propoziție"), "{msgs:?}");
        assert!(v.iter().all(|i| i.lang == ProofLang::Ro), "{v:?}");
    }

    #[test]
    fn a_lower_case_second_sentence_is_flagged() {
        let v = check("Salut. apoi am plecat.");
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!((v[0].message.as_str(), v[0].suggestions[0].as_str()), ("Majusculă la început de propoziție", "Apoi"));
        assert_eq!(&"Salut. apoi am plecat."[v[0].start..v[0].end], "apoi");
        // After a quotation mark or bracket too; never after an abbreviation or an ellipsis.
        assert_eq!(messages("A venit.” apoi a plecat."), ["Majusculă la început de propoziție"]);
        assert!(check("Merge cu dl. Popescu... apoi pleacă.").is_empty());
    }

    #[test]
    fn romanian_grammar_stays_quiet() {
        for t in [
            "Merg la București. Dl. Popescu e deja acolo... încet încet!",
            "Da da, bine bine :)",
            "A sosit la ora 5. pe str. Lungă, nr. 3, etc. și apoi a plecat.",
            "El a spus: „Vin mâine.” Apoi a tăcut.",
            "Nu am nicio carte și niciun caiet.",
            "Nu vreau nici unul, nici altul.",
            "Nu vine azi, ci mâine. Am vrut, dar n-am putut.",
            "Dar cine a venit? A primit un dar frumos.",
            "Cartea a fost un adevărat dar de la el.",
        ] {
            assert!(check(t).is_empty(), "{t}: {:?}", check(t));
        }
    }

    #[test]
    fn nici_un_is_joined() {
        let v = check("Nu am nici un ban și nici o speranță.");
        let fixes: Vec<(&str, &str)> = v.iter().map(|i| (i.message.as_str(), i.suggestions[0].as_str())).collect();
        assert_eq!(fixes, [("Se scrie legat: „niciun”", "niciun"), ("Se scrie legat: „nicio”", "nicio")], "{v:?}");
        let v = check("Nici unul nu a venit.");
        assert_eq!(v.first().map(|i| i.suggestions[0].as_str()), Some("Niciunul"), "{v:?}");
        // The correlative pronouns stay apart; a `nici` in a new sentence doesn't count.
        assert!(check("Nici una, nici alta nu mi-a plăcut.").is_empty());
        assert_eq!(messages("Nu vreau nici unul. Nici altul."), ["Se scrie legat: „niciunul”"]);
        assert_eq!(messages("Nu am nici un ban. Nici el nu are."), ["Se scrie legat: „niciun”"]);
        // Any correlative `nici …, nici …` keeps the words apart (DOOM).
        assert!(check("N-am nici un frate, nici mai mulți.").is_empty());
        // An abbreviation's period doesn't end the sentence the search runs through.
        assert!(check("N-am nici un nr. de telefon, nici alt mijloc de contact.").is_empty());
        // All capitals stay all capitals; an initial capital stays initial.
        let v = check("NU AM NICI UN BAN.");
        assert_eq!(v.first().map(|i| i.suggestions[0].as_str()), Some("NICIUN"), "{v:?}");
    }

    #[test]
    fn comma_before_ci_and_dar() {
        let v = check("Nu vine azi ci mâine.");
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!((v[0].message.as_str(), v[0].suggestions[0].as_str()), ("Virgulă lipsă înainte de „ci”", ", "));
        assert_eq!(&"Nu vine azi ci mâine."[v[0].start..v[0].end], " ");
        assert_eq!(messages("Am vrut dar n-am putut."), ["Virgulă lipsă înainte de „dar”"]);
        // The noun `dar` (a gift) takes no comma.
        assert!(check("I-a făcut un dar.").is_empty());
    }

    #[test]
    fn cedilla_spellings_are_read_too() {
        // `şi` with a cedilla is the same word as `și`.
        assert!(check("Merg şi eu.").is_empty());
        assert_eq!(messages("Nu am nici un leu."), ["Se scrie legat: „niciun”"]);
        // Repeats are compared with the cedilla read as comma-below: `şcoală școală` repeats,
        // and the allowed `puțin puțin` is allowed in either spelling.
        assert_eq!(messages("Merg la şcoală școală."), ["Cuvânt repetat: „școală”"]);
        assert!(check("Mai stai puţin puţin.").is_empty());
    }
}

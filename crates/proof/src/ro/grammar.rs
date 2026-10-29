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
//! - **Contextual diacritics** in enumerated adjacent court phrases (`instanta de judecată`,
//!   `în calitate de parat`), without changing ambiguous standalone words or valid verbs.
//!
//! Context-dependent changes (`nici un`, punctuation before `ci`/`dar`) are left alone:
//! without grammatical analysis they misclassify valid correlative wording, nouns and office
//! abbreviations such as `CI`. This module does not claim full Romanian grammar checking.
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

/// A common typed stand-in for Romanian low opening quotes, without changing the source.
fn ascii_opening_quote(text: &str, at: usize) -> bool {
    let before = text.get(..at).and_then(|s| s.chars().next_back());
    text.get(at..).is_some_and(|s| s.starts_with(",,"))
        && before.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '—' | '–'))
        && text.get(at.saturating_add(2)..).and_then(|s| s.chars().next()).is_some_and(char::is_alphanumeric)
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
            ',' => !ascii_opening_quote(text, i.saturating_add(1)),
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

fn sentence_capitals(text: &str, ws: &[(usize, usize)], capital_first: bool, contextual: &mut [Issue], v: &mut Vec<Issue>) {
    let mut after_end = capital_first;
    // Contextual fixes are in word order. Advance once through them, not a scan per sentence.
    let mut fixes = contextual.iter_mut().peekable();
    for (k, (a, b)) in ws.iter().enumerate() {
        let Some(w) = text.get(*a..*b) else { continue };
        while fixes.peek().is_some_and(|i| i.start < *a) {
            fixes.next();
        }
        if after_end && w.chars().next().is_some_and(char::is_lowercase) && w.chars().all(char::is_alphabetic) {
            let capital = |word: &str| word.chars().next().map(|f| f.to_uppercase().chain(word.chars().skip(1)).collect()).unwrap_or_default();
            if let Some(fix) = fixes.peek_mut().filter(|i| i.start == *a && i.end == *b) {
                // A contextual diacritic fix on this same word also needs the initial capital:
                // offer `Instanța` once, rather than competing `Instanta` / `instanța` marks.
                for suggestion in &mut fix.suggestions {
                    *suggestion = capital(suggestion);
                }
            } else {
                v.push(issue(*a, *b, "Majusculă la început de propoziție".into(), vec![capital(w)]));
            }
        }
        // Only the gap up to the next word decides (`Salut. apoi`: `. ` ends the sentence).
        let next = ws.get(k + 1).map_or(text.len(), |n| n.0);
        after_end = ends_sentence(text.get(*b..next).unwrap_or(""), w);
    }
}

fn doubled_punctuation(text: &str, v: &mut Vec<Issue>) {
    for pat in [",,", "..", ";;", "::", "??", "!!"] {
        for (i, _) in text.match_indices(pat) {
            if pat == ",," && ascii_opening_quote(text, i) {
                continue;
            }
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
    let mut contextual = Vec::new();
    super::context::check(text, &ws, &mut contextual);
    sentence_capitals(text, &ws, capital_first, &mut contextual, &mut v);
    doubled_punctuation(text, &mut v);
    v.extend(contextual);
    v.sort_by_key(|i| (i.start, i.end));
    v
}

#[cfg(test)]
mod tests {
    /// ASCII low opening quotes are not duplicate commas or spaces-before-punctuation errors.
    #[test]
    fn ascii_opening_quotes_are_not_rewritten_as_commas() {
        for text in [r#"El a spus ,,Vin mâine"."#, r#",,Citat"."#, r#"Exemplu —,,Citat"."#, r#"Exemplu [,,Citat"]."#] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        assert!(check("Text,, alt text.").iter().any(|i| i.message == "Punctuație dublă"));
        assert!(check("Text , alt text.").iter().any(|i| i.message == "Spațiu înaintea semnului de punctuație"));
    }

    /// Office abbreviations and contextual wording must not receive speculative replacements.
    #[test]
    fn office_wording_does_not_receive_contextual_grammar_guesses() {
        for text in ["Anexăm copie CI pentru verificare.", "Nu este nici un calculator, nici un telefon.", "Bunul primit este dar, nu împrumut."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
    }

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
    fn cedilla_spellings_are_read_too() {
        // `şi` with a cedilla is the same word as `și`.
        assert!(check("Merg şi eu.").is_empty());
        assert!(check("Nu am nici un leu.").is_empty(), "context-dependent wording stays unchanged");
        // Repeats are compared with the cedilla read as comma-below: `şcoală școală` repeats,
        // and the allowed `puțin puțin` is allowed in either spelling.
        assert_eq!(messages("Merg la şcoală școală."), ["Cuvânt repetat: „școală”"]);
        assert!(check("Mai stai puţin puţin.").is_empty());
    }

    #[test]
    fn contextual_diacritics_recognize_specific_court_phrases() {
        for (text, wrong, fixed) in [
            ("Instanta de judecată a stabilit termenul.", "Instanta", "Instanța"),
            ("Instanta de apel a stabilit termenul.", "Instanta", "Instanța"),
            ("Ne adresăm instantei de recurs.", "instantei", "instanței"),
            ("Cererea se află pe rolul instantei.", "instantei", "instanței"),
            ("Ne prezentăm în fața instantei.", "instantei", "instanței"),
            ("Persoana are calitatea de parat.", "parat", "pârât"),
            ("Persoana figurează în calitate de parat.", "parat", "pârât"),
            ("Cererea se judecă în contradictoriu cu paratul.", "paratul", "pârâtul"),
            ("Reclamantul și paratul au fost citați.", "paratul", "pârâtul"),
            ("Paratul a depus întâmpinare.", "Paratul", "Pârâtul"),
            ("Se solicită citarea paratului.", "paratului", "pârâtului"),
            ("Se solicită obligarea paratului.", "paratului", "pârâtului"),
        ] {
            let found: Vec<_> = check(text).into_iter().filter(|i| i.message.starts_with("Diacritice în context juridic:")).collect();
            assert_eq!(found.len(), 1, "{text}: {found:?}");
            let i = &found[0];
            assert_eq!(text.get(i.start..i.end), Some(wrong), "{text}");
            assert_eq!(i.suggestions, [fixed], "{text}");
            assert_eq!(i.kind, IssueKind::Grammar);
            assert_eq!(i.lang, ProofLang::Ro);
        }
    }

    #[test]
    fn contextual_diacritics_leave_other_senses_and_literals_alone() {
        for text in [
            "Portarul a parat lovitura.",
            "Sportivul a parat atacul.",
            "Atacul a fost parat.",
            "A parat de fiecare dată.",
            "Este cafea instant.",
            "Textul de probă: instanta.",
            "Textul de probă: parat.",
            "Instanta se inițializează.",
            "Instanta de apelare a funcției este separată.",
            "Instanța de judecată a stabilit termenul.",
            "Persoana are calitatea de pârât.",
            "Exemplu: instanta_de_judecată.",
            "Exemplu_instanta de judecată.",
            "Exemplu: instanta.de.judecată.",
            "Exemplu: https://example.invalid/instanta/de/judecată.",
            "Se folosește «instanta» de judecată ca exemplu.",
            "În exemplu, `instanta de judecată` este text literal.",
            "Textul menționează calitate de «parat».",
            "Instanta. De judecată se discută separat.",
            "Instanta\nde judecată este pe două linii.",
            "Se discută despre calitate de. Parat este alt cuvânt.",
        ] {
            let found: Vec<_> = check(text).into_iter().filter(|i| i.message.starts_with("Diacritice în context juridic:")).collect();
            assert!(found.is_empty(), "{text}: {found:?}");
        }
    }

    #[test]
    fn contextual_diacritics_keep_case_unicode_offsets_and_whitespace() {
        use unicode_normalization::UnicodeNormalization;
        for (text, wrong, fixed) in [
            ("INSTAnta de judecată.", "", ""),
            ("INSTANTA DE JUDECATĂ.", "INSTANTA", "INSTANȚA"),
            ("Ne prezentăm în faţa instantei.", "instantei", "instanței"),
            ("Reclamantul şi paratul au fost citați.", "paratul", "pârâtul"),
            ("Cererea este pe\u{a0}rolul\u{a0}instantei.", "instantei", "instanței"),
        ] {
            let source: String = text.nfd().collect();
            let found: Vec<_> = check(&source).into_iter().filter(|i| i.message.starts_with("Diacritice în context juridic:")).collect();
            if wrong.is_empty() {
                assert!(found.is_empty(), "{source}: {found:?}");
                continue;
            }
            assert_eq!(found.len(), 1, "{source}: {found:?}");
            assert_eq!(source.get(found[0].start..found[0].end), Some(wrong));
            assert_eq!(found[0].suggestions, [fixed]);
        }
    }

    #[test]
    fn contextual_diacritics_merge_sentence_capitals_but_keep_list_fragments() {
        let text = "instanta de judecată a stabilit termenul.";
        let found = check(text);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].suggestions, ["Instanța"]);
        let fragment = check_with(text, false);
        assert_eq!(fragment.len(), 1, "{fragment:?}");
        assert_eq!(fragment[0].suggestions, ["instanța"]);
        let next = check("Salut. instanta de judecată a stabilit termenul.");
        assert_eq!(next.len(), 1, "{next:?}");
        assert_eq!(next[0].suggestions, ["Instanța"]);
    }
}

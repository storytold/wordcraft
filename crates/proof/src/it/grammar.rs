//! Italian grammar and punctuation checks: cheap, rule-based and tuned to stay quiet when
//! unsure (a missed mistake costs less than a wrong mark on correct text). Messages are in
//! Italian.
//!
//! - **Repeated word** (`la la`), except words Italian doubles on purpose (`piano piano`,
//!   `sì sì`, interjections).
//! - **Space before punctuation** (`bene ,`) and **double spaces**; emoticons (` :)`) and
//!   ellipses are left alone.
//! - **Capital letter at the start of a sentence**, knowing Italian abbreviations (`ecc.`,
//!   `dott.`, `prof.`, `sig.`), initials, ordinal numbers (`1. posto`) and ellipses.
//! - **Doubled punctuation** (`,,`, `..`, `;;`).
//! - **`po'`**: `un pò` and `un po` → `un po'` (a truncation of `poco`: apostrophe, no accent).
//! - **`qual è`**: `qual'è`, `qual'era` → `qual è`, `qual era` (a truncation, never elided).
//! - **`È`**: `E'` written for the verb → `È`.
//! - **`un` / `un'`**: the apostrophe only before a feminine word. `un'altro`, `un'anno` →
//!   `un altro`, `un anno` (a masculine word in `-o`; `auto` and `eco` are feminine), and
//!   `un altra`, `un amica` → `un'altra`, `un'amica` (a word in `-a`, except the masculine and
//!   common-gender ones: `-ista`, `-ita`, `-ota`, `-cida`, `-ma`, `-eta`, `atleta`, `acrobata`…).
//!   The same for `nessun`, `ciascun`, `qualcun` and `alcun`.
//!
//! No comma rule and no English rule (such as `a`/`an`) applies to Italian text.

use crate::{Issue, IssueKind, ProofLang, words};

/// Words Italian repeats on purpose: emphatic doubling, interjections, onomatopoeia.
const ALLOWED_REPEATS: &[&str] = &[
    "sì", "no", "piano", "bene", "male", "ecco", "oh", "ah", "eh", "mh", "bah", "ehi", "urrà", "evviva", "ciao", "addio", "adagio", "forte", "lento",
    "chiaro", "pian", "poco", "quasi", "presto", "subito", "zitto", "via", "così", "tanto", "molto", "pure", "dai", "su", "avanti", "basta", "aiuto",
];

/// Indefinite articles (and the like) that elide to `un'` before a feminine word only.
const UN_FORMS: &[&str] = &["un", "nessun", "ciascun", "qualcun", "alcun"];

/// Feminine words in `-o` that start with a vowel (`un'auto`, `un'eco`).
const FEMININE_IN_O: &[&str] = &["auto", "autoradio", "eco", "ecografo"];

/// Endings of masculine or common-gender words in `-a` (`un artista`, `un idiota`, `un enigma`).
const MASCULINE_A_ENDINGS: &[&str] = &["ista", "ita", "ota", "cida", "ma", "eta", "nauta", "iatra", "arca"];

/// Words in `-a` after `un` that aren't feminine nouns: masculine or common-gender ones that fit
/// no ending above, and verbs and pronouns (`un ha`, `un una`).
const NOT_FEMININE_IN_A: &[&str] = &["acrobata", "autodidatta", "alfa", "omega", "ha", "una"];

/// Abbreviations written with a final period after which a sentence goes on in lower case
/// (lower case, without the period).
const ABBREVIATIONS: &[&str] = &[
    "ecc",
    "es",
    "dott",
    "prof",
    "profssa",
    "ing",
    "avv",
    "sig",
    "sigg",
    "sigra",
    "sigr",
    "geom",
    "rag",
    "dottssa",
    "avvoc",
    "pag",
    "pagg",
    "tel",
    "art",
    "artt",
    "fig",
    "figg",
    "vol",
    "voll",
    "ed",
    "sez",
    "cap",
    "capp",
    "par",
    "n",
    "num",
    "vs",
    "cfr",
    "cfrt",
    "op",
    "cit",
    "ibid",
    "sez",
    "all",
    "tav",
    "pag",
    "ditta",
    "soc",
    "spa",
    "srl",
    "on",
    "sen",
    "spett",
    "eg",
    "ca",
    "v",
    "vv",
    "sec",
    "min",
    "max",
    "lun",
    "mar",
    "mer",
    "gio",
    "ven",
    "sab",
    "dom",
    "gen",
    "feb",
    "mar",
    "apr",
    "mag",
    "giu",
    "lug",
    "ago",
    "set",
    "ott",
    "nov",
    "dic",
    "genn",
    "febb",
    "sett",
    "dottor",
    "professor",
    "cav",
    "commend",
    "mons",
    "fr",
    "suor",
    "s",
    "ss",
    "b",
    "beato",
];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::It }
}

/// Is `prev`, before a period, an abbreviation, an initial, a number or a Roman numeral (after
/// which a lower-case word doesn't start a sentence)?
fn abbreviation_or_number(prev: &str) -> bool {
    prev.chars().count() <= 1
        || prev.chars().any(|c| c.is_ascii_digit())
        || (prev.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')))
        || ABBREVIATIONS.contains(&prev.to_lowercase().as_str())
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
    rest.starts_with(char::is_whitespace) && rest.trim_start().chars().all(|c| matches!(c, '"' | '“' | '«' | '(' | '[' | '\'' | '—'))
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
            v.push(issue(*a0, *b1, format!("Parola ripetuta: \"{w1}\""), vec![w0.to_string()]));
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
            v.push(issue(i, i + 1 + p.len_utf8(), "Spazio prima della punteggiatura".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Spazio doppio".into(), vec![" ".into()]));
    }
}

fn sentence_capitals(text: &str, ws: &[(usize, usize)], capital_first: bool, v: &mut Vec<Issue>) {
    let mut after_end = capital_first;
    for (a, b) in ws {
        let Some(w) = text.get(*a..*b) else { continue };
        if after_end && w.chars().next().is_some_and(char::is_lowercase) && w.chars().all(char::is_alphabetic) {
            let fixed: String = w.chars().next().map(|f| f.to_uppercase().chain(w.chars().skip(1)).collect()).unwrap_or_default();
            v.push(issue(*a, *b, "Maiuscola a inizio frase".into(), vec![fixed]));
        }
        after_end = ends_sentence(text.get(*b..).unwrap_or(""), w);
    }
}

fn doubled_punctuation(text: &str, v: &mut Vec<Issue>) {
    for pat in [",,", "..", ";;", "::", "??", "!!"] {
        for (i, _) in text.match_indices(pat) {
            if pat == ".." && text.get(i + 2..).is_some_and(|r| r.starts_with('.')) {
                continue; // ellipsis
            }
            v.push(issue(i, i + pat.len(), "Punteggiatura doppia".into(), vec![pat[..1].into()]));
        }
    }
}

/// Is `c` an apostrophe (straight or typographic)?
fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '’')
}

/// `pò` (never accented) and `po` without its apostrophe → `po'`. `Po` (the river) is left alone.
fn po(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for (a, b) in ws {
        let Some(w) = text.get(*a..*b) else { continue };
        let apostrophe = text.get(*b..).and_then(|r| r.chars().next()).is_some_and(is_apostrophe);
        let fix = match w {
            "pò" => "po'",
            "Pò" => "Po'",
            "po" if !apostrophe => "po'",
            _ => continue,
        };
        v.push(issue(*a, *b, "Si scrive «po'», con l'apostrofo e senza accento".into(), vec![fix.into()]));
    }
}

/// `qual'è` → `qual è` (any case): the fix when `word` is `qual` + apostrophe + `è`/`era`.
pub(crate) fn qual_elided(word: &str) -> Option<String> {
    let head_end = word.char_indices().take(4).last().map(|(i, c)| i + c.len_utf8())?;
    let (head, rest) = (word.get(..head_end)?, word.get(head_end..)?);
    let tail = rest.strip_prefix(is_apostrophe)?;
    let tail_ok = matches!(tail.to_lowercase().as_str(), "è" | "era");
    (head.eq_ignore_ascii_case("qual") && tail_ok).then(|| format!("{head} {tail}"))
}

fn qual_e(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for (a, b) in ws {
        if let Some(fix) = text.get(*a..*b).and_then(qual_elided) {
            v.push(issue(*a, *b, "«Qual è» si scrive senza apostrofo".into(), vec![fix]));
        }
    }
}

/// `E'` for the verb → `È`. A quoted letter (`la lettera 'E'`) is left alone.
fn e_apostrophe(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for (a, b) in ws {
        if text.get(*a..*b) != Some("E") {
            continue;
        }
        let mut after = text.get(*b..).unwrap_or("").chars();
        let Some(ap) = after.next().filter(|&c| is_apostrophe(c)) else { continue };
        let ends = after.next().is_none_or(char::is_whitespace);
        let quoted = text.get(..*a).and_then(|t| t.chars().next_back()).is_some_and(|c| is_apostrophe(c) || c == '‘');
        if ends && !quoted {
            v.push(issue(*a, *b + ap.len_utf8(), "«È» si scrive con l'accento, non con l'apostrofo".into(), vec!["È".into()]));
        }
    }
}

/// Does `w` (lower case) start with a vowel or `h`, as the words `un'` elides before?
fn starts_with_vowel(w: &str) -> bool {
    w.chars().next().is_some_and(|c| "aeiouhàèéìòóù".contains(c))
}

/// Is `w` (lower case) a feminine word in `-a` (`altra`, `amica`, `ora`), as far as the ending
/// tells?
fn feminine_in_a(w: &str) -> bool {
    w.chars().count() >= 2 && w.ends_with('a') && !MASCULINE_A_ENDINGS.iter().any(|e| w.ends_with(e)) && !NOT_FEMININE_IN_A.contains(&w)
}

/// `un'` before a masculine word (`un'altro` → `un altro`), and `un` without the apostrophe
/// before a feminine one (`un altra` → `un'altra`). Only lower-case words: a name's gender isn't
/// in its ending.
fn un_apostrophe(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for (a, b) in ws {
        let Some(w) = text.get(*a..*b) else { continue };
        let Some((head, tail)) = w.split_once(is_apostrophe) else { continue };
        if UN_FORMS.contains(&head.to_lowercase().as_str())
            && tail.chars().all(|c| c.is_alphabetic() && c.is_lowercase())
            && starts_with_vowel(tail)
            && tail.ends_with('o')
            && !FEMININE_IN_O.contains(&tail)
        {
            v.push(issue(*a, *b, format!("Davanti a un nome maschile «{head}» non vuole l'apostrofo"), vec![format!("{head} {tail}")]));
        }
    }
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if gap == " "
            && UN_FORMS.contains(&w0.to_lowercase().as_str())
            && w1.chars().all(|c| c.is_alphabetic() && c.is_lowercase())
            && starts_with_vowel(w1)
            && feminine_in_a(w1)
        {
            v.push(issue(*a0, *b1, format!("Davanti a un nome femminile si scrive «{w0}'», con l'apostrofo"), vec![format!("{w0}'{w1}")]));
        }
    }
}

/// Grammar issues in a paragraph of Italian text, in order.
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
    po(text, &ws, &mut v);
    qual_e(text, &ws, &mut v);
    e_apostrophe(text, &ws, &mut v);
    un_apostrophe(text, &ws, &mut v);
    v.sort_by_key(|i| (i.start, i.end));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italian_grammar_flags() {
        test_dict_install();
        let t = "questo questo è un test ,  fatto male.. Vedi dott. rossi";
        let v = check(t);
        let msgs: Vec<&str> = v.iter().map(|i| i.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("ripetuta")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("prima della punteggiatura")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("doppio")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("doppia")), "{msgs:?}");
        assert!(v.iter().all(|i| i.lang == ProofLang::It), "{v:?}");
    }

    #[test]
    fn italian_grammar_stays_quiet() {
        test_dict_install();
        assert!(
            check("Vado a Roma. Il dott. Rossi è già lì... piano piano!").is_empty(),
            "{:?}",
            check("Vado a Roma. Il dott. Rossi è già lì... piano piano!")
        );
        assert!(check("Sì sì, bene bene :)").is_empty());
    }

    /// The fixes the rules offer for `t`, in order.
    fn fixes(t: &str) -> Vec<(String, String)> {
        check(t).into_iter().map(|i| (t.get(i.start..i.end).unwrap_or("").to_string(), i.suggestions.join("|"))).collect()
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn po_takes_an_apostrophe() {
        assert_eq!(fixes("Dammi un pò di pane"), [pair("pò", "po'")]);
        assert_eq!(fixes("Dammi un po di pane"), [pair("po", "po'")]);
        assert_eq!(fixes("Pò di più"), [pair("Pò", "Po'")]);
        assert!(fixes("Dammi un po' di pane, un po’ d'acqua").is_empty());
        assert!(fixes("Il Po scorre a Torino").is_empty(), "the river");
    }

    #[test]
    fn qual_e_has_no_apostrophe() {
        assert_eq!(fixes("Dimmi qual'è il problema"), [pair("qual'è", "qual è")]);
        assert_eq!(fixes("Qual’era la strada?"), [pair("Qual’era", "Qual era")]);
        assert!(fixes("Qual è il problema? Quale era?").is_empty());
        assert!(fixes("Quale'è").is_empty(), "not the truncation");
        assert_eq!(qual_elided("qual'è").as_deref(), Some("qual è"));
        assert_eq!(qual_elided("qua"), None);
        assert_eq!(qual_elided(""), None);
    }

    #[test]
    fn e_apostrophe_becomes_e_grave() {
        assert_eq!(fixes("E' vero"), [pair("E'", "È")]);
        assert_eq!(fixes("Lo so. E’ tardi"), [pair("E’", "È")]);
        assert!(fixes("È vero. E poi?").is_empty());
        assert!(fixes("La lettera 'E' è una vocale").is_empty(), "a quoted letter");
    }

    #[test]
    fn un_apostrophe_follows_the_gender() {
        assert_eq!(fixes("Ho visto un'altro amico"), [pair("un'altro", "un altro")]);
        assert_eq!(fixes("È passato un’anno"), [pair("un’anno", "un anno")]);
        assert_eq!(fixes("Prendo un altra strada"), [pair("un altra", "un'altra")]);
        assert_eq!(fixes("Ho un amica a Roma"), [pair("un amica", "un'amica")]);
        assert_eq!(fixes("Non c'è nessun altra via"), [pair("nessun altra", "nessun'altra")]);
        for ok in [
            "Un altro amico",
            "Un'altra amica",
            "Un'auto nuova",
            "Un'eco lontana",
            "Un artista",
            "Un atleta",
            "Un idiota",
            "Un ipocrita",
            "Un enigma",
            "Un omicida",
            "Un astronauta",
            "Un acrobata",
            "Un'ora",
            "Un'idea",
            "Un uomo",
            "Un'Alfa",
            "Un Andrea",
            "Un ha",
        ] {
            assert!(fixes(ok).is_empty(), "{ok}: {:?}", fixes(ok));
        }
    }

    #[test]
    fn more_emphatic_doubles_are_allowed() {
        for ok in ["Pian piano", "Poco poco", "Zitto zitto", "Via via", "Così così", "Dai dai"] {
            assert!(fixes(ok).is_empty(), "{ok}: {:?}", fixes(ok));
        }
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            for i in check(&s) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
            }
        }

        #[test]
        fn never_panics_on_italian_like_text(s in "[a-zàèéìòóùA-ZÈ ,.;:!?'’‘\"«»…\\-]{0,60}") {
            for i in check(&s) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len() && s.is_char_boundary(i.start) && s.is_char_boundary(i.end));
            }
        }
    }

    fn test_dict_install() {
        crate::it::test_dict::install();
    }
}

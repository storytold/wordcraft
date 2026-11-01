//! French grammar and typography checks: cheap, rule-based and tuned to stay quiet when unsure
//! (a missed mistake costs less than a wrong mark on correct text). Messages are in French.
//!
//! - **Space before `;` `!` `?` `:`**: French typography puts a non-breaking space there — a
//!   narrow one (U+202F) before `;` `!` `?`, a full one (U+00A0) before `:`. Only a missing
//!   space is flagged (`Bonjour!`): an ordinary one stays quiet until typing turns it into a
//!   non-breaking one (no French AutoFormat yet). Quebec French (`fr-CA`) has no space before
//!   `;` `!` `?`, only before `:`. Addresses (`https://…`) and times (`10:30`) are left alone.
//! - **Space inside `« »`**: a missing one after `«` or before `»` (`«oui»`) → U+00A0.
//! - **Straight double quotes** around a phrase (`"bonjour"`) → `« bonjour »`; inch marks
//!   (`27"`) and unbalanced quotes are left alone.
//! - **Repeated word** (`le le`, `que que`), except repetitions French makes on purpose
//!   (`nous nous lavons`, `vous vous trompez`, `si si`, interjections) and a proper noun after
//!   the same lower-case word (`de De…`).
//! - **Capital letter at the start of a sentence** after `.` `!` `?`, knowing French
//!   abbreviations (`etc.`, `M.`, `p.`, `cf.`), initials, numbers, ellipses, addresses,
//!   dialogue (`« Tu viens ? » demanda-t-elle`), inversions (`Quoi ? dit-il`) and exclamations
//!   after an interjection (`Ah ! que c'est beau`).
//! - **Elision** (`de avoir` → `d’avoir`, `je ai` → `j’ai`, `le homme` → `l’homme`) for `le`,
//!   `la`, `de`, `je`, `me`, `te`, `se`, `ne`, `que` before a vowel. Before an `h` only for words
//!   known to have a mute `h`, so aspirated ones (`le hibou`, `la hache`, `le héros`) stay quiet;
//!   never before a proper noun, `onze`, `oui`, `y…` words (`le yaourt`) or `la une`.
//!
//! No agreement or conjugation rule, and no English rule (such as `a`/`an`), applies to French
//! text.

use crate::{Issue, IssueKind, ProofLang, words};

/// Non-breaking space, before `:` and inside `« »`.
const NBSP: char = '\u{A0}';
/// Narrow non-breaking space, before `;` `!` `?`.
const NNBSP: char = '\u{202F}';

/// Words French repeats on purpose: pronominal verbs (`nous nous lavons`), emphasis,
/// interjections, onomatopoeia.
const ALLOWED_REPEATS: &[&str] = &[
    "nous", "vous", "si", "oui", "non", "très", "bon", "vite", "là", "ah", "oh", "eh", "ha", "hé", "ho", "hi", "bla", "chut", "toc", "pan", "bravo",
    "merci", "allez", "encore", "beaucoup", "fort", "tchin", "bye", "couci", "tam", "miam", "glou",
];

/// Abbreviations written with a final period after which a sentence goes on in lower case
/// (lower case, without the period).
const ABBREVIATIONS: &[&str] = &[
    "etc", "cf", "ex", "env", "av", "apr", "mm", "mme", "mmes", "mlle", "mlles", "mgr", "dr", "pr", "me", "st", "ste", "sts", "pp", "vol", "chap",
    "fig", "éd", "coll", "trad", "réf", "tél", "art", "al", "op", "cit", "ibid", "id", "resp", "min", "max", "sq", "sqq", "suiv", "hab", "janv",
    "févr", "avr", "juil", "sept", "oct", "nov", "déc", "bd", "boul", "cie", "éts", "vve", "nb", "ps", "no", "nos", "vs", "qqch", "qqn", "ca",
    "approx", "prof", "dir", "sté", "dépt", "adj", "adv", "fém", "masc", "sing", "plur", "à-d", "ms", "mss", "éds", "trim", "hôp",
];

/// Words after which `!` or `?` doesn't end the sentence (`Ah ! que c'est beau`, `Eh bien ?
/// et alors`).
const INTERJECTIONS: &[&str] = &[
    "ah", "oh", "eh", "hé", "hein", "hélas", "bah", "ô", "aïe", "ouf", "oups", "quoi", "comment", "ha", "ho", "bon", "tiens", "allons", "zut",
    "chut", "bravo", "hourra", "dame", "bien", "là", "oui", "non", "pardon", "voyons",
];

/// Pronouns ending an inversion (`dit-il`, `demanda-t-elle`): after one, a `!` or `?` doesn't
/// end the sentence (`Quoi ? dit-il`).
const INVERTED: &[&str] = &["-il", "-elle", "-on", "-ils", "-elles", "-je", "-tu", "-nous", "-vous"];

/// Speech verbs of a dialogue tag: after one, a `!` or `?` doesn't end the sentence (`— Tu
/// viens ? demanda le voisin`).
const SPEECH_VERBS: &[&str] = &[
    "dit",
    "demanda",
    "répondit",
    "cria",
    "murmura",
    "ajouta",
    "reprit",
    "fit",
    "lança",
    "souffla",
    "répliqua",
    "interrogea",
    "hurla",
    "poursuivit",
    "continua",
    "déclara",
    "expliqua",
    "protesta",
    "soupira",
    "chuchota",
    "insista",
    "s'écria",
    "s’écria",
    "s'exclama",
    "s’exclama",
    "objecta",
    "conclut",
    "dis",
    "dit-on",
];

/// The words that elide before a vowel, and their elided form.
const ELIDING: &[(&str, &str)] =
    &[("le", "l"), ("la", "l"), ("de", "d"), ("je", "j"), ("me", "m"), ("te", "t"), ("se", "s"), ("ne", "n"), ("que", "qu")];

/// Words starting with a vowel that don't take an elision (`le onze`, `le oui`, `de ouate`).
const NO_ELISION: &[&str] = &["onze", "onzième", "onzièmes", "oui", "ouistiti", "ouistitis", "ouate", "ouates", "uhlan", "uhlans"];

/// Words with a mute `h`, which take an elision (`l’homme`, `d’habitude`, `j’habite`). Only these
/// are flagged: an `h` word outside the list may be aspirated (`le hibou`, `la hache`, `le
/// héros`), and staying quiet is the safe side.
const MUTE_H: &[&str] = &[
    "homme",
    "hommes",
    "heure",
    "heures",
    "histoire",
    "histoires",
    "hiver",
    "hivers",
    "hôtel",
    "hôtels",
    "herbe",
    "herbes",
    "habitude",
    "habitudes",
    "habitant",
    "habitants",
    "habit",
    "habits",
    "hôpital",
    "hôpitaux",
    "humain",
    "humains",
    "humanité",
    "honneur",
    "honneurs",
    "horizon",
    "horizons",
    "huile",
    "huiles",
    "hypothèse",
    "hypothèses",
    "harmonie",
    "héritage",
    "héroïne",
    "héroïnes",
    "hésitation",
    "hommage",
    "hommages",
    "horaire",
    "horaires",
    "horreur",
    "horreurs",
    "humeur",
    "humour",
    "hygiène",
    "hymne",
    "habiter",
    "hériter",
    "hésiter",
    "habiller",
    "honorer",
    "habite",
    "habites",
    "habitent",
    "habitais",
    "habitait",
    "hésite",
    "hésites",
    "hésitent",
    "hésitais",
    "hésitait",
    "habille",
    "habillent",
    "honore",
    "hier",
    "hôte",
    "hôtes",
    "hôtesse",
    "humidité",
    "haleine",
    "hebdomadaire",
    "hectare",
    "hectares",
    "hélicoptère",
    "hémisphère",
    "heureux",
    "heureuse",
    "hippopotame",
    "hirondelle",
    "honnête",
    "honnêteté",
    "huître",
    "huîtres",
    "humble",
    "humilité",
    "hydrogène",
    "hypocrite",
    "horloge",
    "hôtellerie",
    "hérédité",
    "historien",
    "historique",
    "humanitaire",
    "hormone",
    "hormones",
    "hospitalité",
];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Fr }
}

/// Is `c` a space that doesn't break (or a thin space some writers use instead)?
fn is_fixed_space(c: char) -> bool {
    matches!(c, '\u{A0}' | '\u{202F}' | '\u{2007}' | '\u{2009}' | '\u{2060}')
}

/// The byte range of the whitespace-delimited token around byte `i` of `text`.
fn token_bounds(text: &str, i: usize) -> (usize, usize) {
    let start = text.get(..i).and_then(|t| t.char_indices().rev().find(|(_, c)| c.is_whitespace())).map_or(0, |(k, c)| k + c.len_utf8());
    let end = text.get(i..).and_then(|t| t.find(char::is_whitespace)).map_or(text.len(), |k| i + k);
    (start, end)
}

/// The whitespace-delimited token around byte `i` of `text`.
fn token_at(text: &str, i: usize) -> &str {
    let (start, end) = token_bounds(text, i);
    text.get(start..end).unwrap_or("")
}

/// Is `token` a web or e-mail address, a path or an HTML entity, where punctuation isn't prose?
fn is_address(token: &str) -> bool {
    token.contains("://") || token.contains('@') || token.starts_with("www.") || token.contains('&') || token.contains('\\')
}

/// `«` + text + `»` in a message, with French spacing.
fn quoted(s: &str) -> String {
    format!("«{NBSP}{s}{NBSP}»")
}

/// `;` `!` `?` `:` without the non-breaking space French puts before them.
/// Quebec typography (`quebec`) puts no space before `;` `!` `?`, only before `:`.
fn high_punctuation(text: &str, quebec: bool, v: &mut Vec<Issue>) {
    let mut token: Option<(usize, usize, bool)> = None;
    for (i, p) in text.char_indices() {
        if !matches!(p, ';' | ':' | '!' | '?') || (quebec && p != ':') {
            continue;
        }
        let before = text.get(..i).and_then(|t| t.chars().next_back());
        let after = text.get(i + p.len_utf8()..).and_then(|t| t.chars().next());
        // One look per token, so a long run of marks stays linear.
        let address = match token {
            Some((start, end, address)) if start <= i && i < end => address,
            _ => {
                let (start, end) = token_bounds(text, i);
                let address = is_address(text.get(start..end).unwrap_or(""));
                token = Some((start, end, address));
                address
            }
        };
        if address {
            continue;
        }
        let (space, kind) = if p == ':' { (NBSP, "insécable") } else { (NNBSP, "fine insécable") };
        let message = format!("Espace {kind} attendue avant {}", quoted(&p.to_string()));
        let fix = format!("{space}{p}");
        // Only a missing space is flagged: an ordinary one stays quiet until typing turns it into
        // a non-breaking one by itself (French AutoFormat).
        if before.is_some_and(char::is_alphabetic) {
            // `10:30`, `a:b`, `C:` and `x;y` aren't prose: the mark must end the word.
            let ends = match p {
                ':' => after.is_none_or(char::is_whitespace),
                _ => after.is_none_or(|a| a.is_whitespace() || matches!(a, '!' | '?' | '»' | '"' | '”' | ')')),
            };
            if ends {
                v.push(issue(i, i + p.len_utf8(), message, vec![fix]));
            }
        }
    }
}

/// `«` and `»` without a non-breaking space inside.
fn guillemets(text: &str, v: &mut Vec<Issue>) {
    for (i, c) in text.char_indices() {
        let len = c.len_utf8();
        match c {
            '«' => {
                let mut next = text.get(i + len..).unwrap_or("").chars();
                let message = format!("Espace insécable attendue après {}", quoted("«"));
                if next.next().is_some_and(char::is_alphanumeric) {
                    v.push(issue(i, i + len, message, vec![format!("«{NBSP}")]));
                }
            }
            '»' => {
                let prev = text.get(..i).unwrap_or("").chars().next_back();
                let message = format!("Espace insécable attendue avant {}", quoted("»"));
                if prev.is_some_and(|n| n.is_alphanumeric() || matches!(n, '.' | '!' | '?' | '…')) {
                    v.push(issue(i, i + len, message, vec![format!("{NBSP}»")]));
                }
            }
            _ => {}
        }
    }
}

/// `"…"` around a phrase, where French uses `« … »`.
fn straight_quotes(text: &str, v: &mut Vec<Issue>) {
    let quotes: Vec<usize> = text.match_indices('"').map(|(i, _)| i).collect();
    // An odd count leaves no telling which quote opens and which closes.
    if !quotes.len().is_multiple_of(2) {
        return;
    }
    for pair in quotes.chunks(2) {
        let [a, b] = pair else { continue };
        let (Some(inner), Some(before), Some(after)) = (text.get(a + 1..*b), text.get(..*a), text.get(b + 1..)) else { continue };
        let opens = before.chars().next_back().is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '—' | '–'));
        let closes = after.chars().next().is_none_or(|c| c.is_whitespace() || matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '…'));
        let phrase = inner.trim();
        if !opens || !closes || !phrase.chars().any(char::is_alphabetic) || phrase.chars().count() > 120 || phrase.contains('\n') {
            continue;
        }
        v.push(issue(*a, b + 1, format!("Guillemets français attendus{NBSP}: {}", quoted(phrase)), vec![quoted(phrase)]));
    }
}

fn repeated_words(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        let lower = w0.to_lowercase();
        let starts_upper = |w: &str| w.chars().next().is_some_and(char::is_uppercase);
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(char::is_alphabetic)
            && lower == w1.to_lowercase()
            && !ALLOWED_REPEATS.contains(&lower.as_str())
            // `de De…`, `Bora Bora`: a capitalized second word is a name.
            && !starts_upper(w1)
        {
            v.push(issue(*a0, *b1, format!("Mot répété{NBSP}: {}", quoted(w1)), vec![w0.to_string()]));
        }
    }
}

/// Is `prev`, before a period, an abbreviation, an initial, a number or a Roman numeral (after
/// which a lower-case word doesn't start a sentence)?
fn abbreviation_or_number(prev: &str) -> bool {
    prev.chars().count() <= 1
        || prev.chars().any(|c| c.is_ascii_digit())
        || prev.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M'))
        || ABBREVIATIONS.contains(&prev.to_lowercase().as_str())
}

/// Does `gap`, between the words `prev` and `next`, end a sentence: a `.`, `!` or `?` (French
/// puts a space before the last two), then a space, with only quotes and brackets around? Not
/// an ellipsis, not after an abbreviation; for `!` and `?`, not after an interjection, not before
/// a closing quote (dialogue goes on: `« Tu viens ? » demanda-t-elle`), not before an inversion
/// or a speech verb (`Quoi ? dit-il`, `— Viens ! cria-t-on`), and never in a `dialogue` line (one
/// opening with a dash).
fn ends_sentence(gap: &str, prev: &str, next: &str, dialogue: bool) -> bool {
    let closing = ['"', '”', '’', '\'', ')', '»', ']'];
    let t = gap.trim_start_matches(|c: char| c.is_whitespace() || is_fixed_space(c) || closing.contains(&c));
    let Some(end) = t.chars().next() else { return false };
    if !matches!(end, '.' | '!' | '?') || t.starts_with("..") {
        return false;
    }
    if end == '.' && abbreviation_or_number(prev) {
        return false;
    }
    let exclamation = end != '.';
    let lower_next = next.to_lowercase();
    if exclamation
        && (dialogue
            || INTERJECTIONS.contains(&prev.to_lowercase().as_str())
            || INVERTED.iter().any(|p| lower_next.ends_with(p))
            || SPEECH_VERBS.contains(&lower_next.as_str()))
    {
        return false;
    }
    let rest = t.trim_start_matches(['.', '!', '?']);
    let after_marks = rest.trim_start_matches(is_fixed_space);
    if after_marks.starts_with(closing) {
        if exclamation {
            return false;
        }
        return ends_sentence_after(after_marks.trim_start_matches(|c: char| closing.contains(&c) || is_fixed_space(c)));
    }
    ends_sentence_after(rest)
}

/// Is `rest` (after the final mark) a space, then nothing but opening quotes and brackets?
fn ends_sentence_after(rest: &str) -> bool {
    rest.starts_with(char::is_whitespace)
        && rest.trim_start().chars().all(|c| c.is_whitespace() || is_fixed_space(c) || matches!(c, '"' | '“' | '«' | '(' | '[' | '—' | '–'))
}

fn sentence_capitals(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    let dialogue = text.trim_start().starts_with(['—', '–']);
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(prev), Some(gap), Some(w)) = (text.get(*a0..*b0), text.get(*b0..*a1), text.get(*a1..*b1)) else { continue };
        let word_like = w.chars().all(|c| c.is_alphabetic() || matches!(c, '\'' | '’' | '-'));
        if word_like && w.chars().next().is_some_and(char::is_lowercase) && ends_sentence(gap, prev, w, dialogue) && !is_address(token_at(text, *a1))
        {
            let fixed: String = w.chars().next().map(|f| f.to_uppercase().chain(w.chars().skip(1)).collect()).unwrap_or_default();
            v.push(issue(*a1, *b1, "Majuscule attendue en début de phrase".into(), vec![fixed]));
        }
    }
}

/// Does `w` (lower case) start with a sound that makes `le`, `de`, `que`… elide?
fn takes_elision(w: &str) -> bool {
    let Some(first) = w.chars().next() else { return false };
    if NO_ELISION.contains(&w) {
        return false;
    }
    match first {
        'a' | 'à' | 'â' | 'ä' | 'e' | 'é' | 'è' | 'ê' | 'ë' | 'i' | 'î' | 'ï' | 'o' | 'ô' | 'ö' | 'u' | 'ù' | 'û' | 'ü' | 'œ' | 'æ' => {
            true
        }
        'h' => MUTE_H.contains(&w),
        // Only `y` itself (`j’y vais`): `le yaourt`, `de yoga`.
        'y' => w == "y",
        _ => false,
    }
}

fn elisions(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if gap.is_empty() || !gap.chars().all(|c| c == ' ') {
            continue;
        }
        let lower0 = w0.to_lowercase();
        let Some((_, elided)) = ELIDING.iter().find(|(full, _)| *full == lower0) else { continue };
        // `le`/`Le` only, not `LE`; never before a proper noun (`de Orléans`) or a capital.
        let capital = w0.chars().next().is_some_and(char::is_uppercase);
        if w0.chars().skip(1).any(char::is_uppercase) || !w1.chars().next().is_some_and(char::is_lowercase) {
            continue;
        }
        if !w1.chars().all(|c| c.is_alphabetic() || c == '-') {
            continue;
        }
        let lower1 = w1.to_lowercase();
        // `le un`, `la une` (the number, the front page); a single letter only where it is a word
        // the elision is certain for (`n’a`, `qu’a`, `j’y`).
        if matches!(lower0.as_str(), "le" | "la") && matches!(lower1.as_str(), "un" | "une") {
            continue;
        }
        let certain = (lower1 == "y" && matches!(lower0.as_str(), "je" | "me" | "te" | "se" | "ne"))
            || (lower1 == "a" && matches!(lower0.as_str(), "ne" | "que"));
        if lower1.chars().count() == 1 && !certain {
            continue;
        }
        if !takes_elision(&lower1) {
            continue;
        }
        let head = if capital { crate::affix::capitalize(elided) } else { (*elided).to_string() };
        let fixed = format!("{head}’{w1}");
        v.push(issue(*a0, *b1, format!("Élision{NBSP}: {}", quoted(&fixed)), vec![fixed]));
    }
}

/// Grammar issues in a paragraph of French text (France's typography), in order.
pub fn check(text: &str) -> Vec<Issue> {
    check_in(text, ProofLang::Fr)
}

/// Grammar issues in a paragraph of `lang` text ([`ProofLang::Fr`], or [`ProofLang::FrCa`] for
/// Quebec typography), in order.
pub fn check_in(text: &str, lang: ProofLang) -> Vec<Issue> {
    let ws: Vec<(usize, usize)> = words(text);
    let mut v = Vec::new();
    high_punctuation(text, lang == ProofLang::FrCa, &mut v);
    guillemets(text, &mut v);
    straight_quotes(text, &mut v);
    repeated_words(text, &ws, &mut v);
    sentence_capitals(text, &ws, &mut v);
    elisions(text, &ws, &mut v);
    for i in &mut v {
        i.lang = lang;
    }
    v.sort_by_key(|i| (i.start, i.end));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The issues of `text` as (flagged text, first suggestion).
    fn found(text: &str) -> Vec<(String, String)> {
        check(text)
            .iter()
            .map(|i| (text.get(i.start..i.end).unwrap_or("?").to_string(), i.suggestions.first().cloned().unwrap_or_default()))
            .collect()
    }

    fn quiet(text: &str) {
        let v = check(text);
        assert!(v.is_empty(), "{text:?} → {v:?}");
    }

    fn one(text: &str, flagged: &str, fix: &str) {
        assert_eq!(found(text), [(flagged.to_string(), fix.to_string())], "{text:?}");
    }

    #[test]
    fn missing_space_before_high_punctuation() {
        one("Bonjour!", "!", "\u{202F}!");
        one("Tu viens?", "?", "\u{202F}?");
        one("Il pleut; il vente.", ";", "\u{202F};");
        one("Note: rien.", ":", "\u{A0}:");
        // `?!`: only the first mark needs the space.
        one("Quoi?!", "?", "\u{202F}?");
        assert!(check("Bonjour!").iter().all(|i| i.lang == ProofLang::Fr && i.message.contains("fine insécable")));
        assert!(check("Note: rien.").iter().all(|i| i.message.contains("insécable") && !i.message.contains("fine")));
    }

    #[test]
    fn an_ordinary_space_stays_quiet_until_french_autoformat() {
        // Typing can't produce a non-breaking space yet, so an ordinary one is accepted.
        quiet("Vraiment ? Bonjour ! Il pleut ; note : rien.");
        quiet("Il dit « oui » hier.");
    }

    #[test]
    fn quebec_has_no_space_before_semicolon_exclamation_and_question() {
        let fr_ca = |t: &str| check_in(t, ProofLang::FrCa);
        assert!(fr_ca("Bonjour! Tu viens? Il pleut; il vente.").is_empty(), "{:?}", fr_ca("Bonjour! Tu viens?"));
        // The colon and the guillemets still want their space in Quebec.
        let v = fr_ca("Note: «oui».");
        assert_eq!(v.len(), 3, "{v:?}");
        assert!(v.iter().all(|i| i.lang == ProofLang::FrCa), "{v:?}");
        assert_eq!(check_in("Bonjour!", ProofLang::Fr).len(), 1, "France still wants the space");
    }

    #[test]
    fn high_punctuation_stays_quiet() {
        quiet("Bonjour\u{202F}! Tu viens\u{202F}? Il pleut\u{202F}; note\u{A0}: rien.");
        quiet("Bonjour\u{A0}! Quoi\u{202F}?! Ah\u{2009}!");
        quiet("Il est 10:30.");
        quiet("Voir https://exemple.org:8080/a?b=c pour la suite.");
        quiet("Écrire à contact@exemple.org!");
        quiet("Super :) et ;-) aussi :D");
        quiet("a:b et x;y");
        quiet("Le fichier C:\\dossier est là.");
    }

    #[test]
    fn missing_space_inside_guillemets() {
        assert_eq!(found("Il dit «oui» hier."), [("«".to_string(), "«\u{A0}".to_string()), ("»".to_string(), "\u{A0}»".to_string())]);
        quiet("Il dit «\u{A0}oui\u{A0}» hier.");
        quiet("Il dit «\u{202F}oui\u{202F}» hier.");
    }

    #[test]
    fn straight_quotes_become_guillemets() {
        one("Il a dit \"bonjour\" hier.", "\"bonjour\"", "«\u{A0}bonjour\u{A0}»");
        one("\"Bonne idée\", dit-elle.", "\"Bonne idée\"", "«\u{A0}Bonne idée\u{A0}»");
        // Inch marks and unbalanced quotes are left alone.
        quiet("Un écran de 27\" et un de 32\" ici.");
        quiet("Il a dit \"bonjour hier.");
    }

    #[test]
    fn repeated_words_are_flagged() {
        one("Le le chat dort.", "Le le", "Le");
        one("Je pense que que tu viens.", "que que", "que");
        one("Il mange la la pomme.", "la la", "la");
    }

    #[test]
    fn deliberate_repetitions_stay_quiet() {
        quiet("Nous nous lavons et vous vous trompez.");
        quiet("Si si, très très bien.");
        quiet("Ah ah, bla bla.");
        quiet("Je crois que qu’il viendra.");
        quiet("Le livre de De Gaulle.");
        quiet("Nous partons à Bora Bora.");
        quiet("Ils ont dit bye bye. Tchin tchin\u{202F}!");
    }

    #[test]
    fn sentence_capitals_are_flagged() {
        one("Il pleut. il fait froid.", "il", "Il");
        one("Il pleut\u{202F}! il fait froid.", "il", "Il");
        one("Tu viens\u{202F}? on part.", "on", "On");
        one("Il pleut. l’orage arrive.", "l’orage", "L’orage");
        one("Il pleut. «\u{A0}tout va bien\u{A0}», dit-il.", "tout", "Tout");
    }

    #[test]
    fn sentence_capitals_stay_quiet() {
        quiet("Voir p. 12 et cf. la note, etc. puis la suite.");
        quiet("M. le maire est là.");
        quiet("Il attend… puis part. Il attend... puis part.");
        quiet("«\u{A0}Tu viens\u{202F}?\u{A0}» demanda-t-elle.");
        quiet("Ah\u{202F}! que c’est beau.");
        quiet("Quoi\u{202F}? dit-il.");
        quiet("Tu viens\u{202F}? demanda le voisin.");
        quiet("— Tu viens\u{202F}? reprends-tu, inquiet.");
        quiet("– Viens\u{202F}! on part.");
        quiet("Voir www.exemple.fr pour la suite.");
        quiet("En 2026. ensuite rien.");
        quiet("il commence en minuscule.");
    }

    #[test]
    fn missing_elisions_are_flagged() {
        one("Il refuse de avoir peur.", "de avoir", "d’avoir");
        one("Je ai faim.", "Je ai", "J’ai");
        one("Voici le homme.", "le homme", "l’homme");
        one("Je sais que il viendra.", "que il", "qu’il");
        one("Le arbre est grand.", "Le arbre", "L’arbre");
        one("Il ne a pas faim.", "ne a", "n’a");
        one("Bon, je y vais.", "je y", "j’y");
        one("Il se est levé.", "se est", "s’est");
        one("Voici la école.", "la école", "l’école");
        one("Il me aime.", "me aime", "m’aime");
    }

    #[test]
    fn aspirated_h_and_other_exceptions_stay_quiet() {
        for w in [
            "hibou",
            "héros",
            "haricot",
            "hasard",
            "haut",
            "hall",
            "hamac",
            "hangar",
            "hockey",
            "homard",
            "huit",
            "hérisson",
            "hors-jeu",
            "onze",
            "oui",
            "yaourt",
        ] {
            quiet(&format!("Voici le {w}."));
        }
        // hiérarchie has an aspirated h, despite the mute h in hiéroglyphe:
        // https://www.academie-francaise.fr/le-hieroglyphe-pour-lhieroglyphe
        for w in ["honte", "hache", "harpe", "hausse", "hiérarchie"] {
            quiet(&format!("Voici la {w}."));
        }
        quiet("Une question de hiérarchie.");
        quiet("Le onze de France. À la une du journal.");
        quiet("Il vient de Orléans.");
        quiet("De a à z, le a et le b.");
        quiet("Le y et le x sont des variables. La dérivée de y.");
        quiet("Je mange de la viande.");
    }

    #[test]
    fn hostile_input_never_panics() {
        for t in ["", " ", "!", " !", "«", "»", " »", "« ", "\"", "\"\"", ". a", "de", "de ", "\u{202F}?", "é!é", "« » « »", ":", " :", "a :)"]
        {
            let _ = check(t);
        }
        // A long run of marks in one token: linear, not one token scan per mark.
        let run = format!("mot{}", "?".repeat(200_000));
        assert_eq!(check(&run).len(), 1);
    }
}

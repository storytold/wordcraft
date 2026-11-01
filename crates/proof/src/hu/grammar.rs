//! Hungarian grammar and punctuation checks: cheap, rule-based and tuned to stay quiet when
//! unsure (a missed comma costs less than a wrong mark on correct text). Messages are in
//! Hungarian.
//!
//! - **Repeated word** (`a a`, `hogy hogy`), except interjections and words Hungarian doubles on
//!   purpose (`igen igen`, `nagyon nagyon`).
//! - **Space before punctuation** (`igen ,`) and **double spaces**; emoticons (` :)`) and
//!   ellipses are left alone.
//! - **Capital letter at the start of a sentence**, knowing Hungarian abbreviations (`pl.`,
//!   `stb.`, `kb.`, `ill.`), initials, ordinal numbers and dates (`3. fejezet`, `2026. október`),
//!   ellipses and dialogue dashes, after which lower case is correct.
//! - **Missing comma** before the conjunctions and relative pronouns that open a clause: `hogy`,
//!   `mert`, `hanem`, `de`, `ha`, `bár`, `noha`, `mivel`, `amikor`, `ahol` and the forms of `ami`,
//!   `aki`, `amely` — unless a conjunction or particle comes first (`és hogy`, `mint ha`, `csak
//!   ha`, `még ha`). `mert` is also the past of `mer` (`nem mert szólni`), so it is left alone
//!   before an infinitive or `volna` and after a negation; `de` is also a Latin and French word
//!   (`de facto`, `Charles de Gaulle`), so it is left alone before those and before a name. A
//!   word only starting like a relative pronoun (`aminosav`, `amiláza`) opens no clause.
//! - **`a` before a vowel**: the article is `az` there (`a alma` → `az alma`). The other way round
//!   isn't flagged, since `az` is also a pronoun (`Az kell nekem.`).
//! - **A month after a year** is written in lower case (`2026. Október` → `2026. október`).
//! - **Doubled punctuation** (`,,` — often a typed opening quote `„` — `..`, `;;`).
//!
//! No English rule (such as `a`/`an`) applies to Hungarian text.

use crate::{Issue, IssueKind, ProofLang, words};

/// Words Hungarian repeats on purpose: interjections, onomatopoeia and emphatic doubling.
const ALLOWED_REPEATS: &[&str] = &[
    "igen", "nem", "jaj", "ha", "he", "hi", "hé", "na", "no", "nos", "ó", "óh", "ah", "aha", "oh", "hű", "haj", "hej", "bumm", "kopp", "tik", "tak",
    "pszt", "nagyon", "lassan", "egyre", "soha", "mindig", "tovább", "jó",
];

/// Abbreviations written with a final period after which a sentence goes on in lower case
/// (lower case, without the period).
const ABBREVIATIONS: &[&str] = &[
    "pl", "stb", "ill", "kb", "ún", "vö", "uo", "ld", "dr", "id", "ifj", "özv", "krt", "sz", "kft", "bt", "rt", "zrt", "nyrt", "ker", "tel", "db",
    "ft", "kg", "km", "cm", "mm", "mp", "évf", "jan", "febr", "márc", "ápr", "máj", "jún", "júl", "aug", "szept", "okt", "nov", "dec", "ti", "tkp",
    "tsz", "fsz", "fszt", "em", "hrsz", "ui", "vmi", "vki", "prof", "főv", "megj", "ford", "szerk", "kiad", "uő", "sk", "etc", "vs", "ca", "kir",
    "bp", "szül", "megh", "ált", "biz", "isk", "főisk", "ea", "gyak", "köt", "fej", "bek", "old", "tört",
];

/// Conjunctions and relative pronouns that open a clause: a comma comes before them.
const CLAUSE_OPENERS: &[&str] = &[
    "hogy",
    "mert",
    "hanem",
    "de",
    "ha",
    "bár",
    "habár",
    "noha",
    "holott",
    "mivel",
    "miközben",
    "mielőtt",
    "miután",
    "mihelyt",
    "amikor",
    "amíg",
    "amint",
    "ameddig",
    "amennyiben",
    "ahogy",
    "ahogyan",
    "ahol",
    "ahová",
    "ahova",
    "ahonnan",
    "amilyen",
    "amekkora",
    "ahány",
    "amióta",
];

/// Stems of the relative pronouns, whose case forms open a clause too (`amit`, `akinek`,
/// `amelyben`), with the plural each takes (`amik`, `akik`, `amelyek`).
const RELATIVE_STEMS: &[(&str, &str)] = &[("ami", "k"), ("aki", "k"), ("amely", "ek"), ("amelyik", "ek")];

/// The case endings of a relative pronoun: `ami` + `nek`, `amely` + `ben`, `akik` + `kel`.
const RELATIVE_ENDINGS: &[&str] = &[
    "", "t", "et", "nek", "nél", "ben", "ből", "be", "n", "en", "ről", "re", "től", "hez", "vel", "kel", "ért", "vé", "ké", "ig", "ként", "képp",
    "képpen",
];

/// Relative pronouns whose ending changes the stem (`amely` + `vel` → `amellyel`).
const RELATIVE_ASSIMILATED: &[&str] = &["amellyel", "amellyé", "amelyikkel"];

/// Words after `de` that make it the Latin or French particle (`de facto`, `de jure`), not the
/// conjunction.
const DE_PARTICLE_BEFORE: &[&str] = &["facto", "jure", "iure", "lege", "luxe", "novo", "visu", "minimis"];

/// Words after which a clause opener takes no comma of its own: conjunctions (`és hogy`),
/// particles (`csak ha`, `főleg mert`) and the first halves of compound conjunctions (`mint ha`,
/// `még ha`, `közvetlenül mielőtt`).
const NO_COMMA_AFTER: &[&str] = &[
    "és",
    "s",
    "meg",
    "vagy",
    "avagy",
    "sem",
    "se",
    "is",
    "de",
    "csak",
    "mint",
    "mintha",
    "hanem",
    "mert",
    "hogy",
    "ha",
    "hacsak",
    "akár",
    "illetve",
    "valamint",
    "azaz",
    "vagyis",
    "hiszen",
    "ugyanis",
    "mivel",
    "pedig",
    "mégpedig",
    "különösen",
    "főleg",
    "főként",
    "kivált",
    "legalábbis",
    "még",
    "már",
    "csupán",
    "pusztán",
    "éppen",
    "épp",
    "pont",
    "kivéve",
    "közvetlenül",
    "röviddel",
    "nemcsak",
    "bár",
    "noha",
    "amikor",
    "ahol",
];

/// Words before which `mert` is the verb `mer` (`nem mert szólni`), not the conjunction.
const MERT_VERB_BEFORE: &[&str] =
    &["nem", "sem", "se", "ne", "alig", "is", "mégsem", "mégis", "nemigen", "jobban", "többet", "sokat", "bátran", "ki"];

/// Months, written in lower case after a year (`2026. október 15.`).
const MONTHS: &[&str] =
    &["január", "február", "március", "április", "május", "június", "július", "augusztus", "szeptember", "október", "november", "december"];

/// Words after which `a` before a vowel may be a symbol (`a és b`), not the article.
const SYMBOL_CONTEXT: &[&str] = &["és", "s", "vagy", "illetve", "ill", "is", "meg", "egyenlő", "plusz", "mínusz"];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Hu }
}

fn capitalize(w: &str) -> String {
    crate::affix::capitalize(w)
}

/// Hungarian vowels.
fn is_vowel(c: char) -> bool {
    matches!(c.to_lowercase().next().unwrap_or(c), 'a' | 'á' | 'e' | 'é' | 'i' | 'í' | 'o' | 'ó' | 'ö' | 'ő' | 'u' | 'ú' | 'ü' | 'ű')
}

/// The article before `w`, as written in the messages: `az` before a vowel, `a` otherwise.
fn article(w: &str) -> &'static str {
    if w.chars().next().is_some_and(is_vowel) { "az" } else { "a" }
}

/// Grammar issues in a paragraph of Hungarian text, in order.
pub fn check(text: &str) -> Vec<Issue> {
    check_with(text, true)
}

/// [`check`], where `capital_first` says whether the first word must start with a capital (not
/// in a list item, which may begin in lower case).
pub fn check_with(text: &str, capital_first: bool) -> Vec<Issue> {
    let ws: Vec<(usize, usize)> = words(text);
    let mut v = Vec::new();
    repeated_words(text, &ws, &mut v);
    spaces(text, &mut v);
    sentence_capitals(text, &ws, capital_first, &mut v);
    missing_commas(text, &ws, &mut v);
    articles(text, &ws, &mut v);
    months(text, &ws, &mut v);
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
            v.push(issue(*a0, *b1, format!("Ismétlődő szó: „{w1}”"), vec![w0.to_string()]));
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
            // ` ...` is an ellipsis, ` .5` a number, ` .docx` and ` .NET` names; ` .Aztán` is a
            // stray space.
            '.' => !matches!(after, Some('.')) && !after.is_some_and(|a| a.is_ascii_digit()) && !file_name_at(rest.get(1..).unwrap_or("")),
            // ` :)`, ` :-)`, ` :D` and similar are emoticons; ` ?!` is still a stray space.
            ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '»' | '’' | '!' | '?')),
            _ => false,
        };
        if flag {
            v.push(issue(i, i + 1 + p.len_utf8(), "Felesleges szóköz az írásjel előtt".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Dupla szóköz".into(), vec![" ".into()]));
    }
}

/// Does `s` start with a file extension or a name written with a leading period: lower-case
/// letters or digits (`docx`, `mp3`) or `NET`, up to the end of the word?
fn file_name_at(s: &str) -> bool {
    let n = s.find(|c: char| !c.is_alphanumeric()).unwrap_or(s.len());
    let Some(name) = s.get(..n) else { return false };
    !name.is_empty() && (name == "NET" || name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

/// Is `prev`, before a period, an abbreviation, an initial, a number (an ordinal or a date:
/// `3. fejezet`, `2026. október`) or a Roman numeral, after which a lower-case word doesn't start
/// a sentence?
fn abbreviation_or_number(prev: &str) -> bool {
    prev.chars().count() <= 1
        || prev.chars().any(|c| c.is_ascii_digit())
        || (prev.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')))
        || ABBREVIATIONS.contains(&prev.to_lowercase().as_str())
        || listed_abbreviation(prev)
}

/// Does the loaded dictionary list `prev` with a period (`érk.`, `ény.`) while the bare form is
/// no word of its own (`ék.` may end a sentence about an `ék`)?
fn listed_abbreviation(prev: &str) -> bool {
    let d = super::dictionary();
    let lower = prev.to_lowercase();
    !d.is_empty() && d.contains_simple(&format!("{lower}.")) && !d.check(&lower)
}

const CLOSING: [char; 8] = ['"', '”', '’', '\'', ')', '»', ']', '\u{1}'];
const OPENING: [char; 8] = ['„', '"', '“', '«', '(', '[', '\'', '‚'];

/// Does the gap between two words end a sentence: a `.`, `!` or `?` (not an ellipsis, not after
/// an abbreviation or a number), then space, with nothing but quotes and brackets around?
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

fn sentence_capitals(text: &str, ws: &[(usize, usize)], capital_first: bool, v: &mut Vec<Issue>) {
    // List items (`– alma,`, or a paragraph formatted as one) may start in lower case.
    let list_item = !capital_first || text.trim_end().ends_with([',', ';']);
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
            v.push(issue(a, b, "A mondat nagybetűvel kezdődik".into(), vec![capitalize(w)]));
        }
    }
}

/// Is `de` at `ws[k]` a name particle (`Charles de Gaulle`, `Jean de la Fontaine`)? A
/// capitalized name stands before it and a capitalized word (or `la`, `le`, `los`, `las`) after
/// it, and the word before is a name the dictionary lists as written, or one it doesn't know: an
/// inflected or common word (`Budapesten de Péter`, `Szép de Anna`) takes a comma. Without a
/// dictionary the capitals decide.
fn name_before(text: &str, ws: &[(usize, usize)], k: usize) -> bool {
    let word = |i: Option<usize>| i.and_then(|i| ws.get(i)).and_then(|&(s, e)| text.get(s..e));
    let (Some(prev), Some(next)) = (word(k.checked_sub(1)), word(k.checked_add(1))) else { return false };
    let capital = |w: &str| w.chars().next().is_some_and(char::is_uppercase);
    if !capital(prev) || !(capital(next) || matches!(next, "la" | "le" | "los" | "las")) {
        return false;
    }
    let d = super::dictionary();
    d.is_empty() || d.is_listed(prev) || !d.check(prev)
}

/// Is `w` (lower case) a form of a relative pronoun: a stem, optionally its plural, and a case
/// ending? (`amiatt` points back, `aminosav` is a noun: neither is one.)
fn relative_pronoun(w: &str) -> bool {
    RELATIVE_ASSIMILATED.contains(&w)
        || RELATIVE_STEMS.iter().any(|&(stem, plural)| {
            w.strip_prefix(stem).is_some_and(|rest| {
                let rest = rest.strip_prefix(plural).unwrap_or(rest);
                RELATIVE_ENDINGS.contains(&rest)
            })
        })
}

/// Does `w` (lower case) open a clause?
fn opens_clause(w: &str) -> bool {
    CLAUSE_OPENERS.contains(&w) || relative_pronoun(w)
}

fn missing_commas(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for k in 1..ws.len() {
        let (Some(&(pa, pb)), Some(&(a, b))) = (ws.get(k - 1), ws.get(k)) else { continue };
        let (Some(prev), Some(w), Some(gap)) = (text.get(pa..pb), text.get(a..b), text.get(pb..a)) else { continue };
        if !w.chars().next().is_some_and(char::is_lowercase) || !opens_clause(w) {
            continue;
        }
        if gap.is_empty() || !gap.chars().all(|c| c == ' ' || c == '\u{a0}') || !prev.chars().all(char::is_alphabetic) {
            continue;
        }
        let p = prev.to_lowercase();
        if NO_COMMA_AFTER.contains(&p.as_str()) {
            continue;
        }
        let next = ws.get(k + 1).and_then(|&(na, nb)| text.get(na..nb)).unwrap_or("");
        if w == "de" && (DE_PARTICLE_BEFORE.contains(&next.to_lowercase().as_str()) || name_before(text, ws, k)) {
            continue;
        }
        if w == "mert" && (MERT_VERB_BEFORE.contains(&p.as_str()) || (next.ends_with("ni") && next.chars().count() > 3) || next == "volna") {
            continue;
        }
        v.push(issue(pb, b, format!("Vessző kell {} „{w}” elé", article(w)), vec![format!(", {w}")]));
    }
}

fn articles(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for k in 0..ws.len() {
        let (Some(&(a, b)), Some(&(na, nb))) = (ws.get(k), ws.get(k + 1)) else { continue };
        let (Some(w), Some(next), Some(gap)) = (text.get(a..b), text.get(na..nb), text.get(b..na)) else { continue };
        if !matches!(w, "a" | "A") || !matches!(gap, " " | "\u{a0}") {
            continue;
        }
        // A capital `A` mid-sentence is a letter or a symbol (`az A pont`), not the article.
        if w == "A" && k > 0 {
            let Some(&(pa, pb)) = ws.get(k - 1) else { continue };
            if !ends_sentence(text.get(pb..a).unwrap_or(""), text.get(pa..pb).unwrap_or("")) {
                continue;
            }
        }
        let lower = next.to_lowercase();
        if next.chars().count() < 2
            || !next.chars().next().is_some_and(|c| c.is_alphabetic() && is_vowel(c))
            || SYMBOL_CONTEXT.contains(&lower.as_str())
        {
            continue;
        }
        let fixed = if w == "A" { "Az" } else { "az" };
        v.push(issue(a, b, "Magánhangzó előtt „az” a névelő".into(), vec![fixed.into()]));
    }
}

fn months(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(year), Some(m), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) || !matches!(gap, ". " | ".\u{a0}") {
            continue;
        }
        let lower = m.to_lowercase();
        if m.chars().next().is_some_and(char::is_uppercase) && MONTHS.contains(&lower.as_str()) && m.chars().skip(1).all(char::is_lowercase) {
            v.push(issue(*a1, *b1, "A hónap nevét kisbetűvel írjuk".into(), vec![lower]));
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
                // `,,` before a word, after a space: a typed opening quote.
                b',' if text.get(end..).and_then(|t| t.chars().next()).is_some_and(char::is_alphabetic)
                    && (i == 0 || text.get(..i).is_some_and(|t| t.ends_with(char::is_whitespace))) =>
                {
                    vec!["„".into()]
                }
                b'.' => vec![".".into(), "…".into()],
                _ => vec![char::from(b).to_string()],
            };
            v.push(issue(i, end, "Kettőzött írásjel".into(), suggestions));
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
        let v = check("Ez a a ház, és és a kert.");
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!(v[0].message, "Ismétlődő szó: „a”");
        assert_eq!(v[0].suggestions, ["a"]);
        assert!(v.iter().all(|i| i.lang == ProofLang::Hu && i.kind == IssueKind::Grammar));
        clean("Igen igen, nagyon nagyon szép. Jaj jaj!");
    }

    #[test]
    fn spaces_and_punctuation() {
        let m = messages("Szép idő van , és jó a kedvem .  Igaz ?");
        assert!(m.contains(&"Felesleges szóköz az írásjel előtt@ ,".to_string()), "{m:?}");
        assert!(m.contains(&"Felesleges szóköz az írásjel előtt@ .".to_string()), "{m:?}");
        assert!(m.contains(&"Felesleges szóköz az írásjel előtt@ ?".to_string()), "{m:?}");
        assert!(m.contains(&"Dupla szóköz@  ".to_string()), "{m:?}");
        clean("Remek :) Tényleg :-) Vagy :D Az érték .5 és három pont ...");
        clean("Megnyitottam a .docx fájlt, a .NET keretrendszerrel és az .mp3 zenét.");
        assert!(messages("Elment .Aztán jött.").iter().any(|m| m.starts_with("Felesleges szóköz")));
        let m = messages("Azt mondta ,,igen” és elment.. Aztán;; vége.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Kettőzött")).count(), 3, "{m:?}");
        assert_eq!(check("Azt mondta ,,igen”.").iter().find(|i| i.message.starts_with("Kettőzött")).unwrap().suggestions, ["„"]);
    }

    #[test]
    fn sentence_start_capitals() {
        let m = messages("ez egy mondat. ez a második! ez a harmadik? igen.");
        assert_eq!(m.iter().filter(|x| x.starts_with("A mondat nagybetűvel")).count(), 4, "{m:?}");
        assert_eq!(check("ez a ház.")[0].suggestions, ["Ez"]);
        // Abbreviations, initials, ordinals, dates, ellipses and dialogue dashes.
        clean("Vettem gyümölcsöt, pl. almát, körtét stb. és zöldséget.");
        clean("Kb. tíz perc múlva, ill. fél óra múlva jön.");
        clean("Dr. Kovács és ifj. Nagy János a 3. fejezetet olvasta.");
        // An abbreviation the dictionary lists with its period, unless the bare form is a word of
        // its own (`ék.` ends the sentence).
        crate::hu::test_dict::install();
        clean("Az érk. dátuma.");
        assert!(messages("Ez egy ék. az másik.").iter().any(|m| m.starts_with("A mondat nagybetűvel")));
        clean("A rendezvény 2026. október 15. napján lesz.");
        clean("Vártam... aztán végre megjött.");
        clean("– Tényleg? – kérdezte. – Igen – felelte.");
        clean("Azt mondta: „Jövök.” Aztán elment.");
        clean("alma,");
    }

    #[test]
    fn missing_commas() {
        let m = messages("Tudom hogy jön. Maradt mert beteg. Nem ő hanem a testvére. Szép de drága. Ez az ami kell. Szólj ha kell.");
        for want in [
            "Vessző kell a „hogy” elé@ hogy",
            "Vessző kell a „mert” elé@ mert",
            "Vessző kell a „hanem” elé@ hanem",
            "Vessző kell a „de” elé@ de",
            "Vessző kell az „ami” elé@ ami",
            "Vessző kell a „ha” elé@ ha",
        ] {
            assert!(m.contains(&want.to_string()), "{want}: {m:?}");
        }
        assert_eq!(check("Tudom hogy jön.")[0].suggestions, [", hogy"]);
        let m = messages("Ismerem azt akinek a háza ott van. Ott van ahol hagytad.");
        assert!(m.contains(&"Vessző kell az „akinek” elé@ akinek".to_string()), "{m:?}");
        assert!(m.contains(&"Vessző kell az „ahol” elé@ ahol".to_string()), "{m:?}");
        // Conjunctions and particles before the opener, and openers at the start.
        clean("Tudom, hogy jön, és hogy marad.");
        clean("Úgy tett, mintha aludna, mint ha álmodna.");
        clean("Eljövök, csak ha hívsz, még ha esik is.");
        clean("Hogy vagy? Ha kell, szólj. Aki keres, talál.");
        // `mert` as the past of `mer`.
        clean("Nem mert szólni, és Péter sem mert volna.");
        clean("Amiatt késtem, mert esett.");
        // `de` as a Latin or French particle, and words that only start like a relative pronoun.
        clean("Ez de facto és de jure is így van. Charles de Gaulle beszélt.");
        clean("A fehérjék aminosavai és a nyál amilázai, az aminok sói.");
        let m = messages("Szép de drága. Ez az amelyben élünk. Láttam azt amellyel jött. Azok akikkel beszéltem.");
        for want in ["„de”", "„amelyben”", "„amellyel”", "„akikkel”"] {
            assert!(m.iter().any(|x| x.contains(want)), "{want}: {m:?}");
        }
        // `de` is a name particle between a name and a capital (or `la`): not after a Hungarian
        // word, inflected names included; `amelynél` and `akinél` are relative pronouns.
        crate::hu::test_dict::install();
        // `Charles` is listed as written, `Jean` unknown: both are names.
        clean("Ezt Jean de la Fontaine írta. Charles de Gaulle beszélt.");
        clean("Jean de la Fontaine írta.");
        let m = messages("Szép de Anna nem jött el. Ez az amelynél jobb nincs. Az akinél van.");
        for want in ["„de”", "„amelynél”", "„akinél”"] {
            assert!(m.iter().any(|x| x.contains(want)), "{want}: {m:?}");
        }
        for t in ["Voltam Budapesten de nem tetszett.", "Elmentem a Tescóba de zárva volt.", "Láttam Budapesten de Péter nem."] {
            assert!(messages(t).iter().any(|x| x.contains("„de”")), "{t}");
        }
    }

    #[test]
    fn articles_before_vowels() {
        let m = messages("Láttam a almát és a embert. A óra jár.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Magánhangzó előtt")).count(), 3, "{m:?}");
        let v = check("A óra jár.");
        assert_eq!(v.iter().find(|i| i.message.starts_with("Magánhangzó")).unwrap().suggestions, ["Az"]);
        clean("Az alma piros, az ember jó, a ház nagy.");
        clean("Az kell nekem, az jó.");
        clean("Legyen a és b két szám. Az A oldal és az a betű.");
    }

    #[test]
    fn months_after_a_year() {
        let v = check("Ülés: 2026. Október 15.");
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].message, "A hónap nevét kisbetűvel írjuk");
        assert_eq!(v[0].suggestions, ["október"]);
        clean("Ülés: 2026. október 15. Október szép hónap.");
    }

    #[test]
    fn correct_hungarian_text_is_clean() {
        clean(
            "Árvíztűrő tükörfúrógép. A kert végében, ahol a fák állnak, egy kis ház van. \
             Azt hiszem, hogy holnap esni fog, de nem biztos. Az, ami történt, nem ismétlődhet meg.",
        );
        clean("Péter nem jött el, mert beteg volt, ezért otthon maradt.");
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = check(&s);
        }

        #[test]
        fn never_panics_on_hungarian_like_text(s in "[a-záéíóöőúüűA-ZÁÉÍÓÖŐÚÜŰ0-9 ,.;:!?„”…\\-]{0,60}") {
            for i in check(&s) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
                proptest::prop_assert!(s.is_char_boundary(i.start) && s.is_char_boundary(i.end));
            }
        }
    }
}

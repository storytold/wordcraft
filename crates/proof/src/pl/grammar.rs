//! Polish grammar and punctuation checks: cheap, rule-based and tuned to stay quiet when unsure
//! (a missed comma costs less than a wrong mark on correct text). Messages are in Polish.
//!
//! - **Repeated word** (`się się`), except interjections and words Polish doubles on purpose
//!   (`to to`, `tak tak`, `ha ha`).
//! - **Space before punctuation** (`tak ,`) and **double spaces**; emoticons (` :)`) and
//!   ellipses are left alone.
//! - **Capital letter at the start of a sentence**, knowing Polish abbreviations (`np.`, `itd.`,
//!   `r.`, `m.in.`), initials, ordinal numbers (`5. miejsce`), ellipses and dialogue dashes
//!   (`– Tak? – zapytał`), after which lower case is correct.
//! - **Missing comma** before subordinating conjunctions and relative pronouns that always open a
//!   clause: `że`, `ponieważ`, `gdyż`, `aby`, `żeby`, `jeśli`, `jeżeli`, `gdyby`, `lecz` and the
//!   forms of `który` — unless a conjunction or particle comes first (`mimo że`, `chyba że`,
//!   `tak aby`, `i który`) or, for `który`, a preposition (`w którym`: the comma goes before it).
//! - **`we`/`ze`** where the next word's consonant cluster requires them (`we wtorek`,
//!   `ze szkoły`, `ze mną`).
//! - **Doubled punctuation** (`,,` — often a typed opening quote `„` — `..`, `;;`).
//!
//! No English rule (such as `a`/`an`) applies to Polish text.

use crate::{Issue, IssueKind, ProofLang, words};

/// Words Polish repeats on purpose: interjections, onomatopoeia and emphatic doubling.
const ALLOWED_REPEATS: &[&str] = &[
    "to", "tak", "nie", "no", "ha", "he", "hi", "hej", "oj", "aj", "ej", "ach", "och", "ech", "puk", "hop", "bum", "cyk", "tik", "pa", "fiu", "la",
    "bardzo", "dawno", "daleko", "długo",
];

/// Abbreviations written with a final period after which a sentence goes on in lower case
/// (lower case, without the period).
const ABBREVIATIONS: &[&str] = &[
    "np", "itd", "itp", "tzn", "tj", "tzw", "ok", "ul", "al", "pl", "os", "in", "wg", "zob", "por", "str", "ss", "nr", "pkt", "ust", "art", "par",
    "rozdz", "tab", "rys", "il", "przyp", "red", "wyd", "oprac", "tłum", "cyt", "jw", "ds", "im", "ks", "św", "bł", "prof", "dr", "doc", "hab",
    "inż", "mgr", "lek", "płk", "gen", "mjr", "kpt", "ppłk", "sierż", "szer", "kmdr", "adm", "abp", "bp", "br", "ub", "bm", "pn", "płd", "płn",
    "wsch", "zach", "gł", "godz", "min", "sek", "tys", "mln", "mld", "zł", "gr", "kg", "dag", "mies", "tyg", "pon", "wt", "śr", "czw", "pt", "sob",
    "niedz", "sty", "lut", "mar", "kwi", "cze", "lip", "sie", "wrz", "paź", "lis", "gru", "ang", "niem", "franc", "łac", "ros", "wł", "hiszp", "pol",
    "lit", "dosł", "przen", "pot", "zdrob", "fot", "ryc", "tel", "kom", "ew", "ob", "obyw", "pp", "zm", "ur", "ca", "vs", "etc", "cdn", "dot", "wz",
    "zw", "zał", "cz", "rr", "ww", "jęz", "jun", "sen", "mec", "red", "dyr", "kier", "proc", "poj", "lp", "lm", "zaw", "woj", "pow", "gm", "wsp",
    "nast", "poprz", "ang", "fr",
];

/// Subordinating conjunctions and relative pronouns that open a clause: a comma comes before
/// them.
const CLAUSE_OPENERS: &[&str] = &[
    "że",
    "ponieważ",
    "gdyż",
    "aby",
    "żeby",
    "jeśli",
    "jeżeli",
    "gdyby",
    "lecz",
    "który",
    "która",
    "które",
    "którego",
    "której",
    "któremu",
    "którą",
    "którym",
    "których",
    "którymi",
    "którzy",
];

/// Words after which a clause opener takes no comma of its own: conjunctions (`i że`), particles
/// (`tylko jeśli`, `zwłaszcza że`) and the first halves of compound conjunctions (`mimo że`,
/// `chyba że`, `tak aby`, `jak gdyby`, `na wypadek gdyby`, `z tym że`).
const NO_COMMA_AFTER: &[&str] = &[
    "i",
    "a",
    "oraz",
    "lub",
    "albo",
    "ani",
    "czy",
    "bądź",
    "ale",
    "lecz",
    "bo",
    "że",
    "aby",
    "żeby",
    "by",
    "niż",
    "jak",
    "jako",
    "niby",
    "tylko",
    "nawet",
    "właśnie",
    "zwłaszcza",
    "szczególnie",
    "głównie",
    "częściowo",
    "jedynie",
    "wyłącznie",
    "chyba",
    "mianowicie",
    "przynajmniej",
    "również",
    "także",
    "też",
    "dopiero",
    "jeszcze",
    "już",
    "zaś",
    "więc",
    "zatem",
    "toteż",
    "przecież",
    "chociaż",
    "choć",
    "choćby",
    "gdy",
    "kiedy",
    "jeśli",
    "jeżeli",
    "gdyby",
    "skoro",
    "zanim",
    "dopóki",
    "póki",
    "aż",
    "co",
    "tak",
    "mimo",
    "pomimo",
    "wtedy",
    "teraz",
    "dziś",
    "dzisiaj",
    "podczas",
    "tym",
    "bardziej",
    "warunkiem",
    "wypadek",
    "zamiast",
    "byle",
    "lada",
    "wiadomo",
    "obojętnie",
    "jedno",
    "wszystkim",
    "może",
    "pewnie",
    "prawdopodobnie",
    "zapewne",
    "dlatego",
    "tyle",
    "dość",
    "ponieważ",
    "gdyż",
    "lecz",
    "albowiem",
    "oto",
    "no",
];

/// Prepositions: a relative pronoun after one takes the comma before the preposition instead
/// (`dom, w którym`).
const PREPOSITIONS: &[&str] = &[
    "w",
    "we",
    "z",
    "ze",
    "na",
    "do",
    "od",
    "ode",
    "o",
    "u",
    "po",
    "za",
    "przed",
    "przede",
    "przez",
    "przeze",
    "przy",
    "nad",
    "nade",
    "pod",
    "pode",
    "dla",
    "bez",
    "beze",
    "ku",
    "między",
    "pomiędzy",
    "wśród",
    "spośród",
    "spod",
    "spoza",
    "sprzed",
    "znad",
    "zza",
    "poza",
    "ponad",
    "poprzez",
    "dzięki",
    "według",
    "wobec",
    "oprócz",
    "prócz",
    "obok",
    "koło",
    "około",
    "wokół",
    "dookoła",
    "naprzeciw",
    "naprzeciwko",
    "wzdłuż",
    "wewnątrz",
    "zewnątrz",
    "względem",
    "przeciw",
    "przeciwko",
    "wbrew",
    "wskutek",
    "śród",
    "wedle",
    "pośród",
    "ponad",
    "niedaleko",
    "blisko",
];

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Pl }
}

fn capitalize(w: &str) -> String {
    crate::affix::capitalize(w)
}

/// Polish vowels (and the accented vowels of loanwords).
fn is_vowel(c: char) -> bool {
    matches!(c.to_lowercase().next().unwrap_or(c), 'a' | 'ą' | 'e' | 'ę' | 'i' | 'o' | 'ó' | 'u' | 'y' | 'é' | 'á' | 'ö' | 'ü' | 'í' | 'ú')
}

fn is_consonant(c: char) -> bool {
    c.is_alphabetic() && !is_vowel(c)
}

/// Grammar issues in a paragraph of Polish text, in order.
pub fn check(text: &str) -> Vec<Issue> {
    let ws: Vec<(usize, usize)> = words(text);
    let mut v = Vec::new();
    repeated_words(text, &ws, &mut v);
    spaces(text, &mut v);
    sentence_capitals(text, &ws, &mut v);
    missing_commas(text, &ws, &mut v);
    we_ze(text, &ws, &mut v);
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
            v.push(issue(*a0, *b1, format!("Powtórzone słowo: „{w1}”"), vec![w0.to_string()]));
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
            ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '»' | '’' | '!' | '?')),
            _ => false,
        };
        if flag {
            v.push(issue(i, i + 1 + p.len_utf8(), "Zbędna spacja przed znakiem interpunkcyjnym".into(), vec![p.to_string()]));
        }
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(issue(i, i + 2, "Podwójna spacja".into(), vec![" ".into()]));
    }
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
const OPENING: [char; 8] = ['„', '"', '“', '«', '(', '[', '\'', '‚'];

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

fn sentence_capitals(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    // List items (`– jabłka,`) may start in lower case.
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
            v.push(issue(a, b, "Wielka litera na początku zdania".into(), vec![capitalize(w)]));
        }
    }
}

fn missing_commas(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for k in 1..ws.len() {
        let (Some(&(pa, pb)), Some(&(a, b))) = (ws.get(k - 1), ws.get(k)) else { continue };
        let (Some(prev), Some(w), Some(gap)) = (text.get(pa..pb), text.get(a..b), text.get(pb..a)) else { continue };
        if !CLAUSE_OPENERS.contains(&w) || gap.is_empty() || !gap.chars().all(|c| c == ' ' || c == '\u{a0}') {
            continue;
        }
        if !prev.chars().all(char::is_alphabetic) {
            continue;
        }
        let p = prev.to_lowercase();
        let p = p.as_str();
        if NO_COMMA_AFTER.contains(&p) {
            continue;
        }
        if w.starts_with("któr") && (PREPOSITIONS.contains(&p) || p == "to") {
            continue;
        }
        v.push(issue(pb, b, format!("Brak przecinka przed „{w}”"), vec![format!(", {w}")]));
    }
}

fn we_ze(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if gap != " " && gap != "\u{a0}" {
            continue;
        }
        let next = w1.to_lowercase();
        let c: Vec<char> = next.chars().collect();
        let at = |i: usize| c.get(i).copied();
        let (long, wanted) = match w0 {
            "w" | "W" => ("we", (at(0) == Some('w') && at(1).is_some_and(is_consonant)) || next == "mnie"),
            "z" | "Z" => {
                // The first sound: `sz` is one consonant (`z szafy`, but `ze szkoły`).
                let after = if at(0) == Some('s') && at(1) == Some('z') { at(2) } else { at(1) };
                let sibilant = matches!(at(0), Some('s' | 'z' | 'ś' | 'ź' | 'ż'));
                ("ze", (sibilant && after.is_some_and(is_consonant)) || next == "mną")
            }
            _ => continue,
        };
        if wanted {
            let fixed = if w0.starts_with(char::is_uppercase) { capitalize(long) } else { long.to_string() };
            v.push(issue(*a0, *b0, format!("Przed tym słowem piszemy „{long}”"), vec![fixed]));
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
            v.push(issue(i, end, "Powtórzony znak interpunkcyjny".into(), suggestions));
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
        let v = check("Ala ma ma kota i się się cieszy.");
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!(v[0].message, "Powtórzone słowo: „ma”");
        assert_eq!(v[0].suggestions, ["ma"]);
        assert!(v.iter().all(|i| i.lang == ProofLang::Pl && i.kind == IssueKind::Grammar));
        clean("Tak tak, to to samo. Ha ha!");
    }

    #[test]
    fn spaces_and_punctuation() {
        let m = messages("Ala ma kota , a kot ma Alę .  Czy na pewno ?");
        assert!(m.contains(&"Zbędna spacja przed znakiem interpunkcyjnym@ ,".to_string()), "{m:?}");
        assert!(m.contains(&"Zbędna spacja przed znakiem interpunkcyjnym@ .".to_string()), "{m:?}");
        assert!(m.contains(&"Zbędna spacja przed znakiem interpunkcyjnym@ ?".to_string()), "{m:?}");
        assert!(m.contains(&"Podwójna spacja@  ".to_string()), "{m:?}");
        clean("Fajnie :) Naprawdę :-) Albo :D Wartość .5 i wielokropek ...");
        let m = messages("Powiedział ,,tak” i poszedł.. Potem;; koniec.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Powtórzony znak")).count(), 3, "{m:?}");
        assert_eq!(check("Powiedział ,,tak”.").iter().find(|i| i.message.starts_with("Powtórzony")).unwrap().suggestions, ["„"]);
        clean("Czekał… i czekał... aż do rana.");
    }

    #[test]
    fn sentence_start_capitals() {
        let m = messages("to jest zdanie. a to drugie! czy trzecie? tak.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Wielka litera")).count(), 4, "{m:?}");
        assert_eq!(check("ala ma kota.")[0].suggestions, ["Ala"]);
        // Abbreviations, initials, ordinals, dates, ellipses and dialogue dashes.
        clean("Kupiłem owoce, np. jabłka, gruszki itp. oraz warzywa.");
        clean("W 1990 r. powstała firma, m.in. dzięki dotacji.");
        clean("Spotkałem J. Kowalskiego i prof. Nowaka przy ul. Długiej.");
        clean("Zajął 5. miejsce w XX w. sporcie.");
        clean("Czekałem... aż w końcu przyszedł.");
        clean("– Naprawdę? – zapytał. – Tak – odpowiedziała.");
        clean("Powiedział: „Wracam”. Potem wyszedł.");
        // A list item may start in lower case.
        clean("jabłka,");
    }

    #[test]
    fn missing_commas() {
        let m = messages("Wiem że przyjdzie. Mam kota który śpi. Zostałem bo musiałem. Idę aby zdążyć. Nie przyszedł ponieważ padało.");
        for want in [
            "Brak przecinka przed „że”@ że",
            "Brak przecinka przed „który”@ który",
            "Brak przecinka przed „aby”@ aby",
            "Brak przecinka przed „ponieważ”@ ponieważ",
        ] {
            assert!(m.contains(&want.to_string()), "{want}: {m:?}");
        }
        assert_eq!(check("Wiem że tak.")[0].suggestions, [", że"]);
        // Compound conjunctions, particles, conjunctions and prepositions take no comma there.
        clean("Przyszedł, mimo że padało, chyba że nie.");
        clean("Zrobię to, tylko jeśli zdążę, i to tak aby nikt nie widział.");
        clean("To dom, w którym mieszkam, i ogród, przez który idę.");
        clean("Wiem, że przyjdzie, i że zostanie.");
        clean("Myślę, że gdyby przyszedł, byłoby dobrze.");
        clean("Zachowywał się, jak gdyby nigdy nic.");
        clean("Który z nich? To który wybierasz?");
        clean("Po raz nie wiadomo który.");
        clean("Dlatego że padało, zostaliśmy.");
    }

    #[test]
    fn we_and_ze() {
        let m = messages("Spotkamy się w wtorek w Wrocławiu z szkoły z mną.");
        assert_eq!(m.iter().filter(|x| x.starts_with("Przed tym słowem")).count(), 4, "{m:?}");
        let v = check("Z szkoły wracam.");
        assert_eq!(v[0].suggestions, ["Ze"]);
        clean("Byłem w wodzie, w wieku dziesięciu lat, z szafą, z siostrą i z zamkiem.");
        clean("Wracam ze szkoły we wtorek ze mną.");
    }

    #[test]
    fn correct_polish_text_is_clean() {
        clean(
            "Zażółć gęślą jaźń. Chrząszcz brzmi w trzcinie, a źdźbło trawy się kołysze. \
             Przyjdę jutro, chyba że będzie padać. Książka, którą czytam, jest ciekawa.",
        );
        clean("Pięćdziesięciu uczniów przyszło, ponieważ ogłoszono konkurs.");
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = check(&s);
        }

        #[test]
        fn never_panics_on_polish_like_text(s in "[a-ząćęłńóśźżA-Z ,.;:!?„”…\\-]{0,60}") {
            for i in check(&s) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
                proptest::prop_assert!(s.is_char_boundary(i.start) && s.is_char_boundary(i.end));
            }
        }
    }
}

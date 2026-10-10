//! WordCraft proofing.
//!
//! - **Spelling** against the public-domain Moby word list (~160k words, the same list that
//!   drives hyphenation), with inflections (`-s`, `-es`, `-ed`, `-ing`, `-ly`, `'s`…), a user
//!   dictionary and ignore lists; suggestions by edit distance. German text is checked against
//!   LanguageTool's German word forms (~550k, CC BY-SA 4.0) with German capitalisation and
//!   compounds ([`german`]); text in other languages is left alone.
//! - **Grammar** checks that are cheap and reliable: repeated words, `a`/`an`, capital letter
//!   after a sentence end, spaces before punctuation, doubled spaces.
//! - **Hyphenation** ([`hyphen`]): dictionary, Liang patterns, heuristic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod dict;
mod german;
pub mod hyphen;
pub mod patterns;
pub mod wordlist;

use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};

/// A problem found in text: byte range, kind, message and suggestions.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub start: usize,
    pub end: usize,
    pub kind: IssueKind,
    pub message: String,
    pub suggestions: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind {
    Spelling,
    Grammar,
}

fn user_words() -> &'static RwLock<HashSet<String>> {
    static U: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    U.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Add a word to the user dictionary ("Add to Dictionary").
pub fn add_word(w: &str) {
    user_words().write().unwrap_or_else(|e| e.into_inner()).insert(w.to_lowercase());
}

/// Is `w` (any case) in the user dictionary?
fn user_knows(w: &str) -> bool {
    user_words().read().unwrap_or_else(|e| e.into_inner()).contains(&w.to_lowercase())
}

/// Words in the user dictionary.
pub fn user_dictionary() -> Vec<String> {
    let mut v: Vec<String> = user_words().read().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect();
    v.sort();
    v
}

/// Common words the source list lacks (modern vocabulary, contractions).
const COMMON: &str = "a an the and or but if of to in on at by for with from as is am are was were be been being it its we us our you your he him his she her they them their i me my mine this that these those there here what which who whom whose when where why how all any both each few more most other some such no nor not only own same so than too very can will just should now do does did done has have had having may might must shall would could up down out over under again further then once off into onto upon about above below between through during before after until while because per via vs etc ok yes yeah oh ah hi hey bye one two three four five six seven eight nine ten first last next new old get got go goes went gone see saw seen say said make made take took come came give gave know knew think thought look want need use used find found tell told ask asked feel felt try left keep let put mean set run show turn move live play pay hear hold bring begin seem help talk start might";

const EXTRA: &[&str] = &[
    "don't",
    "doesn't",
    "didn't",
    "can't",
    "won't",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "shouldn't",
    "wouldn't",
    "couldn't",
    "i'm",
    "i've",
    "i'd",
    "i'll",
    "you're",
    "you've",
    "you'll",
    "you'd",
    "we're",
    "we've",
    "we'll",
    "they're",
    "they've",
    "they'll",
    "it's",
    "that's",
    "there's",
    "here's",
    "what's",
    "let's",
    "email",
    "emails",
    "online",
    "website",
    "websites",
    "internet",
    "software",
    "app",
    "apps",
    "smartphone",
    "blog",
    "login",
    "logout",
    "username",
    "wifi",
    "toolbar",
    "workflow",
    "workflows",
    "dataset",
    "datasets",
    "metadata",
    "startup",
    "startups",
    "covid",
    "selfie",
    "podcast",
    "podcasts",
    "hashtag",
    "wordcraft",
    "artcraft",
    "discord",
    "github",
    "rust",
    "ok",
    "okay",
    "unlabelled",
    "labelled",
    "colour",
    "colours",
    "favourite",
    "centre",
    "organise",
];

fn known_core(w: &str) -> bool {
    let d = dict::Dictionary::en_us();
    if d.get(w).is_some() || EXTRA.contains(&w) || COMMON.split(' ').any(|c| c == w) {
        return true;
    }
    if user_words().read().unwrap_or_else(|e| e.into_inner()).contains(w) {
        return true;
    }
    // Inflections.
    let stems: [(&str, &[&str]); 12] = [
        ("'s", &[""]),
        ("s", &[""]),
        ("es", &["", "e"]),
        ("ies", &["y"]),
        ("ed", &["", "e"]),
        ("ied", &["y"]),
        ("ing", &["", "e"]),
        ("ly", &["", "le"]),
        ("ily", &["y"]),
        ("er", &["", "e"]),
        ("est", &["", "e"]),
        ("ness", &[""]),
    ];
    for (suf, adds) in stems {
        if let Some(stem) = w.strip_suffix(suf)
            && stem.len() >= 2
        {
            for a in adds {
                let s = format!("{stem}{a}");
                if d.get(&s).is_some() || EXTRA.contains(&s.as_str()) {
                    return true;
                }
            }
            // Doubled consonant: running → run, stopped → stop.
            let b = stem.as_bytes();
            if (suf == "ing" || suf == "ed" || suf == "er" || suf == "est")
                && b.len() >= 3
                && b.last() == b.get(b.len() - 2)
                && let Some(s) = stem.get(..stem.len() - 1)
                && d.get(s).is_some()
            {
                return true;
            }
        }
    }
    // Prefixes.
    for pre in ["un", "re", "non", "pre", "over", "under", "co", "mis"] {
        if let Some(rest) = w.strip_prefix(pre)
            && rest.len() >= 3
            && d.get(rest).is_some()
        {
            return true;
        }
    }
    false
}

/// A language proofing has a word list for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    English,
    German,
    /// German as written in Switzerland: `ss` where Germany writes `ß`.
    SwissGerman,
}

/// The proofing language of text marked with a BCP 47 tag (`en-US`, `de-DE`, `de-CH`), or `None`
/// when proofing has no word list for it: such text is left alone rather than flagged word by
/// word. Text without a language counts as English.
pub fn proofing_language(lang: Option<&str>) -> Option<Language> {
    let tag = match lang {
        None | Some("") => return Some(Language::English),
        Some(tag) => tag,
    };
    let mut parts = tag.split(['-', '_', '.']);
    match parts.next().map(str::to_ascii_lowercase).as_deref() {
        Some("en") => Some(Language::English),
        Some("de") => Some(if parts.any(|p| p.eq_ignore_ascii_case("ch")) { Language::SwissGerman } else { Language::German }),
        Some("gsw") => Some(Language::SwissGerman),
        _ => None,
    }
}

/// Does proofing check text in this language (a BCP 47 tag such as `en-US` or `de-DE`)?
pub fn checks_language(lang: Option<&str>) -> bool {
    proofing_language(lang).is_some()
}

/// Is `word` spelled correctly in English? Numbers, single letters, ALL-CAPS acronyms, URLs and
/// words with digits are accepted.
pub fn is_correct(word: &str) -> bool {
    is_correct_in(word, Language::English)
}

/// Is `word` spelled correctly in `lang`?
pub fn is_correct_in(word: &str, lang: Language) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
    let w = w.trim_matches('\'');
    if w.chars().count() <= 1 || w.chars().any(|c| c.is_ascii_digit()) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if w.chars().all(|c| !c.is_lowercase()) && w.chars().count() <= 6 {
        return true;
    }
    if !w.is_ascii() && !w.chars().any(|c| c.is_ascii_alphabetic()) {
        return true; // other scripts: no dictionary
    }
    match lang {
        Language::English => {}
        Language::German => return german::is_correct(w, false),
        Language::SwissGerman => return german::is_correct(w, true),
    }
    let lower = w.to_lowercase().replace('’', "'");
    if known_core(&lower) {
        return true;
    }
    // Hyphenated compounds: each part.
    if lower.contains('-') {
        return lower.split('-').all(|p| p.is_empty() || known_core(p));
    }
    false
}

/// Spelling suggestions for a misspelled English word (best first, at most `max`).
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(word, max, Language::English)
}

/// Spelling suggestions for a word misspelled in `lang` (best first, at most `max`).
pub fn suggest_in(word: &str, max: usize, lang: Language) -> Vec<String> {
    if lang != Language::English {
        return german::suggest(word, max);
    }
    let lower = word.to_lowercase();
    let n = lower.chars().count();
    if n == 0 || n > 40 {
        return Vec::new();
    }
    let d = dict::Dictionary::en_us();
    let mut scored: Vec<(usize, i64, String)> = Vec::new();
    let target: Vec<char> = lower.chars().collect();
    for (w, _) in d.iter() {
        let wn = w.chars().count();
        if wn + 2 < n || wn > n + 2 {
            continue;
        }
        let dist = damerau(&target, w);
        if dist <= 2 {
            let same_first = w.chars().next() == target.first().copied();
            scored.push((dist, if same_first { 0 } else { 1 }, w.to_string()));
        }
    }
    scored.sort_by(|a, b| (a.0, a.1, a.2.len().abs_diff(n)).cmp(&(b.0, b.1, b.2.len().abs_diff(n))).then(a.2.cmp(&b.2)));
    let cap = word.chars().next().is_some_and(char::is_uppercase);
    let upper = word.chars().all(|c| !c.is_lowercase());
    scored
        .into_iter()
        .take(max)
        .map(|(_, _, w)| {
            if upper && n > 1 {
                w.to_uppercase()
            } else if cap {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or(w)
            } else {
                w
            }
        })
        .collect()
}

/// Optimal string alignment distance, early-exit at 3.
fn damerau(a: &[char], b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut prev2 = vec![0usize; m + 1];
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        if let Some(c) = cur.get_mut(0) {
            *c = i;
        }
        let mut row_min = i;
        for j in 1..=m {
            let (ai, bj) = (a.get(i - 1), b.get(j - 1));
            let cost = usize::from(ai != bj);
            let mut v = (prev.get(j).copied().unwrap_or(9) + 1)
                .min(cur.get(j - 1).copied().unwrap_or(9) + 1)
                .min(prev.get(j - 1).copied().unwrap_or(9) + cost);
            if i > 1 && j > 1 && a.get(i - 1) == b.get(j - 2) && a.get(i - 2) == b.get(j - 1) {
                v = v.min(prev2.get(j - 2).copied().unwrap_or(9) + 1);
            }
            if let Some(c) = cur.get_mut(j) {
                *c = v;
            }
            row_min = row_min.min(v);
        }
        if row_min > 2 {
            return 3;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.get(m).copied().unwrap_or(9)
}

/// Words in `text`: (byte start, byte end).
pub fn words(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let is_w = |c: char| c.is_alphanumeric() || c == '\'' || c == '’' || c == '-';
    for (i, c) in text.char_indices() {
        match (start, is_w(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    // Trim surrounding apostrophes/hyphens.
    out.into_iter()
        .filter_map(|(a, b)| {
            let w = text.get(a..b)?;
            let lead = w.len() - w.trim_start_matches(['\'', '’', '-']).len();
            let trail = w.len() - w.trim_end_matches(['\'', '’', '-']).len();
            (a + lead < b - trail).then_some((a + lead, b - trail))
        })
        .collect()
}

/// Spelling issues in a paragraph of English text (no suggestions; ask [`suggest`] on demand).
pub fn check_spelling(text: &str) -> Vec<Issue> {
    check_spelling_in(text, Language::English)
}

/// A host or file name such as `portfolio.example.com` or `brief.docx` (a dot inside the token,
/// a short ending of letters after the last one): not words to check.
fn domain_like(token: &str) -> bool {
    let t = token.trim_end_matches(|c: char| !c.is_alphanumeric());
    t.rsplit_once('.').is_some_and(|(head, ext)| {
        !head.is_empty() && !head.ends_with('.') && (2..=6).contains(&ext.chars().count()) && ext.chars().all(|c| c.is_ascii_alphabetic())
    }) && !t.contains(char::is_whitespace)
}

/// Spelling and grammar issues in a paragraph whose runs may be in different languages.
/// `runs` gives each byte range's proofing language (`None`: not checked, e.g. a link or text
/// marked "Do not check spelling"). An issue is kept when every run it touches is in the
/// language it was found in. Grammar rules are English and apply to English text only.
pub fn check_text(text: &str, runs: &[(std::ops::Range<usize>, Option<Language>)]) -> Vec<Issue> {
    let only = |i: &Issue, lang: Language| {
        let mut touched = false;
        for (r, l) in runs {
            if r.start < i.end && i.start < r.end {
                if *l != Some(lang) {
                    return false;
                }
                touched = true;
            }
        }
        touched
    };
    let mut langs: Vec<Language> = runs.iter().filter_map(|(_, l)| *l).collect();
    langs.sort();
    langs.dedup();
    let mut v = Vec::new();
    for lang in langs {
        v.extend(check_spelling_in(text, lang).into_iter().filter(|i| only(i, lang)));
        if lang == Language::English {
            v.extend(check_grammar(text).into_iter().filter(|i| only(i, lang)));
        }
    }
    v.sort_by_key(|i| i.start);
    v
}

/// Spelling issues in a paragraph of text in `lang`.
pub fn check_spelling_in(text: &str, lang: Language) -> Vec<Issue> {
    let mut v = Vec::new();
    for (a, b) in words(text) {
        let Some(w) = text.get(a..b) else { continue };
        // Skip URLs/emails.
        let ts = text.get(..a).and_then(|t| t.rfind(char::is_whitespace)).map(|i| i + 1).unwrap_or(0);
        let te = text.get(b..).and_then(|t| t.find(char::is_whitespace)).map(|i| b + i).unwrap_or(text.len());
        let token = text.get(ts..te).unwrap_or("");
        if token.contains("://") || token.contains('@') || token.starts_with("www.") || domain_like(token) {
            continue;
        }
        if !is_correct_in(w, lang) {
            v.push(Issue { start: a, end: b, kind: IssueKind::Spelling, message: "Possible spelling mistake".into(), suggestions: Vec::new() });
        }
    }
    v
}

/// Grammar issues in a paragraph of text.
pub fn check_grammar(text: &str) -> Vec<Issue> {
    let mut v = Vec::new();
    let ws = words(text);
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if gap.trim().is_empty()
            && w0.eq_ignore_ascii_case(w1)
            && w0.chars().all(char::is_alphabetic)
            && !matches!(w0.to_lowercase().as_str(), "had" | "that" | "bye" | "so")
        {
            v.push(Issue {
                start: *a0,
                end: *b1,
                kind: IssueKind::Grammar,
                message: format!("Repeated word: \"{w1}\""),
                suggestions: vec![w0.to_string()],
            });
        }
        if gap == " " && (w0 == "a" || w0 == "A") && w1.chars().next().is_some_and(|c| "aeiouAEIOU".contains(c)) && !starts_consonant_sound(w1) {
            v.push(Issue {
                start: *a0,
                end: *b0,
                kind: IssueKind::Grammar,
                message: "Use \"an\" before a vowel sound".into(),
                suggestions: vec![if w0 == "A" { "An" } else { "an" }.into()],
            });
        }
        if gap == " "
            && (w0 == "an" || w0 == "An")
            && w1.chars().next().is_some_and(|c| c.is_alphabetic() && !"aeiouAEIOUhH".contains(c))
            && !w1.chars().all(|c| c.is_uppercase())
        {
            v.push(Issue {
                start: *a0,
                end: *b0,
                kind: IssueKind::Grammar,
                message: "Use \"a\" before a consonant sound".into(),
                suggestions: vec![if w0 == "An" { "A" } else { "a" }.into()],
            });
        }
    }
    // Space before punctuation; doubled spaces.
    for (i, _) in text.match_indices(" ,").chain(text.match_indices(" .")).chain(text.match_indices(" ;")) {
        if text.get(i + 2..).is_some_and(|r| r.starts_with('.') || r.starts_with(',')) {
            continue; // "..." or numbers like " .5"
        }
        v.push(Issue {
            start: i,
            end: i + 2,
            kind: IssueKind::Grammar,
            message: "Remove the space before the punctuation".into(),
            suggestions: vec![text.get(i + 1..i + 2).unwrap_or("").to_string()],
        });
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(Issue { start: i, end: i + 2, kind: IssueKind::Grammar, message: "Extra space".into(), suggestions: vec![" ".into()] });
    }
    // Sentence start capital.
    let mut after_end = true;
    for (a, b) in &ws {
        let Some(w) = text.get(*a..*b) else { continue };
        if after_end && w.chars().next().is_some_and(char::is_lowercase) && w.chars().all(char::is_alphabetic) {
            let fixed: String = w.chars().next().map(|f| f.to_uppercase().chain(w.chars().skip(1)).collect()).unwrap_or_default();
            v.push(Issue {
                start: *a,
                end: *b,
                kind: IssueKind::Grammar,
                message: "Capitalize the first word of a sentence".into(),
                suggestions: vec![fixed],
            });
        }
        let tail = text.get(*b..).unwrap_or("");
        let next_non_space = tail.trim_start_matches(['"', '”', ')', '\'']).chars().next();
        after_end = matches!(next_non_space, Some('.' | '!' | '?')) && tail.trim_start_matches(['.', '!', '?', '"', '”', ')']).starts_with(' ');
    }
    v.sort_by_key(|i| i.start);
    v
}

fn starts_consonant_sound(w: &str) -> bool {
    let l = w.to_lowercase();
    ["uni", "use", "usu", "eu", "one", "once", "ubiq", "uti", "ure", "uro"].iter().any(|p| l.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_with_a_word_list_are_checked() {
        for lang in [None, Some(""), Some("en"), Some("en-US"), Some("EN-gb"), Some("en_AU")] {
            assert_eq!(proofing_language(lang), Some(Language::English), "{lang:?}");
        }
        for lang in [Some("de-DE"), Some("de"), Some("de-AT"), Some("de_DE.UTF-8")] {
            assert_eq!(proofing_language(lang), Some(Language::German), "{lang:?}");
        }
        assert_eq!(proofing_language(Some("de-CH")), Some(Language::SwissGerman));
        for lang in [Some("fr-FR"), Some("eng"), Some("e"), Some("-"), Some("ja")] {
            assert!(!checks_language(lang), "{lang:?}");
        }
    }

    #[test]
    fn mixed_language_paragraphs() {
        let text = "Thsi is wrnog. Das ist ein Fehlr im Haushaltsbudget.";
        let split = text.find("Das").unwrap();
        let runs = vec![(0..split, Some(Language::English)), (split..text.len(), Some(Language::German))];
        let found: Vec<&str> = check_text(text, &runs).iter().filter(|i| i.kind == IssueKind::Spelling).map(|i| &text[i.start..i.end]).collect();
        assert_eq!(found, ["Thsi", "wrnog", "Fehlr"]);
        // German grammar isn't checked by the English rules; text in other languages is left alone.
        let german_only = vec![(0..text.len(), Some(Language::German))];
        assert!(check_text("ist ist gut", &german_only).is_empty());
        assert!(check_text(text, &[(0..text.len(), None)]).is_empty());
        let s = suggest_in("Fehlr", 3, Language::German);
        assert!(s.iter().any(|w| w == "Fehler"), "{s:?}");
        assert!(is_correct_in("Straße", Language::German) && is_correct_in("Fussball", Language::SwissGerman));
        // Host and file names aren't words.
        assert!(check_spelling_in("Mehr auf portfolio.example.com und in brief.docx.", Language::German).is_empty());
        assert!(!check_spelling_in("Ein Fehlr. Noch einer.", Language::German).is_empty());
    }

    #[test]
    fn dictionary_loads() {
        assert!(dict::Dictionary::en_us().len() > 100_000);
    }

    #[test]
    fn spelling() {
        for w in [
            "if",
            "we",
            "hello",
            "world",
            "running",
            "studios",
            "don't",
            "Thursday",
            "NASA",
            "2026",
            "co-operate",
            "unhappy",
            "colour",
            "it's",
            "stopped",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["helo", "wrold", "sentense", "recieve"] {
            assert!(!is_correct(w), "{w}");
        }
        let s = suggest("recieve", 5);
        assert!(s.contains(&"receive".to_string()), "{s:?}");
        assert!(suggest("Wrold", 3).contains(&"World".to_string()));
    }

    #[test]
    fn spelling_issues_in_text() {
        let t = "This sentense has a mispelled word, see https://example.com/xyzq.";
        let v = check_spelling(t);
        let bad: Vec<&str> = v.iter().map(|i| &t[i.start..i.end]).collect();
        assert_eq!(bad, vec!["sentense", "mispelled"]);
    }

    #[test]
    fn grammar_issues() {
        let t = "this is is a apple , and an car.  Done";
        let v = check_grammar(t);
        let msgs: Vec<&str> = v.iter().map(|i| i.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("Repeated")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("\"an\"")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("\"a\" before")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("space before")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Capitalize")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Extra space")), "{msgs:?}");
        assert!(check_grammar("A university is an hour away.").is_empty(), "{:?}", check_grammar("A university is an hour away."));
    }

    #[test]
    fn user_dictionary_works() {
        assert!(!is_correct("zxqwordcrafty"));
        add_word("zxqwordcrafty");
        assert!(is_correct("zxqwordcrafty"));
    }

    #[test]
    fn hyphenation() {
        let l = hyphen::Limits::default();
        assert!(hyphen::hyphenate_word("hyphenation", &l).matches('-').count() >= 2);
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = check_spelling(&s);
            let _ = check_grammar(&s);
            let _ = suggest(&s, 3);
            let _ = hyphen::hyphen_points(&s, &hyphen::Limits::default());
        }
    }
}

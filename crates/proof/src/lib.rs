//! WordCraft proofing, per language. A run's language (its BCP 47 tag) picks the tools through
//! [`ProofLang`]; the `*_in` functions take it, the older English-only ones remain.
//!
//! - **English** ([`ProofLang::En`]; also text with no language): spelling against the
//!   public-domain Moby word list (~160k words, the same list that drives hyphenation), with
//!   inflections (`-s`, `-es`, `-ed`, `-ing`, `-ly`, `'s`…), a user dictionary and ignore lists;
//!   suggestions by edit distance. Grammar checks that are cheap and reliable: repeated words,
//!   `a`/`an`, capital letter after a sentence end, spaces before punctuation, doubled spaces.
//! - **Polish** ([`ProofLang::Pl`], module [`pl`]): spelling against the SJP.PL dictionary
//!   (Apache-2.0 option; ~350k stems with Hunspell-style affix rules, ~4.5 million forms, read by
//!   [`affix`]), suggestions tuned for Polish (diacritics, `ż`/`rz`, `u`/`ó`, missing spaces),
//!   and Polish grammar rules with Polish messages.
//! - **Other languages** ([`ProofLang::Other`]): no spelling or grammar marks, like Word without
//!   the language's proofing tools.
//! - **Hyphenation** ([`hyphen`]): dictionary, Liang patterns (English, Polish), heuristic.
//!
//! [`check_text`] checks a paragraph whose runs are in different languages.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod affix;
pub mod dict;
pub mod hyphen;
pub mod lang;
pub mod patterns;
pub mod pl;

use std::collections::HashSet;
use std::ops::Range;
use std::sync::{OnceLock, RwLock};

pub use lang::ProofLang;

/// A problem found in text: byte range, kind, message and suggestions.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub start: usize,
    pub end: usize,
    pub kind: IssueKind,
    /// In the language of the text it was found in (Polish rules explain in Polish).
    pub message: String,
    pub suggestions: Vec<String>,
    /// The language the text was checked in: spelling suggestions come from its dictionary
    /// ([`suggest_in`]).
    pub lang: ProofLang,
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

/// Add a word to the user dictionary ("Add to Dictionary"). It applies to every language.
pub fn add_word(w: &str) {
    user_words().write().unwrap_or_else(|e| e.into_inner()).insert(w.to_lowercase());
}

/// Is `lower` (a lower-case word) in the user dictionary?
pub(crate) fn in_user_dictionary(lower: &str) -> bool {
    user_words().read().unwrap_or_else(|e| e.into_inner()).contains(lower)
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

/// Is `word` spelled correctly? Numbers, single letters, ALL-CAPS acronyms, URLs and words with
/// digits are accepted.
pub fn is_correct(word: &str) -> bool {
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

/// Spelling suggestions for a misspelled word (best first, at most `max`).
pub fn suggest(word: &str, max: usize) -> Vec<String> {
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
    check_spelling_in(text, ProofLang::En)
}

/// Spelling issues in a paragraph of text in `lang` (none for a language without a dictionary;
/// no suggestions: ask [`suggest_in`] on demand).
pub fn check_spelling_in(text: &str, lang: ProofLang) -> Vec<Issue> {
    if !lang.is_proofed() {
        return Vec::new();
    }
    words(text).into_iter().filter_map(|(a, b)| spelling_issue(text, a, b, lang)).collect()
}

/// The spelling issue of the word at `a..b` of `text`, if it is misspelled in `lang`. Words in
/// web and e-mail addresses are skipped.
fn spelling_issue(text: &str, a: usize, b: usize, lang: ProofLang) -> Option<Issue> {
    let w = text.get(a..b)?;
    let ts = text.get(..a).and_then(|t| t.rfind(char::is_whitespace)).map(|i| i + 1).unwrap_or(0);
    let te = text.get(b..).and_then(|t| t.find(char::is_whitespace)).map(|i| b + i).unwrap_or(text.len());
    let token = text.get(ts..te).unwrap_or("");
    if token.contains("://") || token.contains('@') || token.starts_with("www.") || is_correct_in(w, lang) {
        return None;
    }
    let message = match lang {
        ProofLang::Pl => "Możliwy błąd pisowni",
        _ => "Possible spelling mistake",
    };
    Some(Issue { start: a, end: b, kind: IssueKind::Spelling, message: message.into(), suggestions: Vec::new(), lang })
}

/// Is `word` spelled correctly in `lang`? Always true in a language without a dictionary.
pub fn is_correct_in(word: &str, lang: ProofLang) -> bool {
    match lang {
        ProofLang::En => is_correct(word),
        ProofLang::Pl => pl::is_correct(word),
        ProofLang::Other => true,
    }
}

/// Spelling suggestions for a word misspelled in `lang` (best first, at most `max`).
pub fn suggest_in(word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    match lang {
        ProofLang::En => suggest(word, max),
        ProofLang::Pl => pl::suggest(word, max),
        ProofLang::Other => Vec::new(),
    }
}

/// Grammar issues in a paragraph of text in `lang`: the English rules, the Polish ones
/// ([`pl::grammar`]), or none for a language WordCraft doesn't proof.
pub fn check_grammar_in(text: &str, lang: ProofLang) -> Vec<Issue> {
    match lang {
        ProofLang::En => check_grammar(text),
        ProofLang::Pl => pl::grammar::check(text),
        ProofLang::Other => Vec::new(),
    }
}

/// Spelling and grammar issues in a paragraph whose text is in several languages, in order.
/// `spans` gives the language of byte ranges of `text` (in order, not overlapping); text outside
/// them is English. Each word is spelled in the language of its first letter. Each language's
/// grammar rules read the whole paragraph (so sentence boundaries are seen) but report only
/// inside that language's spans: an English quotation in a Polish paragraph gets English
/// spelling and is never told a Polish rule.
pub fn check_text(text: &str, spans: &[(Range<usize>, ProofLang)]) -> Vec<Issue> {
    // Sorted, empty ones dropped, neighbours of one language merged.
    let mut sorted: Vec<&(Range<usize>, ProofLang)> = spans.iter().filter(|(r, _)| r.start < r.end).collect();
    sorted.sort_by_key(|s| s.0.start);
    let mut merged: Vec<(Range<usize>, ProofLang)> = Vec::new();
    for (r, l) in sorted {
        match merged.last_mut() {
            Some((m, ml)) if *ml == *l && m.end == r.start => m.end = r.end,
            _ => merged.push((r.clone(), *l)),
        }
    }
    let lang_at = |i: usize| -> ProofLang {
        let k = merged.partition_point(|(r, _)| r.end <= i);
        merged.get(k).filter(|(r, _)| r.start <= i).map_or(ProofLang::En, |s| s.1)
    };
    let mut v: Vec<Issue> = words(text).into_iter().filter_map(|(a, b)| spelling_issue(text, a, b, lang_at(a))).collect();
    let mut langs: Vec<ProofLang> = merged.iter().map(|s| s.1).collect();
    let mut covered = 0;
    for (r, _) in &merged {
        if r.start > covered {
            break;
        }
        covered = covered.max(r.end);
    }
    if covered < text.len() || merged.is_empty() {
        langs.push(ProofLang::En);
    }
    langs.sort();
    langs.dedup();
    for lang in langs {
        v.extend(check_grammar_in(text, lang).into_iter().filter(|i| lang_at(i.start) == lang));
    }
    v.sort_by_key(|i| (i.start, i.end));
    v
}

/// Grammar issues in a paragraph of English text.
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
                lang: ProofLang::En,
            });
        }
        if gap == " " && (w0 == "a" || w0 == "A") && w1.chars().next().is_some_and(|c| "aeiouAEIOU".contains(c)) && !starts_consonant_sound(w1) {
            v.push(Issue {
                start: *a0,
                end: *b0,
                kind: IssueKind::Grammar,
                message: "Use \"an\" before a vowel sound".into(),
                suggestions: vec![if w0 == "A" { "An" } else { "an" }.into()],
                lang: ProofLang::En,
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
                lang: ProofLang::En,
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
            lang: ProofLang::En,
        });
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(Issue {
            start: i,
            end: i + 2,
            kind: IssueKind::Grammar,
            message: "Extra space".into(),
            suggestions: vec![" ".into()],
            lang: ProofLang::En,
        });
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
                lang: ProofLang::En,
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

    fn flagged(t: &str, v: &[Issue]) -> Vec<String> {
        v.iter().filter(|i| i.kind == IssueKind::Spelling).map(|i| t[i.start..i.end].to_string()).collect()
    }

    #[test]
    fn each_language_uses_its_own_tools() {
        let pl = "Zażółć gęślą jaźń, chrząszcz brzmi w trzcinie.";
        assert!(check_spelling_in(pl, ProofLang::Pl).is_empty(), "{:?}", check_spelling_in(pl, ProofLang::Pl));
        assert!(check_spelling_in(pl, ProofLang::En).len() >= 3, "Polish words are English misspellings");
        assert!(check_spelling_in("Nous écrivons en français.", ProofLang::Other).is_empty());
        assert!(check_grammar_in("this is is wrong", ProofLang::Other).is_empty());
        assert!(is_correct_in("anything", ProofLang::Other) && suggest_in("xyzzy", 5, ProofLang::Other).is_empty());
        let v = check_spelling_in("Płynie żeka.", ProofLang::Pl);
        assert_eq!(flagged("Płynie żeka.", &v), ["żeka"]);
        assert_eq!((v[0].lang, v[0].message.as_str()), (ProofLang::Pl, "Możliwy błąd pisowni"));
        assert!(suggest_in("żeka", 5, ProofLang::Pl).contains(&"rzeka".to_string()));
        assert!(suggest_in("recieve", 5, ProofLang::En).contains(&"receive".to_string()));
        // Polish grammar speaks Polish and has no English a/an rule.
        let g = check_grammar_in("Wiem że to jest a apple.", ProofLang::Pl);
        assert!(g.iter().any(|i| i.message == "Brak przecinka przed „że”"), "{g:?}");
        assert!(!g.iter().any(|i| i.message.contains("\"an\"")), "{g:?}");
    }

    #[test]
    fn mixed_language_paragraphs_check_each_run_in_its_language() {
        // A Polish paragraph quoting English: English spelling and rules inside the quote only.
        let t = "Napisał „the quick brown fox is is here” i poszedł spać, bo żeka wylała.";
        let (a, b) = (t.find("the").unwrap(), t.find('”').unwrap());
        let spans = [(0..a, ProofLang::Pl), (a..b, ProofLang::En), (b..t.len(), ProofLang::Pl)];
        let v = check_text(t, &spans);
        assert_eq!(flagged(t, &v), ["żeka"], "{v:?}");
        let rep = v.iter().find(|i| i.kind == IssueKind::Grammar).unwrap();
        assert_eq!((rep.lang, rep.message.as_str()), (ProofLang::En, "Repeated word: \"is\""), "{v:?}");
        // The English rules don't run over the Polish text (no "Capitalize" for the lower-case
        // Polish words), and the Polish rules don't run over the English quote.
        assert!(v.iter().all(|i| i.lang == ProofLang::Pl || (a..b).contains(&i.start)), "{v:?}");
        // Text outside every span is English; no spans at all is English.
        assert_eq!(flagged("helo świat", &check_text("helo świat", &[])), ["helo", "świat"]);
        assert_eq!(flagged("helo świat", &check_text("helo świat", &[(5..12, ProofLang::Pl)])), ["helo"]);
        // A language without tools: nothing flagged there.
        assert!(check_text("Bonjour mes amis", &[(0..16, ProofLang::Other)]).is_empty());
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = check_spelling(&s);
            let _ = check_grammar(&s);
            let _ = suggest(&s, 3);
            let _ = hyphen::hyphen_points(&s, &hyphen::Limits::default());
            for lang in [ProofLang::En, ProofLang::Pl, ProofLang::Other] {
                let _ = check_spelling_in(&s, lang);
                let _ = check_grammar_in(&s, lang);
                let _ = is_correct_in(&s, lang);
                let _ = hyphen::hyphen_points_in(&s, &hyphen::Limits::default(), lang);
            }
            let _ = lang::normalize_tag(&s);
            let _ = ProofLang::from_tag(Some(&s));
        }

        #[test]
        fn polish_suggestions_never_panic(s in "[a-ząćęłńóśźżA-ZĄĆĘŁŃÓŚŹŻ'’ -]{0,14}") {
            let _ = suggest_in(&s, 4, ProofLang::Pl);
        }

        #[test]
        fn mixed_spans_never_panic(s in "\\PC{0,60}", cuts in proptest::collection::vec((0usize..80, 0usize..80, 0u8..3), 0..6)) {
            let langs = [ProofLang::En, ProofLang::Pl, ProofLang::Other];
            let spans: Vec<(Range<usize>, ProofLang)> = cuts.iter().map(|&(a, b, l)| (a.min(b)..a.max(b), langs[l as usize % 3])).collect();
            for i in check_text(&s, &spans) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
            }
        }
    }
}

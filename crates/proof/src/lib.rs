//! WordCraft proofing.
//!
//! - **Spelling** against the public-domain Moby word list (~160k words, the same list that
//!   drives hyphenation), with inflections (`-s`, `-es`, `-ed`, `-ing`, `-ly`, `'s`…), a user
//!   dictionary and ignore lists; suggestions by edit distance.
//! - **Grammar** checks that are cheap and reliable: repeated words, `a`/`an`, capital letter
//!   after a sentence end, spaces before punctuation, doubled spaces.
//! - **Hyphenation** ([`hyphen`]): dictionary, Liang patterns, heuristic.
//!
//! Arabic and other Arabic-script text: tokenization keeps zero-width joiners inside words
//! (Persian نیم‌فاصله) and diacritics with their letters; spelling has no bundled Arabic
//! dictionary yet, so Arabic-script words pass (the custom dictionary applies in every script).
//! The English-only rules route by script: `a`/`an` need a Latin next word and sentence
//! capitals need cased letters, so neither fires on Arabic; script-neutral ones (repetition,
//! spacing) apply. [`strip_arabic_diacritics`] and [`arabic_search_pattern`] define the
//! diacritic policy for search: diacritics match optionally by default, exactly on request,
//! without rewriting stored text.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod dict;
pub mod hyphen;
pub mod patterns;

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
/// digits are accepted. Arabic-script words have no bundled dictionary yet (see the module
/// docs): they pass, unless the user dictionary says otherwise — "Add to Dictionary" works in
/// any script, so custom Arabic words are honoured here first.
pub fn is_correct(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
    let w = w.trim_matches('\'');
    if w.chars().count() <= 1 || w.chars().any(|c| c.is_ascii_digit()) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if w.chars().all(|c| !c.is_lowercase()) && w.chars().count() <= 6 {
        return true;
    }
    let lower = w.to_lowercase().replace('’', "'");
    // The custom dictionary comes before the script bail-out below, so words the user added
    // are honoured in every script.
    if user_words().read().unwrap_or_else(|e| e.into_inner()).contains(&lower) {
        return true;
    }
    if !w.is_ascii() && !w.chars().any(|c| c.is_ascii_alphabetic()) {
        return true; // other scripts: no dictionary
    }
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
    // Zero-width joiners are word-internal: Persian نیم‌فاصله (U+200C) joins half-spaces, and
    // U+200D joins emoji sequences; neither starts a word on its own (trimmed below).
    let is_w = |c: char| c.is_alphanumeric() || c == '\'' || c == '’' || c == '-' || c == '\u{200C}' || c == '\u{200D}';
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
    // Trim surrounding apostrophes/hyphens/joiners.
    out.into_iter()
        .filter_map(|(a, b)| {
            let w = text.get(a..b)?;
            let lead = w.len() - w.trim_start_matches(['\'', '’', '-', '\u{200C}', '\u{200D}']).len();
            let trail = w.len() - w.trim_end_matches(['\'', '’', '-', '\u{200C}', '\u{200D}']).len();
            (a + lead < b - trail).then_some((a + lead, b - trail))
        })
        .collect()
}

/// An Arabic diacritic (haraka): U+064B–U+0655 (fathatan..hamza below), U+0670 (superscript
/// alef) and the Quranic marks U+06D6–U+06DC, U+06DF–U+06E4, U+06E7–06E8, U+06EA–U+06ED.
/// Letters, tatweel (U+0640) and the Quranic ayah marks (U+06DD etc.) are NOT diacritics.
pub fn is_arabic_diacritic(c: char) -> bool {
    matches!(c,
        '\u{064B}'..='\u{0655}' | '\u{0670}' | '\u{06D6}'..='\u{06DC}' | '\u{06DF}'..='\u{06E4}' | '\u{06E7}'..='\u{06E8}' | '\u{06EA}'..='\u{06ED}')
}

/// `text` with Arabic diacritics removed (matching only; stored content is never rewritten).
/// A fathatan on an alef leaves the alef: "بِسْمِ" → "بسم".
pub fn strip_arabic_diacritics(text: &str) -> String {
    text.chars().filter(|c| !is_arabic_diacritic(*c)).collect()
}

/// An Arabic-script letter (for search expansion): alphanumeric, in an Arabic block, and not
/// itself a diacritic.
pub fn is_arabic_letter(c: char) -> bool {
    c.is_alphanumeric()
        && !is_arabic_diacritic(c)
        && matches!(c,
        '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

/// A literal (non-regex) query as a diacritic-insensitive regex, or `None` when the query has
/// no Arabic letters (the caller uses the query escaped as-is). Diacritics are stripped from
/// the query and every remaining Arabic letter allows optional diacritics after it, so "بسم"
/// and "بِسْمِ" both match "بِسْمِ" — including its marks in the match range, without touching
/// stored text.
pub fn arabic_search_pattern(query: &str) -> Option<String> {
    if !query.chars().any(is_arabic_letter) {
        return None;
    }
    const MARKS: &str = r"[\u064b-\u0655\u0670\u06d6-\u06dc\u06df-\u06e4\u06e7-\u06e8\u06ea-\u06ed]*";
    let mut pat = String::with_capacity(query.len() * 2);
    for c in strip_arabic_diacritics(query).chars() {
        pat.push_str(&regex::escape(&c.to_string()));
        if is_arabic_letter(c) {
            pat.push_str(MARKS);
        }
    }
    Some(pat)
}

/// Spelling issues in a paragraph of text (no suggestions; ask [`suggest`] on demand).
pub fn check_spelling(text: &str) -> Vec<Issue> {
    let mut v = Vec::new();
    for (a, b) in words(text) {
        let Some(w) = text.get(a..b) else { continue };
        // Skip URLs/emails.
        let ts = text.get(..a).and_then(|t| t.rfind(char::is_whitespace)).map(|i| i + 1).unwrap_or(0);
        let te = text.get(b..).and_then(|t| t.find(char::is_whitespace)).map(|i| b + i).unwrap_or(text.len());
        let token = text.get(ts..te).unwrap_or("");
        if token.contains("://") || token.contains('@') || token.starts_with("www.") {
            continue;
        }
        if !is_correct(w) {
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
            && w1.chars().next().is_some_and(|c| c.is_ascii_alphabetic() && !"aeiouAEIOUhH".contains(c))
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
        // The custom dictionary works in every script (Arabic has no bundled dictionary).
        add_word("كتابي");
        assert!(is_correct("كتابي"));
        assert!(user_dictionary().contains(&"كتابي".to_string()));
    }

    #[test]
    fn grammar_policy_for_arabic() {
        // Script-neutral rules still apply to Arabic; English-only rules route by script.
        let v = check_grammar("الكتاب الكتاب");
        assert!(v.iter().any(|i| i.message.contains("Repeated")), "{v:?}");
        let v = check_grammar("نص  بمسافة");
        assert!(v.iter().any(|i| i.message.contains("Extra space")), "{v:?}");
        assert!(check_grammar("a الكتاب").iter().all(|i| !i.message.contains("\"an\"")));
        assert!(check_grammar("an الكتاب").iter().all(|i| !i.message.contains("\"a\" before")), "no Latin rule on Arabic");
        assert!(check_grammar("سلام. دنیا").iter().all(|i| !i.message.contains("Capitalize")));
    }

    #[test]
    fn arabic_tokenization_keeps_joiners_and_marks() {
        // Persian half-spaces join words; diacritics stay with their letters; byte ranges exact.
        let t = "می‌خواهم بِسْمِ test";
        let ws = words(t);
        assert_eq!(ws.len(), 3);
        assert_eq!(&t[ws[0].0..ws[0].1], "می‌خواهم");
        assert_eq!(&t[ws[1].0..ws[1].1], "بِسْمِ");
        assert_eq!(&t[ws[2].0..ws[2].1], "test");
        // A lone joiner is trimmed, not a word.
        assert_eq!(words("\u{200C}"), Vec::new());
        // Arabic-script words pass spelling (no bundled dictionary); suggestions stay empty.
        assert!(is_correct("می‌خواهم") && is_correct("بِسْمِ") && is_correct("كتاب"));
        assert!(suggest("كتاب", 3).is_empty());
        assert!(check_spelling(t).is_empty());
    }

    #[test]
    fn arabic_diacritic_policy() {
        assert!(is_arabic_diacritic('\u{064B}') && is_arabic_diacritic('\u{0652}') && is_arabic_diacritic('\u{0670}'));
        assert!(is_arabic_diacritic('\u{06D6}') && is_arabic_diacritic('\u{06ED}'));
        assert!(!is_arabic_diacritic('ب') && !is_arabic_diacritic('\u{0640}') && !is_arabic_diacritic(' '));
        assert_eq!(strip_arabic_diacritics("بِسْمِ"), "بسم");
        assert_eq!(strip_arabic_diacritics("طٰه"), "طه");
        assert_eq!(strip_arabic_diacritics("hello بسم"), "hello بسم");
        // Letters, tatweel and ayah marks survive stripping.
        assert_eq!(strip_arabic_diacritics("مـ"), "مـ");
        assert!(is_arabic_letter('ب') && is_arabic_letter('پ'));
        assert!(!is_arabic_letter('\u{064B}') && !is_arabic_letter('a'));
    }

    #[test]
    fn arabic_search_pattern_expands_marks() {
        assert_eq!(arabic_search_pattern("hello"), None);
        assert_eq!(arabic_search_pattern(""), None);
        let pat = arabic_search_pattern("بسم").expect("arabic");
        let re = regex::Regex::new(&pat).unwrap();
        assert!(re.is_match("بسم") && re.is_match("بِسْمِ") && re.is_match("بَسْمَ"));
        assert!(!re.is_match("بسن"));
        // A query carrying its own marks matches the same set (marks never anchor).
        let pat = arabic_search_pattern("بِسْمِ").expect("arabic");
        assert!(regex::Regex::new(&pat).unwrap().is_match("بسم"));
        // The match spans the marks: byte ranges stay exact.
        let re = regex::Regex::new(&arabic_search_pattern("بسم").unwrap()).unwrap();
        let m = re.find("x بِسْمِ y").unwrap();
        assert_eq!(&"x بِسْمِ y"[m.start()..m.end()], "بِسْمِ");
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
            let _ = strip_arabic_diacritics(&s);
            let _ = words(&s);
            if let Some(p) = arabic_search_pattern(&s) {
                let _ = regex::Regex::new(&p);
            }
        }
    }
}

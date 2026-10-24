//! Results from an external checker (LanguageTool, #575).
//!
//! An external checker runs somewhere else (a server) and answers later, so its results are kept
//! here, keyed by the exact paragraph text they were computed for and the language they were
//! checked in. A paragraph whose text has a
//! stored result in its language shows those issues instead of the built-in checks ([`crate::check`]); any edit
//! changes the text, so a stale result can never land on the wrong words — the paragraph falls
//! back to the built-in checks until the checker answers for the new text.
//!
//! Each stored result has a [`stamp`] (a counter that only rises) that layout folds into its
//! per-paragraph cache key, so a result arriving redraws only its own paragraph.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{OnceLock, RwLock};

use crate::Issue;

/// At most this many paragraphs are remembered; past it the store starts over (results are
/// asked for again as paragraphs are seen).
pub const MAX_ENTRIES: usize = 20_000;

/// At most this many issues are kept per paragraph.
pub const MAX_ISSUES: usize = 1_000;

#[derive(Default)]
struct Store {
    /// (Text, language) → (stamp, issues). The same text in two languages has two results.
    map: HashMap<(String, String), (u64, Vec<Issue>)>,
    next: u64,
}

fn store() -> &'static RwLock<Store> {
    static S: OnceLock<RwLock<Store>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(Store::default()))
}

/// Entries in the store, readable without the lock (the hot path asks this every layout).
static LEN: AtomicUsize = AtomicUsize::new(0);

/// Remember the issues an external checker found in `text` checked in `language`. Issues outside the text, empty, or
/// not on character boundaries are dropped; the rest are sorted by start.
pub fn put(text: &str, language: &str, issues: Vec<Issue>) {
    let mut issues: Vec<Issue> = issues
        .into_iter()
        .filter(|i| i.start < i.end && i.end <= text.len() && text.is_char_boundary(i.start) && text.is_char_boundary(i.end))
        .take(MAX_ISSUES)
        .collect();
    issues.sort_by_key(|i| (i.start, i.end));
    let mut s = store().write().unwrap_or_else(|e| e.into_inner());
    let key = (text.to_string(), language.to_string());
    if s.map.len() >= MAX_ENTRIES && !s.map.contains_key(&key) {
        s.map.clear();
    }
    s.next = s.next.saturating_add(1);
    let stamp = s.next;
    s.map.insert(key, (stamp, issues));
    LEN.store(s.map.len(), Ordering::Relaxed);
}

/// The stored issues for exactly `text` checked in `language`, if any.
pub fn get(text: &str, language: &str) -> Option<Vec<Issue>> {
    if LEN.load(Ordering::Relaxed) == 0 {
        return None;
    }
    store().read().unwrap_or_else(|e| e.into_inner()).map.get(&(text.to_string(), language.to_string())).map(|(_, v)| v.clone())
}

/// Whether a result is stored for exactly `text` checked in `language`. A result in another
/// language (the paragraph's language changed) doesn't count: it is checked again, and kept
/// beside the other.
pub fn has(text: &str, language: &str) -> bool {
    get_stamp(text, language) != 0
}

fn get_stamp(text: &str, language: &str) -> u64 {
    if LEN.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    store().read().unwrap_or_else(|e| e.into_inner()).map.get(&(text.to_string(), language.to_string())).map(|(s, _)| *s).unwrap_or(0)
}

/// Which stored result applies to `text` in `language` (0: none). Changes whenever a new result
/// is stored for it, so layout can key its cache on it.
pub fn stamp(text: &str, language: &str) -> u64 {
    get_stamp(text, language)
}

/// Is `code` shaped like a language code (`en`, `en-US`, `de-DE-x-simple-language`)?
pub fn is_language_code(code: &str) -> bool {
    (2..=35).contains(&code.len())
        && code.starts_with(|c: char| c.is_ascii_alphabetic())
        && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !code.ends_with('-')
}

/// The language code to check a paragraph in: `mode` is the setting (`auto`, `document` or a
/// code), `para` the paragraph's own language tag.
pub fn language_for(mode: &str, para: Option<&str>) -> String {
    let code = match mode.trim() {
        "" | "auto" => None,
        "document" => para.map(str::trim),
        c => Some(c),
    };
    code.filter(|c| is_language_code(c)).map(str::to_string).unwrap_or_else(|| "auto".to_string())
}

/// A paragraph's same-language spans (#763): byte ranges into `text`, in order and covering it,
/// each with the language to check it in. `runs` are the paragraph's runs (byte range, language
/// tag as the document gives it) and `language_of` maps a tag to the checker's language. Adjacent
/// runs in the same language share a span; blank runs (spaces, deleted text, objects) join their
/// neighbour instead of splitting one.
pub fn language_spans(
    text: &str,
    runs: &[(Range<usize>, Option<&str>)],
    language_of: impl Fn(Option<&str>) -> String,
) -> Vec<(Range<usize>, String)> {
    let len = text.len();
    let blank = |t: &str| t.chars().all(|c| c.is_whitespace() || c == '\u{1}' || c == '\u{FFFC}');
    let mut spans: Vec<(Range<usize>, String, bool)> = Vec::new();
    for (r, tag) in runs {
        let r = r.start.min(len)..r.end.min(len);
        if r.is_empty() {
            continue;
        }
        let language = language_of(*tag);
        let is_blank = text.get(r.clone()).is_none_or(blank);
        match spans.last_mut() {
            Some((last, l, last_blank)) if is_blank || *l == language || *last_blank => {
                last.end = r.end;
                if *last_blank && !is_blank {
                    *l = language;
                    *last_blank = false;
                }
            }
            _ => spans.push((r, language, is_blank)),
        }
    }
    if spans.is_empty() {
        spans.push((0..len, language_of(runs.first().and_then(|(_, t)| *t)), false));
    }
    // The runs cover the text; if they ever fall short, the last span takes the rest.
    if let Some(last) = spans.last_mut() {
        last.0.end = len;
    }
    spans.into_iter().map(|(r, l, _)| (r, l)).collect()
}

/// The "language" a paragraph's result is stored under: the language itself when the paragraph
/// has one, else every span's (`en-US@0-8,fr-FR@8-29`), so the result is found again only while
/// its languages and boundaries are unchanged.
pub fn signature(spans: &[(Range<usize>, String)]) -> String {
    match spans {
        [(_, l)] => l.clone(),
        _ => spans.iter().map(|(r, l)| format!("{l}@{}-{}", r.start, r.end)).collect::<Vec<_>>().join(","),
    }
}

fn mode() -> &'static RwLock<String> {
    static M: OnceLock<RwLock<String>> = OnceLock::new();
    M.get_or_init(|| RwLock::new("auto".to_string()))
}

/// Set the language setting the app checks with, so layout asks for the same result.
pub fn set_mode(m: &str) {
    *mode().write().unwrap_or_else(|e| e.into_inner()) = m.to_string();
}

/// The language the checker uses for a paragraph tagged `para` under the current setting.
pub fn wanted(para: Option<&str>) -> String {
    language_for(&mode().read().unwrap_or_else(|e| e.into_inner()), para)
}

/// Whether anything is stored (cheap; lets callers skip work when no external checker runs).
pub fn is_empty() -> bool {
    LEN.load(Ordering::Relaxed) == 0
}

/// Forget every stored result (the checker was turned off or its settings changed).
pub fn clear() {
    let mut s = store().write().unwrap_or_else(|e| e.into_inner());
    s.map.clear();
    LEN.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(tag: Option<&str>) -> String {
        language_for("document", tag)
    }

    /// #763: runs in one language make one span (and the plain language as signature); a blank
    /// run joins its neighbour; a second language splits the paragraph.
    #[test]
    fn language_spans_group_runs_and_name_mixed_paragraphs() {
        let text = "Hello world bonjour";
        let one = language_spans(text, &[(0..5, Some("en-US")), (5..6, Some("fr-FR")), (6..19, Some("en-US"))], lang);
        assert_eq!(one, vec![(0..19, "en-US".to_string())]);
        assert_eq!(signature(&one), "en-US");
        let two = language_spans(text, &[(0..12, Some("en-US")), (12..19, Some("fr-FR"))], lang);
        assert_eq!(two, vec![(0..12, "en-US".to_string()), (12..19, "fr-FR".to_string())]);
        assert_eq!(signature(&two), "en-US@0-12,fr-FR@12-19");
        // Runs past the text, and no runs at all, still cover it.
        assert_eq!(language_spans(text, &[(0..99, Some("en-US"))], lang), vec![(0..19, "en-US".to_string())]);
        assert_eq!(language_spans("", &[], lang), vec![(0..0, "auto".to_string())]);
    }
}

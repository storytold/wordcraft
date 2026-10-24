//! Results from an external checker (LanguageTool, #575).
//!
//! An external checker runs somewhere else (a server) and answers later, so its results are kept
//! here, keyed by the exact paragraph text they were computed for and the language they were
//! checked in: the same text in two languages keeps both results (#1283), so neither paragraph
//! asks again in a loop. A paragraph whose text has a
//! stored result shows those issues instead of the built-in checks ([`crate::check`]); any edit
//! changes the text, so a stale result can never land on the wrong words — the paragraph falls
//! back to the built-in checks until the checker answers for the new text.
//!
//! Each stored result has a [`stamp`] (a counter that only rises) that layout folds into its
//! per-paragraph cache key, so a result arriving redraws only its own paragraph.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{OnceLock, RwLock};

use crate::{Issue, ProofLang};

/// At most this many paragraphs are remembered; past it the store starts over (results are
/// asked for again as paragraphs are seen).
pub const MAX_ENTRIES: usize = 20_000;

/// At most this many languages are kept per text (the oldest goes first).
pub const MAX_LANGUAGES: usize = 8;

/// At most this many issues are kept per paragraph.
pub const MAX_ISSUES: usize = 1_000;

/// One result: the language it was checked in, its stamp and its issues.
struct Entry {
    language: String,
    /// The proofing language of the paragraph it was asked for (what layout can see).
    proof_lang: ProofLang,
    stamp: u64,
    issues: Vec<Issue>,
}

#[derive(Default)]
struct Store {
    /// Text → its results, one per language, the newest last.
    map: HashMap<String, Vec<Entry>>,
    next: u64,
}

impl Store {
    fn latest(&self, text: &str) -> Option<&Entry> {
        self.map.get(text).and_then(|v| v.last())
    }

    /// The newest result asked for in `lang`, else the newest of all.
    fn for_lang(&self, text: &str, lang: ProofLang) -> Option<&Entry> {
        let v = self.map.get(text)?;
        v.iter().rev().find(|e| e.proof_lang == lang).or_else(|| v.last())
    }
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
    put_as(text, language, ProofLang::default(), issues);
}

/// [`put`] for a paragraph whose proofing language is `proof_lang`: [`get_for`] finds the result
/// by it, so two paragraphs with the same text in two languages each show their own (#1283).
pub fn put_as(text: &str, language: &str, proof_lang: ProofLang, issues: Vec<Issue>) {
    let mut issues: Vec<Issue> = issues
        .into_iter()
        .filter(|i| i.start < i.end && i.end <= text.len() && text.is_char_boundary(i.start) && text.is_char_boundary(i.end))
        .take(MAX_ISSUES)
        .collect();
    issues.sort_by_key(|i| (i.start, i.end));
    let mut s = store().write().unwrap_or_else(|e| e.into_inner());
    if s.map.len() >= MAX_ENTRIES && !s.map.contains_key(text) {
        s.map.clear();
    }
    s.next = s.next.saturating_add(1);
    let stamp = s.next;
    let entries = s.map.entry(text.to_string()).or_default();
    entries.retain(|e| e.language != language);
    if entries.len() >= MAX_LANGUAGES {
        entries.remove(0);
    }
    entries.push(Entry { language: language.to_string(), proof_lang, stamp, issues });
    LEN.store(s.map.len(), Ordering::Relaxed);
}

/// The stored issues for exactly `text`, if any: the newest result when it was checked in more
/// than one language (layout doesn't know the checker's language choice; the same text in two
/// languages is rare, and each paragraph is still checked in its own).
pub fn get(text: &str) -> Option<Vec<Issue>> {
    if LEN.load(Ordering::Relaxed) == 0 {
        return None;
    }
    store().read().unwrap_or_else(|e| e.into_inner()).latest(text).map(|e| e.issues.clone())
}

/// The stored issues for exactly `text` for a paragraph in `lang`: the result asked for in that
/// language when there is one, else the newest.
pub fn get_for(text: &str, lang: ProofLang) -> Option<Vec<Issue>> {
    if LEN.load(Ordering::Relaxed) == 0 {
        return None;
    }
    store().read().unwrap_or_else(|e| e.into_inner()).for_lang(text, lang).map(|e| e.issues.clone())
}

/// Whether a result is stored for exactly `text` checked in `language`. A result in another
/// language (the paragraph's language changed, or another paragraph with the same text is in
/// another language) doesn't count: it is checked again, and both results are kept.
pub fn has(text: &str, language: &str) -> bool {
    LEN.load(Ordering::Relaxed) > 0
        && store().read().unwrap_or_else(|e| e.into_inner()).map.get(text).is_some_and(|v| v.iter().any(|e| e.language == language))
}

/// Which stored result applies to `text` (0: none). Changes whenever a new result is stored for
/// it, so layout can key its cache on it.
pub fn stamp(text: &str) -> u64 {
    if LEN.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    // The sum over the languages: any new result redraws the paragraph.
    store().read().unwrap_or_else(|e| e.into_inner()).map.get(text).map(|v| v.iter().map(|e| e.stamp).fold(0u64, u64::saturating_add)).unwrap_or(0)
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

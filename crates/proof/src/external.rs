//! Results from an external checker (LanguageTool, #575).
//!
//! An external checker runs somewhere else (a server) and answers later, so its results are kept
//! here, keyed by the exact paragraph text they were computed for, with the language they were
//! checked in. A paragraph whose text has a
//! stored result shows those issues instead of the built-in checks ([`crate::check`]); any edit
//! changes the text, so a stale result can never land on the wrong words — the paragraph falls
//! back to the built-in checks until the checker answers for the new text.
//!
//! Each stored result has a [`stamp`] (a counter that only rises) that layout folds into its
//! per-paragraph cache key, so a result arriving redraws only its own paragraph.

use std::collections::HashMap;
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
    /// Text → (stamp, language, issues).
    map: HashMap<String, (u64, String, Vec<Issue>)>,
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
    if s.map.len() >= MAX_ENTRIES && !s.map.contains_key(text) {
        s.map.clear();
    }
    s.next = s.next.saturating_add(1);
    let stamp = s.next;
    s.map.insert(text.to_string(), (stamp, language.to_string(), issues));
    LEN.store(s.map.len(), Ordering::Relaxed);
}

/// The stored issues for exactly `text`, if any.
pub fn get(text: &str) -> Option<Vec<Issue>> {
    if LEN.load(Ordering::Relaxed) == 0 {
        return None;
    }
    store().read().unwrap_or_else(|e| e.into_inner()).map.get(text).map(|(_, _, v)| v.clone())
}

/// Whether a result is stored for exactly `text` checked in `language`. A result in another
/// language (the paragraph's language changed) doesn't count: it is checked again, and the new
/// result replaces it.
pub fn has(text: &str, language: &str) -> bool {
    LEN.load(Ordering::Relaxed) > 0 && store().read().unwrap_or_else(|e| e.into_inner()).map.get(text).is_some_and(|(_, l, _)| l == language)
}

/// Which stored result applies to `text` (0: none). Changes whenever a new result is stored for
/// it, so layout can key its cache on it.
pub fn stamp(text: &str) -> u64 {
    if LEN.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    store().read().unwrap_or_else(|e| e.into_inner()).map.get(text).map(|(s, _, _)| *s).unwrap_or(0)
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

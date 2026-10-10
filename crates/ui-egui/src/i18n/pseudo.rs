//! A test-only pseudo-language: every string that goes through the catalog lookup comes back
//! wrapped in `⟦…⟧`, so text drawn without the marks never went through translation.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    // Marked strings live for the test process; lookups hand out `&'static str`.
    static CACHE: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
}

pub fn on() -> bool {
    ON.get()
}

pub fn set(on: bool) {
    ON.set(on);
}

pub fn mark(s: &str) -> &'static str {
    CACHE.with_borrow_mut(|c| *c.entry(s.to_string()).or_insert_with(|| Box::leak(format!("⟦{}⟧", s.replace('\n', "⟧\n⟦")).into_boxed_str())))
}

/// `text` without its marked segments. Lines are marked one by one, since multi-line labels are
/// often drawn a line at a time.
pub fn unmarked(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '⟦' => depth += 1,
            '⟧' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

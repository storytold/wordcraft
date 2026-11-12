//! A test-only pseudo-language: while it is on (per thread), every string that goes through a
//! lookup comes back as the craft-i18n crate's pseudo locale marks it, wrapped in `⟦…⟧`, so text
//! drawn without the marks never went through translation.

use std::cell::Cell;
use std::collections::BTreeSet;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
}

/// Every English string looked up in the pseudo-language so far, in this process (the crate
/// keeps one list for every thread).
pub fn looked_up() -> BTreeSet<String> {
    super::I18N.looked_up()
}

pub fn on() -> bool {
    ON.get()
}

pub fn set(on: bool) {
    ON.set(on);
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

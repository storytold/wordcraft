//! Whether the command running on this thread was asked for by automation: an MCP client
//! (`wordcraft-cli mcp`, headless or bridged to the app with `--connect`), a program on the app's
//! control channel (the web app's `window.wordcraft` page API uses it too), or the commands of
//! `wordcraft-cli run` (`--cmd`, `--script`). The person at the keyboard is not automation.
//!
//! An agent can be steered by text in the very document it edits, so a few things the person may
//! do are refused from automation. Today that is attaching a template that isn't a file on this
//! computer (#6101): Word fetches the attached template when it opens the document, and a
//! template on another machine makes the recipient's computer connect there, which can leak
//! their Windows (NTLM) credentials.

use std::cell::Cell;

thread_local! {
    /// Whether this thread runs a command for automation.
    static AUTOMATION: Cell<bool> = const { Cell::new(false) };
}

/// Puts the previous state back when dropped (also when `f` panics).
struct Restore(bool);

impl Drop for Restore {
    fn drop(&mut self) {
        AUTOMATION.with(|a| a.set(self.0));
    }
}

/// Run `f` as automation: the commands it runs on this thread are automation's.
pub fn from_automation<T>(f: impl FnOnce() -> T) -> T {
    let _restore = Restore(AUTOMATION.with(|a| a.replace(true)));
    f()
}

/// Whether this thread runs a command for automation ([`from_automation`]).
pub fn active() -> bool {
    AUTOMATION.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scope_ends_with_the_call_and_nests() {
        assert!(!active());
        from_automation(|| {
            assert!(active());
            from_automation(|| assert!(active()));
            assert!(active(), "an inner scope keeps the outer one");
        });
        assert!(!active());
    }

    #[test]
    fn the_scope_ends_when_the_call_panics() {
        let r = std::panic::catch_unwind(|| from_automation(|| panic!("in the call")));
        assert!(r.is_err());
        assert!(!active());
    }
}

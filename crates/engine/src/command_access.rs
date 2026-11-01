//! The commands automation may run.
//!
//! `wordcraft-cli mcp --allow-commands <list>` keeps an agent to the commands it was given, the
//! way `--automation-read-root` / `--automation-write-root` ([`crate::file_access`]) keep it to
//! folders. While a [`CommandAllowlist`] is in force:
//!
//! - every command a client asks for (MCP `execute`, each command of `batch`, and the commands
//!   behind `new_document`, `open_document`, `save_document`, `type_text`, `select_text`,
//!   `get_text` and `list_documents`) must be on the list;
//! - so must every step a macro plays back (`tools.macros {"run"}`): a macro is a way to run
//!   commands, so it can't be a way around the list. A macro recorded or defined before the list
//!   was in force is held to it as well.
//!
//! Commands a command runs itself (Heading 1 applying its style, say) are part of the command
//! that was allowed and aren't checked again. Methods that only read the document without a
//! command (`inspect_document`, `render_page`, `parity`) stay available.
//!
//! A list names command ids (`format.bold`) and prefixes (`format.*`, every command whose id
//! starts with `format.`), separated by commas, spaces or new lines; `#` starts a comment. An id
//! the registry doesn't know, or a prefix that matches no command, is an error, so a typo can't
//! quietly leave the agent with less (or, after a rename, a different command) than intended.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use crate::Registry;

/// Longest list accepted (bytes), so a list read from a file can't exhaust memory.
pub const MAX_LIST_BYTES: usize = 256 * 1024;

/// The commands automation may run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandAllowlist {
    ids: BTreeSet<String>,
    prefixes: BTreeSet<String>,
}

impl CommandAllowlist {
    /// Parse `list` against the commands of `registry`. An empty list is an error: it would
    /// allow nothing, which is never what someone typing the flag meant.
    pub fn parse(list: &str, registry: &Registry) -> Result<Self, String> {
        if list.len() > MAX_LIST_BYTES {
            return Err(format!("command list too long: {} bytes (at most {MAX_LIST_BYTES})", list.len()));
        }
        let mut ids = BTreeSet::new();
        let mut prefixes = BTreeSet::new();
        for line in list.lines() {
            let line = line.split_once('#').map_or(line, |(before, _)| before);
            for item in line.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()) {
                if let Some(prefix) = item.strip_suffix(".*") {
                    if prefix.is_empty() || prefix.contains('*') {
                        return Err(format!("bad command prefix `{item}`: use `group.*`"));
                    }
                    let dotted = format!("{prefix}.");
                    if !registry.all().iter().any(|spec| spec.id.starts_with(&dotted)) {
                        return Err(format!("`{item}` matches no command"));
                    }
                    prefixes.insert(dotted);
                } else if item.contains('*') {
                    return Err(format!("bad command pattern `{item}`: only a trailing `.*` is allowed"));
                } else if registry.get(item).is_some() {
                    ids.insert(item.to_string());
                } else {
                    return Err(format!("unknown command `{item}` (wordcraft-cli commands lists them)"));
                }
            }
        }
        if ids.is_empty() && prefixes.is_empty() {
            return Err("the command list is empty".into());
        }
        Ok(Self { ids, prefixes })
    }

    /// Whether `id` is on the list.
    pub fn allows(&self, id: &str) -> bool {
        self.ids.contains(id) || self.prefixes.iter().any(|p| id.starts_with(p.as_str()))
    }

    /// The listed ids and prefixes (`group.*`), sorted, for logs and messages.
    pub fn describe(&self) -> Vec<String> {
        self.ids.iter().cloned().chain(self.prefixes.iter().map(|p| format!("{p}*"))).collect()
    }
}

thread_local! {
    /// This thread's list; `None`: the process's ([`confine_process`]).
    static SCOPE: RefCell<Option<Arc<CommandAllowlist>>> = const { RefCell::new(None) };
}

/// The list in force in every thread outside [`confine`].
static PROCESS: OnceLock<Arc<CommandAllowlist>> = OnceLock::new();

/// Put `list` in force in the whole process, for good. `false` when a list was in force already
/// (it stays as it was).
pub fn confine_process(list: Arc<CommandAllowlist>) -> bool {
    PROCESS.set(list).is_ok()
}

/// Puts the list it replaced back when dropped (also when `f` panics).
struct Restore(Option<Arc<CommandAllowlist>>);

impl Drop for Restore {
    /// Put the thread's previous list (or none) back in force.
    fn drop(&mut self) {
        let previous = self.0.take();
        SCOPE.with(|s| *s.borrow_mut() = previous);
    }
}

/// Run `f` with `list` in force on this thread (`None`: as things stand).
pub fn confine<T>(list: Option<&Arc<CommandAllowlist>>, f: impl FnOnce() -> T) -> T {
    match list {
        Some(l) => {
            let _restore = Restore(SCOPE.with(|s| s.replace(Some(l.clone()))));
            f()
        }
        None => f(),
    }
}

/// The list in force on this thread, if any.
pub fn current() -> Option<Arc<CommandAllowlist>> {
    SCOPE.with(|s| s.borrow().clone()).or_else(|| PROCESS.get().cloned())
}

/// May automation run `id` here? Always, without a list in force.
pub fn check(id: &str) -> Result<(), String> {
    match current() {
        Some(list) if !list.allows(id) => Err(format!("command `{id}` is not allowed: it is not on this session's --allow-commands list")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    /// A session with an empty document, for its command registry.
    fn session() -> Session {
        Session::new(wordcraft_doc::Document::new())
    }

    #[test]
    fn ids_prefixes_and_comments() {
        let s = session();
        let r = &s.registry;
        let l = CommandAllowlist::parse("text.insert, format.*\n# saving\nfile.save  # in place\n", r).unwrap();
        assert!(l.allows("text.insert") && l.allows("format.bold") && l.allows("file.save"));
        assert!(!l.allows("file.open") && !l.allows("formatx.bold") && !l.allows("tools.macros"));
        // `format.*` is the `format.` group, not every id starting with "format".
        assert!(!l.allows("format"));
        assert_eq!(l.describe(), vec!["file.save", "text.insert", "format.*"]);
    }

    #[test]
    fn typos_and_empty_lists_are_errors() {
        let s = session();
        let r = &s.registry;
        for bad in ["", "  # nothing\n", "format.bld", "nope.*", "*", ".*", "format*", "fo*rmat.bold", "format.**"] {
            assert!(CommandAllowlist::parse(bad, r).is_err(), "{bad:?}");
        }
        assert!(CommandAllowlist::parse(&"x".repeat(MAX_LIST_BYTES + 1), r).is_err());
    }

    #[test]
    fn a_thread_scope_wins_and_is_restored() {
        let s = session();
        let r = &s.registry;
        let l = Arc::new(CommandAllowlist::parse("text.insert", r).unwrap());
        assert!(check("file.open").is_ok(), "no list: everything");
        confine(Some(&l), || {
            assert!(check("text.insert").is_ok());
            let e = check("file.open").unwrap_err();
            assert!(e.contains("file.open") && e.contains("--allow-commands"), "{e}");
        });
        assert!(check("file.open").is_ok(), "the scope ends with confine");
    }
}

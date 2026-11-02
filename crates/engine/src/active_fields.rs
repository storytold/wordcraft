//! Field types another application acts on when it opens the document or updates its fields
//! (#3628): DDE and DDEAUTO start a program, INCLUDETEXT, INCLUDEPICTURE, LINK, IMPORT and
//! INCLUDE fetch their target, over the network for a UNC path. WordCraft never evaluates them,
//! but the people a document goes to open it in Word.
//!
//! An agent can be steered by text in the very document it edits, so a command run for
//! automation (MCP, the control channel: inside [`from_automation`]) can't put one in a document.
//! The person at the keyboard still can, and so can automation on a computer whose host sets
//! `WORDCRAFT_ALLOW_ACTIVE_FIELDS=1`: a setting of the host, never a command parameter, so a
//! prompt can't turn it on.

use std::cell::Cell;

use crate::CmdError;

/// The field types refused from automation, upper case.
pub const ACTIVE_FIELDS: &[&str] = &["DDE", "DDEAUTO", "INCLUDETEXT", "INCLUDEPICTURE", "LINK", "IMPORT", "INCLUDE"];

/// The environment variable a host sets to `1` to let automation insert them anyway.
pub const ALLOW_ENV: &str = "WORDCRAFT_ALLOW_ACTIVE_FIELDS";

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

/// Run `f` as automation: the commands it runs on this thread can't insert [`ACTIVE_FIELDS`].
pub fn from_automation<T>(f: impl FnOnce() -> T) -> T {
    let _restore = Restore(AUTOMATION.with(|a| a.replace(true)));
    f()
}

/// The first of `instr`'s field types, its own and those of the fields nested in it (`{ … }`),
/// that is in [`ACTIVE_FIELDS`].
pub fn active_field(instr: &str) -> Option<&'static str> {
    std::iter::once(instr).chain(instr.split('{').skip(1)).find_map(|code| {
        let name = code.trim_start().split(|c: char| c.is_whitespace() || c == '}' || c == '\\').next().unwrap_or("");
        ACTIVE_FIELDS.iter().copied().find(|f| f.eq_ignore_ascii_case(name))
    })
}

/// Refuse `instr` when it is or holds an active field and this thread runs a command for
/// automation, unless the host allows them.
pub fn check(instr: &str) -> Result<(), CmdError> {
    if !AUTOMATION.with(Cell::get) || std::env::var_os(ALLOW_ENV).is_some_and(|v| v == "1") {
        return Ok(());
    }
    match active_field(instr) {
        Some(name) => {
            Err(CmdError::Disabled(format!("{name} fields can't be inserted from automation (the host can allow them with {ALLOW_ENV}=1)")))
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Session;

    fn session() -> Session {
        Session::new(wordcraft_doc::Document::new())
    }

    fn codes(s: &Session) -> Vec<String> {
        crate::cmd::fields::story_fields(s, wordcraft_doc::StoryRef::Body).into_iter().map(|f| f.instr).collect()
    }

    #[test]
    fn active_fields_are_found_in_the_code_and_nested_fields() {
        assert_eq!(active_field(r#"DDEAUTO excel "C:\x.xls""#), Some("DDEAUTO"));
        assert_eq!(active_field(r#"includepicture "\\host\share\a.png" \d"#), Some("INCLUDEPICTURE"));
        assert_eq!(active_field(r#"IF { MERGEFIELD A } = "1" "{ INCLUDETEXT \\h\s\t.docx }" "no""#), Some("INCLUDETEXT"));
        assert_eq!(active_field("{LINK Excel.Sheet.8 x.xls}"), Some("LINK"));
        for passive in ["PAGE", r#"DATE \@ "d MMMM""#, "MERGEFIELD Linkage", r"REF Include_note \h", "DOCPROPERTY DDE"] {
            assert_eq!(active_field(passive), None, "{passive}");
        }
    }

    #[test]
    fn automation_cannot_insert_an_active_field() {
        // #3628: an agent steered by the document's text.
        let mut s = session();
        for instr in ACTIVE_FIELDS.iter().map(|f| format!(r#"{f} "\\attacker\share\x""#)) {
            let r = from_automation(|| s.run("insert.field", &json!({"instr": instr})));
            assert!(matches!(r, Err(CmdError::Disabled(_))), "{instr}: {r:?}");
        }
        let nested = r#"IF 1 = 1 "{ DDEAUTO cmd "/c calc" }" """#;
        assert!(matches!(from_automation(|| s.run("insert.field", &json!({"instr": nested}))), Err(CmdError::Disabled(_))));
        assert!(codes(&s).is_empty());
        // Passive fields still go in.
        from_automation(|| s.run("insert.field", &json!({"instr": "PAGE"}))).unwrap();
        assert_eq!(codes(&s), ["PAGE"]);
    }

    #[test]
    fn automation_cannot_rewrite_a_field_into_an_active_one() {
        let mut s = session();
        s.run("insert.field", &json!({"instr": "PAGE"})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        let r = from_automation(|| s.run("fields.setCode", &json!({"code": r#"INCLUDETEXT "\\host\s\t.docx""#})));
        assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
        assert_eq!(codes(&s), ["PAGE"]);
        // A merge rule's text can't smuggle one in either.
        let r = from_automation(|| s.run("mailings.rules", &json!({"rule": "IF", "field": "A", "value": "1", "then": "{ DDE cmd x }"})));
        assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
    }

    #[test]
    fn the_person_at_the_keyboard_can_insert_them() {
        let mut s = session();
        s.run("insert.field", &json!({"instr": "INCLUDEPICTURE \"pic.png\""})).unwrap();
        // Outside the automation scope again after it.
        assert!(from_automation(|| s.run("insert.field", &json!({"instr": "LINK x"}))).is_err());
        s.run("insert.field", &json!({"instr": "LINK x"})).unwrap();
        assert_eq!(codes(&s), ["INCLUDEPICTURE \"pic.png\"", "LINK x"]);
    }
}

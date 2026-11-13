use crate::doc_state::Selection;
#[cfg(test)]
use crate::session::*;
#[cfg(test)]
use wordcraft_doc::{Document, Pos};

/// The caret identity used for the process-monotonic IME state revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ImeCaretState {
    Body(Selection),
    Equation { session: u64, pos: wordcraft_doc::math_edit::MathPos },
}

/// One checked allocation from a process-monotonic IME revision counter.
fn allocate_ime_state_revision(counter: &std::sync::atomic::AtomicU64) -> Option<u64> {
    counter.try_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |n| n.checked_add(1)).ok()
}

static IME_STATE_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Allocates the next process-monotonic IME state revision, returning `None` when the counter is
/// exhausted.
pub(super) fn next_ime_state_revision() -> Option<u64> {
    allocate_ime_state_revision(&IME_STATE_REVISION)
}

#[cfg(test)]
mod ime_revision_tests {
    use super::*;
    use serde_json::json;

    /// Checked allocations stop at `u64::MAX` and report exhaustion instead of wrapping.
    #[test]
    fn checked_revision_allocation_refuses_to_wrap() {
        let counter = std::sync::atomic::AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_ime_state_revision(&counter), Some(u64::MAX - 1));
        assert_eq!(allocate_ime_state_revision(&counter), None, "exhaustion fails closed");
        assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), u64::MAX);
    }

    /// An unchanged caret reuses its revision, while a direct selection assignment is observed and
    /// advances the revision.
    #[test]
    fn selection_and_caret_motion_advance_the_unavailable_revision() {
        let mut s = Session::new(Document::from_text("hello"));
        let first = s.ime_state_revision().expect("a revision");
        assert_eq!(s.ime_state_revision(), Some(first), "an unchanged caret reuses its revision");
        s.sel = Selection::caret(Pos::body(0, 2));
        let moved = s.ime_state_revision().expect("a revision");
        assert!(moved > first, "a direct selection assignment is caught");
    }

    /// A selection-only command advances the revision on the next read, while a committed state
    /// change advances it immediately.
    #[test]
    fn successful_state_changes_allocate_but_selection_only_commands_do_not() {
        let mut s = Session::new(Document::from_text("hello"));
        let first = s.ime_state_revision().expect("a revision");
        s.run("select.all", &json!({})).unwrap();
        // `run` does not allocate for selection-only work; the next stamp sees the new caret.
        assert!(s.ime_state_revision() > Some(first));
        let selected = s.ime_state_revision().expect("a revision");
        s.run("para.alignCenter", &json!({})).unwrap();
        assert!(s.ime_state_revision() > Some(selected), "a committed state change advances");
    }

    /// After the revision allocator is exhausted, a real equation edit still reports its classified
    /// change but cannot mint a revision.
    #[test]
    fn exhausted_allocator_keeps_a_real_equation_commit_uncertifiable() {
        let mut s = Session::new(Document::new());
        s.run("insert.equation", &json!({})).unwrap();
        s.exhaust_ime_state_revision();
        assert_eq!(s.ime_state_revision(), None, "exhaustion fails closed");
        s.run_ime("equation.type", &json!({"text": "x"})).unwrap();
        let outcome = s.ime_edit_outcome();
        assert!(outcome.classified && outcome.content_changed && outcome.state_changed, "{outcome:?}");
        assert_eq!(s.ime_state_revision(), None, "a real producer command cannot mint a revision");
    }

    /// The application-change signal is consumed once; commands run through `run_ime` do not set it,
    /// while a normal command does.
    #[test]
    fn application_origin_signal_is_taken_once_and_ime_commands_do_not_set_it() {
        let mut s = Session::new(Document::from_text("hello"));
        s.run("para.alignCenter", &json!({})).unwrap();
        assert!(s.take_pending_ime_application_change(), "a normal command signals");
        assert!(!s.take_pending_ime_application_change(), "taking clears it");
        s.run_ime("text.insert", &json!({"text": "x"})).unwrap();
        assert!(!s.take_pending_ime_application_change(), "an IME transaction is not an application change");
        s.run("text.insert", &json!({"text": "y"})).unwrap();
        assert!(s.take_pending_ime_application_change());
    }

    /// Unknown and refused IME commands restore the normal origin without signalling an application
    /// change, and a nested command inside a macro inherits the IME origin.
    #[test]
    fn ime_origin_is_restored_and_nested_commands_inherit_it() {
        let mut s = Session::new(Document::from_text("hello"));
        s.run("tools.macros", &json!({"define": {"name": "centre", "steps": [{"command": "para.alignCenter", "params": {}}]}})).unwrap();
        s.take_pending_ime_application_change();

        // Unknown and refused commands restore the normal application origin; the failed run
        // itself leaves no application-change signal.
        assert!(s.run_ime("no.such.command", &json!({})).is_err());
        assert!(!s.take_pending_ime_application_change(), "an unknown IME command is not an application change");
        assert!(s.run_ime("equation.backspace", &json!({})).is_err());
        assert!(!s.take_pending_ime_application_change(), "a refused IME command is not an application change");

        // A nested `run` inherits the IME origin from `run_ime`; the same macro run normally is
        // an application change.
        s.run_ime("tools.macros", &json!({"run": "centre"})).unwrap();
        assert!(!s.take_pending_ime_application_change(), "the nested command inherits the IME origin");
        s.run("tools.macros", &json!({"run": "centre"})).unwrap();
        assert!(s.take_pending_ime_application_change(), "a normal macro run is application-origin");
    }
}

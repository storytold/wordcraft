//! Regressions where main's "a command that changes nothing is not an edit" (#23) meets the
//! migration's input-method application-change and state-revision tracking.
//!
//! Kept in its own module so the merged `tests.rs` is not touched.

use serde_json::json;

use crate::Session;

/// Creates an empty-document session for the IME no-op regressions.
fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

/// Runs a command in the test session, panicking with the command id on error.
fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// An *unclassified* no-op (`text.backspace` at the very start of the body) is not an edit and
/// must not look like one to the input method: no effect flags, no application-change signal, no
/// settled-revision move, and no history/dirty/revision side effects.
#[test]
fn a_no_op_command_is_not_an_ime_state_change() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "x"}));
    run(&mut s, "caret.docStart", json!({}));
    assert!(s.pending.is_none(), "caret movement dropped any pending formatting");
    s.take_pending_ime_application_change();
    let revision = s.ime_state_revision().expect("an IME revision");
    let rev = s.rev();
    let depth = s.undo_depth();
    let dirty = s.dirty;

    run(&mut s, "text.backspace", json!({}));

    let outcome = s.ime_edit_outcome();
    assert!(!outcome.classified, "the handler leaves this no-op unclassified: {outcome:?}");
    assert!(!outcome.content_changed && !outcome.state_changed, "{outcome:?}");
    assert!(!s.take_pending_ime_application_change(), "a no-op is not an application change");
    assert_eq!(s.ime_state_revision(), Some(revision), "the settled IME revision did not move");
    assert_eq!((s.undo_depth(), s.dirty, s.rev()), (depth, dirty, rev));
}

/// `document.setText` with identical text replaces the body under the same document id, so the
/// IME replacement epoch must advance — but because nothing observable changed it is not an edit
/// and must not signal an application-origin change (main's #23 no-op rule).
#[test]
fn a_same_text_set_text_advances_the_body_epoch_without_an_application_change() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Same body"}));
    s.take_pending_ime_application_change();
    let epoch = s.ime_body_epoch();
    let document_id = s.document_id();
    let rev = s.rev();
    let depth = s.undo_depth();

    run(&mut s, "document.setText", json!({"text": "Same body"}));

    assert!(s.ime_body_epoch() > epoch, "the body is replaced again under the same document id");
    assert_eq!(s.document_id(), document_id, "the document id itself is unchanged");
    assert!(!s.take_pending_ime_application_change(), "an identical setText is not an application change");
    assert_eq!((s.undo_depth(), s.rev()), (depth, rev));
}

/// The IME body replacement promise (`DeleteSurrounding`, then the commit that replaces the
/// erased text) joins only a `text.insert` at the selection the deletion left. Tab, an ordinary
/// command, consumes it: it is no IME replacement receipt itself, and the commit after it owns a
/// new Undo step instead of joining the deletion.
#[test]
fn tab_between_an_ime_replacement_and_its_commit_keeps_the_steps_apart() {
    let text = |s: &Session| s.doc.plain_text(wordcraft_doc::StoryRef::Body);

    // The promise itself joins the commit at the unchanged selection.
    let mut s = self::s();
    run(&mut s, "text.insert", json!({"text": "ab"}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(text(&s), "a", "DeleteSurrounding erased the b");
    s.join_next_ime_text_replacement();
    let depth = s.undo_depth();
    run(&mut s, "text.insert", json!({"text": "c"}));
    assert_eq!(text(&s), "ac");
    assert_eq!(s.undo_depth(), depth, "the commit joins the deletion's step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "ab", "one Undo restores the deletion and its commit");

    // Tab consumes the promise: the commit no longer joins the deletion.
    let mut s = self::s();
    run(&mut s, "text.insert", json!({"text": "ab"}));
    run(&mut s, "text.backspace", json!({}));
    s.join_next_ime_text_replacement();
    let depth = s.undo_depth();
    run(&mut s, "text.tab", json!({}));
    let outcome = s.ime_edit_outcome();
    assert!(!outcome.classified, "Tab is not an IME replacement receipt: {outcome:?}");
    assert_eq!(s.undo_depth(), depth + 1, "Tab owns its Undo step");
    run(&mut s, "text.insert", json!({"text": "c"}));
    assert_eq!(text(&s), "a\tc");
    assert_eq!(s.undo_depth(), depth + 2, "the commit after Tab starts its own Undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "a\t", "Undo takes back only the commit");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "a", "then the Tab");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "ab", "and the IME deletion on its own");

    // Returning directly to the promised selection must not resurrect a promise consumed by
    // Tab. Do not use a caret command here: that command would itself consume a leaked promise.
    let mut s = self::s();
    run(&mut s, "text.insert", json!({"text": "ab"}));
    run(&mut s, "text.backspace", json!({}));
    s.join_next_ime_text_replacement();
    let promised_selection = s.sel.clone();
    run(&mut s, "text.tab", json!({}));
    let depth = s.undo_depth();
    s.sel = promised_selection;
    run(&mut s, "text.insert", json!({"text": "c"}));
    assert_eq!(text(&s), "ac\t");
    assert_eq!(s.undo_depth(), depth + 1, "returning to the old selection cannot reuse the consumed promise");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "a\t", "only the new insert is undone");
}

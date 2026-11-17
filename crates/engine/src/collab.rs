//! The seam a collaboration layer plugs into.
//!
//! The engine knows that something may want to hear about a command it just ran. It deliberately
//! knows nothing about what that something is. A relay, a log file and a test double are all the
//! same shape from here.
//!
//! This is the only thing the engine gains for shared editing. Ordering, locking, transport and
//! credentials all live outside it (`apps/wordcraft/src/collab.rs`).
//!
//! One thing here is WordCraft's own. A spreadsheet command carries its own address, so `cell.set`
//! means the same thing wherever it lands. A word processor command means "here": Bold, Backspace
//! and typing all act on the caret. So a command is forwarded with the selection it ran against,
//! and [`Session::apply_remote`](crate::Session::apply_remote) runs it against that selection
//! rather than against the reader's own.

use serde_json::Value as Json;
use wordcraft_doc::{Pos, StoryRef};

use crate::Selection;

/// Receives commands that originated on this machine.
///
/// Called after a command has changed the document, so an implementation sees exactly the
/// `(id, params)` pairs that changed it. Commands that only move the caret, open a dialog or
/// write a file do not change it and never arrive here.
///
/// A command applied through [`Session::apply_remote`](crate::Session::apply_remote) is not
/// reported, which is what stops two peers echoing one edit back and forth forever.
pub trait CollabSink: Send + Sync {
    /// Forward one local command. Must not block: it runs inside the command.
    ///
    /// `at` is the selection the command ran against. Every WordCraft command needs it, because
    /// none of them says in its parameters where it acted.
    ///
    /// `scope` is that selection as a lock region ([`scope_of`]).
    fn on_local(&self, id: &str, params: &Json, at: &Selection, scope: &str);
}

/// Whether a command may leave this machine, or be applied if it arrives.
///
/// A mutating command is necessary but not enough. Undo, redo and repeat act on this peer's
/// own history. File commands are marked pure and would not be forwarded locally; they are
/// still refused on the way in, so a peer cannot open or save a path on this machine.
pub fn shareable(id: &str, mutates: bool) -> bool {
    mutates && !id.starts_with("file.") && !matches!(id, "edit.undo" | "edit.undoTo" | "edit.redo" | "edit.repeat")
}

/// The region a command acted on, as a lock scope.
///
/// The paragraph is the unit. Two people writing in different paragraphs are never about the same
/// region. A scope is opaque to the relay, which compares scopes for equality and nothing else,
/// so `/body/3` and `/part/2/0/1/4` are both just strings to it.
///
/// A path is positional, so an insert above a paragraph renumbers it. Both peers apply the same
/// ordered sequence of commands, so both spell the same paragraph the same way at the same point
/// in that sequence. A scope named before an edit elsewhere lands is the case this cannot name.
pub fn scope_of(at: &Pos) -> String {
    match at.story {
        StoryRef::Body => format!("/body/{}", at.path),
        StoryRef::Part(part) => format!("/part/{part}/{}", at.path),
    }
}

/// Where a position belongs after an edit of `grew` characters at `edit`.
///
/// The reader's caret stays where they put it. Text that appeared or went away earlier in the
/// same paragraph moved everything after it, so the offset moves with it. A positive `grew` is
/// text that appeared.
///
/// An edit that split or joined paragraphs cannot be followed this way. The offset is clamped
/// into the paragraph it names instead of being guessed at, which is what
/// [`Session::clamp_selection`](crate::Session) does for every position anyway.
pub(crate) fn shifted(mut at: Pos, edit: &Pos, grew: isize) -> Pos {
    if at.story != edit.story || at.path != edit.path || at.off < edit.off {
        return at;
    }
    // A caret inside removed text lands where the removal happened, never before it.
    at.off = at.off.saturating_add_signed(grew).max(edit.off);
    at
}

/// Both ends of a selection, moved by [`shifted`].
pub(crate) fn follow(sel: Selection, edit: &Pos, grew: isize) -> Selection {
    Selection { anchor: shifted(sel.anchor, edit, grew), focus: shifted(sel.focus, edit, grew) }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use wordcraft_doc::{Document, Pos, StoryRef};

    use super::*;
    use crate::Session;

    /// Everything a sink was told: the command, where it ran, and its scope.
    #[derive(Default)]
    struct Heard(Mutex<Vec<(String, Selection, String)>>);

    impl CollabSink for Heard {
        fn on_local(&self, id: &str, _params: &Json, at: &Selection, scope: &str) {
            self.0.lock().unwrap().push((id.to_owned(), at.clone(), scope.to_owned()));
        }
    }

    fn two_paragraphs() -> (Session, Arc<Heard>) {
        let mut session = Session::new(Document::from_text("First\nSecond"));
        let heard = Arc::new(Heard::default());
        session.collab = Some(heard.clone());
        (session, heard)
    }

    fn text(session: &Session) -> String {
        session.doc.plain_text(StoryRef::Body)
    }

    #[test]
    fn an_edit_is_reported_with_the_paragraph_it_happened_in() {
        let (mut session, heard) = two_paragraphs();
        session.sel = Selection::caret(Pos::body(1, 3));
        session.run("text.insert", &json!({"text": "X"})).unwrap();
        let heard = heard.0.lock().unwrap();
        assert_eq!(heard.len(), 1, "one edit, one report");
        assert_eq!(heard[0].0, "text.insert");
        assert_eq!(heard[0].1.focus, Pos::body(1, 3), "where the sender was, not where they ended up");
        assert_eq!(heard[0].2, "/body/1", "the paragraph is the lock region");
    }

    #[test]
    fn moving_the_caret_is_not_an_edit_and_is_never_reported() {
        let (mut session, heard) = two_paragraphs();
        session.run("caret.right", &json!({})).unwrap();
        session.run("select.all", &json!({})).unwrap();
        session.run("file.info", &json!({})).unwrap();
        assert!(heard.0.lock().unwrap().is_empty(), "nothing that leaves the document alone is shared");
    }

    #[test]
    fn a_remote_edit_lands_where_its_sender_was_and_is_not_sent_back() {
        let (mut session, heard) = two_paragraphs();
        // The reader is in the second paragraph. The sender was at the start of the first.
        session.sel = Selection::caret(Pos::body(1, 0));
        let there = Selection::caret(Pos::body(0, 0));
        session.apply_remote("text.insert", &json!({"text": "AB"}), Some(there)).unwrap();
        assert_eq!(text(&session), "ABFirst\nSecond", "their text went into their own paragraph");
        assert!(heard.0.lock().unwrap().is_empty(), "a remote edit is not echoed back to the peer that sent it");
        assert_eq!(session.sel.focus, Pos::body(1, 0), "an edit in another paragraph leaves the reader's caret alone");
    }

    #[test]
    fn the_readers_caret_moves_with_text_inserted_before_it() {
        let (mut session, _heard) = two_paragraphs();
        session.sel = Selection::caret(Pos::body(0, 3));
        let there = Selection::caret(Pos::body(0, 0));
        session.apply_remote("text.insert", &json!({"text": "AB"}), Some(there)).unwrap();
        assert_eq!(text(&session), "ABFirst\nSecond");
        assert_eq!(session.sel.focus, Pos::body(0, 5), "two characters appeared before the caret, so it moved two");
    }

    #[test]
    fn text_inserted_after_the_reader_leaves_their_caret_where_it_was() {
        let (mut session, _heard) = two_paragraphs();
        session.sel = Selection::caret(Pos::body(0, 1));
        let there = Selection::caret(Pos::body(0, 5));
        session.apply_remote("text.insert", &json!({"text": "AB"}), Some(there)).unwrap();
        assert_eq!(text(&session), "FirstAB\nSecond");
        assert_eq!(session.sel.focus, Pos::body(0, 1));
    }

    #[test]
    fn a_remote_edit_is_not_on_the_local_undo_stack() {
        let (mut session, _heard) = two_paragraphs();
        session.apply_remote("text.insert", &json!({"text": "AB"}), Some(Selection::caret(Pos::body(0, 0)))).unwrap();
        assert_eq!(text(&session), "ABFirst\nSecond");
        assert!(!session.can_undo(), "a peer's edit is not ours to undo");
        session.run("edit.undo", &json!({})).unwrap();
        assert_eq!(text(&session), "ABFirst\nSecond", "empty undo is a no-op");
    }

    #[test]
    fn a_local_edit_after_a_remote_one_is_still_ours_to_undo() {
        let (mut session, _heard) = two_paragraphs();
        session.apply_remote("text.insert", &json!({"text": "AB"}), Some(Selection::caret(Pos::body(0, 0)))).unwrap();
        session.sel = Selection::caret(Pos::body(0, 7));
        session.run("text.insert", &json!({"text": "Y"})).unwrap();
        assert_eq!(text(&session), "ABFirstY\nSecond");
        session.run("edit.undo", &json!({})).unwrap();
        assert_eq!(text(&session), "ABFirst\nSecond", "undo reverts the local typing, not the peer's insert");
    }

    #[test]
    fn a_remote_file_command_is_refused_and_does_not_run() {
        let (mut session, heard) = two_paragraphs();
        let err = session.apply_remote("file.new", &json!({}), None).unwrap_err().to_string();
        assert!(err.contains("not a shared edit"), "{err}");
        assert_eq!(text(&session), "First\nSecond");
        assert!(heard.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_local_undo_is_not_sent_to_peers() {
        let (mut session, heard) = two_paragraphs();
        session.run("text.insert", &json!({"text": "X"})).unwrap();
        session.run("edit.undo", &json!({})).unwrap();
        let heard = heard.0.lock().unwrap();
        assert_eq!(heard.len(), 1, "the insert is shared; the undo is not");
        assert_eq!(heard[0].0, "text.insert");
    }

    #[test]
    fn a_scope_names_the_story_and_the_paragraph() {
        assert_eq!(scope_of(&Pos::body(3, 0)), "/body/3");
        assert_eq!(scope_of(&Pos::new(StoryRef::Part(2), wordcraft_doc::Path(vec![1, 0, 1, 4]), 7)), "/part/2/1/0/1/4");
    }
}

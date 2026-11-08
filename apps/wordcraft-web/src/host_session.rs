//! Host-side collaboration routing.
//!
//! `host.rs` is browser-only: it owns the `craft_web_host` transport and turns
//! `wordcraft:session` / `wordcraft:committed` into [`HostEvent`]s. This module
//! keeps the state and the queue they land in free of `web_sys`, so the
//! handling is tested headlessly on the host.

use std::cell::RefCell;
use std::rc::Rc;

use wordcraft_collab_web::HostEvent;

pub use wordcraft_collab_web::{HostAction, HostSession, HostState};

/// Events the bridge's callback queued for the next frame.
pub type HostInbox = Rc<RefCell<Vec<HostEvent>>>;

/// Queue one host message. Called from the bridge's message handler, which
/// runs inside a browser event; the app applies it on the next frame.
pub fn deliver(inbox: &HostInbox, event: HostEvent) {
    inbox.borrow_mut().push(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `wordcraft:session` opens a session only when the bridge delivers it:
    /// nothing else in the app dials a socket.
    #[test]
    fn a_session_message_is_stored_and_asks_for_a_connection() {
        let inbox: HostInbox = HostInbox::default();
        let session = HostSession::new("session-7", "wss://collab.example/ws?room=7", "jwt", "editor");
        deliver(&inbox, HostEvent::Session(session.clone()));

        let mut state = HostState::default();
        let mut actions = Vec::new();
        for event in inbox.borrow_mut().drain(..) {
            actions.push(state.on_event(event));
        }
        assert_eq!(actions, vec![HostAction::Connect(session.clone())]);
        assert!(state.has_session());
        let stored = state.session.as_ref().expect("the session is kept for the app");
        assert_eq!(stored.session_id(), "session-7");
        assert_eq!(stored.ws_url(), "wss://collab.example/ws?room=7");
        assert_eq!(stored.role(), "editor");
        assert_eq!(stored.token(), "jwt");
        assert!(!format!("{stored:?}").contains("jwt"), "the token must not be printable");
    }

    /// A `wordcraft:committed` is a result: the app records the version and
    /// etag the cabinet stored and shows it, and opens nothing.
    #[test]
    fn a_committed_message_is_recorded_without_connecting() {
        let inbox: HostInbox = HostInbox::default();
        deliver(&inbox, HostEvent::Committed { version: 12, etag: "\"sha12\"".into() });
        let mut state = HostState::default();
        let mut actions = Vec::new();
        for event in inbox.borrow_mut().drain(..) {
            actions.push(state.on_event(event));
        }
        assert_eq!(actions, vec![HostAction::Committed { version: 12, etag: "\"sha12\"".into() }]);
        assert_eq!(state.committed, Some((12, "\"sha12\"".into())));
        assert!(!state.has_session());
    }

    /// Single-user mode: no host message, no session, no action.
    #[test]
    fn without_host_messages_the_app_stays_single_user() {
        let inbox: HostInbox = HostInbox::default();
        assert!(inbox.borrow().is_empty());
        let state = HostState::default();
        assert!(!state.has_session());
        assert!(state.committed.is_none());
    }
}

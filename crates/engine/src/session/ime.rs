use super::ime_state::{ImeCaretState, next_ime_state_revision};
use super::{CmdError, ImeEditOutcome, Session, Value};

impl Session {
    /// The current command's explicit IME effect classification.
    pub fn ime_edit_outcome(&self) -> ImeEditOutcome {
        self.ime_edit_outcome
    }

    /// Set the effect classification from a command handler. [`Session::run`] fills
    /// `selection_changed` and enforces the content/state invariant after the command.
    pub(crate) fn set_ime_edit_outcome(&mut self, outcome: ImeEditOutcome) {
        self.ime_edit_outcome = outcome;
    }

    /// Take the pending application-origin change signal. The UI owns one session, so a simple
    /// bool is enough; it deliberately survives later commands until drained.
    pub fn take_pending_ime_application_change(&mut self) -> bool {
        std::mem::take(&mut self.pending_ime_application_change)
    }

    /// Simulate a globally exhausted IME revision allocator in this session (test support).
    ///
    /// This only ever makes receipt classification fail closed. It exists so front-end producer
    /// tests can exercise the real `ime_stamp` path instead of hand-building an `Unavailable`
    /// stamp; production builds do not compile it.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn exhaust_ime_state_revision(&mut self) {
        self.ime_state_exhausted = true;
        self.ime_state_revision = None;
    }

    /// The checked, process-monotonic revision for IME receipts with unavailable surrounding
    /// text. Selection/caret motion allocates on the next call; `None` means exhausted.
    pub fn ime_state_revision(&mut self) -> Option<u64> {
        self.sync_ime_caret_state();
        self.ime_state_revision
    }

    /// Returns the caret identity used for IME state revisions: the equation session and position
    /// while editing, otherwise the body selection.
    pub(super) fn current_ime_caret_state(&self) -> ImeCaretState {
        match &self.math {
            Some(m) => ImeCaretState::Equation { session: m.session, pos: m.pos.clone() },
            None => ImeCaretState::Body(self.sel.clone()),
        }
    }

    /// Allocate the next IME state revision, or store `None` when the allocator is exhausted.
    /// Test support can pin exhaustion for this session so a front-end producer test sees the
    /// real `ime_stamp`/receipt path fail closed.
    pub(super) fn reset_ime_state_revision(&mut self) {
        #[cfg(any(test, feature = "test-support"))]
        if self.ime_state_exhausted {
            self.ime_state_revision = None;
            return;
        }
        // v5: allocation failure must store `None`, never retain a stale revision.
        self.ime_state_revision = next_ime_state_revision();
    }

    /// Allocates a new revision only when the body selection or the equation session and position
    /// differs from the last observed state.
    fn sync_ime_caret_state(&mut self) {
        let state = self.current_ime_caret_state();
        if self.ime_state_seen.as_ref() != Some(&state) {
            self.ime_state_seen = Some(state);
            self.reset_ime_state_revision();
        }
    }

    /// Run one command as the input method's own transaction.
    ///
    /// Commands run inside this scope are IME-origin: a successful command run *outside* this
    /// scope sets the pending application-change signal, but nothing inside it does. Nested commands inherit
    /// the origin. The previous origin is restored on every return, including unknown, disabled or
    /// refused commands.
    pub fn run_ime(&mut self, id: &str, params: &Value) -> Result<Value, CmdError> {
        let previous = std::mem::replace(&mut self.ime_origin, true);
        let result = self.run(id, params);
        self.ime_origin = previous;
        result
    }
}

//! Saves that finish after the save command has returned (#1315).
//!
//! In the browser, Save hands the bytes to the save picker; the user picks a place (or cancels)
//! and the file is written and closed later, long after `download` returned. The host starts a
//! ticket for such a save ([`AsyncSaves::start`]) while `download` runs and reports how it ended
//! ([`AsyncSaves::finish`]); the app reads the outcomes each frame. A "Save changes?" prompt
//! whose Save went to the picker holds its New/Open back until its ticket ends: it runs on
//! success, and on a cancel or failure the document stays open, still unsaved, and the prompt
//! comes back with the reason.
//!
//! Plain `Rc<Cell<…>>` state: no locks, no borrow that could fail.

use std::cell::Cell;
use std::rc::Rc;

/// How a save that finished later ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The file is written and closed.
    Saved,
    /// The user cancelled the picker; nothing was written.
    Cancelled,
    /// The picker, the write or the close failed (the reason).
    Failed(String),
}

#[derive(Default)]
struct State {
    next: u64,
    /// The ticket the current `download` call started, not yet claimed by the app.
    started: Option<u64>,
    /// Tickets started and not finished.
    pending: Vec<u64>,
    /// Tickets finished and not yet read by the app.
    finished: Vec<(u64, SaveOutcome)>,
}

/// Saves in flight, shared between the host's `download` hook and the app (a cheap clone).
#[derive(Clone, Default)]
pub struct AsyncSaves(Rc<Cell<State>>);

impl AsyncSaves {
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.0.take();
        let r = f(&mut state);
        self.0.set(state);
        r
    }

    /// The host's `download` hook: the save it is starting finishes later. Returns its ticket,
    /// to pass to [`Self::finish`].
    pub fn start(&self) -> u64 {
        self.with(|s| {
            s.next = s.next.wrapping_add(1);
            let ticket = s.next;
            s.started = Some(ticket);
            s.pending.push(ticket);
            ticket
        })
    }

    /// The save `ticket` ended. An unknown or already finished ticket is ignored.
    pub fn finish(&self, ticket: u64, outcome: SaveOutcome) {
        self.with(|s| {
            if let Some(i) = s.pending.iter().position(|&t| t == ticket) {
                s.pending.swap_remove(i);
                s.finished.push((ticket, outcome));
            }
        });
    }

    /// Saves started and not finished (the browser's leave-page guard warns while there are any).
    pub fn pending(&self) -> usize {
        self.with(|s| s.pending.len())
    }

    /// The ticket the last `download` call started, if it started one (claimed once).
    pub(crate) fn take_started(&self) -> Option<u64> {
        self.with(|s| s.started.take())
    }

    /// The saves that ended since the last call, oldest first.
    pub(crate) fn take_finished(&self) -> Vec<(u64, SaveOutcome)> {
        self.with(|s| std::mem::take(&mut s.finished))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tickets_start_finish_and_are_read_once() {
        let saves = AsyncSaves::default();
        let a = saves.start();
        let b = saves.start();
        assert_ne!(a, b);
        assert_eq!(saves.take_started(), Some(b));
        assert_eq!(saves.take_started(), None);
        assert_eq!(saves.pending(), 2);
        saves.finish(a, SaveOutcome::Cancelled);
        saves.finish(a, SaveOutcome::Saved);
        saves.finish(999, SaveOutcome::Saved);
        assert_eq!(saves.pending(), 1);
        assert_eq!(saves.take_finished(), vec![(a, SaveOutcome::Cancelled)]);
        assert!(saves.take_finished().is_empty());
    }
}

//! Saving the preferences soon after they change (#5740).
//!
//! `ui.json` used to be written only when the app quit cleanly (`on_exit`). A document opened
//! from the file manager starts another WordCraft, which reads `ui.json` as it stands, and a
//! window that crashes or is killed never quits cleanly. Either way a setting changed in the
//! running app (Check spelling as you type, say) came back as it was before. The app now writes
//! the preferences a moment after they change, and still on exit. When several WordCraft
//! processes run, the last one to write wins, as it did on exit.

use std::time::{Duration, Instant};

/// How long after a check the next one is due.
pub const INTERVAL: Duration = Duration::from_millis(750);

/// When the preferences are next compared with what was last saved, and what that was.
pub struct PrefsSaver {
    /// The preferences as last read or written, serialized as they are saved.
    last: Vec<u8>,
    /// No check before this.
    next: Instant,
}

impl PrefsSaver {
    /// Start from `current`, the preferences as they stand once the app has read `ui.json`:
    /// nothing is written until they change.
    pub fn new(current: Vec<u8>, now: Instant) -> Self {
        Self { last: current, next: later(now) }
    }

    /// Run once a frame. When a check is due, `current` gives the preferences as they would be
    /// saved now; if they differ from the last saved ones, `write` saves them. Before that,
    /// returns how long until the check is due: the caller wakes up then, so a change made
    /// by this frame is saved even if nothing else happens.
    pub fn tick(&mut self, now: Instant, current: impl FnOnce() -> Option<Vec<u8>>, write: impl FnOnce(&[u8])) -> Option<Duration> {
        if now < self.next {
            return Some(self.next.saturating_duration_since(now));
        }
        self.next = later(now);
        if let Some(bytes) = current()
            && bytes != self.last
        {
            write(&bytes);
            // A failed write (logged by `write`) is not retried until the preferences change
            // again, so a read-only settings folder doesn't fill the log.
            self.last = bytes;
        }
        None
    }
}

/// When the check after one at `now` is due.
fn later(now: Instant) -> Instant {
    now.checked_add(INTERVAL).unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_change_is_written_once_a_check_is_due() {
        let t0 = Instant::now();
        let mut saver = PrefsSaver::new(b"spelling on".to_vec(), t0);
        let mut written: Vec<Vec<u8>> = Vec::new();

        // Not due yet: nothing is read or written, and the caller is told when to come back.
        let wait = saver.tick(t0, || panic!("not read before the check is due"), |_| panic!("not written"));
        assert_eq!(wait, Some(INTERVAL));

        // Due, unchanged: nothing is written.
        let t1 = t0 + INTERVAL;
        assert_eq!(saver.tick(t1, || Some(b"spelling on".to_vec()), |b| written.push(b.to_vec())), None);
        assert!(written.is_empty());

        // Changed: written once at the next check, not again while it stays the same.
        let t2 = t1 + INTERVAL / 2;
        assert!(saver.tick(t2, || Some(b"spelling off".to_vec()), |b| written.push(b.to_vec())).is_some(), "the next check is not due yet");
        let t3 = t1 + INTERVAL;
        assert_eq!(saver.tick(t3, || Some(b"spelling off".to_vec()), |b| written.push(b.to_vec())), None);
        let t4 = t3 + INTERVAL;
        assert_eq!(saver.tick(t4, || Some(b"spelling off".to_vec()), |b| written.push(b.to_vec())), None);
        assert_eq!(written, [b"spelling off".to_vec()]);

        // Preferences that can't be serialized are skipped, not written empty.
        let t5 = t4 + INTERVAL;
        assert_eq!(saver.tick(t5, || None, |b| written.push(b.to_vec())), None);
        assert_eq!(written.len(), 1);
    }
}

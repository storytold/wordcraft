//! Saving the preferences soon after they change (#5740).
//!
//! `ui.json` used to be written only when the app quit cleanly (`on_exit`). A document opened
//! from the file manager starts another WordCraft, which reads `ui.json` as it stands, and a
//! window that crashes or is killed never quits cleanly. Either way a setting changed in the
//! running app came back as it was before. The app now writes the preferences a moment after
//! they change, and on exit only if they changed since.
//!
//! Several WordCraft processes can run at once, each with its own copy of the preferences. A
//! save writes only the settings this process changed: each top-level setting it left alone is
//! kept as the file has it, so one process doesn't put back what another changed meanwhile. When
//! two change the same setting, the last to save wins. A process doesn't take in what another
//! saved until it starts again.

use serde_json::{Map, Value};
use std::time::{Duration, Instant};

/// How long after a check the next one is due.
pub const INTERVAL: Duration = Duration::from_millis(750);

/// How long after a failed write it is tried again (at the next frame after that).
pub const RETRY: Duration = Duration::from_secs(30);

/// When the preferences are next compared with what this process last saved, and what that was.
pub struct PrefsSaver {
    /// This process's preferences as last read or saved: what a save is compared with, to know
    /// which settings it changed.
    base: Value,
    /// No check before this.
    next: Instant,
    /// The last write failed (and was logged): it is tried again, without logging again.
    failing: bool,
}

/// Where a save's preferences come from and go to.
pub trait Store {
    /// This process's preferences now, as `ui.json` holds them (`None`: they can't be serialized).
    fn current(&mut self) -> Option<Value>;
    /// `ui.json` as it is now (`None`: missing or unreadable).
    fn disk(&mut self) -> Option<Value>;
    /// Write `bytes` to `ui.json`.
    fn write(&mut self, bytes: &[u8]) -> Result<(), String>;
}

impl PrefsSaver {
    /// Start from `current`, the preferences as they stand once the app has read `ui.json`:
    /// nothing is written until they change.
    pub fn new(current: Value, now: Instant) -> Self {
        Self { base: current, next: later(now, INTERVAL), failing: false }
    }

    /// Run at the end of each frame, after anything in it could change a setting. When a check
    /// is due and the preferences changed, the changed settings are saved. Before that, returns
    /// how long until the check is due: the caller wakes up then, so a change made by this frame
    /// is saved even if nothing else happens. An idle window that changed nothing isn't woken.
    pub fn tick(&mut self, now: Instant, store: &mut impl Store) -> Option<Duration> {
        if now < self.next {
            return Some(self.next.saturating_duration_since(now));
        }
        self.next = later(now, INTERVAL);
        let current = store.current()?;
        if current != self.base {
            self.save(now, current, store);
        }
        None
    }

    /// On exit: save what changed since the last save. Nothing changed, nothing is written (so
    /// another process's later save stands), unless there is no readable `ui.json` yet.
    pub fn flush(&mut self, store: &mut impl Store) {
        let Some(current) = store.current() else { return };
        if current != self.base || store.disk().is_none() {
            self.save(Instant::now(), current, store);
        }
    }

    fn save(&mut self, now: Instant, current: Value, store: &mut impl Store) {
        let merged = merge(&self.base, &current, store.disk());
        let written = serde_json::to_vec_pretty(&merged).map_err(|e| format!("cannot serialize: {e}")).and_then(|bytes| store.write(&bytes));
        match written {
            Ok(()) => {
                self.base = current;
                self.failing = false;
            }
            // Tried again after a while, with the change still to save; logged once, so a
            // read-only settings folder doesn't fill the log.
            Err(e) => {
                if !self.failing {
                    log::warn!("preferences: {e}");
                }
                self.failing = true;
                self.next = later(now, RETRY);
            }
        }
    }
}

/// The preferences to write: `disk`'s, with each top-level setting `current` changed since `base`
/// (or that `disk` lacks) taken from `current`. Without a readable `disk` (or with `current` not
/// an object), `current`. Settings only `disk` has stay; the app's [`Store`] gives the file as
/// this version reads it, so those are ones this version writes only when they aren't empty.
fn merge(base: &Value, current: &Value, disk: Option<Value>) -> Value {
    let (Some(mine), Some(Value::Object(mut out))) = (current.as_object(), disk) else { return current.clone() };
    let before = base.as_object().cloned().unwrap_or_else(Map::new);
    for (key, value) in mine {
        if before.get(key) != Some(value) || !out.contains_key(key) {
            out.insert(key.clone(), value.clone());
        }
    }
    Value::Object(out)
}

/// The instant `after` past `now`.
fn later(now: Instant, after: Duration) -> Instant {
    now.checked_add(after).unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A `ui.json` in memory, and this process's preferences.
    #[derive(Default)]
    struct Memory {
        prefs: Value,
        file: Option<Value>,
        writes: usize,
        fail: bool,
        reads: usize,
    }

    impl Store for Memory {
        fn current(&mut self) -> Option<Value> {
            self.reads += 1;
            Some(self.prefs.clone()).filter(|v| !v.is_null())
        }
        fn disk(&mut self) -> Option<Value> {
            self.file.clone()
        }
        fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
            if self.fail {
                return Err("read-only".into());
            }
            self.writes += 1;
            self.file = Some(serde_json::from_slice(bytes).map_err(|e| e.to_string())?);
            Ok(())
        }
    }

    #[test]
    fn only_a_change_is_written_once_a_check_is_due() {
        let t0 = Instant::now();
        let start = json!({"proofing": true, "theme": "light"});
        let mut m = Memory { prefs: start.clone(), file: Some(start.clone()), ..Default::default() };
        let mut saver = PrefsSaver::new(start, t0);

        // Not due yet: nothing is read or written, and the caller is told when to come back.
        assert_eq!(saver.tick(t0, &mut m), Some(INTERVAL));
        assert_eq!((m.reads, m.writes), (0, 0));

        // Due, unchanged: nothing is written.
        let t1 = t0 + INTERVAL;
        assert_eq!(saver.tick(t1, &mut m), None);
        assert_eq!(m.writes, 0);

        // Changed: written once at the next check, not again while it stays the same.
        m.prefs["proofing"] = json!(false);
        let t2 = t1 + INTERVAL / 2;
        assert!(saver.tick(t2, &mut m).is_some(), "the next check is not due yet");
        let t3 = t1 + INTERVAL;
        assert_eq!(saver.tick(t3, &mut m), None);
        assert_eq!(saver.tick(t3 + INTERVAL, &mut m), None);
        assert_eq!(m.writes, 1);
        assert_eq!(m.file, Some(json!({"proofing": false, "theme": "light"})));

        // Preferences that can't be serialized are skipped, not written empty.
        m.prefs = Value::Null;
        assert_eq!(saver.tick(t3 + INTERVAL * 2, &mut m), None);
        assert_eq!(m.writes, 1);
    }

    /// #5740 review: two WordCraft processes. B started before A changed a setting; B's own
    /// change saves only what B changed, so A's change stays in the file, now and when B quits.
    #[test]
    fn a_save_keeps_what_another_process_saved() {
        let t0 = Instant::now();
        let start = json!({"proofing": true, "theme": "light", "recent": ["a.docx"]});
        let mut b = Memory { prefs: start.clone(), file: Some(start.clone()), ..Default::default() };
        let mut saver = PrefsSaver::new(start, t0);
        // A (another process) turns spelling off and opens a file; the file says so.
        b.file = Some(json!({"proofing": false, "theme": "light", "recent": ["new.docx", "a.docx"], "onlyInTheFile": 1}));
        // B changes the theme.
        b.prefs["theme"] = json!("dark");
        assert_eq!(saver.tick(t0 + INTERVAL, &mut b), None);
        assert_eq!(b.file, Some(json!({"proofing": false, "theme": "dark", "recent": ["new.docx", "a.docx"], "onlyInTheFile": 1})));
        // B quits having changed nothing more: nothing is written, A's later change stands.
        b.file.as_mut().unwrap()["proofing"] = json!(true);
        let writes = b.writes;
        saver.flush(&mut b);
        assert_eq!(b.writes, writes);
        assert_eq!(b.file.as_ref().unwrap()["proofing"], true);
        // With no ui.json at all, quitting writes one.
        b.file = None;
        saver.flush(&mut b);
        assert_eq!(b.file.as_ref().unwrap()["theme"], "dark");
        // A setting B changes again is B's, whatever the file says.
        b.prefs["proofing"] = json!(false);
        saver.flush(&mut b);
        assert_eq!(b.file.as_ref().unwrap()["proofing"], false);
    }

    /// #5740 review: a failed write is logged once and tried again after a while, with the
    /// change still to save, instead of being given up until the next change.
    #[test]
    fn a_failed_write_is_tried_again() {
        let t0 = Instant::now();
        let start = json!({"proofing": true});
        let mut m = Memory { prefs: start.clone(), file: Some(start.clone()), fail: true, ..Default::default() };
        let mut saver = PrefsSaver::new(start, t0);
        m.prefs["proofing"] = json!(false);
        let t1 = t0 + INTERVAL;
        assert_eq!(saver.tick(t1, &mut m), None);
        assert_eq!(m.writes, 0);
        assert!(saver.tick(t1 + INTERVAL, &mut m).is_some(), "not before the retry is due");
        m.fail = false;
        assert_eq!(saver.tick(t1 + RETRY, &mut m), None);
        assert_eq!(m.file, Some(json!({"proofing": false})), "saved on the retry");
    }

    #[test]
    fn merging_keeps_unknown_and_unchanged_settings() {
        let base = json!({"a": 1, "b": 2});
        assert_eq!(merge(&base, &json!({"a": 1, "b": 3}), Some(json!({"a": 9, "b": 2, "z": 0}))), json!({"a": 9, "b": 3, "z": 0}));
        // A setting the file lacks is written; an unreadable file is replaced.
        assert_eq!(merge(&base, &json!({"a": 1, "b": 2, "c": 4}), Some(json!({"a": 1}))), json!({"a": 1, "b": 2, "c": 4}));
        assert_eq!(merge(&base, &json!({"a": 5}), None), json!({"a": 5}));
        assert_eq!(merge(&base, &json!({"a": 5}), Some(json!([1]))), json!({"a": 5}));
    }
}

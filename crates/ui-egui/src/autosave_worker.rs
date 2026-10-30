//! AutoSave that writes off the UI thread. The UI thread stamps and clones the document (the
//! recovery worker's pattern) and hands the copy here; a worker thread serializes and writes
//! it, so typing never waits for the disk. Only one write runs at a time — while one is in
//! flight the app skips AutoSave ticks and tries again when its outcome is read
//! ([`AutoSaveWorker::poll`]). Explicit saves and document close wait for the pending write
//! before continuing: atomic replacement alone cannot prevent an older write landing last.

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::path::PathBuf;
    use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
    use std::time::Duration;

    use wordcraft_engine::cmd::file::{PreparedSave, write_prepared};

    /// How a background AutoSave ended: the file written, or the path and the reason it wasn't.
    pub type Outcome = (PathBuf, Result<(), String>);

    /// A background AutoSave writer, driven like the recovery worker: hand it a job, read the
    /// outcome later. When the worker is gone (spawn failed, or the app is shutting down),
    /// `submit` returns the rejected job so AutoSave can write it inline.
    pub struct AutoSaveWorker {
        tx: std::sync::mpsc::Sender<PreparedSave>,
        events: Receiver<Outcome>,
        pending: Option<PathBuf>,
    }

    impl AutoSaveWorker {
        pub fn spawn() -> Self {
            Self::spawn_with(write_prepared)
        }

        fn spawn_with(write: impl Fn(&PreparedSave) -> Result<(), String> + Send + 'static) -> Self {
            let (tx, jobs) = std::sync::mpsc::channel::<PreparedSave>();
            let (events, rx) = std::sync::mpsc::channel::<Outcome>();
            // On a failed spawn the closure (and with it the job receiver) is dropped, so
            // `submit`'s sends fail and the app falls back to writing inline.
            let _ = std::thread::Builder::new().name("wordcraft-autosave".into()).spawn(move || {
                while let Ok(save) = jobs.recv() {
                    let outcome = write(&save);
                    if events.send((save.path, outcome)).is_err() {
                        return;
                    }
                }
            });
            Self { tx, events: rx, pending: None }
        }

        /// Whether a write is in flight; AutoSave waits for its outcome before the next one.
        pub fn is_busy(&self) -> bool {
            self.pending.is_some()
        }

        /// Hand a prepared save to the worker, returning it if the worker is gone. Only call
        /// this when [`Self::is_busy`] is false.
        pub fn submit(&mut self, save: PreparedSave) -> Option<PreparedSave> {
            let path = save.path.clone();
            match self.tx.send(save) {
                Ok(()) => {
                    self.pending = Some(path);
                    None
                }
                Err(e) => Some(e.0),
            }
        }

        /// The outcome of a finished write, if one has ended. Reading it re-arms the worker.
        pub fn poll(&mut self) -> Option<Outcome> {
            match self.events.try_recv() {
                Ok(outcome) => {
                    self.pending = None;
                    Some(outcome)
                }
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => self.disconnected(),
            }
        }

        /// Wait (bounded) for the write in flight, returning its outcome; used when the window
        /// closes. A timeout keeps the write busy, so callers can refuse to save or close
        /// while it could still overwrite the file. `None` if nothing finished in time.
        pub fn settle(&mut self, timeout: Duration) -> Option<Outcome> {
            if !self.is_busy() {
                return None;
            }
            let timeout = timeout.max(Duration::from_millis(1));
            match self.events.recv_timeout(timeout) {
                Ok(outcome) => {
                    self.pending = None;
                    Some(outcome)
                }
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => self.disconnected(),
            }
        }

        fn disconnected(&mut self) -> Option<Outcome> {
            self.pending.take().map(|path| (path, Err("AutoSave worker stopped before confirming the save".into())))
        }

        #[cfg(test)]
        pub(crate) fn dead() -> Self {
            let (tx, jobs) = std::sync::mpsc::channel();
            let (events, rx) = std::sync::mpsc::channel();
            drop(jobs);
            drop(events);
            Self { tx, events: rx, pending: None }
        }

        #[cfg(test)]
        pub(crate) fn controlled(write: impl Fn(&PreparedSave) -> Result<(), String> + Send + 'static) -> Self {
            Self::spawn_with(write)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn disconnected_pending_worker_reports_failure_once() {
            for settle in [false, true] {
                let mut worker = AutoSaveWorker::dead();
                let path = PathBuf::from("pending.docx");
                worker.pending = Some(path.clone());
                let (failed_path, result) = if settle { worker.settle(Duration::from_millis(1)) } else { worker.poll() }.unwrap();
                assert_eq!(failed_path, path);
                assert!(result.is_err());
                assert!(!worker.is_busy());
                assert!(worker.poll().is_none());
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use imp::{AutoSaveWorker, Outcome};

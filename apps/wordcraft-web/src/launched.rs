//! Documents the system launched the installed web app with (`web::launch` reads them): opened
//! one at a time, and reported in one status line once the last has opened, because each open
//! clears the status bar and would hide a failure shown earlier.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use wordcraft_ui_egui::{WordApp, i18n};

/// Largest launched file read into memory; wasm32 has 4 GB at most, shared with the app.
const MAX_LAUNCHED_BYTES: f64 = (256u32 << 20) as f64;

/// Whether a launched file's size is within the limit, checked before the bytes are read.
/// The browser reports the size as a float; a value that isn't finite and nonnegative is refused.
pub(crate) fn size_within_limit(size: f64) -> bool {
    size.is_finite() && (0.0..=MAX_LAUNCHED_BYTES).contains(&size)
}

/// Most launched files waiting to be read, over every launch; the operating system can pass a
/// whole folder's selection.
pub(crate) const MAX_WAITING_FILES: usize = 20;

/// Launched files not read yet, whether a reader is going through them, and how many files the
/// launches since the last report left out. Generic so tests need no browser file handles.
pub(crate) struct Launches<T> {
    waiting: VecDeque<T>,
    reading: bool,
    omitted: usize,
}

impl<T> Default for Launches<T> {
    fn default() -> Self {
        Self { waiting: VecDeque::new(), reading: false, omitted: 0 }
    }
}

/// Take a launch's files into the queue: at most [`MAX_WAITING_FILES`] wait, in the order they
/// arrive, over every launch. Returns whether a new reader has to start; while one is running, a
/// launch only queues behind it, so no second reader can interleave reads.
pub(crate) fn admit<T>(l: &mut Launches<T>, handles: impl IntoIterator<Item = T>) -> bool {
    for handle in handles {
        if l.waiting.len() < MAX_WAITING_FILES {
            l.waiting.push_back(handle);
        } else {
            l.omitted = l.omitted.saturating_add(1);
        }
    }
    let start = !l.reading && (!l.waiting.is_empty() || l.omitted > 0);
    l.reading |= start;
    start
}

/// The reader's next step. Nothing is read while a file waits to be opened, so one launched
/// file's bytes at most are held at a time; the file after a failed read still gets its turn.
pub(crate) fn next_step<T>(reading: &mut Launches<T>, drained: bool) -> Step<T> {
    if !drained {
        return Step::Wait;
    }
    match reading.waiting.pop_front() {
        Some(handle) => Step::Read(handle),
        None => {
            reading.reading = false;
            Step::Finish(std::mem::take(&mut reading.omitted))
        }
    }
}

pub(crate) enum Step<T> {
    Wait,
    Read(T),
    Finish(usize),
}

/// Most failed files one report names on the status bar; it counts the rest.
const MAX_NAMED_FAILURES: usize = 3;

/// What the reader hands the app, shared between `web::launch` and the frame that opens files.
pub(crate) type Shared = Rc<RefCell<Launched>>;

/// Launched files read and waiting to open, and the outcome of the launches since the last report.
#[derive(Default)]
pub(crate) struct Launched {
    ready: VecDeque<(String, Vec<u8>)>,
    failures: Failures,
    omitted: usize,
    limit: usize,
    /// The reader has queued every file and counted the rest: report once `ready` is empty.
    finished: bool,
}

impl Launched {
    /// A reader starts on new launches: no report until it finishes too.
    pub(crate) fn start(&mut self) {
        self.finished = false;
    }

    /// A file read and ready to open.
    pub(crate) fn push(&mut self, name: String, bytes: Vec<u8>) {
        self.ready.push_back((name, bytes));
    }

    /// Whether every file read so far has been opened, so the next may be read.
    pub(crate) fn is_drained(&self) -> bool {
        self.ready.is_empty()
    }

    /// A file that couldn't be read or opened.
    pub(crate) fn failed(&mut self, name: String, error: String) {
        self.failures.add(name, error);
    }

    /// The reader is done; `omitted` files were left out past `limit` waiting files.
    pub(crate) fn finish(&mut self, omitted: usize, limit: usize) {
        self.omitted = self.omitted.saturating_add(omitted);
        self.limit = limit;
        self.finished = true;
    }

    /// The status line for the files that failed and those left out, or `None` when all opened.
    fn report(self) -> Option<String> {
        let omission = (self.omitted > 0).then(|| {
            i18n::fmt(
                i18n::t("{count} more files weren't opened: WordCraft opens up to {limit} at a time."),
                &[("count", &self.omitted.to_string()), ("limit", &self.limit.to_string())],
            )
        });
        let parts: Vec<String> = self.failures.report().into_iter().chain(omission).collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

/// One frame's work: open the next launched file through `file.open`, as a dropped file opens,
/// and count it if that fails; once the reader is done and the last file has opened, put the report
/// on the status bar. Waits while a dialog is up, so a file asking for its password keeps the
/// prompt until it is answered. Returns whether it did anything, so the caller asks for another
/// frame.
pub(crate) fn open_next(launched: &Shared, app: &mut WordApp) -> bool {
    if app.dialog.is_some() {
        return false;
    }
    let next = launched.borrow_mut().ready.pop_front();
    if let Some((name, bytes)) = next {
        let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
        drop(bytes);
        match app.run("file.open", serde_json::json!({"path": name.as_str(), "data": data})) {
            Ok(_) => app.ui.backstage = false,
            Err(error) => launched.borrow_mut().failed(name, error),
        }
        return true;
    }
    let done = {
        let mut l = launched.borrow_mut();
        if !l.finished {
            return false;
        }
        std::mem::take(&mut *l)
    };
    if let Some(msg) = done.report() {
        app.status(msg);
    }
    true
}

/// The launched files that couldn't be read or opened, for one status line: the reason when one
/// file failed, else a few names and a count (each file's reason is in the log), as Add Font
/// reports.
#[derive(Default)]
struct Failures {
    count: usize,
    names: Vec<String>,
    first_error: String,
}

impl Failures {
    /// Count a failed file; the first failure's reason is kept for a lone report.
    fn add(&mut self, name: String, error: String) {
        if self.count == 0 {
            self.first_error = error;
        }
        self.count = self.count.saturating_add(1);
        if self.names.len() < MAX_NAMED_FAILURES {
            self.names.push(name);
        }
    }

    /// The failures' part of the status line, or `None` when every file opened.
    fn report(self) -> Option<String> {
        match (self.count, self.names.first()) {
            (0, _) | (_, None) => None,
            (1, Some(name)) => Some(i18n::fmt(i18n::t("Couldn't open {path}: {error}"), &[("path", name), ("error", &self.first_error)])),
            (count, Some(_)) => {
                let mut names = self.names.join(", ");
                if count > self.names.len() {
                    let more = count.saturating_sub(self.names.len()).to_string();
                    names = i18n::fmt(i18n::t("{names} and {n} more"), &[("names", &names), ("n", &more)]);
                }
                Some(i18n::fmt(i18n::t("{count} files couldn't be opened: {names}."), &[("count", &count.to_string()), ("names", &names)]))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(n: usize) -> Option<String> {
        let mut f = Failures::default();
        for i in 1..=n {
            f.add(format!("f{i}.docx"), format!("reason {i}"));
        }
        f.report()
    }

    /// Nothing failed: no status line, so the left-out count (if any) stands alone.
    #[test]
    fn no_failures_report_nothing() {
        assert_eq!(failed(0), None);
    }

    /// One failure says why, as any other open does.
    #[test]
    fn one_failure_gives_its_reason() {
        assert_eq!(failed(1).as_deref(), Some("Couldn't open f1.docx: reason 1"));
    }

    /// Several name the files; past three the rest are counted, so no failure goes unmentioned.
    #[test]
    fn several_failures_are_named_then_counted() {
        assert_eq!(failed(3).as_deref(), Some("3 files couldn't be opened: f1.docx, f2.docx, f3.docx."));
        assert_eq!(failed(5).as_deref(), Some("5 files couldn't be opened: f1.docx, f2.docx, f3.docx and 2 more."));
    }

    fn app() -> WordApp {
        WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), wordcraft_ui_egui::Services::default())
    }

    fn open_all(launched: &Shared, app: &mut WordApp) {
        for _ in 0..10 {
            open_next(launched, app);
        }
    }

    /// A file that was read but doesn't open is in the final report, after a later file opened
    /// and cleared the status bar.
    #[test]
    fn a_file_that_fails_to_open_is_reported_after_the_rest_open() {
        let launched = Shared::default();
        let mut a = app();
        {
            let mut l = launched.borrow_mut();
            l.start();
            l.push("broken.docx".into(), b"not a zip".to_vec());
            l.push("notes.md".into(), b"# Notes\n".to_vec());
            l.finish(0, 20);
        }
        open_all(&launched, &mut a);
        let status = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(status.starts_with("Couldn't open broken.docx: "), "{status}");
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Notes"), "the later file opened");
    }

    /// No report while the reader is still going: more files may yet fail.
    #[test]
    fn nothing_is_reported_before_the_reader_finishes() {
        let launched = Shared::default();
        let mut a = app();
        {
            let mut l = launched.borrow_mut();
            l.start();
            l.push("broken.docx".into(), b"not a zip".to_vec());
        }
        assert!(!launched.borrow().is_drained(), "the reader waits while a file is unopened");
        open_all(&launched, &mut a);
        assert!(launched.borrow().is_drained(), "the reader may read the next file");
        a.status_msg = None;
        open_all(&launched, &mut a);
        assert_eq!(a.status_msg, None);
        launched.borrow_mut().finish(2, 20);
        open_all(&launched, &mut a);
        let status = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(status.contains("broken.docx") && status.ends_with("2 more files weren't opened: WordCraft opens up to 20 at a time."), "{status}");
    }

    /// A launch during a read queues behind it: no second reader, and reads keep their order,
    /// over launches too.
    #[test]
    fn a_launch_during_a_read_queues_behind_it_in_order() {
        let mut q = Launches::default();
        assert!(admit(&mut q, [1, 2]), "the first launch starts a reader");
        assert!(!admit(&mut q, [3]), "a second launch starts no second reader");
        for expected in [1, 2, 3] {
            match next_step(&mut q, true) {
                Step::Read(n) => assert_eq!(n, expected),
                _ => panic!("expected file {expected}"),
            }
        }
        assert!(matches!(next_step(&mut q, true), Step::Finish(0)));
        assert!(admit(&mut q, [4]), "a later launch starts a reader again");
        assert!(matches!(next_step(&mut q, true), Step::Read(4)));
    }

    /// Nothing is read while a file is unopened, which is what bounds the bytes held to one file.
    #[test]
    fn an_unopened_file_stops_the_next_read() {
        let mut q = Launches::default();
        admit(&mut q, [1, 2]);
        assert!(matches!(next_step(&mut q, false), Step::Wait));
        assert!(matches!(next_step(&mut q, true), Step::Read(1)));
        assert!(matches!(next_step(&mut q, false), Step::Wait), "file 2 waits until file 1 opened");
        assert!(matches!(next_step(&mut q, true), Step::Read(2)));
        assert!(matches!(next_step(&mut q, true), Step::Finish(0)));
    }

    /// The files past the limit are counted, per launch, without a file being opened for them.
    #[test]
    fn files_past_the_limit_are_counted_and_reported() {
        let mut q = Launches::default();
        admit(&mut q, 0..MAX_WAITING_FILES);
        assert_eq!(q.waiting.len(), MAX_WAITING_FILES);
        assert_eq!(q.omitted, 0);
        admit(&mut q, [100, 101, 102]);
        assert_eq!(q.omitted, 3);
        assert_eq!(q.waiting.len(), MAX_WAITING_FILES);
        for n in 0..MAX_WAITING_FILES {
            assert!(matches!(next_step(&mut q, true), Step::Read(m) if m == n));
        }
        assert!(matches!(next_step(&mut q, true), Step::Finish(3)));

        let launched = Shared::default();
        launched.borrow_mut().start();
        launched.borrow_mut().finish(3, MAX_WAITING_FILES);
        let status = Launched::report(std::mem::take(&mut *launched.borrow_mut())).unwrap_or_default();
        assert_eq!(status, "3 more files weren't opened: WordCraft opens up to 20 at a time.");
    }

    /// A launch whose files all arrive while the queue is full still ends, so the count is
    /// reported even when every one of them was left out.
    #[test]
    fn a_launch_that_only_overflows_still_finishes() {
        let mut q = Launches::default();
        admit(&mut q, 0..MAX_WAITING_FILES);
        // The reader has taken the first file and is reading it; the queue is still full.
        assert!(matches!(next_step(&mut q, true), Step::Read(0)));
        assert!(!admit(&mut q, [100, 101, 102]), "a second launch must not start another reader");
        assert_eq!(q.omitted, 2, "100 took the slot the first read freed; 101 and 102 were left out");
        assert!(!admit(&mut q, []), "the running reader keeps going");
        for n in 1..MAX_WAITING_FILES {
            assert!(matches!(next_step(&mut q, true), Step::Read(m) if m == n));
        }
        assert!(matches!(next_step(&mut q, true), Step::Read(100)), "the file that filled the freed slot opens last");
        assert!(matches!(next_step(&mut q, true), Step::Finish(2)), "both left-out files are counted");
    }

    /// The size limit is checked on the reported size before the bytes are read: exactly 256 MB
    /// is allowed, one byte over is refused, and a nonsensical size is refused rather than
    /// truncated.
    #[test]
    fn the_size_limit_is_checked_at_the_boundary() {
        assert!(size_within_limit(0.0));
        assert!(size_within_limit(MAX_LAUNCHED_BYTES));
        assert!(!size_within_limit(MAX_LAUNCHED_BYTES + 1.0));
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(!size_within_limit(bad), "{bad}");
        }
    }

    /// While a dialog is up (a launched file asks for its password), the next file waits.
    #[test]
    fn an_open_dialog_holds_the_next_file() {
        let launched = Shared::default();
        let mut a = app();
        {
            let mut l = launched.borrow_mut();
            l.start();
            l.push("first.md".into(), b"# First\n".to_vec());
            l.push("second.md".into(), b"# Second\n".to_vec());
            l.finish(0, MAX_WAITING_FILES);
        }
        a.dialog = Some(wordcraft_ui_egui::dialogs::Dialog::RenameDocument { name: "test".into(), message: String::new(), focus: false });
        assert!(!open_next(&launched, &mut a));
        assert_eq!(launched.borrow().ready.len(), 2, "nothing opens while the dialog is up");
        a.dialog = None;
        open_all(&launched, &mut a);
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Second"), "the last file opens afterwards");
    }
}

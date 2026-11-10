//! Test helper: the warnings this crate logs, captured per test thread, so a test can check that
//! a limit which leaves something out says so.

use std::cell::RefCell;

struct Capture;

thread_local! {
    static LINES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }

    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            LINES.with(|l| l.borrow_mut().push(r.args().to_string()));
        }
    }

    fn flush(&self) {}
}

static CAPTURE: Capture = Capture;

/// `f`'s result and the warnings logged on this thread while it ran.
pub(crate) fn warnings<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    // Only this logger is ever installed in a test binary; a second call keeps it.
    let _ = log::set_logger(&CAPTURE);
    log::set_max_level(log::LevelFilter::Warn);
    LINES.with(|l| l.borrow_mut().clear());
    let out = f();
    (out, LINES.with(|l| std::mem::take(&mut *l.borrow_mut())))
}

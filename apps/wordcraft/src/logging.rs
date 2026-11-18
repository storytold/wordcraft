//! The desktop app's logger: `log::` records go to standard error and to a log file.
//!
//! Without a logger every `log::warn!`/`log::error!` in the workspace (an unreadable part of a
//! .docx, a picture or font a PDF export had to leave out, a PNG that would not encode) vanished. A
//! launch from a desktop menu has no terminal, so the file is what a bug report can attach:
//! `<settings dir>/logs/wordcraft.log`, next to the preferences (`prefs_path` in `main.rs`). Each
//! start moves the previous log to `wordcraft.1.log` (and that one to `.2`), so the log of a run
//! that crashed survives the next launch. Runs without preferences (`WORDCRAFT_NO_PREFS`) log to
//! standard error only.
//!
//! Levels: `info` for WordCraft's own crates, `warn` for everything else (wgpu and naga are
//! chatty). `RUST_LOG` replaces that with env_logger-style directives: `debug`,
//! `warn,wordcraft_docx=trace`, `wgpu_core=info`. A directive ending in `*` matches every target
//! that starts with it (`wordcraft*=debug`).
//!
//! Records logged before the settings directory is known are kept (up to [`MAX_PENDING`]) and
//! written once the file is attached. Writing never panics: a file that can't be created or
//! written leaves standard error as the only sink.
//!
//! The panic hook ([`install_panic_hook`]) reports every panic, caught by `Session::run`'s guard or
//! not, through this logger. Text a panic message quotes between backticks (a slice of a string,
//! which can be the user's writing) is left out.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

use log::LevelFilter;

/// The current log file's name inside the log directory.
pub const LOG_FILE: &str = "wordcraft.log";
/// How many previous logs are kept (`wordcraft.1.log` … `wordcraft.<KEEP>.log`).
pub const KEEP: usize = 2;
/// The log file stops growing past this size (a runaway warning can't fill the disk).
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Records kept in memory until the log file is attached.
pub const MAX_PENDING: usize = 512;
/// The built-in filter when `RUST_LOG` is unset or empty.
pub const DEFAULT_FILTER: &str = "warn,wordcraft*=info";

/// Per-target level filter parsed from env_logger-style directives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    default: LevelFilter,
    directives: Vec<(String, LevelFilter)>,
}

impl Filter {
    /// Parse `spec`; unknown levels and empty parts are skipped. Without a bare level, targets no
    /// directive names log errors only.
    pub fn parse(spec: &str) -> Filter {
        let mut f = Filter { default: LevelFilter::Error, directives: Vec::new() };
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some((name, level)) => {
                    let name = name.trim();
                    if let (false, Some(level)) = (name.is_empty(), parse_level(level)) {
                        f.directives.push((name.to_owned(), level));
                    }
                }
                None => match parse_level(part) {
                    Some(level) => f.default = level,
                    // A bare target name: everything from it (env_logger does the same).
                    None => f.directives.push((part.to_owned(), LevelFilter::Trace)),
                },
            }
        }
        f
    }

    /// The level that applies to `target` (the most specific matching directive wins).
    pub fn level_for(&self, target: &str) -> LevelFilter {
        self.directives.iter().filter(|(name, _)| matches(name, target)).max_by_key(|(name, _)| name.len()).map_or(self.default, |(_, level)| *level)
    }

    /// The most verbose level any target can reach (for `log::set_max_level`).
    pub fn max(&self) -> LevelFilter {
        self.directives.iter().map(|(_, level)| *level).fold(self.default, Ord::max)
    }
}

fn parse_level(s: &str) -> Option<LevelFilter> {
    s.trim().parse().ok()
}

/// `name` matches `target` itself and its submodules (`a` matches `a` and `a::b`, not `ab`);
/// `name*` matches every target starting with `name`.
fn matches(name: &str, target: &str) -> bool {
    match name.strip_suffix('*') {
        Some(prefix) => target.starts_with(prefix),
        None => target.strip_prefix(name).is_some_and(|rest| rest.is_empty() || rest.starts_with("::")),
    }
}

/// `2026-10-08T07:59:17.728Z` (UTC, milliseconds). A clock before 1970 reads as the epoch (the
/// `duration_since` clamp); past the last date chrono can represent (year 262143) the proleptic
/// Gregorian date is kept via the 400-year cycle rather than falling back to the epoch, so the
/// output never changes for a value the old formatter accepted and logging never panics.
pub fn timestamp(t: SystemTime) -> String {
    let since = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, since.subsec_millis())
}

/// Days since 1970-01-01 → proleptic Gregorian (year, month, day).
///
/// `timestamp` clamps a pre-1970 clock to the epoch, so `days` is an unsigned count that can run
/// past `NaiveDate`'s ±262143-year range. The Gregorian calendar repeats every 400 years
/// (146_097 days), so chrono converts the day within one cycle and whole cycles go back into the
/// year (an `i128`, then saturated into `i64`), keeping chrono as the only calendar logic and
/// reproducing the old `civil_from_days` result exactly without hand-rolled day math. 1970-01-01
/// is `num_days_from_ce` 719_163.
fn civil_from_days(days: u64) -> (i64, u32, u32) {
    use chrono::Datelike;
    let (cycles, rest) = (days / 146_097, days % 146_097);
    // `rest + 719_163` is 719_163..865_259, always a valid CE day.
    match i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
        Some(d) => {
            let y = i128::from(d.year()) + i128::from(cycles) * 400;
            (i64::try_from(y).unwrap_or(i64::MAX), d.month(), d.day())
        }
        None => (1970, 1, 1), // unreachable
    }
}

fn numbered(dir: &Path, n: usize) -> PathBuf {
    if n == 0 { dir.join(LOG_FILE) } else { dir.join(format!("wordcraft.{n}.log")) }
}

/// Shift the previous logs up one (`.log` → `.1.log` → … → `.<KEEP>.log`, the oldest dropped)
/// and create a fresh, empty log file in `dir` (created if missing).
pub fn rotate(dir: &Path) -> std::io::Result<(PathBuf, File)> {
    std::fs::create_dir_all(dir)?;
    for n in (1..=KEEP).rev() {
        match std::fs::rename(numbered(dir, n - 1), numbered(dir, n)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    let path = numbered(dir, 0);
    let file = File::create(&path)?;
    Ok((path, file))
}

/// One formatted log line: `<timestamp> <LEVEL> [<thread>] <target>: <message>\n`.
pub fn format_line(ts: &str, level: log::Level, thread: &str, target: &str, message: &str) -> String {
    let message = message.strip_suffix('\n').unwrap_or(message);
    format!("{ts} {level:<5} [{thread}] {target}: {message}\n")
}

/// Where formatted lines go besides standard error: kept in memory until a file is attached,
/// then appended to it until it reaches its size cap.
pub struct Sink {
    file: Option<File>,
    pending: Vec<String>,
    dropped: usize,
    written: u64,
    max_bytes: u64,
    capped: bool,
}

impl Sink {
    pub fn new(max_bytes: u64) -> Sink {
        Sink { file: None, pending: Vec::new(), dropped: 0, written: 0, max_bytes, capped: false }
    }

    /// Append `line` to the file, or keep it until [`Sink::attach`].
    pub fn write(&mut self, line: &str) {
        if self.file.is_none() {
            if self.capped {
                // No file is coming ([`Sink::no_file`]): standard error had the line.
            } else if self.pending.len() < MAX_PENDING {
                self.pending.push(line.to_owned());
            } else {
                self.dropped = self.dropped.saturating_add(1);
            }
            return;
        }
        self.append(line);
    }

    /// Start writing to `file` (a fresh one: the size cap starts over), first the lines kept so far.
    pub fn attach(&mut self, file: File) {
        self.file = Some(file);
        self.written = 0;
        self.capped = false;
        for line in std::mem::take(&mut self.pending) {
            self.append(&line);
        }
        if self.dropped > 0 {
            let note = format!("{} earlier log lines were dropped before the log file was opened\n", self.dropped);
            self.dropped = 0;
            self.append(&note);
        }
    }

    /// There will be no log file: forget the lines kept for it and keep no more.
    pub fn no_file(&mut self) {
        self.file = None;
        self.pending = Vec::new();
        self.dropped = 0;
        self.capped = true;
    }

    fn append(&mut self, line: &str) {
        if self.capped {
            return;
        }
        let Some(file) = self.file.as_mut() else { return };
        let len = u64::try_from(line.len()).unwrap_or(u64::MAX);
        let text = if self.written.saturating_add(len) > self.max_bytes {
            self.capped = true;
            format!("log file reached {} bytes; later records go to standard error only\n", self.max_bytes)
        } else {
            self.written = self.written.saturating_add(len);
            line.to_owned()
        };
        // A full disk or a vanished file must not take the app down: drop the file sink.
        if file.write_all(text.as_bytes()).is_err() {
            self.file = None;
            self.capped = true;
        }
    }
}

/// The installed logger: a [`Filter`], standard error and a [`Sink`].
pub struct AppLogger {
    filter: Filter,
    stderr: bool,
    sink: Mutex<Sink>,
}

impl AppLogger {
    pub fn new(filter: Filter, stderr: bool) -> AppLogger {
        AppLogger { filter, stderr, sink: Mutex::new(Sink::new(MAX_FILE_BYTES)) }
    }

    /// Rotate the logs in `dir` and send records to the fresh file; returns its path. On an error
    /// standard error stays the only sink.
    pub fn attach_dir(&self, dir: &Path) -> Result<PathBuf, String> {
        // A thread that panicked while logging leaves the sink usable: take it back.
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        match rotate(dir) {
            Ok((path, file)) => {
                sink.attach(file);
                Ok(path)
            }
            Err(e) => {
                sink.no_file();
                Err(format!("{}: {e}", dir.display()))
            }
        }
    }

    /// No log file for this run (runs without preferences): log to standard error only.
    pub fn no_file(&self) {
        self.sink.lock().unwrap_or_else(PoisonError::into_inner).no_file();
    }
}

impl log::Log for AppLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.filter.level_for(metadata.target())
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Formatted before the sink is locked, so a message that logs while it is formatted can't deadlock.
        let thread = std::thread::current();
        let line =
            format_line(&timestamp(SystemTime::now()), record.level(), thread.name().unwrap_or("?"), record.target(), &record.args().to_string());
        if self.stderr {
            // No terminal (a Windows GUI build, a closed pipe) is not an error worth reporting.
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        self.sink.lock().unwrap_or_else(PoisonError::into_inner).write(&line);
    }

    fn flush(&self) {}
}

/// Install the logger (filter from `RUST_LOG`, else [`DEFAULT_FILTER`]); `None` when another
/// logger was installed first. Call [`AppLogger::attach_dir`] once the settings directory is known.
pub fn install() -> Option<&'static AppLogger> {
    static LOGGER: OnceLock<AppLogger> = OnceLock::new();
    let spec = std::env::var("RUST_LOG").ok().filter(|s| !s.trim().is_empty());
    let filter = Filter::parse(spec.as_deref().unwrap_or(DEFAULT_FILTER));
    let max = filter.max();
    let logger = LOGGER.get_or_init(|| AppLogger::new(filter, true));
    log::set_logger(logger).ok()?;
    log::set_max_level(max);
    Some(logger)
}

/// Replace text a panic message quotes between backticks with `…`: std quotes the string it was
/// slicing (`byte index 5 is not a char boundary; … of `<the string>``), and that string can be
/// the user's writing. An unpaired backtick leaves the rest of the message out too.
pub fn redact_quoted(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for (i, part) in message.split('`').enumerate() {
        if i % 2 == 1 {
            out.push_str("`…");
        } else {
            if i > 0 {
                out.push('`');
            }
            out.push_str(part);
        }
    }
    out
}

/// The text the panic hook logs: where it happened, the redacted message and, when captured
/// (`RUST_BACKTRACE`), the backtrace.
pub fn panic_report(location: Option<&str>, message: Option<&str>, backtrace: Option<&str>) -> String {
    let message = message.map_or_else(|| "(no message)".to_owned(), redact_quoted);
    let mut report = format!("panic at {}: {message}", location.unwrap_or("an unknown place"));
    if let Some(trace) = backtrace {
        report.push('\n');
        report.push_str(trace.trim_end());
    }
    report
}

/// Report every panic (caught by `Session::run`'s guard or not) through the logger, so it reaches
/// the log file too; standard error alone when `RUST_LOG` turned errors off.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().map(ToString::to_string);
        let trace = std::backtrace::Backtrace::capture();
        let trace = (trace.status() == std::backtrace::BacktraceStatus::Captured).then(|| trace.to_string());
        let report = panic_report(location.as_deref(), info.payload_as_str(), trace.as_deref());
        if log::log_enabled!(log::Level::Error) {
            log::error!("{report}");
        } else {
            // `eprintln!` panics on a broken stderr pipe, and a panic inside the panic hook aborts.
            let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("wordcraft: {report}\n"));
        }
    }));
}

#[cfg(test)]
mod tests;

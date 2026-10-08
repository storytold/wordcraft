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

/// `2026-10-08T07:59:17.728Z` (UTC, milliseconds). A clock before 1970 reads as the epoch.
pub fn timestamp(t: SystemTime) -> String {
    let since = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, since.subsec_millis())
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day) (Howard Hinnant's
/// `civil_from_days`, unsigned because `timestamp` clamps at the epoch). Saturating, so no clock
/// value can overflow it.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days.saturating_add(719_468);
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = era.saturating_mul(400).saturating_add(yoe).saturating_add(u64::from(m <= 2));
    (y, m, d)
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
            eprintln!("wordcraft: {report}");
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("wordcraft-logging-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("read log")
    }

    #[test]
    fn the_default_filter_shows_wordcraft_info_and_other_crates_warnings() {
        let f = Filter::parse(DEFAULT_FILTER);
        assert_eq!(f.level_for("wordcraft"), LevelFilter::Info);
        assert_eq!(f.level_for("wordcraft::control_server"), LevelFilter::Info);
        assert_eq!(f.level_for("wordcraft_docx::package"), LevelFilter::Info);
        assert_eq!(f.level_for("wordcraft_ui_egui"), LevelFilter::Info);
        assert_eq!(f.level_for("wgpu_core::device"), LevelFilter::Warn);
        assert_eq!(f.level_for("naga"), LevelFilter::Warn);
        assert_eq!(f.max(), LevelFilter::Info);
    }

    #[test]
    fn directives_follow_env_logger_and_the_most_specific_one_wins() {
        let f = Filter::parse("info,wgpu_core=error,wordcraft_pdf=trace,wordcraft_pdf::fonts=off");
        assert_eq!(f.level_for("eframe"), LevelFilter::Info);
        assert_eq!(f.level_for("wgpu_core::instance"), LevelFilter::Error);
        assert_eq!(f.level_for("wordcraft_pdf::images"), LevelFilter::Trace);
        assert_eq!(f.level_for("wordcraft_pdf::fonts"), LevelFilter::Off);
        assert_eq!(f.max(), LevelFilter::Trace);
        // A module name is matched at `::` boundaries, not as a bare prefix.
        assert_eq!(f.level_for("wgpu_core_extra"), LevelFilter::Info);
        // A trailing `*` is a prefix.
        assert_eq!(Filter::parse("wordcraft*=debug").level_for("wordcraft_layout::lines"), LevelFilter::Debug);
        assert_eq!(Filter::parse("wordcraft*=debug").level_for("eframe"), LevelFilter::Error);
        // A bare target name sets that target to the most verbose level, as env_logger does.
        assert_eq!(Filter::parse("naga").level_for("naga::front"), LevelFilter::Trace);
        assert_eq!(Filter::parse("naga").level_for("eframe"), LevelFilter::Error);
        // Levels are case-insensitive and surrounding blanks are ignored.
        assert_eq!(Filter::parse(" WARN , wordcraft = Debug ").level_for("wordcraft"), LevelFilter::Debug);
        assert_eq!(Filter::parse(" WARN , wordcraft = Debug ").level_for("eframe"), LevelFilter::Warn);
    }

    #[test]
    fn hostile_specs_never_panic_and_fall_back_sensibly() {
        let long = "x".repeat(100_000);
        let many = "a=info,".repeat(10_000);
        for spec in [
            "",
            ",,,",
            "=",
            "==",
            "=debug",
            "wordcraft=",
            "nonsense=loud",
            "🦀=info",
            "*",
            "*=",
            "=*",
            "a=b=c",
            "warn,,wordcraft=DEBUG",
            "\0",
            "é::ü=trace",
            &long,
            &many,
        ] {
            let f = Filter::parse(spec);
            for target in ["", "wordcraft", "🦀", "a::b", "*"] {
                let _ = f.level_for(target);
            }
            let _ = f.max();
        }
        assert_eq!(Filter::parse("warn,,wordcraft=DEBUG").level_for("wordcraft"), LevelFilter::Debug);
        // An unknown level is skipped: that target keeps the default.
        assert_eq!(Filter::parse("nonsense=loud").level_for("nonsense"), LevelFilter::Error);
        assert_eq!(Filter::parse("wordcraft=").level_for("wordcraft"), LevelFilter::Error);
        assert_eq!(Filter::parse("=debug").max(), LevelFilter::Error);
        assert_eq!(Filter::parse("🦀=info").level_for("🦀::claw"), LevelFilter::Info);
    }

    #[test]
    fn timestamps_are_utc_with_milliseconds() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_millis(1_791_446_357_728)), "2026-10-08T07:59:17.728Z");
        // Leap day, and the day after it.
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(1_709_164_800)), "2024-02-29T00:00:00.000Z");
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(1_709_251_199)), "2024-02-29T23:59:59.000Z");
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(1_709_251_200)), "2024-03-01T00:00:00.000Z");
        // 2000 is a leap year (divisible by 400), 2100 is not.
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29T00:00:00.000Z");
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(4_107_542_400)), "2100-03-01T00:00:00.000Z");
        // Before the epoch (a clock set wrong) clamps instead of panicking.
        assert_eq!(timestamp(UNIX_EPOCH - Duration::from_secs(5)), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn a_line_carries_time_level_thread_target_and_message() {
        assert_eq!(
            format_line("2026-10-08T07:59:17.728Z", log::Level::Warn, "main", "wordcraft_docx::package", "skipping unreadable zip entry 3"),
            "2026-10-08T07:59:17.728Z WARN  [main] wordcraft_docx::package: skipping unreadable zip entry 3\n"
        );
        // A multi-line message keeps its lines; the record still ends in one newline.
        assert!(format_line("t", log::Level::Error, "w", "x", "a\nb\n").ends_with("x: a\nb\n"));
    }

    #[test]
    fn rotation_keeps_the_previous_logs_and_starts_an_empty_file() {
        let dir = temp_dir("rotate");
        for run in 1..=4 {
            let (path, _file) = rotate(&dir).expect("rotate");
            assert_eq!(path, dir.join(LOG_FILE));
            assert_eq!(read(&path), "");
            std::fs::write(&path, format!("run {run}")).expect("write");
        }
        assert_eq!(read(&dir.join(LOG_FILE)), "run 4");
        assert_eq!(read(&dir.join("wordcraft.1.log")), "run 3");
        assert_eq!(read(&dir.join("wordcraft.2.log")), "run 2");
        assert!(!dir.join("wordcraft.3.log").exists(), "only {KEEP} old logs are kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_into_an_unusable_directory_is_an_error_not_a_panic() {
        let dir = temp_dir("blocked");
        std::fs::create_dir_all(dir.parent().expect("parent")).expect("tmp");
        std::fs::write(&dir, "a file where the directory should be").expect("block");
        assert!(rotate(&dir).is_err());
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn lines_logged_before_the_file_exists_are_written_when_it_is_attached() {
        let dir = temp_dir("pending");
        let (path, file) = rotate(&dir).expect("rotate");
        let mut sink = Sink::new(MAX_FILE_BYTES);
        sink.write("early 1\n");
        sink.write("early 2\n");
        sink.attach(file);
        sink.write("late\n");
        assert_eq!(read(&path), "early 1\nearly 2\nlate\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pending_buffer_is_bounded_and_says_how_much_it_dropped() {
        let dir = temp_dir("bounded");
        let (path, file) = rotate(&dir).expect("rotate");
        let mut sink = Sink::new(MAX_FILE_BYTES);
        for i in 0..MAX_PENDING + 5 {
            sink.write(&format!("line {i}\n"));
        }
        sink.attach(file);
        let text = read(&path);
        assert_eq!(text.lines().filter(|l| l.starts_with("line ")).count(), MAX_PENDING);
        assert!(text.contains("5 earlier log lines were dropped"), "{text}");
        assert!(text.contains("line 0\n") && !text.contains(&format!("line {MAX_PENDING}\n")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exactly_a_full_pending_buffer_drops_nothing() {
        let dir = temp_dir("full");
        let (path, file) = rotate(&dir).expect("rotate");
        let mut sink = Sink::new(MAX_FILE_BYTES);
        for i in 0..MAX_PENDING {
            sink.write(&format!("line {i}\n"));
        }
        sink.attach(file);
        let text = read(&path);
        assert_eq!(text.lines().count(), MAX_PENDING);
        assert!(!text.contains("dropped"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_file_nothing_is_kept_in_memory() {
        let mut sink = Sink::new(MAX_FILE_BYTES);
        sink.write("early\n");
        sink.no_file();
        sink.write("late\n");
        assert!(sink.pending.is_empty() && sink.dropped == 0);
    }

    #[test]
    fn the_file_stops_at_its_size_cap_with_one_note() {
        let dir = temp_dir("cap");
        let (path, file) = rotate(&dir).expect("rotate");
        let mut sink = Sink::new(100);
        sink.attach(file);
        for i in 0..50 {
            sink.write(&format!("line {i:02} with some padding\n"));
        }
        let text = read(&path);
        assert_eq!(text.lines().filter(|l| l.starts_with("line ")).count(), 3, "{text}");
        assert_eq!(text.matches("log file reached").count(), 1, "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_logger_filters_records_and_writes_them_to_the_attached_file() {
        use log::Log;
        let dir = temp_dir("logger");
        let logger = AppLogger::new(Filter::parse(DEFAULT_FILTER), false);
        let record = |level, target: &'static str, msg: &'static str| {
            logger.log(&log::Record::builder().level(level).target(target).args(format_args!("{msg}")).build());
        };
        record(log::Level::Info, "wordcraft", "before the settings directory");
        record(log::Level::Info, "wgpu_core::device", "too chatty");
        let path = logger.attach_dir(&dir).expect("attach");
        assert_eq!(path, dir.join(LOG_FILE));
        record(log::Level::Warn, "wgpu_hal::vulkan", "a real warning");
        record(log::Level::Error, "wordcraft_render", "PNG encode: bad size 0×0");
        record(log::Level::Debug, "wordcraft", "below info");
        let text = read(&path);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[0].contains(" INFO  [") && lines[0].ends_with("wordcraft: before the settings directory"), "{text}");
        assert!(lines[1].ends_with("wgpu_hal::vulkan: a real warning"), "{text}");
        assert!(lines[2].contains(" ERROR [") && lines[2].ends_with("wordcraft_render: PNG encode: bad size 0×0"), "{text}");
        assert!(logger.enabled(&log::Metadata::builder().level(log::Level::Info).target("wordcraft_ui_egui").build()));
        assert!(!logger.enabled(&log::Metadata::builder().level(log::Level::Info).target("naga").build()));
        // Attaching again rotates: the first log becomes `.1`.
        logger.attach_dir(&dir).expect("attach again");
        assert_eq!(read(&dir.join("wordcraft.1.log")), text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_from_many_threads_are_whole_lines() {
        use log::Log;
        let dir = temp_dir("threads");
        let logger = std::sync::Arc::new(AppLogger::new(Filter::parse("info"), false));
        let path = logger.attach_dir(&dir).expect("attach");
        let workers: Vec<_> = (0..8)
            .map(|t| {
                let l = std::sync::Arc::clone(&logger);
                std::thread::spawn(move || {
                    for i in 0..100 {
                        l.log(
                            &log::Record::builder().level(log::Level::Info).target("wordcraft").args(format_args!("thread {t} record {i}")).build(),
                        );
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().expect("worker");
        }
        let text = read(&path);
        assert_eq!(text.lines().count(), 800);
        assert!(text.lines().all(|l| l.contains(" wordcraft: thread ")), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attaching_to_an_unusable_directory_reports_an_error_and_keeps_logging() {
        use log::Log;
        let dir = temp_dir("unusable");
        std::fs::create_dir_all(dir.parent().expect("parent")).expect("tmp");
        std::fs::write(&dir, "not a directory").expect("block");
        let logger = AppLogger::new(Filter::parse("info"), false);
        logger.log(&log::Record::builder().level(log::Level::Error).target("x").args(format_args!("early")).build());
        assert!(logger.attach_dir(&dir).is_err());
        logger.log(&log::Record::builder().level(log::Level::Error).target("x").args(format_args!("still fine")).build());
        // Nothing waits in memory for a file that won't come.
        let sink = logger.sink.lock().expect("sink");
        assert!(sink.pending.is_empty() && sink.file.is_none());
        drop(sink);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn a_poisoned_sink_still_logs() {
        use log::Log;
        let dir = temp_dir("poison");
        let logger = std::sync::Arc::new(AppLogger::new(Filter::parse("info"), false));
        let l = std::sync::Arc::clone(&logger);
        let _ = std::thread::spawn(move || {
            let _guard = l.sink.lock().expect("lock");
            panic!("poison the sink");
        })
        .join();
        assert!(logger.sink.is_poisoned());
        let path = logger.attach_dir(&dir).expect("attach");
        logger.log(&log::Record::builder().level(log::Level::Warn).target("wordcraft").args(format_args!("after the poison")).build());
        assert!(read(&path).contains("wordcraft: after the poison"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn panic_messages_leave_out_quoted_text() {
        // What std says when a string is sliced inside a character: the string is the user's text.
        let msg = "byte index 5 is not a char boundary; it is inside 'é' (bytes 4..6) of `Dear Ms Smith, my secret`";
        assert_eq!(redact_quoted(msg), "byte index 5 is not a char boundary; it is inside 'é' (bytes 4..6) of `…`");
        assert_eq!(redact_quoted("byte index 99 is out of range of `my diary`[...]"), "byte index 99 is out of range of `…`[...]");
        assert_eq!(redact_quoted("a `b` c `d` e"), "a `…` c `…` e");
        // An unpaired backtick hides the rest rather than risk leaking it.
        assert_eq!(redact_quoted("of `half a sentence"), "of `…");
        assert_eq!(
            redact_quoted("no quotes: index out of bounds: the len is 3 but the index is 5"),
            "no quotes: index out of bounds: the len is 3 but the index is 5"
        );
        assert_eq!(redact_quoted(""), "");
        assert_eq!(redact_quoted("``"), "`…`");
        assert_eq!(redact_quoted("🦀`🦀`🦀"), "🦀`…`🦀");
    }

    #[test]
    fn a_panic_report_names_the_place_and_the_redacted_message() {
        assert_eq!(
            panic_report(Some("crates/engine/src/cmd/edit.rs:12:5"), Some("of `secret`"), None),
            "panic at crates/engine/src/cmd/edit.rs:12:5: of `…`"
        );
        assert_eq!(panic_report(None, None, None), "panic at an unknown place: (no message)");
        assert_eq!(panic_report(Some("a.rs:1:1"), Some("boom"), Some("   0: main\n")), "panic at a.rs:1:1: boom\n   0: main");
    }
}

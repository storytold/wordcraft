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
    // Before the epoch (a clock set wrong) clamps to the epoch instead of panicking.
    assert_eq!(timestamp(UNIX_EPOCH - Duration::from_secs(5)), "1970-01-01T00:00:00.000Z");
    // Past chrono's range (a clock set absurdly far ahead) keeps the proleptic Gregorian date
    // the old formatter gave, rather than falling back to the epoch. FredApps' counterexample:
    // UNIX_EPOCH + 10_000_000_000_000 s was 318857-05-20T17:46:40.000Z and must stay so.
    // Windows' SystemTime holds only up to about year 30828, so this is checked where it fits.
    if let Some(far) = UNIX_EPOCH.checked_add(Duration::from_secs(10_000_000_000_000)) {
        assert_eq!(timestamp(far), "318857-05-20T17:46:40.000Z");
    }
    // Windows' SystemTime can't hold u64::MAX / 2 seconds, so it is only checked where it exists.
    if let Some(far) = UNIX_EPOCH.checked_add(Duration::from_secs(u64::MAX / 2)) {
        assert_eq!(timestamp(far), "292277026596-12-04T15:30:07.000Z");
    }
    // Years past 9999 keep the plain zero-padded year (no `+`).
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(253_402_300_800)), "10000-01-01T00:00:00.000Z");
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
                    l.log(&log::Record::builder().level(log::Level::Info).target("wordcraft").args(format_args!("thread {t} record {i}")).build());
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

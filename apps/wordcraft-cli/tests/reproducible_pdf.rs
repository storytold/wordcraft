//! Direct PDF exports use the saved timestamp without changing the source document.
use std::process::Command;

#[test]
fn direct_pdf_exports_use_the_pinned_epoch() {
    check_creation_date("", "20261005140300");
}

#[test]
fn direct_pdf_exports_preserve_an_existing_creation_date() {
    check_creation_date("2000-01-01T00:00:00Z", "20000101000000");
}

/// A .docx records `dcterms:created` as text and the reader takes it as it finds it, so a
/// document can arrive holding something that is not a date at all. The PDF writer cannot put
/// that in a document and fell back to the clock, so two exports of the same file differed even
/// with `SOURCE_DATE_EPOCH` set -- the one thing it is there to stop (noted in review).
#[test]
fn direct_pdf_exports_use_the_pinned_epoch_when_the_existing_date_is_unreadable() {
    for created in ["not-a-date", "", "   ", "2000-13-01T00:00:00Z", "2000-01-32T00:00:00Z", "99-01-01", "today"] {
        check_creation_date(created, "20261005140300");
    }
}

fn check_creation_date(created: &str, expected: &str) {
    // A sequence number, not the clock alone: `as_nanos` carries microsecond granularity on
    // macOS, so two of these tests starting in the same microsecond took the same directory,
    // wrote each other's `input.json` and failed the preservation assertion below -- a flake in
    // this harness, not in the export. `create_dir` rather than `create_dir_all` so that if a
    // name ever does repeat it fails here instead of quietly sharing a directory again.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("wordcraft-pdf-date-{}-{unique}-{seq}", std::process::id()));
    std::fs::create_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut doc = wordcraft_doc::Document::from_text("Original reproducible PDF check.");
    doc.core.created = created.into();
    let input = dir.join("input.json");
    let source = serde_json::to_vec(&doc).unwrap();
    std::fs::write(&input, &source).unwrap();
    for command in ["file.exportPdf", "file.printPdf"] {
        let output = dir.join(format!("{command}.pdf"));
        let params = serde_json::json!({"path": output});
        let run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
            .arg("run")
            .arg("--file")
            .arg(&input)
            .arg("--cmd")
            .arg(format!("{command}={params}"))
            .env("SOURCE_DATE_EPOCH", "1791208980")
            .output()
            .unwrap();
        assert!(run.status.success(), "{command}: {}", String::from_utf8_lossy(&run.stderr));
        let bytes = std::fs::read(output).unwrap();
        let date = format!("/CreationDate(D:{expected}");
        assert!(bytes.windows(date.len()).any(|part| part == date.as_bytes()), "{command} must use {expected}");
        assert_eq!(std::fs::read(&input).unwrap(), source, "export must preserve its source file");
    }
}

/// A fresh directory for one test's files.
fn scratch(name: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("wordcraft-{name}-{}-{unique}", std::process::id()));
    std::fs::create_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    dir
}

fn cli(env: &str, cmds: &[String]) -> std::process::Output {
    let mut run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    run.arg("run").env("SOURCE_DATE_EPOCH", env);
    for c in cmds {
        run.arg("--cmd").arg(c);
    }
    run.output().unwrap()
}

/// The pinned date is the file's, not the edits': a comment and a tracked change made in the
/// same run keep the time they were made, so a pinned epoch cannot back-date them (noted in
/// review). Saved as flat XML, which holds both in plain text.
#[test]
fn a_pinned_epoch_dates_the_file_and_not_the_edits_in_it() {
    let out = scratch("pinned-edits").join("edits.xml");
    let before = wordcraft_engine::cmd::now_iso();
    let run = cli(
        "1791208980",
        &[
            r#"review.trackChanges={"value": true}"#.to_string(),
            r#"text.insert={"text": "tracked"}"#.to_string(),
            r#"review.newComment={"text": "note"}"#.to_string(),
            format!("file.saveAs={}", serde_json::json!({"path": out})),
        ],
    );
    let after = wordcraft_engine::cmd::now_iso();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let xml = std::fs::read_to_string(&out).unwrap();
    assert!(xml.contains(r#"<dcterms:modified xsi:type="dcterms:W3CDTF">2026-10-05T14:03:00Z</dcterms:modified>"#), "the file's date is pinned");

    let dates: Vec<&str> = xml.split(r#"w:date=""#).skip(1).filter_map(|rest| rest.split('"').next()).collect();
    assert!(xml.contains("<w:ins ") && xml.contains("<w:comment "), "a tracked insertion and a comment were saved");
    assert!(dates.len() >= 2, "both carry a date: {dates:?}");
    for date in dates {
        assert!((before.as_str()..=after.as_str()).contains(&date), "{date} is the clock's, between {before} and {after}");
    }
}

/// A value that is set and cannot be used is said out loud, and the run still succeeds: the
/// files take the clock's date, as they would with the variable unset (noted in review).
#[test]
fn an_unusable_epoch_is_reported_on_stderr() {
    let dir = scratch("epoch-warning");
    let save = |name: &str| format!("file.exportPdf={}", serde_json::json!({"path": dir.join(name)}));
    let run = cli("later", &[save("later.pdf")]);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains(r#"warning: SOURCE_DATE_EPOCH="later" is not a whole number of seconds"#), "{stderr}");
    let pdf = std::fs::read(dir.join("later.pdf")).unwrap();
    let pinned = b"/CreationDate(D:20261005140300";
    assert!(pdf.windows(b"/CreationDate(D:".len()).any(|w| w == b"/CreationDate(D:"), "the export has a creation date");
    assert!(!pdf.windows(pinned.len()).any(|w| w == pinned), "and it is the clock's, not a pinned one");

    for quiet in ["1791208980", ""] {
        let run = cli(quiet, &[save("quiet.pdf")]);
        assert!(run.status.success());
        assert!(!String::from_utf8_lossy(&run.stderr).contains("SOURCE_DATE_EPOCH"), "{quiet:?} says nothing");
    }
}

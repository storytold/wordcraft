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

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

fn check_creation_date(created: &str, expected: &str) {
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("wordcraft-pdf-date-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
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

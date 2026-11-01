//! `convert --report` writes a JSON sidecar for a PDF conversion, and only then.
use std::process::Command;
use wordcraft_doc::{CharProps, Document, Paragraph, para_block};

fn doc_with_named_font(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::Builder::new().prefix(&format!("wc-cli-report-{tag}-")).tempdir().unwrap();
    let mut doc = Document::new();
    doc.body = vec![para_block(Paragraph::with_text("Report me", CharProps { font: Some("Source Sans 3".into()), ..Default::default() }))];
    let input = dir.path().join("in.json");
    std::fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    (dir, input)
}

fn convert(input: &std::path::Path, out: &std::path::Path, report: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    cmd.arg("convert").arg(input).arg(out);
    if report {
        cmd.arg("--report");
    }
    // The sidecar assertions below assume no epoch is in effect; a build machine
    // may export one for the whole build, so the child never inherits it. The
    // epoch case sets it explicitly in its own test.
    cmd.env_remove("SOURCE_DATE_EPOCH");
    cmd.output().unwrap()
}

#[test]
fn convert_report_writes_the_sidecar() {
    let (dir, input) = doc_with_named_font("sidecar");
    let out = dir.path().join("out.pdf");
    let run = convert(&input, &out, true);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("out.pdf.report.json")).unwrap()).unwrap();
    assert_eq!(report["format"], "pdf");
    assert_eq!(report["pages"], 1);
    assert_eq!(report["pdfVersion"], "1.7");
    assert_eq!(report["tagged"], true);
    assert_eq!(report["pdfUa"], false);
    assert!(report["sourceDateEpoch"].is_null());
    assert_eq!(report["requestedFonts"], serde_json::json!(["Source Sans 3"]));
    let embedded = report["embeddedFonts"].as_array().unwrap();
    assert!(embedded.iter().any(|f| f == "Source Sans 3"), "embedded fonts: {embedded:?}");
}

#[test]
fn convert_report_records_the_epoch_in_effect() {
    let (dir, input) = doc_with_named_font("epoch");
    let out = dir.path().join("epoch.pdf");
    let run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
        .arg("convert")
        .arg(&input)
        .arg(&out)
        .arg("--report")
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("epoch.pdf.report.json")).unwrap()).unwrap();
    assert_eq!(report["sourceDateEpoch"], 1_700_000_000);
}

#[test]
fn convert_report_names_an_unembeddable_font_as_requested_only() {
    let dir = tempfile::Builder::new().prefix("wc-cli-report-missing-").tempdir().unwrap();
    let mut doc = Document::new();
    doc.body = vec![para_block(Paragraph::with_text(
        "Set in a font this machine does not have",
        CharProps { font: Some("Fictional Display Pro".into()), ..Default::default() },
    ))];
    let input = dir.path().join("in.json");
    std::fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let out = dir.path().join("out.pdf");
    let run = convert(&input, &out, true);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("out.pdf.report.json")).unwrap()).unwrap();
    assert_eq!(report["requestedFonts"], serde_json::json!(["Fictional Display Pro"]));
    let embedded = report["embeddedFonts"].as_array().unwrap();
    assert!(!embedded.iter().any(|f| f == "Fictional Display Pro"), "embedded fonts: {embedded:?}");
    assert!(!embedded.is_empty(), "a substitute font is embedded in its place");
}

#[test]
fn convert_without_report_writes_no_sidecar() {
    let (dir, input) = doc_with_named_font("plain");
    let out = dir.path().join("plain.pdf");
    let run = convert(&input, &out, false);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(!dir.path().join("plain.pdf.report.json").exists());
}

#[test]
fn report_is_refused_for_non_pdf_output() {
    let (dir, input) = doc_with_named_font("refused");
    let out = dir.path().join("out.txt");
    let run = convert(&input, &out, true);
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("--report is only for PDF output"));
}

#[cfg(unix)]
#[test]
fn convert_report_replaces_an_existing_sidecar_symlink() {
    let (dir, input) = doc_with_named_font("symlink");
    let out = dir.path().join("out.pdf");
    let sidecar = dir.path().join("out.pdf.report.json");
    let unrelated = dir.path().join("unrelated.txt");
    std::fs::write(&unrelated, "Keep this file intact.").unwrap();
    std::os::unix::fs::symlink(&unrelated, &sidecar).unwrap();

    let run = convert(&input, &out, true);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "Keep this file intact.");
    assert!(!std::fs::symlink_metadata(&sidecar).unwrap().file_type().is_symlink());
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
    assert_eq!(report["format"], "pdf");
    assert_eq!(report["pages"], 1);
}

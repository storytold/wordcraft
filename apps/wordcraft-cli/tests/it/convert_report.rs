//! `convert --report` writes a JSON sidecar for a PDF conversion, and only then.
use std::process::Command;
use wordcraft_doc::{CharProps, Document, Paragraph, para_block};

fn doc_with_named_font(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("wc-cli-report-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut doc = Document::new();
    doc.body = vec![para_block(Paragraph::with_text("Report me", CharProps { font: Some("Source Sans 3".into()), ..Default::default() }))];
    let input = dir.join("in.json");
    std::fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    (dir, input)
}

fn convert(input: &std::path::Path, out: &std::path::Path, report: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    cmd.arg("convert").arg(input).arg(out);
    if report {
        cmd.arg("--report");
    }
    cmd.output().unwrap()
}

#[test]
fn convert_report_writes_the_sidecar() {
    let (dir, input) = doc_with_named_font("sidecar");
    let out = dir.join("out.pdf");
    let run = convert(&input, &out, true);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("out.pdf.report.json")).unwrap()).unwrap();
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
fn convert_without_report_writes_no_sidecar() {
    let (dir, input) = doc_with_named_font("plain");
    let out = dir.join("plain.pdf");
    let run = convert(&input, &out, false);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(!dir.join("plain.pdf.report.json").exists());
}

#[test]
fn report_is_refused_for_non_pdf_output() {
    let (dir, input) = doc_with_named_font("refused");
    let out = dir.join("out.txt");
    let run = convert(&input, &out, true);
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("--report is only for PDF output"));
}

//! `wordcraft-cli run` with documents opened along the way: `file.new` and `file.open` open
//! another document and make it active, and `--save` saves the active one.

use std::path::Path;
use std::process::{Command, Output};

fn run(args: &[&str], out: &Path) -> Output {
    let mut cli = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    cli.arg("run").args(args).arg("--save").arg(out);
    cli.output().unwrap()
}

#[test]
fn run_saves_the_document_opened_last() {
    let dir = std::env::temp_dir().join(format!("wordcraft-cli-documents-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, out) = (dir.join("opened.txt"), dir.join("out.txt"));
    std::fs::write(&src, "Opened\n").unwrap();
    let open = format!(r#"file.open={{"path":{}}}"#, serde_json::json!(src.to_string_lossy()));

    // A script that starts a new document and types into it saves that document.
    let made = run(&["--template", "sample", "--cmd", "file.new", "--cmd", r#"text.insert={"text":"Fresh"}"#], &out);
    let fresh = std::fs::read_to_string(&out).unwrap_or_default();
    // A file opened after typing is what is saved; the typed document stays out of it.
    let opened = run(&["--template", "blank", "--cmd", r#"text.insert={"text":"Typed"}"#, "--cmd", &open, "--cmd", "caret.docEnd"], &out);
    let reopened = std::fs::read_to_string(&out).unwrap_or_default();
    // A run starts with one open document.
    let mut cli = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    let listed = cli.args(["run", "--template", "sample", "--print", "--cmd", "view.switchWindows"]).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(made.status.success(), "stderr: {}", String::from_utf8_lossy(&made.stderr));
    assert_eq!(fresh.trim(), "Fresh");
    assert!(opened.status.success(), "stderr: {}", String::from_utf8_lossy(&opened.stderr));
    assert_eq!(reopened.trim(), "Opened");
    assert!(listed.status.success(), "stderr: {}", String::from_utf8_lossy(&listed.stderr));
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert_eq!(stdout.matches("\"id\"").count(), 1, "{stdout}");
}

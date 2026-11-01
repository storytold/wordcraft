//! #866: `wordcraft-cli text` omits tracked deletions (it does not go through `document.text`).

use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
}

#[test]
fn text_omits_tracked_deletions() {
    let dir = std::env::temp_dir().join(format!("wc-cli-866-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.docx");
    let path_s = path.to_str().unwrap();

    let make = cli()
        .args([
            "run",
            "--save",
            path_s,
            "--cmd",
            r#"text.insert={"text":"Signed by Alice Smith, Partner."}"#,
            "--cmd",
            r#"review.trackChanges={"value":true}"#,
            "--cmd",
            r#"select.text={"text":"Alice Smith, Partner"}"#,
            "--cmd",
            r#"text.insert={"text":"Bob Jones, Director"}"#,
        ])
        .output()
        .expect("run");
    assert!(make.status.success(), "run failed: {}", String::from_utf8_lossy(&make.stderr));

    let out = cli().args(["text", path_s]).output().expect("text");
    assert!(out.status.success(), "text failed: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim_end(), "Signed by Bob Jones, Director.");
    assert!(!stdout.contains("Alice Smith"), "{stdout}");

    let _ = std::fs::remove_dir_all(&dir);
}

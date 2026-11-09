//! #6055: the JSON that `info`, `inspect` and `run --print` write holds text from the file (a
//! title, the document's words). Bidi controls, C1 controls and DEL are written as `\uXXXX`
//! escapes, so a file can't reorder or drive the terminal; the JSON reads back to the same text.

use std::process::{Command, Output};

use serde_json::Value;

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).args(args).output().unwrap()
}

/// Every character a terminal could act on or that reorders what it shows.
const HOSTILE: &str = "A\u{202e}B\u{2066}C\u{9b}D\u{85}E\u{7f}F\u{1b}[2JG\u{2028}H\u{200f}I\u{61c}J";
/// Text that must stay readable as it is.
const PLAIN: &str = "Café — “quoted” 日本語";

fn assert_terminal_safe(stdout: &str, what: &str) {
    for c in stdout.chars() {
        let hostile =
            c.is_control() && c != '\n' || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
        assert!(!hostile, "{what}: raw U+{:04X} in the output: {stdout:?}", c as u32);
    }
}

fn contains_string(v: &Value, want: &str) -> bool {
    match v {
        Value::String(s) => s == want,
        Value::Array(a) => a.iter().any(|x| contains_string(x, want)),
        Value::Object(o) => o.values().any(|x| contains_string(x, want)),
        _ => false,
    }
}

#[test]
fn json_output_from_a_file_has_no_raw_terminal_controls() {
    let dir = std::env::temp_dir().join(format!("wc-cli-6055-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.docx");
    let path_s = path.to_str().unwrap();
    let title = format!("{HOSTILE} {PLAIN}");
    // A DOCX can't hold ESC (not a character of XML 1.0): the writer leaves it out.
    let stored = title.replace('\u{1b}', "");
    let make = cli(&[
        "run",
        "--save",
        path_s,
        "--cmd",
        &format!("text.insert={}", serde_json::json!({"text": title})),
        "--cmd",
        &format!("file.properties={}", serde_json::json!({"title": title})),
    ]);
    assert!(make.status.success(), "run failed: {}", String::from_utf8_lossy(&make.stderr));

    for args in [vec!["info", path_s], vec!["inspect", path_s]] {
        let out = cli(&args);
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert_terminal_safe(&stdout, args[0]);
        // The same value comes back from a JSON parser, and ordinary text isn't escaped.
        let parsed: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{}: not JSON ({e}): {stdout}", args[0]));
        assert!(contains_string(&parsed, &stored), "{}: the text doesn't read back unchanged: {stdout}", args[0]);
        assert!(stdout.contains(PLAIN), "{}: ordinary text was escaped: {stdout}", args[0]);
    }

    let out = cli(&["run", "--file", path_s, "--print", "--cmd", "file.info"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_terminal_safe(&stdout, "run --print");
    let json = stdout.strip_prefix("file.info: ").unwrap_or_else(|| panic!("unexpected line: {stdout:?}"));
    let parsed: Value = serde_json::from_str(json.trim_end()).unwrap();
    assert!(contains_string(&parsed, &stored), "run --print: the text doesn't read back unchanged: {stdout}");
    assert!(stdout.contains(PLAIN), "run --print: ordinary text was escaped: {stdout}");

    let _ = std::fs::remove_dir_all(&dir);
}

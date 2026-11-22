//! A write's `wrote <path>` confirmation is a result, so it goes to stdout, where a script that
//! captures the output finds it; stderr keeps errors and diagnostics (#7070).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wc-cli-wrote-{name}-{}", std::process::id()));
    // A failed run leaves its directory behind (cleanup runs only after the assertions), and a
    // later process can reuse the PID: start clean so a stale file cannot pass a check.
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The confirmation is on stdout and not on stderr, and the file is there.
fn assert_wrote_on_stdout(run: &Output, out: &Path) {
    let (stdout, stderr) = (String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    assert!(run.status.success(), "status {:?}, stderr: {stderr}", run.status);
    assert!(out.is_file(), "{} was not written", out.display());
    let line = format!("wrote {}", out.display());
    assert!(stdout.lines().any(|l| l == line), "stdout lacks `{line}`: {stdout:?}");
    assert!(!stderr.contains("wrote "), "the confirmation is on stderr too: {stderr:?}");
}

#[test]
fn convert_confirms_the_write_on_stdout() {
    let d = dir("convert");
    let (input, out) = (d.join("in.txt"), d.join("out.docx"));
    std::fs::write(&input, "Hello").unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).arg("convert").arg(&input).arg(&out).output().unwrap();
    assert_wrote_on_stdout(&run, &out);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn run_confirms_each_save_on_stdout() {
    let d = dir("run");
    let out = d.join("out.docx");
    let run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
        .args(["run", "--template", "blank", "--cmd", r#"text.insert={"text":"x"}"#, "--save"])
        .arg(&out)
        .output()
        .unwrap();
    assert_wrote_on_stdout(&run, &out);
    let _ = std::fs::remove_dir_all(&d);
}

/// A reader that has already hung up does not stop the job: every save still runs, though its
/// confirmation has nowhere to go. A PNG is written only in the confirming loop, so it shows a
/// save that was skipped.
#[test]
fn run_keeps_saving_after_stdout_closes() {
    let d = dir("closed");
    let job = d.join("job.json");
    std::fs::write(
        &job,
        r#"{"template": "blank", "commands": [{"command": "text.insert", "params": {"text": "x"}}], "save": ["first.docx", "second.png"]}"#,
    )
    .unwrap();
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).args(["run", "--script"]).arg(&job).stdout(writer).output().unwrap();
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "status {:?}, stderr: {stderr}", run.status);
    for out in ["first.docx", "second.png"] {
        assert!(d.join(out).is_file(), "{out} was not written; stderr: {stderr}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// The helper starts from a clean directory, so a file left by an earlier failed run cannot
/// satisfy a later run's existence checks.
#[test]
fn dir_resets_leftover_state() {
    let d = dir("leftover");
    std::fs::write(d.join("second.png"), b"stale").unwrap();
    let d = dir("leftover");
    assert!(!d.join("second.png").exists(), "a stale second.png survived dir()");
    let _ = std::fs::remove_dir_all(&d);
}

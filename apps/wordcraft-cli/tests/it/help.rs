//! `--help` or `-h` after any subcommand prints the usage and exits 0, before options are checked
//! or files and folders opened (#968).

use std::process::{Command, Output};

const SUBCOMMANDS: [&str; 10] = ["convert", "info", "text", "inspect", "render", "run", "commands", "parity", "mcp", "zotero"];

fn cli(args: &[&str]) -> Output {
    // `output()` gives the child no stdin, so `mcp` can't wait for a client.
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).args(args).output().unwrap()
}

fn assert_prints_usage(args: &[&str]) {
    let out = cli(args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{args:?}: status {:?}, stderr: {stderr}", out.status);
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("wordcraft-cli — "), "{args:?}: stdout: {}", String::from_utf8_lossy(&out.stdout));
    assert!(stderr.is_empty(), "{args:?}: stderr: {stderr}");
}

#[test]
fn every_subcommand_prints_usage_for_help_and_h() {
    assert_prints_usage(&["--help"]);
    assert_prints_usage(&["-h"]);
    for sub in SUBCOMMANDS {
        for flag in ["--help", "-h"] {
            assert_prints_usage(&[sub, flag]);
        }
    }
}

#[test]
fn help_comes_before_opening_files_and_folders() {
    assert_prints_usage(&["info", "missing.docx", "--help"]);
    assert_prints_usage(&["convert", "missing.docx", "out.pdf", "-h"]);
    assert_prints_usage(&["mcp", "--automation-read-root", "/definitely/not/a/folder", "--help"]);
}

#[test]
fn misspelled_options_and_unknown_commands_still_fail() {
    let out = cli(&["info", "--hepl"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("info: unknown option --hepl"));
    let out = cli(&["frobnicate", "--help"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown command `frobnicate`"));
}

//! `wordcraft-cli update --check` must print exactly one JSON object on stdout (#15).
//!
//! The fixture is a *newer* release, so the command reaches install detection and runs the
//! `dpkg-query -S` probe. A fake `dpkg-query` on `PATH` writes junk to stdout; if the probe's
//! output were inherited instead of captured, it would corrupt the JSON response.

use std::fs;
use std::process::{Command, Stdio};

/// Unix only: the fake probe is a `/bin/sh` script.
#[cfg(unix)]
#[test]
fn update_check_subprocess_stdout_is_one_json_value() {
    let dir = std::env::temp_dir().join(format!("wordcraft-update-check-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).expect("bin dir");
    let fixture = dir.join("latest.json");
    fs::write(&fixture, br#"{"tag_name":"v99.0.0","draft":false,"prerelease":false,"body":"Notes","html_url":"https://github.com/storytold/wordcraft/releases/tag/v99.0.0","assets":[]}"#)
        .expect("fixture");

    // A probe that would contaminate stdout if its output were inherited; it records that it ran.
    let marker = dir.join("probe-ran");
    let dpkg = bin.join("dpkg-query");
    fs::write(&dpkg, format!("#!/bin/sh\necho wordcraft-capture-probe\ntouch '{}'\nexit 0\n", marker.display())).expect("fake dpkg-query");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dpkg, fs::Permissions::from_mode(0o755)).expect("chmod fake dpkg-query");
    }

    let existing = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(&existing))).expect("PATH");

    let out = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
        .args(["update", "--check"])
        .env("WORDCRAFT_UPDATE_CHECK_FIXTURE", &fixture)
        .env("WORDCRAFT_UPDATE_CHECK_EXE", "/usr/bin/wordcraft")
        .env("PATH", &path)
        .env_remove("WORDCRAFT_INSTALL")
        .env_remove("FLATPAK_ID")
        .env_remove("APPIMAGE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn cli");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success() || out.status.code() == Some(0), "cli exited: {:?}", out.status);
    let trimmed = stdout.trim();
    assert!(trimmed.starts_with('{') && trimmed.ends_with('}'), "stdout must be one JSON object, got: {stdout:?}");
    let v: serde_json::Value = serde_json::from_str(trimmed).expect("stdout parses as JSON");
    assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("available"), "a newer release is offered");
    assert!(!stdout.contains("wordcraft-capture-probe"), "install probes must not leak to stdout: {stdout}");

    // Linux runs the `dpkg-query` probe for a `/usr/bin` path; the fake proves that path was taken.
    #[cfg(target_os = "linux")]
    assert!(marker.exists(), "the install-detection probe ran");
    let _ = fs::remove_dir_all(&dir);
}

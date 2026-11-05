//! `wordcraft --control` checks its port before the window opens (#1426).

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn app(args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wordcraft"))
        .args(args)
        // With no display, an app that tries to open a window fails at once instead.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        // A bad `--control` must not fall back to this port either.
        .env("WORDCRAFT_CONTROL_PORT", "7981")
        .env("WORDCRAFT_NO_PREFS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("wordcraft {args:?} was still running after 30 s: it started the app");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn a_bad_port_is_an_error_naming_it() {
    let cases: [(&[&str], &str); 8] = [
        (&["--control", "-99999999999999999999"], ", got \"-99999999999999999999\""),
        (&["--control", "abc"], ", got \"abc\""),
        (&["--control", "79811"], ", got \"79811\""),
        (&["--control", "0"], ", got \"0\""),
        (&["--control", "-5"], ", got \"-5\""),
        (&["--control", "abc", "--control", "7981"], ", got \"abc\""),
        (&["--control"], ""),
        (&["--control", "--sample"], ""),
    ];
    for (args, got) in cases {
        let out = app(args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?}: stderr: {stderr}");
        assert_eq!(stderr, format!("wordcraft: --control needs a port number from 1 to 65535{got}\n"), "{args:?}");
    }
}

#[test]
fn a_flag_after_control_is_read_as_a_flag() {
    let out = app(&["--control", "--version"]);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "status {:?}, stderr: {stderr}", out.status);
    assert!(stdout.starts_with("wordcraft "), "stdout: {stdout}");
}

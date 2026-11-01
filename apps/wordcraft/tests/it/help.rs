//! `wordcraft --help` and `-h` print the options and exit without opening a window (#968).

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn app(flag: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wordcraft"))
        .arg(flag)
        // With no display, an app that tries to open a window fails at once instead.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WORDCRAFT_CONTROL_PORT")
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
            panic!("wordcraft {flag} was still running after 30 s: it started the app");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn help_and_h_print_the_options_and_exit() {
    for flag in ["--help", "-h"] {
        let out = app(flag);
        let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "{flag}: status {:?}, stderr: {stderr}", out.status);
        assert!(stdout.starts_with("wordcraft — "), "{flag}: stdout: {stdout}");
        for option in ["--sample", "--control PORT", "--version", "-h, --help"] {
            assert!(stdout.contains(option), "{flag}: {option} missing from: {stdout}");
        }
        assert!(stderr.is_empty(), "{flag}: stderr: {stderr}");
    }
}

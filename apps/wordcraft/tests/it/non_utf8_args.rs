//! An argument that isn't valid Unicode, such as a Latin-1 file name, never panics the app (#1440).

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn app(args: &[OsString]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wordcraft"))
        .args(args)
        // With no display, an app that tries to open a window fails at once instead.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
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
            panic!("wordcraft {args:?} was still running after 30 s: it started the app");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

/// The file opens once the window is up (`open_non_unicode`, unit-tested in `main.rs`). Not on
/// macOS, where the app opens its window without a display variable and so keeps running.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_file_with_such_a_name_starts_the_app() {
    let out = app(&["--sample".into(), OsString::from_vec(b"caf\xe9.docx".to_vec())]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panic"), "stderr: {stderr}");
    // It went on to open the window, which fails without a display.
    assert!(stderr.contains("WinitEventLoop"), "stderr: {stderr}");
    assert_eq!(out.status.code(), Some(1), "stderr: {stderr}");
}

#[test]
fn a_control_port_that_isnt_unicode_is_an_error_naming_it() {
    let out = app(&["--sample".into(), "--control".into(), OsString::from_vec(b"79\xe981".to_vec())]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert_eq!(stderr, "wordcraft: argument 3 is not valid Unicode: 79\u{FFFD}81\n");
}

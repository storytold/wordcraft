//! A WordCraft started by an AppImage app inherits that app's `APPIMAGE`, which the self-update
//! would take for this copy's file and replace (#5653). The real program, started both ways.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const NOTICE: &str = "ignoring the AppImage";

/// A folder of its own for one test, removed when it ends.
struct Tmp(PathBuf);

impl Tmp {
    /// An empty folder named after `test` and this process, made afresh.
    fn new(test: &str) -> Tmp {
        let dir = std::env::temp_dir().join(format!("wordcraft-appimage-it-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }
}

impl Drop for Tmp {
    /// Removes the folder and what the test left in it.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs the real app with `args`, as an AppImage app with `appimage` mounted at `appdir` would
/// start it, and waits up to 30 s for it to exit (with no display it can't open a window).
fn app(args: &[OsString], appimage: &Path, appdir: &Path) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wordcraft"))
        .args(args)
        .env("APPIMAGE", appimage)
        .env("APPDIR", appdir)
        .env("ARGV0", "other-app")
        .env("OWD", "/")
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
    // A start-over that kept the variables would start over for ever, and end here.
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("wordcraft {args:?} was still running after 30 s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

/// The notice comes once: the program it started over as no longer sees the variables.
#[test]
fn another_programs_variables_are_dropped_once_and_the_program_carries_on() {
    let tmp = Tmp::new("dropped");
    let other = tmp.0.join("other-app.AppImage");
    std::fs::write(&other, b"another program").unwrap();
    let out = app(&["--version".into()], &other, &tmp.0);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "status {:?}, stderr: {stderr}", out.status);
    assert!(stdout.starts_with("wordcraft "), "stdout: {stdout}");
    assert_eq!(stderr.matches(NOTICE).count(), 1, "stderr: {stderr}");
    assert!(stderr.contains("other-app.AppImage"), "stderr: {stderr}");
    assert_eq!(std::fs::read(&other).unwrap(), b"another program");
}

/// Arguments reach the program it starts over as byte for byte, even one that isn't Unicode.
#[test]
fn starting_over_keeps_the_arguments() {
    let tmp = Tmp::new("args");
    let out = app(&["--sample".into(), "--control".into(), OsString::from_vec(b"79\xe981".to_vec())], &tmp.0.join("gone"), &tmp.0);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("argument 3 is not valid Unicode: 79\u{FFFD}81"), "stderr: {stderr}");
}

/// Its own image's variables stay, or the self-update couldn't find the file to replace.
#[test]
fn its_own_images_variables_stay() {
    let exe = Path::new(env!("CARGO_BIN_EXE_wordcraft")).canonicalize().unwrap();
    let mount = exe.parent().unwrap();
    let out = app(&["--version".into()], &mount.join("WordCraft.AppImage"), mount);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "status {:?}, stderr: {stderr}", out.status);
    assert!(stdout.starts_with("wordcraft "), "stdout: {stdout}");
    assert!(!stderr.contains(NOTICE), "stderr: {stderr}");
}

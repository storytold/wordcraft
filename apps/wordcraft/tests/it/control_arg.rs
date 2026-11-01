//! `wordcraft --control` checks its port before the window opens (#1426).

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn app(args: &[&str]) -> Output {
    app_env(args, &[])
}

/// The app with `env` set, and the control token and root variables otherwise unset.
fn app_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_wordcraft"));
    cmd.args(args)
        // With no display, an app that tries to open a window fails at once instead.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        // A bad `--control` must not fall back to this port either.
        .env("WORDCRAFT_CONTROL_PORT", "7981")
        .env("WORDCRAFT_NO_PREFS", "1");
    for var in ["WORDCRAFT_CONTROL_TOKEN", "WORDCRAFT_CONTROL_TOKEN_FILE", "WORDCRAFT_AUTOMATION_READ_ROOT", "WORDCRAFT_AUTOMATION_WRITE_ROOT"] {
        cmd.env_remove(var);
    }
    cmd.envs(env.iter().copied());
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
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

/// `--control-token`, `--control-token-file` and the automation roots (#5047): a missing or bad
/// value, or a token together with a token file, stops the launch with status 2 before the window
/// opens, as in PhotoCraft. (The control port comes from `WORDCRAFT_CONTROL_PORT` here.)
#[test]
fn bad_control_token_and_root_options_are_usage_errors() {
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const CONFIGURE: &str = "wordcraft: cannot configure the control channel: ";
    let cases: [(&[&str], &[(&str, &str)], String); 7] = [
        (&["--control-token"], &[], "wordcraft: --control-token needs a value\n".into()),
        (
            &["--automation-read-root", "--sample"],
            &[],
            "wordcraft: --automation-read-root needs a value; write --automation-read-root=<value> for one starting with '-'\n".into(),
        ),
        (&["--control-token=a", "--control-token=b"], &[], "wordcraft: --control-token given twice\n".into()),
        (&["--control-token", "abcd"], &[], format!("{CONFIGURE}--control-token: a control token must be exactly 64 hexadecimal digits\n")),
        (
            &["--control-token", KEY, "--control-token-file", "/t"],
            &[],
            format!("{CONFIGURE}--control-token cannot be used with --control-token-file\n"),
        ),
        (
            &["--control-token-file", "/t"],
            &[("WORDCRAFT_CONTROL_TOKEN", KEY)],
            format!("{CONFIGURE}WORDCRAFT_CONTROL_TOKEN cannot be used with --control-token-file\n"),
        ),
        (&[], &[("WORDCRAFT_AUTOMATION_WRITE_ROOT", "/definitely/not/a/folder")], format!("{CONFIGURE}cannot open the automation write root")),
    ];
    for (args, env, message) in cases {
        let out = app_env(args, env);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?} {env:?}: stderr: {stderr}");
        assert!(stderr.starts_with(&message), "{args:?} {env:?}: {stderr}");
    }
    // `--version` later on the line still wins over a bad option.
    let out = app(&["--control-token", "--version"]);
    assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).starts_with("wordcraft "), "{out:?}");
}

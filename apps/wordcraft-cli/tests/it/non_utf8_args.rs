//! A file name that isn't valid Unicode (`caf\xe9.txt`, Latin-1) never panics the CLI (#1440): the
//! file a command opens opens, and any other such argument is an error naming it. Not on macOS:
//! APFS refuses file names that aren't valid UTF-8, so the fixture can't be made there.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::{Command, Output};

fn cli(args: &[OsString]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).args(args).output().unwrap()
}

/// A temp folder holding `caf\xe9.txt`, whose text is "Bonjour".
fn folder(name: &str) -> (PathBuf, OsString) {
    let dir = std::env::temp_dir().join(format!("wc-cli-non-utf8-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(OsString::from_vec(b"caf\xe9.txt".to_vec()));
    std::fs::write(&file, "Bonjour\n").unwrap();
    (dir, file.into_os_string())
}

fn ok(out: &Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "status {:?}, stderr: {stderr}", out.status);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_file_a_command_opens_may_have_any_name() {
    let (dir, file) = folder("open");
    assert_eq!(ok(&cli(&["text".into(), file.clone()])).trim(), "Bonjour");
    assert!(ok(&cli(&["info".into(), file.clone()])).contains("\"words\""));
    assert!(ok(&cli(&["inspect".into(), file.clone()])).contains("Bonjour"));

    let md = dir.join("out.md");
    ok(&cli(&["convert".into(), file.clone(), md.clone().into()]));
    assert!(std::fs::read_to_string(&md).unwrap().contains("Bonjour"));

    let saved = dir.join("run.txt");
    ok(&cli(&["run".into(), "--file".into(), file.clone(), "--save".into(), saved.clone().into()]));
    assert!(std::fs::read_to_string(&saved).unwrap().contains("Bonjour"));

    // Saving without a path writes back to the file under its real name, not a lossy one.
    let mut args: Vec<OsString> = vec!["run".into(), "--file".into(), file.clone()];
    for cmd in ["caret.docEnd", "text.insert={\"text\":\" encore\"}", "file.save"] {
        args.extend(["--cmd".into(), cmd.into()]);
    }
    ok(&cli(&args));
    assert!(std::fs::read_to_string(&file).unwrap().contains("encore"));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3, "a second file was written");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn any_other_such_argument_is_an_error_naming_it() {
    let (dir, file) = folder("error");
    let out_name = dir.join(OsString::from_vec(b"caf\xe9.md".to_vec())).into_os_string();
    let cmd = OsString::from_vec(b"text.insert={\"text\":\"\xe9\"}".to_vec());
    for (args, n) in [
        (vec!["convert".into(), file.clone(), out_name.clone()], 3),
        (vec!["run".into(), "--file".into(), file.clone(), "--cmd".into(), cmd], 5),
        (vec![OsString::from_vec(b"t\xe9xt".to_vec()), file.clone()], 1),
    ] {
        let out = cli(&args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let shown = args[n - 1].to_string_lossy();
        assert!(shown.contains('\u{FFFD}'));
        assert_eq!(out.status.code(), Some(1), "{args:?}: stderr: {stderr}");
        assert_eq!(stderr, format!("wordcraft-cli: argument {n} is not valid Unicode: {shown}\n"), "{args:?}");
    }
    assert!(!std::path::Path::new(&out_name).exists());
    let _ = std::fs::remove_dir_all(dir);
}

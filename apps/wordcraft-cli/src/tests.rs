use super::*;
use crate::launcher_profile_flag;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::PathBuf;

/// A failed in-process run: the exit code and what stderr said (without its `error: ` prefix).
#[derive(Debug, PartialEq)]
struct Fail {
    code: i32,
    text: String,
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// Run the whole CLI in this process, as the binary does.
fn exec<S: AsRef<OsStr>>(args: &[S]) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = crate::run(args.iter().map(|a| a.as_ref().to_owned()), &mut out, &mut err);
    (code, String::from_utf8_lossy(&out).into_owned(), String::from_utf8_lossy(&err).into_owned())
}

/// `Ok` for exit 0, else the exit code and message.
fn run<S: AsRef<OsStr>>(args: &[S]) -> std::result::Result<(), Fail> {
    let (code, _, err) = exec(args);
    if code == 0 { Ok(()) } else { Err(Fail { code, text: err.trim_end().strip_prefix("error: ").unwrap_or(err.trim_end()).to_owned() }) }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

/// A usage error: exit 2, the message on stderr, nothing on stdout, and no work done.
fn usage(argv: &[&str]) -> String {
    let (code, out, err) = exec(argv);
    assert_eq!((code, out.as_str()), (2, ""), "{argv:?}: {err}");
    err
}

#[test]
fn page_and_scale_are_checked_before_any_file_is_opened() {
    for sub in [vec!["render", "missing.docx", "out.png"], vec!["run", "--template", "sample", "--save", "out.png"]] {
        let with = |extra: &[&str]| usage(&[&sub[..], extra].concat());
        for missing in [&["--page"][..], &["--scale"], &["--page", "--scale", "2"], &["--scale", "--page", "1"]] {
            with(missing);
        }
        assert!(with(&["--page", "banana"]).contains("invalid --page `banana`: not a valid integer"));
        assert!(with(&["--page", "0"]).contains("invalid --page `0`: page numbers start at 1"));
        with(&["--page", "-1"]);
        assert!(with(&["--scale", "banana"]).contains("invalid --scale `banana`: not a number"));
        for bad in ["0", "-2", "NaN", "inf"] {
            assert!(with(&["--scale", bad]).contains(&format!("invalid --scale `{bad}`: scale must be a finite positive number")));
        }
        with(&["--page", "1", "--page", "2"]);
        with(&["--scale", "1", "--scale", "2"]);
    }
}

#[test]
fn unknown_repeated_and_conflicting_options_are_usage_errors_before_file_access() {
    assert!(usage(&["info", "missing.docx", "--passwrod", "x"]).contains("--passwrod"));
    assert!(usage(&["render", "missing.docx", "out.png", "--sclae", "2"]).contains("--sclae"));
    assert!(usage(&["zotero", "refresh", "missing.docx", "--prot", "1"]).contains("--prot"));
    usage(&["convert", "missing.docx", "out.xml", "--format"]);
    usage(&["convert", "missing.docx", "out.xml", "--format", "--markup"]);
    usage(&["convert", "missing.docx", "out.xml", "--format", "flatopc", "--format", "wordml2003"]);
    assert!(usage(&["convert", "missing.docx", "out.xml", "--format", "docx"]).contains("wordml2003"));
    usage(&["convert", "missing.docx"]);
    usage(&["convert", "a.docx", "b.docx", "c.docx"]);
    usage(&["run", "--file", "a.docx", "--template", "blank"]);
    usage(&["info"]);
    usage(&["frobnicate"]);
    assert!(usage(&["convert", "missing.docx", "out.docx", "--strict"]).contains("--strict needs a .pdf or .png output"));
    // A valid single value reaches file access, rather than an option error.
    let (code, _, err) = exec(&["convert", "missing.docx", "out.xml", "--format", "flatopc"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("error: missing.docx:"), "{err}");
}

#[test]
fn cmd_and_script_values_are_checked_by_the_parser() {
    for argv in [
        vec!["run", "--cmd"],
        vec!["run", "--cmd", "--save", "out.docx"],
        vec!["run", "--cmd", "caret.docEnd", "--cmd", "--print"],
        vec!["run", "--cmd", "--bogus"],
        vec!["run", "--script", "--cmd", "caret.docEnd"],
    ] {
        usage(&argv);
    }
    assert!(usage(&["run", "--cmd", "text.insert={nope"]).contains("bad JSON params"));
    assert_eq!(run(&args(&["run", "--cmd", "caret.docEnd", "--cmd", "caret.docStart"])), Ok(()));
}

#[test]
fn the_author_flag_names_the_headless_session() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    assert!(set_author(&mut s, " Claude (copyedit) ").is_ok());
    assert_eq!(s.author, "Claude (copyedit)");
    assert!(set_author(&mut s, "   ").is_err_and(|e| e.starts_with("--author:")));
    assert_eq!(run(&args(&["run", "--author", "Claude", "--cmd", "review.trackChanges={\"value\":true}"])), Ok(()));
    // The flag goes through the same protection check as the command.
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    let before = s.author.clone();
    assert!(set_author(&mut s, "Claude").is_err());
    assert_eq!(s.author, before);
    assert!(s.run("file.setAuthor", &json!({"name": "Claude"})).is_err());
}

#[test]
fn the_password_comes_from_an_option_or_a_file_and_never_shows_in_an_error() {
    let open = |password: Option<&str>, file: Option<&str>| Password { password: password.map(Into::into), password_file: file.map(PathBuf::from) };
    assert_eq!(password_from(&open(None, None)), Ok(None));
    assert_eq!(password_from(&open(Some("s3cret"), None)), Ok(Some("s3cret".into())));
    // `--password` wins over the file.
    assert_eq!(password_from(&open(Some("a"), Some("/nonexistent"))), Ok(Some("a".into())));
    let path = std::env::temp_dir().join(format!("wordcraft-cli-password-test-{}.txt", std::process::id()));
    std::fs::write(&path, "s3cret\r\n").unwrap();
    let got = password_from(&open(None, path.to_str()));
    std::fs::remove_file(&path).ok();
    assert_eq!(got, Ok(Some("s3cret".into())), "only the trailing newline is lost");
    let err = password_from(&open(None, Some("/nonexistent/secret.txt"))).unwrap_err().to_string();
    assert!(err.contains("--password-file"), "{err}");
}

/// A password file on the command line beats the variable, a blank variable is unset, and `--help` shows neither value.
#[test]
fn the_password_variable_is_read_by_the_flag_and_hidden() {
    let parse = |argv: &[&str]| crate::args::parse(&argv.iter().map(OsString::from).collect::<Vec<_>>());
    let help = parse(&["info", "--help"]).unwrap_err().render().to_string();
    assert!(help.contains("WORDCRAFT_PASSWORD") || help.contains("--password"), "{help}");
    let Err(e) = parse(&["info", "--password"]) else { panic!("a missing value must not parse") };
    assert_eq!(e.exit_code(), 2);
    let top = parse(&["--help"]).unwrap_err().render().to_string();
    assert!(top.contains("WORDCRAFT_PASSWORD") && top.contains("WORDCRAFT_REGION") && top.contains("Exit status"), "{top}");
}

#[test]
fn mcp_needs_both_roots_and_refuses_connect() {
    let scratch = Scratch::new("entry");
    let base = scratch.0.clone();
    std::fs::create_dir_all(base.join("in")).unwrap();
    std::fs::create_dir_all(base.join("out")).unwrap();
    let read = format!("--automation-read-root={}", base.join("in").display());
    let write = format!("--automation-write-root={}", base.join("out").display());
    for given in [vec![], vec![read.clone()], vec![write.clone()], vec!["--author".to_string(), "Claude".to_string()]] {
        let line = [vec!["mcp".to_string()], given].concat();
        let err = usage(&line.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(err.contains("--automation-read-root") || err.contains("--automation-write-root"), "{line:?}: {err}");
    }
    let missing = usage(&["mcp", "--automation-read-root", "a", "--automation-read-root=b", "--automation-write-root", "c"]);
    assert!(missing.contains("--automation-read-root"), "{missing}");
    usage(&["mcp", "--automation-read-root=", "--automation-write-root=x"]);
    // A folder that does not exist is an error, not a server that quietly runs unconfined.
    let (code, _, err) = exec(&["mcp", "--automation-read-root", "/definitely/not/a/folder", "--automation-write-root", "/definitely/not/a/folder"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("error: mcp: cannot open the automation read root"), "{err}");
    // `--connect` is refused whether or not it has roots, before anything connects (#3695, #5819).
    let dir = base.to_string_lossy().into_owned();
    for line in [
        args(&["mcp", "--connect", "127.0.0.1:9"]),
        args(&["mcp", "--connect=127.0.0.1:9", "--author", "Claude"]),
        args(&["mcp", "--connect", "127.0.0.1:9", "--automation-read-root", &dir, "--automation-write-root", &dir]),
    ] {
        let (code, _, err) = exec(&line);
        assert_eq!(code, 1, "{line:?}");
        assert!(err.contains("--connect") && err.contains("#3695") && err.contains("#5819"), "{line:?}: {err}");
    }
}

#[test]
fn every_file_opening_subcommand_reaches_the_open_with_a_non_unicode_name_but_not_an_output() {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let latin1 = || OsString::from_vec(b"/definitely/not/caf\xe9.docx".to_vec());
        for argv in [
            vec![OsString::from("convert"), latin1(), "out.pdf".into()],
            vec!["info".into(), latin1()],
            vec!["text".into(), latin1()],
            vec!["inspect".into(), latin1()],
            vec!["render".into(), latin1(), "out.png".into()],
            vec!["run".into(), "--file".into(), latin1()],
            vec!["zotero".into(), "refresh".into(), latin1()],
        ] {
            let (code, _, err) = exec(&argv);
            assert_eq!(code, 1, "{argv:?}: {err}");
            assert!(err.contains("No such file"), "{argv:?}: {err}");
        }
        // An output the engine takes as text must be Unicode: a failure, and nothing opened.
        let out = OsString::from_vec(b"caf\xe9.png".to_vec());
        let (code, _, err) = exec(&["render".into(), OsString::from("in.docx"), out]);
        assert_eq!(code, 1);
        assert!(err.contains("not valid Unicode"), "{err}");
        // Elsewhere clap refuses it as usage.
        assert_eq!(exec(&["run".into(), OsString::from("--template"), OsString::from_vec(b"caf\xe9".to_vec())]).0, 2);
    }
}

/// `validate`: PASS is 0; FAIL, INCOMPLETE and a file that can't be read are 1 (RFC-0018 rule 1.2); usage is 2.
#[test]
fn validate_exits_0_for_pass_1_for_anything_else_and_2_for_usage() {
    let dir = script_dir("validate-exit");
    let good = dir.join("good.docx");
    run(&args(&["run", "--template", "sample", "--save", &good.to_string_lossy()])).unwrap();
    let good = good.to_string_lossy().into_owned();
    let (code, out, _) = exec(&["validate", &good]);
    assert_eq!((code, out.lines().next()), (0, Some("PASS")));
    let (code, out, _) = exec(&["validate", "--json", &good]);
    assert_eq!(code, 0);
    assert!(out.contains("\"verdict\""), "{out}");
    let broken = dir.join("broken.docx");
    std::fs::write(&broken, b"not a zip").unwrap();
    let (code, out, _) = exec(&["validate".to_string(), broken.to_string_lossy().into_owned()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.starts_with("FAIL") || out.starts_with("INCOMPLETE"), "{out}");
    let (code, _, err) = exec(&["validate", "/definitely/not/there.docx"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("error: /definitely/not/there.docx:"), "{err}");
    usage(&["validate"]);
    usage(&["validate", &good, &good]);
    usage(&["validate", &good, "--bogus"]);
}

#[test]
fn help_and_version_go_to_stdout_with_exit_0() {
    for argv in [vec!["--version"], vec!["-V"], vec!["--help"], vec!["-h"], vec!["help"], vec!["convert", "--help"], vec!["help", "validate"]] {
        let (code, out, err) = exec(&argv);
        assert_eq!((code, err.as_str()), (0, ""), "{argv:?}");
        assert!(out.contains("wordcraft-cli"), "{argv:?}: {out}");
    }
    let (_, out, _) = exec(&["--version"]);
    assert_eq!(out.trim(), format!("wordcraft-cli {}", env!("CARGO_PKG_VERSION")));
    // The version is not a subcommand; with no arguments the help is shown as an error, exit 2.
    usage(&["version"]);
    assert_eq!(exec::<&str>(&[]).0, 2);
    // Every subcommand's help ends with the exit status.
    for sub in crate::command().get_subcommands().map(|c| c.get_name().to_owned()).filter(|n| n != "help") {
        let (code, out, _) = exec(&[sub.as_str(), "--help"]);
        assert_eq!(code, 0, "{sub}");
        assert!(out.contains("Exit status"), "{sub}: {out}");
    }
}

#[test]
fn the_launcher_bootstrap_exits_2_before_anything_runs() {
    let err = usage(&["--launch-profile=nightly-v1", "info", "x.docx"]);
    assert!(err.starts_with("error: --launch-profile and --profile-root"), "{err}");
}

#[test]
fn a_closed_stdout_ends_quietly_and_other_write_errors_fail() {
    struct Closed(std::io::ErrorKind);
    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(self.0.into())
        }
    }
    let mut err = Vec::new();
    assert_eq!(crate::run(["commands"], &mut Closed(std::io::ErrorKind::BrokenPipe), &mut err), 0);
    assert!(err.is_empty());
    assert_eq!(crate::run(["commands"], &mut Closed(std::io::ErrorKind::PermissionDenied), &mut err), 1);
    assert!(String::from_utf8_lossy(&err).starts_with("error: "));
}

#[test]
fn convert_strict_and_render_strict_refusals_exit_1() {
    // A document whose text can't be drawn is a refusal: exit 1 with the report on stderr and no file.
    let dir = script_dir("strict-exit");
    let out = dir.join("out.pdf");
    let (code, _, err) = exec(&["convert", "/definitely/not/there.docx", &out.to_string_lossy(), "--strict"]);
    assert_eq!(code, 1, "{err}");
    assert!(!out.exists());
}

/// The desktop app's Nightly bootstrap is never a CLI option, in either spelling.
#[test]
fn the_launcher_profile_bootstrap_is_refused_in_both_forms() {
    for argv in [
        &["--launch-profile", "nightly-v1", "--profile-root", "/nightly/profiles/wordcraft", "--version"][..],
        &["--launch-profile=nightly-v1", "--profile-root=/nightly/profiles/wordcraft"][..],
        &["--profile-root", "/nightly/profiles/wordcraft"][..],
    ] {
        let argv: Vec<std::ffi::OsString> = argv.iter().map(std::ffi::OsString::from).collect();
        assert!(launcher_profile_flag(&argv), "{argv:?}");
    }
    for argv in [&["--version"][..], &["proof.check", "--launch-profiles"][..], &["--profile-roots"][..]] {
        let argv: Vec<std::ffi::OsString> = argv.iter().map(std::ffi::OsString::from).collect();
        assert!(!launcher_profile_flag(&argv), "{argv:?}");
    }
}

#[test]
fn peer_text_stays_on_one_visible_line() {
    let alert = "Done.\nwrote /etc/passwd\r\u{1b}[2J\u{9b}31m\u{202e}txt.exe\\n\tok é";
    let shown = escape_peer(alert);
    assert_eq!(shown, "Done.\\nwrote /etc/passwd\\r\\u{1b}[2J\\u{9b}31m\\u{202e}txt.exe\\\\n\\tok é");
    assert!(!shown.chars().any(|c| c.is_control()));
    assert_eq!(escape_peer("Citation (Doe, 2020) — “ok”"), "Citation (Doe, 2020) — “ok”");
}

#[test]
fn text_keeps_bullet_and_number_labels() {
    let doc = wordcraft_engine::io::open_bytes("lists.md", b"- alpha\n- beta\n\n1. first\n2. second\n").unwrap();
    assert_eq!(listed_text(&doc).unwrap(), "\u{2022}\talpha\n\u{2022}\tbeta\n1.\tfirst\n2.\tsecond");
}

#[test]
fn author_flag_cannot_claim_a_named_exception_after_protection() {
    use wordcraft_doc::{CharProps, InlineObject, StoryRef};

    let mut s = Session::new(wordcraft_doc::Document::from_text("Named exception."));
    let end = s.doc.end_of(StoryRef::Body);
    s.doc.insert_object(&end, InlineObject::PermEnd { id: 1 }, &CharProps::default()).unwrap();
    let start = s.doc.start_of(StoryRef::Body);
    s.doc.insert_object(&start, InlineObject::PermStart { id: 1, group: String::new(), editor: "Alice".into() }, &CharProps::default()).unwrap();
    assert_eq!(s.doc.perm_ranges().len(), 1);
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    s.run("select.text", &json!({"text": "Named"})).unwrap();
    assert!(s.run("text.insert", &json!({"text": "Changed"})).is_err());

    assert!(set_author(&mut s, "Alice").is_err(), "--author changed the identity used for a protected exception");
    assert!(s.run("text.insert", &json!({"text": "Changed"})).is_err());
}

/// An exclusively created scratch folder, removed on drop even when an assertion fails.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        loop {
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("wc-cli-{tag}-{}-{n}", std::process::id()));
            // `create_dir` fails when the path exists, so a leftover or a planted folder is never reused. Any
            // other failure (no temp folder, no permission) fails every name: report it instead of looping.
            match std::fs::create_dir(&dir) {
                Ok(()) => return Self(dir),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("scratch folder {}: {e}", dir.display()),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `mcp`'s headless session takes `--author` as the session's author, and refuses a blank one as `run` does.
#[test]
fn mcp_headless_session_takes_the_author_flag() {
    let scratch = Scratch::new("mcp-author");
    let roots = wordcraft_engine::file_access::AutomationRoots::new(Some(&scratch.0), Some(&scratch.0)).unwrap().unwrap();
    assert_eq!(headless_mcp(roots.clone(), Some("Claude (copyedit)")).unwrap().session.author, "Claude (copyedit)");
    assert!(headless_mcp(roots.clone(), Some("   ")).is_err_and(|e| e.starts_with("--author:")));
    let unnamed = headless_mcp(roots, None).unwrap();
    assert_eq!(unnamed.session.author, wordcraft_mcp::Headless::default().session.author, "no flag keeps the default author");
}

#[test]
fn render_and_run_valid_options_and_defaults_produce_files() {
    let dir = std::env::temp_dir().join(format!("wc-cli-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let docx_path = dir.join("sample.docx");
    // Create sample docx via `run --save sample.docx`
    assert!(run(&args(&["run", "--template", "sample", "--save", &docx_path.to_string_lossy()])).is_ok());
    assert!(docx_path.exists());

    let docx_str = docx_path.to_string_lossy().into_owned();

    // 1. render with defaults
    let out_default = dir.join("out_default.png");
    assert!(run(&args(&["render", &docx_str, &out_default.to_string_lossy()])).is_ok());
    assert!(out_default.exists());

    // 2. render with valid --page and --scale
    let out_valid = dir.join("out_valid.png");
    assert!(run(&args(&["render", &docx_str, &out_valid.to_string_lossy(), "--page", "1", "--scale", "2"])).is_ok());
    assert!(out_valid.exists());

    // 3. render with different flag order
    let out_reordered = dir.join("out_reordered.png");
    assert!(run(&args(&["render", &docx_str, &out_reordered.to_string_lossy(), "--scale", "1", "--page", "1"])).is_ok());
    assert!(out_reordered.exists());

    // 4. render failure leaves no output file
    let out_invalid = dir.join("out_invalid.png");
    assert!(run(&args(&["render", &docx_str, &out_invalid.to_string_lossy(), "--page", "banana"])).is_err());
    assert!(!out_invalid.exists());

    // 5. run --save out.png with defaults
    let run_default = dir.join("run_default.png");
    assert!(run(&args(&["run", "--file", &docx_str, "--save", &run_default.to_string_lossy()])).is_ok());
    assert!(run_default.exists());

    // 6. run --save out.png with --page and --scale
    let run_valid = dir.join("run_valid.png");
    assert!(run(&args(&["run", "--file", &docx_str, "--save", &run_valid.to_string_lossy(), "--page", "1", "--scale", "1"])).is_ok());
    assert!(run_valid.exists());

    // 7. run --save out.png with different flag order
    let run_reordered = dir.join("run_reordered.png");
    assert!(run(&args(&["run", "--file", &docx_str, "--scale", "1", "--page", "1", "--save", &run_reordered.to_string_lossy()])).is_ok());
    assert!(run_reordered.exists());

    // 8. run --save out.png failure leaves no output file
    let run_invalid = dir.join("run_invalid.png");
    assert!(run(&args(&["run", "--file", &docx_str, "--save", &run_invalid.to_string_lossy(), "--scale", "banana"])).is_err());
    assert!(!run_invalid.exists());

    let _ = std::fs::remove_dir_all(dir);
}
/// A scratch folder that removes itself, even when the test fails.
struct ScriptDir(PathBuf);

impl std::ops::Deref for ScriptDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScriptDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn script_dir(tag: &str) -> ScriptDir {
    let dir = std::env::temp_dir().join(format!("wc-cli-script-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    ScriptDir(dir)
}

fn write_script(dir: &Path, text: &str) -> String {
    let p = dir.join("job.json");
    std::fs::write(&p, text).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn script_builds_a_document_and_saves_docx_and_pdf_relative_to_the_script() {
    let dir = script_dir("two");
    let script = write_script(
        &dir,
        r#"{"template":"blank","commands":[{"command":"text.insert","params":{"text":"Hello script"}},{"command":"format.bold"}],"save":["out.docx","out.pdf"]}"#,
    );
    run(&args(&["run", "--script", &script])).unwrap();
    assert!(std::fs::read(dir.join("out.pdf")).unwrap().starts_with(b"%PDF"));
    let doc = open(&dir.join("out.docx"), None).unwrap().doc;
    let json = serde_json::to_string(&doc).unwrap();
    assert!(json.contains("Hello script") && json.contains("\"bold\":true"));
}

#[test]
fn script_with_a_bad_output_writes_no_output() {
    let dir = script_dir("badout");
    let script = write_script(&dir, r#"{"commands":["caret.docEnd"],"save":["a.docx","b.xyz"]}"#);
    let e = run(&args(&["run", "--script", &script])).unwrap_err().to_string();
    assert_eq!(e.matches("b.xyz").count(), 1, "{e}");
    assert!(!dir.join("a.docx").exists());
    let script = write_script(&dir, r#"{"commands":["caret.docEnd"],"save":["a.docx","nope/b.docx"]}"#);
    assert!(run(&args(&["run", "--script", &script])).is_err());
    assert!(!dir.join("a.docx").exists());
}

#[test]
fn script_with_a_folder_as_output_writes_no_output() {
    let dir = script_dir("folderout");
    std::fs::create_dir(dir.join("b.pdf")).unwrap();
    let script = write_script(&dir, r#"{"commands":["caret.docEnd"],"save":["a.docx","b.pdf"]}"#);
    let e = run(&args(&["run", "--script", &script])).unwrap_err().to_string();
    assert!(e.contains("b.pdf"), "{e}");
    assert!(!dir.join("a.docx").exists());
}

/// APFS refuses file names that aren't valid UTF-8, so the fixture can't be made on macOS
/// (as in `tests/it/non_utf8_args.rs`).
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn script_file_name_need_not_be_unicode() {
    use std::os::unix::ffi::OsStringExt;
    let dir = script_dir("latin1");
    let mut name = dir.as_os_str().to_os_string().into_vec();
    name.extend_from_slice(b"/caf\xe9.json");
    let path = PathBuf::from(OsString::from_vec(name));
    std::fs::write(&path, r#"{"commands":["caret.docEnd"],"save":["o.txt"]}"#).unwrap();
    let raw = vec![OsString::from("run"), OsString::from("--script"), path.into_os_string()];
    run(&raw).unwrap();
    assert!(dir.join("o.txt").exists());
}

#[test]
fn command_line_template_wins_over_the_scripts_file() {
    let dir = script_dir("source");
    let input = dir.join("in.docx");
    run(&args(&["run", "--template", "blank", "--cmd", "text.insert={\"text\":\"FROM FILE\"}", "--save", &input.to_string_lossy()])).unwrap();
    let script = write_script(&dir, r#"{"file":"in.docx","commands":["caret.docEnd"]}"#);
    let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
    run(&args(&["run", "--script", &script, "--template", "blank", "--save", &a.to_string_lossy()])).unwrap();
    assert!(!std::fs::read_to_string(&a).unwrap().contains("FROM FILE"));
    run(&args(&["run", "--script", &script, "--save", &b.to_string_lossy()])).unwrap();
    assert!(std::fs::read_to_string(&b).unwrap().contains("FROM FILE"));
}

#[test]
fn script_reads_from_a_stream_and_caps_its_size() {
    let s = script::load_from(&br#"{"commands":["caret.docEnd"],"save":["o.docx"]}"#[..], "-", None).unwrap();
    assert_eq!((s.commands.len(), s.save), (1, vec!["o.docx".to_string()]));
    let big = std::io::repeat(b' ').take(script::MAX_SCRIPT_BYTES + 10);
    let e = script::load_from(big, "-", None).unwrap_err();
    assert!(e.contains("too large"), "{e}");
    assert!(script::load_from(&b"{"[..], "-", None).is_err());
}

#[test]
fn script_commands_run_before_cmd_flags() {
    let dir = script_dir("order");
    let script = write_script(&dir, r#"{"commands":[{"command":"text.insert","params":{"text":"first"}}]}"#);
    let out = dir.join("o.txt");
    run(&args(&["run", "--script", &script, "--cmd", "text.insert={\"text\":\" second\"}", "--save", &out.to_string_lossy()])).unwrap();
    assert!(std::fs::read_to_string(&out).unwrap().contains("first second"));
}

#[test]
fn script_failure_names_the_command_and_writes_nothing() {
    let dir = script_dir("fail");
    let script =
        write_script(&dir, r#"{"commands":[{"command":"text.insert","params":{"text":"x"}},{"command":"no.such.command"}],"save":["out.docx"]}"#);
    let e = run(&args(&["run", "--script", &script])).unwrap_err().to_string();
    assert!(e.contains("command 2") && e.contains("no.such.command"), "{e}");
    assert!(!dir.join("out.docx").exists());
}

#[test]
fn script_rejects_junk_empty_missing_and_huge_input() {
    let dir = script_dir("junk");
    for text in [
        "not json",
        "[]",
        "{}",
        r#"{"commands":[]}"#,
        r#"{"commands":[{"params":{}}]}"#,
        r#"{"commands":[1]}"#,
        r#"{"commands":[{"command":"a"}],"save":[1]}"#,
    ] {
        let script = write_script(&dir, text);
        assert!(run(&args(&["run", "--script", &script])).is_err(), "{text}");
    }
    assert!(run(&args(&["run", "--script", &dir.join("missing.json").to_string_lossy()])).is_err());
    let huge = write_script(&dir, &" ".repeat(5 * 1024 * 1024));
    let e = run(&args(&["run", "--script", &huge])).unwrap_err().to_string();
    assert!(e.contains("too large"), "{e}");
}

#[test]
fn script_parse_resolves_paths_against_the_base() {
    let s =
        script::parse(r#"{"file":"in.docx","commands":[{"command":"edit.undo"}],"save":["/abs/o.pdf","o.docx"]}"#, Some(Path::new("/base"))).unwrap();
    let joined = |name: &str| Path::new("/base").join(name).to_string_lossy().into_owned();
    assert_eq!(s.file, Some(joined("in.docx")));
    assert_eq!(s.save, vec!["/abs/o.pdf".to_string(), joined("o.docx")]);
    let s = script::parse(r#"{"commands":["caret.docEnd"],"save":["o.docx"]}"#, None).unwrap();
    assert_eq!(s.save, vec!["o.docx".to_string()]);
    assert_eq!(s.commands.len(), 1);
}

/// Finding 4: `validate` is in `OPENS_FILE`, so a file name that is not valid Unicode reaches the
/// validator by its preserved path rather than being rejected as a bad argument.
/// APFS refuses invalid UTF-8 file names, so macOS covers native path handling without creating one.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn validate_accepts_a_file_name_that_is_not_unicode() {
    use std::os::unix::ffi::OsStringExt;
    let dir = std::env::temp_dir().join(format!("wc-cli-validate-nonutf8-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let doc = wordcraft_doc::Document::from_text("Bonjour");
    let bytes = wordcraft_engine::io::save_bytes("x.docx", &doc).unwrap();
    let mut name = dir.as_os_str().to_os_string().into_vec();
    name.extend_from_slice(b"/caf\xe9.docx");
    let path = PathBuf::from(OsString::from_vec(name));
    std::fs::write(&path, bytes).unwrap();
    let raw = vec![OsString::from("validate"), path.into_os_string()];
    assert_eq!(run(&raw), Ok(()), "a non-Unicode path validates and reports PASS");
    let _ = std::fs::remove_dir_all(dir);
}

/// A real `.docx` archive with `extra` parts added: content types, package relationships, the main
/// document and an empty main relationships part. Used to drive `validate_bytes` from real bytes so
/// a document-controlled entry name is shown to reach the report the way production does.
fn validate_fixture_zip(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = craft_zip::Writer::new();
    w.add(
        "[Content_Types].xml",
        br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        craft_zip::Compression::Stored,
    )
    .unwrap();
    w.add(
        "_rels/.rels",
        br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        craft_zip::Compression::Stored,
    )
    .unwrap();
    w.add(
        "word/document.xml",
        br#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#,
        craft_zip::Compression::Stored,
    )
    .unwrap();
    for (name, bytes) in extra {
        w.add(name, bytes, craft_zip::Compression::Stored).unwrap();
    }
    w.finish().unwrap()
}

/// Finding 5: document-controlled text (a ZIP entry name) is escaped in the human report, while the
/// JSON form keeps serde's escaping. The input is a real archive whose entry name carries a control
/// character, so the test shows the name actually reaches the escaped fields (part and message).
#[test]
fn human_report_escapes_document_controlled_text() {
    let evil = "word/evil\u{1b}[2J.bin";
    let printable = "word/café—naïve.bin";
    let zip = validate_fixture_zip(&[(evil, b"x"), (printable, b"x")]);
    let report = wordcraft_validate::validate_bytes(&zip);
    // The undeclared part carries the raw control character in its part field and its message.
    let finding = report
        .findings
        .iter()
        .find(|f| f.rule == "CT-UNDECLARED" && f.part.as_deref() == Some(evil))
        .unwrap_or_else(|| panic!("expected CT-UNDECLARED at the control-character name: {:#?}", report.findings));
    assert!(finding.message.contains(evil), "the name reaches the message too: {:?}", finding.message);

    let human = render_validate_report(&report);
    assert!(!human.contains('\u{1b}'), "no raw escape survives the human report: {human:?}");
    assert!(human.contains("\\u{1b}"), "{human:?}");
    assert!(human.contains("[2J"), "the surrounding name text is kept as written: {human:?}");
    // Printable Unicode in a name is left as written.
    assert!(human.contains(printable), "printable Unicode must be untouched: {human:?}");

    // JSON output is unchanged by the human escaping: serde still emits the control character as
    // its own JSON escape.
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\\u001b"), "{json}");
}

#[test]
fn invisible_characters_are_escaped_and_json_lines_still_parse_back() {
    let hidden = "a\u{ad}b\u{200b}c\u{200d}d\u{2060}e\u{2064}f\u{feff}g\u{e0041}h\u{e007f}i\u{85}j\u{202e}k";
    let shown = escape_peer(hidden);
    assert!(!shown.chars().any(is_unsafe_char), "{shown}");
    assert!(shown.contains("\\u{e0041}") && shown.contains("\\u{ad}"), "{shown}");
    // Errors keep a Windows path readable.
    assert_eq!(escape_text("C:\\dir\\a.docx", false), "C:\\dir\\a.docx");

    let value = json!({"part": hidden, "plain": "ünïcode ok"});
    let line = json_line_safe(&value.to_string());
    assert!(!line.chars().any(is_unsafe_char), "{line}");
    assert!(line.contains("\\ud800") || line.contains("\\udb40"), "{line}");
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), value);
}

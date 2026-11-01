use super::*;
use std::io::Read;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
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
fn validators_accept_only_the_documented_options() {
    assert!(validate_options("info", &args(&["file.docx"]), PASSWORD_OPTIONS).is_ok());
    assert!(validate_options("render", &args(&["in.docx", "out.png", "--page", "2", "--scale", "1.5"]), RENDER_OPTIONS).is_ok());
    assert!(validate_options("run", &args(&["--cmd", "text.insert={}", "--cmd", "edit.undo", "--print"]), RUN_OPTIONS).is_ok());
    assert!(validate_options("commands", &args(&["--json"]), COMMANDS_OPTIONS).is_ok());
    assert!(validate_options("parity", &args(&["--markdown"]), PARITY_OPTIONS).is_ok());
    assert!(validate_options("mcp", &args(&["--connect", "127.0.0.1:9000"]), MCP_OPTIONS).is_ok());
    assert!(validate_options("render", &args(&["in.docx", "out.png", "--page"]), RENDER_OPTIONS).is_ok());
    assert!(
        validate_options(
            "zotero",
            &args(&["refresh", "in.docx", "--cmd", "caret.docEnd", "--save", "out.docx", "--trace", "--port", "23119"]),
            ZOTERO_OPTIONS
        )
        .is_ok()
    );
}

#[test]
fn an_option_value_is_never_read_as_a_path() {
    // `--password`'s value sits between the paths and would be taken for one without
    // `positionals`, so `render` would write its picture over a file called after the password.
    assert_eq!(positionals(&args(&["in.docx", "--password", "secret", "out.png"]), RENDER_OPTIONS), vec!["in.docx", "out.png"]);
    assert_eq!(positionals(&args(&["in.docx", "out.png", "--page", "2"]), RENDER_OPTIONS), vec!["in.docx", "out.png"]);
    // A bare flag takes no value, so the next argument stays a path.
    assert_eq!(positionals(&args(&["--print", "in.docx"]), RUN_OPTIONS), vec!["in.docx"]);
    // `convert` is the one with a value-taking flag between the two paths.
    assert_eq!(positionals(&args(&["in.docx", "--format", "flatopc", "out.docx"]), CONVERT_OPTIONS), vec!["in.docx", "out.docx"]);
}

#[test]
fn the_password_comes_from_an_option_a_file_or_the_environment() {
    assert_eq!(password_from(&args(&["f.docx"])), Ok(None));
    assert_eq!(password_from(&args(&["f.docx", "--password", "s3cret"])), Ok(Some("s3cret".into())));
    // `--password` wins over the other two sources.
    assert_eq!(password_from(&args(&["--password", "a", "--password-file", "/nonexistent"])), Ok(Some("a".into())));
}

#[test]
fn a_password_file_loses_only_its_trailing_newline() {
    let path = std::env::temp_dir().join("wordcraft-cli-password-test.txt");
    std::fs::write(&path, "s3cret\r\n").unwrap();
    let got = password_from(&args(&["f.docx", "--password-file", &path.to_string_lossy()]));
    std::fs::remove_file(&path).ok();
    assert_eq!(got, Ok(Some("s3cret".into())));
}

#[test]
fn a_missing_password_file_is_named_without_the_password() {
    let err = password_from(&args(&["f.docx", "--password-file", "/nonexistent/secret.txt"])).unwrap_err();
    assert!(err.contains("--password-file"), "{err}");
}

#[test]
fn unknown_options_are_rejected_before_file_access() {
    assert_eq!(run(&args(&["info", "missing.docx", "--passwrod", "x"])), Err("info: unknown option --passwrod".into()));
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--sclae", "2"])), Err("render: unknown option --sclae".into()));
    assert_eq!(run(&args(&["zotero", "refresh", "missing.docx", "--prot", "1"])), Err("zotero: unknown option --prot".into()));
}

#[test]
fn help_or_h_after_a_subcommand_asks_for_the_usage() {
    for cmd in SUBCOMMANDS {
        assert!(wants_help(cmd, &args(&["in.docx", "--help"])), "{cmd}");
        assert!(wants_help(cmd, &args(&["-h"])), "{cmd}");
        assert!(!wants_help(cmd, &args(&["in.docx", "--hepl"])), "{cmd}");
    }
    assert!(!wants_help("frobnicate", &args(&["--help"])));
}

#[test]
fn usage_lists_exactly_the_subcommands() {
    let listed: Vec<&str> = USAGE
        .lines()
        .filter_map(|line| line.strip_prefix("  wordcraft-cli ")?.split_whitespace().next())
        .filter(|word| !word.starts_with(['-', '[']))
        .collect();
    assert_eq!(listed, SUBCOMMANDS);
}

#[test]
fn text_keeps_bullet_and_number_labels() {
    let doc = wordcraft_engine::io::open_bytes("lists.md", b"- alpha\n- beta\n\n1. first\n2. second\n").unwrap();
    assert_eq!(listed_text(&doc).unwrap(), "\u{2022}\talpha\n\u{2022}\tbeta\n1.\tfirst\n2.\tsecond");
}

#[test]
fn author_flag_names_the_headless_session() {
    assert!(validate_options("run", &args(&["--author", "Claude (copyedit)", "--cmd", "edit.undo"]), RUN_OPTIONS).is_ok());
    assert!(validate_options("mcp", &args(&["--author", "Claude"]), MCP_OPTIONS).is_ok());
    let mut s = Session::new(wordcraft_doc::Document::new());
    assert!(set_author(&mut s, " Claude (copyedit) ").is_ok());
    assert_eq!(s.author, "Claude (copyedit)");
    assert!(set_author(&mut s, "   ").is_err_and(|e| e.starts_with("--author:")));
    assert!(run(&args(&["run", "--author", "Claude", "--cmd", "review.trackChanges={\"value\":true}"])).is_ok());
    // The app's user name is the person's own; a bridge flag must not rewrite it.
    assert!(run(&args(&["mcp", "--connect", "127.0.0.1:9", "--author", "Claude"])).is_err_and(|e| e.to_string().contains("--author")));
    // A malformed address fails at startup, not on every retried call.
    assert!(run(&args(&["mcp", "--connect", "127.0.0.1:notaport"])).is_err_and(|e| e.to_string().contains("numeric port")));
    assert!(run(&args(&["mcp", "--connect", "notaport"])).is_err_and(|e| e.to_string().contains("numeric port")));
    // The flag goes through the same protection check as the command.
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    let before = s.author.clone();
    assert!(set_author(&mut s, "Claude").is_err());
    assert_eq!(s.author, before);
    assert!(s.run("file.setAuthor", &json!({"name": "Claude"})).is_err());
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

#[test]
fn automation_root_flags_are_taken_out_of_the_mcp_arguments() {
    let base = std::env::temp_dir().join(format!("wc-cli-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("in")).unwrap();
    std::fs::create_dir_all(base.join("out")).unwrap();
    let (read, write) = (base.join("in").to_string_lossy().into_owned(), base.join("out").to_string_lossy().into_owned());

    let (roots, left) =
        automation_roots("mcp", &args(&["--automation-read-root", &read, "--author", "Claude", &format!("--automation-write-root={write}")]))
            .unwrap();
    let roots = roots.unwrap();
    assert_eq!(left, args(&["--author", "Claude"]));
    assert!(roots.read_root().is_some() && roots.write_root().is_some());
    assert!(validate_options("mcp", &left, MCP_OPTIONS).is_ok());

    // A root left out grants none of its access.
    let (roots, _) = automation_roots("mcp", &args(&["--automation-read-root", &read])).unwrap();
    let roots = roots.unwrap();
    assert!(roots.write_root().is_none());
    assert!(roots.check(&format!("{write}/x.docx"), wordcraft_engine::file_access::Access::Write).unwrap_err().contains("write authority is absent"));

    // No flags: nothing is confined and the arguments pass through.
    let (roots, left) = automation_roots("mcp", &args(&["--connect", "127.0.0.1:9"])).unwrap();
    assert!(roots.is_none());
    assert_eq!(left, args(&["--connect", "127.0.0.1:9"]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn automation_root_flags_reject_bad_operands_and_missing_folders() {
    for bad in [
        args(&["--automation-read-root"]),
        args(&["--automation-read-root", "--author"]),
        args(&["--automation-write-root="]),
        args(&["--automation-read-root", "a", "--automation-read-root=b"]),
    ] {
        assert!(automation_roots("mcp", &bad).is_err(), "{bad:?}");
    }
    let e = automation_roots("mcp", &args(&["--automation-read-root", "/definitely/not/a/folder"])).err().unwrap();
    assert!(e.starts_with("mcp: cannot open the automation read root"), "{e}");
    // Through the command line: an error, not a server that quietly runs unconfined.
    assert!(run(&args(&["mcp", "--automation-write-root", "/definitely/not/a/folder"])).is_err());
}

#[cfg(unix)]
#[test]
fn only_the_opened_file_may_be_other_than_unicode() {
    use std::os::unix::ffi::OsStringExt;
    let latin1 = || OsString::from_vec(b"caf\xe9.docx".to_vec());
    let ok = |raw: Vec<OsString>| text_args(&raw).unwrap().1;
    assert_eq!(ok(vec!["zotero".into(), "refresh".into(), latin1()]), Some(PathBuf::from(latin1())));
    assert_eq!(ok(vec!["render".into(), latin1(), "out.png".into(), "--page".into(), "2".into()]), Some(PathBuf::from(latin1())));
    assert_eq!(ok(vec!["run".into(), "--print".into(), "--file".into(), latin1()]), Some(PathBuf::from(latin1())));
    assert_eq!(ok(vec!["info".into(), "in.docx".into()]), None);
    // Every command that opens a file reaches the open with it (no such file here), so
    // `OPENS_FILE` and the commands' own arguments agree.
    let missing = || OsString::from_vec(b"/definitely/not/caf\xe9.docx".to_vec());
    let mut opening: Vec<Vec<OsString>> = OPENS_FILE
        .iter()
        .map(|(cmd, n)| [vec![(*cmd).into()], vec![OsString::from("refresh"); *n], vec![missing(), "out.png".into()]].concat())
        .collect();
    opening.push(vec!["run".into(), "--file".into(), missing()]);
    for raw in opening {
        let e = run(&raw).unwrap_err().to_string();
        assert!(e.contains("No such file"), "{raw:?}: {e}");
    }
    for (raw, n) in [
        (vec!["zotero".into(), latin1(), "in.docx".into()], 2),
        (vec!["render".into(), "in.docx".into(), latin1()], 3),
        (vec!["run".into(), "--save".into(), latin1()], 3),
        (vec!["commands".into(), latin1()], 2),
    ] {
        assert_eq!(text_args(&raw).unwrap_err(), format!("argument {n} is not valid Unicode: caf\u{FFFD}.docx"));
    }
}

#[test]
fn automation_roots_are_an_error_with_connect() {
    let dir = std::env::temp_dir().to_string_lossy().into_owned();
    let e = run(&args(&["mcp", "--connect", "127.0.0.1:9", "--automation-read-root", &dir])).unwrap_err().to_string();
    assert!(e.contains("--automation-read-root") && e.contains("headless"), "{e}");
}

#[test]
fn render_rejects_missing_or_invalid_page_and_scale() {
    // Missing operands
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--page"])), Err("render: --page requires a value".into()));
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--scale"])), Err("render: --scale requires a value".into()));
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--page", "--scale", "2"])), Err("render: --page requires a value".into()));
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--scale", "--page", "1"])), Err("render: --scale requires a value".into()));

    // Non-numeric operands
    assert_eq!(
        run(&args(&["render", "missing.docx", "out.png", "--page", "banana"])),
        Err("render: invalid --page `banana`: not a valid integer".into())
    );
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--scale", "banana"])), Err("render: invalid --scale `banana`: not a number".into()));

    // Out-of-range values
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--page", "0"])), Err("render: invalid --page `0`: page numbers start at 1".into()));
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--page", "-1"])), Err("render: invalid --page `-1`: not a valid integer".into()));
    assert_eq!(
        run(&args(&["render", "missing.docx", "out.png", "--scale", "0"])),
        Err("render: invalid --scale `0`: scale must be a positive number".into())
    );
    assert_eq!(
        run(&args(&["render", "missing.docx", "out.png", "--scale", "-2"])),
        Err("render: invalid --scale `-2`: scale must be a positive number".into())
    );
    assert_eq!(
        run(&args(&["render", "missing.docx", "out.png", "--scale", "NaN"])),
        Err("render: invalid --scale `NaN`: scale must be a positive number".into())
    );
    assert_eq!(
        run(&args(&["render", "missing.docx", "out.png", "--scale", "inf"])),
        Err("render: invalid --scale `inf`: scale must be a positive number".into())
    );

    // Duplicate options
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--page", "1", "--page", "2"])), Err("render: duplicate --page option".into()));
    assert_eq!(run(&args(&["render", "missing.docx", "out.png", "--scale", "1", "--scale", "2"])), Err("render: duplicate --scale option".into()));
}

#[test]
fn run_rejects_missing_or_invalid_page_and_scale() {
    // Missing operands
    assert_eq!(run(&args(&["run", "--template", "sample", "--save", "out.png", "--page"])), Err("run: --page requires a value".into()));
    assert_eq!(run(&args(&["run", "--template", "sample", "--save", "out.png", "--scale"])), Err("run: --scale requires a value".into()));
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--page", "--scale", "2"])),
        Err("run: --page requires a value".into())
    );
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--scale", "--page", "1"])),
        Err("run: --scale requires a value".into())
    );

    // Non-numeric operands
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--page", "banana"])),
        Err("run: invalid --page `banana`: not a valid integer".into())
    );
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--scale", "banana"])),
        Err("run: invalid --scale `banana`: not a number".into())
    );

    // Out-of-range values
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--page", "0"])),
        Err("run: invalid --page `0`: page numbers start at 1".into())
    );
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--page", "-1"])),
        Err("run: invalid --page `-1`: not a valid integer".into())
    );
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--scale", "0"])),
        Err("run: invalid --scale `0`: scale must be a positive number".into())
    );
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--scale", "-1"])),
        Err("run: invalid --scale `-1`: scale must be a positive number".into())
    );
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--scale", "NaN"])),
        Err("run: invalid --scale `NaN`: scale must be a positive number".into())
    );
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--scale", "inf"])),
        Err("run: invalid --scale `inf`: scale must be a positive number".into())
    );

    // Duplicate options
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--page", "1", "--page", "2"])),
        Err("run: duplicate --page option".into())
    );
    assert_eq!(
        run(&args(&["run", "--template", "sample", "--save", "out.png", "--scale", "1", "--scale", "2"])),
        Err("run: duplicate --scale option".into())
    );
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
fn script_value_missing_is_reported() {
    let e = run(&args(&["run", "--script", "--cmd", "caret.docEnd"])).unwrap_err().to_string();
    assert!(e.contains("--script requires a value"), "{e}");
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

//! `wordcraft-cli mcp` with automation roots writes each path the roots refuse to standard error,
//! as given and as resolved, so the person running the server can see what an agent tried to
//! reach (#4396). One refused request is one line.

use std::io::Write;
use std::process::{Command, Stdio};

fn tool_call(id: u32, name: &str, path: &str) -> String {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": {"path": path}}}).to_string()
}

/// `path` as the log shows it: without Windows' `\\?\` prefix.
fn shown(path: &std::path::Path) -> String {
    path.to_string_lossy().trim_start_matches(r"\\?\").to_string()
}

#[test]
fn a_refused_path_is_logged_once_with_its_resolved_form() {
    let base = std::env::temp_dir().join(format!("wc-cli-refused-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    std::fs::write(base.join("outside/secret.txt"), "TOP SECRET").unwrap();
    let sep = std::path::MAIN_SEPARATOR;
    let given = format!("{}{sep}..{sep}outside{sep}secret.txt", base.join("in").display());
    let resolved = shown(&std::fs::canonicalize(base.join("outside/secret.txt")).unwrap());
    // A link outside the write root that leads into it: the write would replace the link (#3735),
    // so the log names that entry, not only the file inside the root it leads to.
    #[cfg(unix)]
    let link = {
        std::fs::write(base.join("out/doc.docx"), "x").unwrap();
        std::os::unix::fs::symlink(base.join("out/doc.docx"), base.join("outside/alias.docx")).unwrap();
        let target = shown(&std::fs::canonicalize(base.join("out/doc.docx")).unwrap());
        let entry = shown(&std::fs::canonicalize(base.join("outside")).unwrap().join("alias.docx"));
        Some((tool_call(2, "save_document", &base.join("outside/alias.docx").to_string_lossy()), format!("(resolved: {target}, entry {entry})")))
    };
    #[cfg(not(unix))]
    let link: Option<(String, String)> = None;
    let calls: Vec<String> = std::iter::once(tool_call(1, "open_document", &given)).chain(link.as_ref().map(|(call, _)| call.clone())).collect();

    let (stdout, stderr) = serve(&[("--automation-read-root", &base.join("in")), ("--automation-write-root", &base.join("out"))], &calls);
    assert!(stdout.contains("outside the read root"), "the agent is told: {stdout}");
    let logged: Vec<&str> = stderr.lines().filter(|l| l.contains("automation path rejected")).collect();
    assert_eq!(logged.len(), calls.len(), "one line per refused request: {stderr}");
    let line = logged[0];
    assert!(line.contains("outside the read root"), "{line}");
    assert!(line.contains(&given), "the path as given: {line}");
    assert!(line.contains(&format!("(resolved: {resolved})")), "the path as resolved: {line}");
    if let Some((_, expected)) = &link {
        assert!(logged[1].contains("outside the write root") && logged[1].contains(expected), "the entry a write would replace: {}", logged[1]);
    }
    let _ = std::fs::remove_dir_all(base);
}

/// The refusal lines in `stderr`.
fn refusals(stderr: &str) -> Vec<&str> {
    stderr.lines().filter(|l| l.contains("automation path rejected")).collect()
}

/// #4649 review: a save to an .xml file probes its head (is it Word 2003 XML?) without logging, so
/// an allowed save logs nothing, and one outside both roots logs its write refusal only.
#[test]
fn a_save_probe_logs_nothing() {
    let base = std::env::temp_dir().join(format!("wc-cli-refused-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let calls = [
        tool_call(1, "save_document", &base.join("out/report.xml").to_string_lossy()),
        tool_call(2, "save_document", &base.join("outside/report.xml").to_string_lossy()),
    ];
    let (stdout, stderr) = serve(&[("--automation-read-root", &base.join("in")), ("--automation-write-root", &base.join("out"))], &calls);
    assert!(base.join("out/report.xml").is_file(), "the allowed save is written: {stdout}");
    let logged = refusals(&stderr);
    assert_eq!(logged.len(), 1, "only the refused save, once: {stderr}");
    assert!(logged[0].contains("outside the write root"), "{}", logged[0]);
    let _ = std::fs::remove_dir_all(base);
}

/// #4649 review: an HTML file whose pictures point outside the read root is opened without them,
/// and its pictures log nothing: one open never logs a line per picture.
#[test]
fn pictures_outside_the_root_log_nothing() {
    let base = std::env::temp_dir().join(format!("wc-cli-refused-img-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    std::fs::write(base.join("outside/pic.png"), "not really a picture").unwrap();
    let html = format!("<html><body><p>Hello</p>{}</body></html>", "<img src=\"../outside/pic.png\">".repeat(20));
    std::fs::write(base.join("in/page.html"), html).unwrap();
    let calls = [tool_call(1, "open_document", &base.join("in/page.html").to_string_lossy())];
    let (stdout, stderr) = serve(&[("--automation-read-root", &base.join("in")), ("--automation-write-root", &base.join("out"))], &calls);
    assert!(!stdout.contains("isError\":true"), "the page opens: {stdout}");
    assert_eq!(refusals(&stderr), Vec::<&str>::new(), "{stderr}");
    let _ = std::fs::remove_dir_all(base);
}

/// Runs `wordcraft-cli mcp` with `roots`, sends `calls`, and returns its stdout and stderr.
fn serve(roots: &[(&str, &std::path::Path)], calls: &[String]) -> (String, String) {
    let mut cli = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    cli.arg("mcp");
    for (flag, dir) in roots {
        cli.arg(flag).arg(dir);
    }
    let mut child = cli.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for call in calls {
        writeln!(stdin, "{call}").unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

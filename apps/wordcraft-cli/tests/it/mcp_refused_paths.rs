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

/// Without write authority a save is refused before its path is touched, and the log doesn't
/// resolve it either (on Windows a UNC path would reach the network): the path as given only.
#[test]
fn a_refusal_without_authority_logs_the_path_unresolved() {
    let base = std::env::temp_dir().join(format!("wc-cli-refused-noauth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("in")).unwrap();
    let given = base.join("in/x.docx").to_string_lossy().into_owned();
    let (stdout, stderr) = serve(&[("--automation-read-root", &base.join("in"))], &[tool_call(1, "save_document", &given)]);
    assert!(stdout.contains("write authority is absent"), "the agent is told: {stdout}");
    let logged: Vec<&str> = stderr.lines().filter(|l| l.contains("write authority is absent")).collect();
    assert_eq!(logged.len(), 1, "one line per refused request: {stderr}");
    assert!(logged[0].ends_with(&given) && !logged[0].contains("resolved"), "the path as given only: {}", logged[0]);
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

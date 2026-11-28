//! Shared rule 18 (#7124): `wordcraft-cli mcp` refuses to start without both
//! `--automation-read-root` and `--automation-write-root`, refuses `--connect` while the desktop
//! app doesn't confine its control channel (#3695, #5819), and with both roots refuses files
//! outside them, through the real binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

/// An exclusively created folder holding `in` (the read root, with `a.txt`), `out` (the write
/// root) and `outside` (with `secret.txt`). Removed on drop, even when an assertion fails.
struct Folders(PathBuf);

impl Folders {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = loop {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("wc-cli-mcp-roots-{tag}-{}-{n}", std::process::id()));
            // `create_dir` fails when the path exists, so a leftover or a planted folder is never reused. Any
            // other failure (no temp folder, no permission) fails every name: report it instead of looping.
            match std::fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("scratch folder {}: {e}", dir.display()),
            }
        };
        let folders = Self(base);
        for d in ["in", "out", "outside"] {
            std::fs::create_dir_all(folders.0.join(d)).unwrap();
        }
        std::fs::write(folders.0.join("in/a.txt"), "Inside text").unwrap();
        std::fs::write(folders.0.join("outside/secret.txt"), "TOP SECRET").unwrap();
        folders
    }
}

impl Drop for Folders {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Run `wordcraft-cli mcp <args>`, send `lines` on stdin, then hang up.
fn mcp(args: &[String], lines: &[Value]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
        .arg("mcp")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in lines {
        // A server that refused to start has closed its end: that is the expected outcome.
        let _ = writeln!(stdin, "{line}");
    }
    drop(stdin);
    child.wait_with_output().unwrap()
}

const PING: &str = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

fn refused_at_start(args: &[String], why: &[&str]) {
    let out = mcp(args, &[serde_json::from_str(PING).unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{args:?} started: stderr {stderr}");
    assert!(out.stdout.is_empty(), "{args:?} served: {}", String::from_utf8_lossy(&out.stdout));
    for w in why {
        assert!(stderr.contains(w), "{args:?}: `{w}` missing from {stderr}");
    }
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn mcp_refuses_to_start_without_both_roots() {
    let folders = Folders::new("start");
    let base = folders.0.clone();
    let (read, write) = (s(&base.join("in")), s(&base.join("out")));
    let both = ["--automation-read-root", "--automation-write-root"];
    refused_at_start(&[], &both);
    refused_at_start(&["--author".into(), "Claude".into()], &both);
    refused_at_start(&["--automation-read-root".into(), read.clone()], &both);
    refused_at_start(&[format!("--automation-write-root={write}")], &both);
    // Both: it serves.
    let out = mcp(&["--automation-read-root".into(), read, "--automation-write-root".into(), write], &[serde_json::from_str(PING).unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr: {stderr}");
    let reply: Value = serde_json::from_slice(out.stdout.split(|b| *b == b'\n').next().unwrap()).unwrap();
    assert_eq!(reply["id"], 1, "{reply}");
    assert!(reply.get("error").is_none(), "{reply}");
}

#[test]
fn mcp_connect_is_refused_until_the_app_confines_its_control_channel() {
    let folders = Folders::new("connect");
    let base = folders.0.clone();
    let (read, write) = (s(&base.join("in")), s(&base.join("out")));
    let why = ["--connect", "#3695", "#5819"];
    refused_at_start(&["--connect".into(), "127.0.0.1:9".into()], &why);
    refused_at_start(
        &["--connect".into(), "127.0.0.1:9".into(), "--automation-read-root".into(), read, "--automation-write-root".into(), write],
        &why,
    );
}

/// `tools/call` request `id` for `name` with `arguments`.
fn tool(id: u64, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
}

fn execute(id: u64, command: &str, params: Value) -> Value {
    tool(id, "execute", json!({"command": command, "params": params}))
}

#[test]
fn mcp_with_both_roots_refuses_files_outside_them() {
    let folders = Folders::new("files");
    let base = folders.0.clone();
    let sep = std::path::MAIN_SEPARATOR;
    let (inside, outside) = (base.join("in"), base.join("outside"));
    let secret = s(&outside.join("secret.txt"));
    let up = |root: &str, rest: &str| format!("{}{sep}..{sep}{rest}", s(&base.join(root)));
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, base.join("out/away")).is_ok()
        && std::os::unix::fs::symlink(outside.join("secret.txt"), inside.join("leak.txt")).is_ok();
    #[cfg(not(unix))]
    let linked = false;

    // (id, request, refused with). Reads first, then writes, then the allowed ones.
    let mut cases = vec![
        (2, tool(2, "open_document", json!({"path": secret})), Some("outside the read root")),
        (3, execute(3, "file.open", json!({"path": secret})), Some("outside the read root")),
        (4, execute(4, "file.open", json!({"path": up("in", "outside/secret.txt")})), Some("outside the read root")),
        (5, execute(5, "insert.picture", json!({"path": s(&outside.join("pic.png"))})), Some("outside the read root")),
        (6, execute(6, "insert.textFromFile", json!({"path": secret})), Some("outside the read root")),
        (7, tool(7, "save_document", json!({"path": s(&outside.join("a.docx"))})), Some("outside the write root")),
        (8, execute(8, "file.saveAs", json!({"path": s(&outside.join("b.docx"))})), Some("outside the write root")),
        (9, execute(9, "file.exportPdf", json!({"path": s(&outside.join("c.pdf"))})), Some("outside the write root")),
        (10, execute(10, "file.exportPng", json!({"path": s(&outside.join("d.png"))})), Some("outside the write root")),
        (11, execute(11, "file.saveAs", json!({"path": up("out", "outside/e.docx")})), Some("outside the write root")),
        // The write root isn't readable, nor the read root writable.
        (12, execute(12, "file.saveAs", json!({"path": s(&inside.join("f.docx"))})), Some("outside the write root")),
        (13, tool(13, "open_document", json!({"path": s(&inside.join("a.txt"))})), None),
        (14, execute(14, "file.saveAs", json!({"path": s(&base.join("out/g.docx"))})), None),
    ];
    if linked {
        cases.push((15, tool(15, "open_document", json!({"path": s(&inside.join("leak.txt"))})), Some("outside the read root")));
        cases.push((16, execute(16, "file.saveAs", json!({"path": s(&base.join("out/away/h.docx"))})), Some("outside the write root")));
    }
    let requests: Vec<Value> = cases.iter().map(|(_, r, _)| r.clone()).collect();
    let out = mcp(&["--automation-read-root".into(), s(&inside), "--automation-write-root".into(), s(&base.join("out"))], &requests);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr: {stderr}");
    let replies: Vec<Value> =
        String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect();
    for (id, request, why) in &cases {
        let reply = replies.iter().find(|r| r["id"] == *id).unwrap_or_else(|| panic!("no reply to {request}"));
        let result = &reply["result"];
        let text: String = result["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect();
        match why {
            Some(why) => assert!(result["isError"] == true && text.contains(why), "{request} should be refused ({why}): {reply}"),
            None => assert!(result["isError"] != true, "{request}: {reply}"),
        }
    }
    let mut written: Vec<String> = std::fs::read_dir(&outside).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    written.sort();
    assert_eq!(written, ["secret.txt"], "nothing was written outside the roots");
    assert!(!inside.join("f.docx").exists());
    assert!(base.join("out/g.docx").is_file());
}

//! The CLI binary re-executes itself on Linux glibc with `MALLOC_TRIM_THRESHOLD_` fixed so freed
//! page/render buffers are returned to the OS while respecting caller-supplied values.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use wordcraft_engine::malloc_tuning::{TRIM_THRESHOLD, TRIM_THRESHOLD_VAR};

struct TempDirs(PathBuf);

impl TempDirs {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = loop {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("wc-cli-malloc-{tag}-{}-{n}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("temp dir {}: {e}", dir.display()),
            }
        };
        std::fs::create_dir_all(root.join("in")).unwrap();
        std::fs::create_dir_all(root.join("out")).unwrap();
        Self(root)
    }
}

impl Drop for TempDirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn mcp_environ(tag: &str, set: Option<&str>) -> Vec<String> {
    let dirs = TempDirs::new(tag);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    for var in [TRIM_THRESHOLD_VAR, "MALLOC_MMAP_THRESHOLD_", "MALLOC_TOP_PAD_", "MALLOC_MMAP_MAX_", "MALLOC_ARENA_MAX", "GLIBC_TUNABLES"] {
        cmd.env_remove(var);
    }
    if let Some(v) = set {
        cmd.env(TRIM_THRESHOLD_VAR, v);
    }
    let mut child = cmd
        .args(["mcp", "--automation-read-root", &s(&dirs.0.join("in")), "--automation-write-root", &s(&dirs.0.join("out"))])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"ping"}}"#).unwrap();
    let mut reply = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut reply).unwrap();
    assert!(reply.contains(r#""id":1"#), "mcp answered after re-exec: {reply}");
    let environ = std::fs::read(format!("/proc/{}/environ", child.id())).unwrap();
    drop(stdin);
    assert!(child.wait().unwrap().success());
    environ.split(|&b| b == 0).map(|e| String::from_utf8_lossy(e).into_owned()).collect()
}

#[test]
fn sets_the_trim_threshold() {
    let env = mcp_environ("default", None);
    assert!(env.contains(&format!("{TRIM_THRESHOLD_VAR}={TRIM_THRESHOLD}")), "{env:?}");
}

#[test]
fn keeps_a_value_the_user_set() {
    let env = mcp_environ("explicit", Some("999999"));
    assert!(env.contains(&format!("{TRIM_THRESHOLD_VAR}=999999")), "{env:?}");
}

//! Memberships: `<seleload(&p) m = load(&p).ok_or_elsetings>/chat-members/<instance-or-local-port>-<handle>.json`, mode 0600,
//! written atomically, pruned when their window is gone.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::Failure;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Membership {
    pub addr: String,
    pub handle: String,
    pub key: String,
}

pub struct Store {
    dir: PathBuf,
}

/// Longest membership file read (one is about 150 bytes): a longer one is corrupt.
const MAX_FILE: u64 = 4096;

fn parse(text: &str) -> Option<Membership> {
    let v: Value = serde_json::from_str(text).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let m = Membership { addr: s("addr")?, handle: s("handle")?, key: s("key")? };
    wordcraft_chat::rules::valid_handle(&m.handle).then_some(m)
}

/// The membership in the file at `p`; `None` when it cannot be read, is too long or is not one.
fn load(p: &Path) -> Option<Membership> {
    let mut text = String::new();
    let n = std::fs::File::open(p).ok()?.take(MAX_FILE.saturating_add(1)).read_to_string(&mut text).ok()?;
    if u64::try_from(n).ok()? > MAX_FILE {
        return None;
    }
    parse(&text)
}

/// Nothing listens on the membership's port any more (only a refused connection counts).
pub fn window_gone(m: &Membership) -> bool {
    m.addr.to_socket_addrs().ok().and_then(|mut a| a.next()).is_some_and(
        |sa| matches!(TcpStream::connect_timeout(&sa, Duration::from_millis(300)), Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused),
    )
}

impl Store {
    pub fn new(dir: PathBuf) -> Store {
        Store { dir }
    }

    pub fn default_dir() -> Option<PathBuf> {
        wordcraft_control_key::settings_dir().map(|d| d.join("chat-members"))
    }

    pub fn file_name(m: &Membership) -> String {
        let port = m.addr.rsplit_once(':').map(|(_, p)| p).unwrap_or("0");
        let place = wordcraft_control_key::instance_id().unwrap_or_else(|| format!("local-{port}"));
        format!("{place}-{}.json", m.handle)
    }

    pub fn save(&self, m: &Membership) -> Result<PathBuf, Failure> {
        let fail = |e: std::io::Error| Failure::error(format!("cannot save the membership in {}: {e}", self.dir.display()));
        wordcraft_chat::log::private_dir(&self.dir).map_err(fail)?;
        let name = Store::file_name(m);
        let (path, tmp) = (self.dir.join(&name), self.dir.join(format!(".tmp-{name}")));
        let body = json!({"addr": m.addr, "handle": m.handle, "key": m.key}).to_string();
        let _ = std::fs::remove_file(&tmp);
        let mut open = std::fs::OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
        let written = open.open(&tmp).and_then(|mut f| f.write_all(body.as_bytes())).and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(fail(e));
        }
        Ok(path)
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|r| {
                r.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "json") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    /// The one membership that matches `handle` and `addr`.
    pub fn find(&self, handle: Option<&str>, addr: Option<&str>) -> Result<(Membership, PathBuf), Failure> {
        let mut hits = Vec::new();
        for p in self.files() {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if handle.is_some_and(|h| !name.ends_with(&format!("-{h}.json"))) {
                continue;
            }
            let m = load(&p).ok_or_else(|| Failure::usage(format!("corrupt membership file {}", p.display())))?;
            if addr.is_none_or(|a| a == m.addr) {
                hits.push((m, p));
            }
        }
        match hits.len() {
            0 => Err(Failure::usage("no membership found: join first, or check --as and --addr")),
            1 => hits.pop().ok_or_else(|| Failure::usage("no membership found")),
            _ => Err(Failure::usage("several memberships match: use --as @name and --addr HOST:PORT")),
        }
    }

    pub fn forget(&self, path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    /// Delete memberships whose window is gone; how many.
    pub fn prune(&self, gone: &dyn Fn(&Membership) -> bool) -> usize {
        let mut n: usize = 0;
        for p in self.files() {
            if let Some(m) = load(&p)
                && gone(&m)
            {
                self.forget(&p);
                n = n.saturating_add(1);
            }
        }
        n
    }
}

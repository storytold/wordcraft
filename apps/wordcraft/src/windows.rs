//! The desktop app's side of View › Window (#322): a registry of the running WordCraft windows
//! and a small authenticated loopback channel between them.
//!
//! Each window is its own process with one document. At start it binds `127.0.0.1` on a free
//! port, picks a random id and key, and writes `<settings>/windows/<id>.json` (mode 0600 on Unix):
//! `{"id", "port", "key", "title"}`. The file is removed when the window closes. Other windows
//! list the folder in the background: an entry whose port refuses connections, or that doesn't
//! answer and hasn't been rewritten for two minutes (each window rewrites its entry every 30 s), is
//! left over from a crash and is removed; unreadable, oversized or malformed entries are ignored.
//!
//! The channel takes one JSON line per connection, `{"key", "from", "msg"}` with `msg` a
//! [`PeerMessage`], and nothing else: it can bring the window to the front, place it and scroll
//! it in step with its Side by Side partner, never run commands or read the document. Lines are
//! at most 4 KiB, the key is compared in constant time, a connection has 2 s to send its line,
//! and at most 16 are served at once. The full control channel (`--control`) stays opt-in.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wordcraft_engine::cmd::view::OtherWindow;
use wordcraft_ui_egui::windows::{Envelope, PeerMessage, WindowHost};

use crate::control_server::{Timed, keys_match, random_key, write_key_file};

/// Longest request or reply line, and largest registry entry read.
const MAX_LINE: usize = 4096;
/// How long a connection has to send its line.
const FIRST_LINE: Duration = Duration::from_secs(2);
/// Connect and reply timeout when talking to another window.
const IO_TIMEOUT: Duration = Duration::from_millis(500);
/// How often the list of other windows is refreshed.
const REFRESH: Duration = Duration::from_secs(1);
/// At most this many other windows are listed, and this many connections served at once.
const MAX_WINDOWS: usize = 64;
const MAX_CONNECTIONS: usize = 16;
/// Titles are cut to this many characters.
const MAX_TITLE: usize = 200;
/// Each window rewrites its entry this often…
const HEARTBEAT: Duration = Duration::from_secs(30);
/// …so an entry that doesn't answer and is older than this is left over from a crash.
const STALE: Duration = Duration::from_secs(120);

/// A registry entry: how to reach a window.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    id: u64,
    port: u16,
    key: String,
    title: String,
    /// When the file was last written (not part of it).
    modified: Option<std::time::SystemTime>,
}

impl Entry {
    fn to_json(&self) -> Value {
        json!({"id": self.id, "port": self.port, "key": self.key, "title": self.title})
    }

    /// The entry in a registry file named `<stem>.json`, if it is well formed: the id matches
    /// the file name, the port isn't 0, the key is 64 hex digits. The title loses control
    /// characters and is cut to [`MAX_TITLE`] characters.
    fn parse(stem: &str, text: &str) -> Option<Entry> {
        let v: Value = serde_json::from_str(text).ok()?;
        let id = v.get("id")?.as_u64().filter(|id| id.to_string() == stem)?;
        let port = v.get("port")?.as_u64().and_then(|p| u16::try_from(p).ok()).filter(|p| *p != 0)?;
        let key = v.get("key")?.as_str().filter(|k| k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit()))?.to_string();
        let title = v.get("title").and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect();
        Some(Entry { id, port, key, title, modified: None })
    }

    fn other(&self) -> OtherWindow {
        let title = if self.title.trim().is_empty() { "WordCraft".to_string() } else { self.title.clone() };
        OtherWindow { id: self.id, title }
    }
}

/// The registry file of this window; [`RegistryFile::remove`] it when the window closes.
pub struct RegistryFile {
    path: PathBuf,
    /// Held while the file is written; `true` once it is removed, so it isn't written again.
    closed: Arc<Mutex<bool>>,
}

impl RegistryFile {
    pub fn remove(&self) {
        let mut closed = self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *closed = true;
        let _ = std::fs::remove_file(&self.path);
    }
}

/// This window's [`WindowHost`]: the list the background thread keeps and the queue it sends.
pub struct Windows {
    id: u64,
    others: Arc<Mutex<Vec<OtherWindow>>>,
    title: Arc<Mutex<String>>,
    out: Sender<(u64, PeerMessage)>,
    incoming: Receiver<Envelope>,
}

impl WindowHost for Windows {
    fn id(&self) -> u64 {
        self.id
    }
    fn others(&self) -> Vec<OtherWindow> {
        self.others.lock().map(|o| o.clone()).unwrap_or_default()
    }
    fn send(&self, to: u64, msg: PeerMessage) {
        let _ = self.out.send((to, msg));
    }
    fn publish(&self, title: &str) {
        if let Ok(mut t) = self.title.lock()
            && *t != title
        {
            *t = title.to_string();
        }
    }
    fn incoming(&self) -> Vec<Envelope> {
        self.incoming.try_iter().collect()
    }
}

/// Join the registry in `dir`: serve the channel, write this window's entry and keep the list of
/// the others in the background. `None` (View › Window stays disabled) when there is no port,
/// random key or writable folder.
pub fn start(dir: &Path, ctx: egui::Context) -> Option<(Windows, RegistryFile)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| log::warn!("window channel: {e}")).ok()?;
    let port = listener.local_addr().ok()?.port();
    let key = random_key().map_err(|e| log::warn!("window channel: {e}")).ok()?;
    let id = random_id()?;
    let me = Entry { id, port, key: key.clone(), title: String::new(), modified: None };
    let file = RegistryFile { path: dir.join(format!("{id}.json")), closed: Arc::new(Mutex::new(false)) };
    if let Err(e) = write_entry(dir, &me, &file.closed) {
        log::warn!("window registry: {e}");
        return None;
    }
    let (tx, incoming) = channel();
    serve_on(listener, key, tx, ctx);
    let (out, queue) = channel();
    let host = Windows { id, others: Arc::default(), title: Arc::default(), out, incoming };
    let worker = Worker {
        dir: dir.to_path_buf(),
        me,
        closed: file.closed.clone(),
        others: host.others.clone(),
        title: host.title.clone(),
        known: Vec::new(),
        written: Instant::now(),
    };
    std::thread::spawn(move || worker.run(&queue));
    Some((host, file))
}

/// A random window id that survives JSON numbers (below 2⁵³) and is never 0.
fn random_id() -> Option<u64> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| log::warn!("window id: {e}")).ok()?;
    Some((u64::from_le_bytes(b) & ((1 << 53) - 1)).max(1))
}

/// Write `<id>.json` whole (a temporary file renamed over it), unless the window has closed.
fn write_entry(dir: &Path, e: &Entry, closed: &Mutex<bool>) -> Result<(), String> {
    let closed = closed.lock().map_err(|_| "registry lock poisoned".to_string())?;
    if *closed {
        return Ok(());
    }
    let tmp = dir.join(format!("{}.json.tmp", e.id));
    write_key_file(&tmp, &e.to_json().to_string())?;
    std::fs::rename(&tmp, dir.join(format!("{}.json", e.id))).map_err(|err| format!("cannot write {}: {err}", dir.display()))
}

/// The well-formed entries in `dir` other than `me`'s, by id.
fn read_entries(dir: &Path, me: u64) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for f in rd.flatten().take(MAX_WINDOWS * 4) {
        let path = f.path();
        let Some(stem) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".json")) else { continue };
        if stem == me.to_string() || !f.metadata().is_ok_and(|m| m.is_file() && m.len() <= MAX_LINE as u64) {
            continue;
        }
        if let Some(e) = std::fs::read_to_string(&path).ok().and_then(|t| Entry::parse(stem, &t)) {
            out.push(Entry { modified: f.metadata().and_then(|m| m.modified()).ok(), ..e });
        }
    }
    out.sort_by_key(|e| e.id);
    out
}

/// The entries in `dir` whose windows answer. Entries left over from a crash are removed: their
/// port refuses connections, or (Windows answers a closed port only after a few seconds) they
/// don't answer and are older than [`STALE`].
fn live_entries(dir: &Path, me: u64) -> Vec<Entry> {
    let mut live = Vec::new();
    for e in read_entries(dir, me) {
        match send_to(&e, me, &PeerMessage::Ping) {
            Ok(()) => live.push(e),
            Err(err) => {
                let old = e.modified.and_then(|m| m.elapsed().ok()).is_some_and(|age| age > STALE);
                if err.kind() == std::io::ErrorKind::ConnectionRefused || old {
                    log::info!("window registry: removing stale entry {}", e.id);
                    let _ = std::fs::remove_file(dir.join(format!("{}.json", e.id)));
                }
            }
        }
        if live.len() >= MAX_WINDOWS {
            break;
        }
    }
    live
}

/// Send one message to a window and wait for its answer.
fn send_to(e: &Entry, from: u64, msg: &PeerMessage) -> std::io::Result<()> {
    let mut s = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, e.port)), IO_TIMEOUT)?;
    s.set_write_timeout(Some(IO_TIMEOUT))?;
    let line = json!({"key": e.key, "from": from, "msg": msg});
    writeln!(s, "{line}")?;
    let mut reply = Vec::new();
    BufReader::new(Timed::new(s, Instant::now() + IO_TIMEOUT)).take(MAX_LINE as u64).read_until(b'\n', &mut reply)?;
    let ok = serde_json::from_slice::<Value>(&reply).ok().and_then(|v| v.get("ok").and_then(Value::as_bool)) == Some(true);
    if ok { Ok(()) } else { Err(std::io::Error::other("refused")) }
}

/// The background thread: sends queued messages and refreshes the list and this window's entry.
struct Worker {
    dir: PathBuf,
    me: Entry,
    closed: Arc<Mutex<bool>>,
    others: Arc<Mutex<Vec<OtherWindow>>>,
    title: Arc<Mutex<String>>,
    /// The other windows at the last refresh.
    known: Vec<Entry>,
    /// When this window's entry was last written.
    written: Instant,
}

impl Worker {
    fn run(mut self, queue: &Receiver<(u64, PeerMessage)>) {
        let mut refreshed: Option<Instant> = None;
        loop {
            let mut batch = Vec::new();
            match queue.recv_timeout(REFRESH) {
                Ok(m) => {
                    // Scrolling queues a message a frame: take what else arrives right away.
                    std::thread::sleep(Duration::from_millis(10));
                    batch.push(m);
                    batch.extend(queue.try_iter());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            for (to, msg) in coalesce(batch) {
                if !self.known.iter().any(|e| e.id == to) {
                    self.known = read_entries(&self.dir, self.me.id);
                }
                if let Some(e) = self.known.iter().find(|e| e.id == to)
                    && let Err(err) = send_to(e, self.me.id, &msg)
                {
                    log::info!("window {to}: {err}");
                }
            }
            if refreshed.is_none_or(|t| t.elapsed() >= REFRESH) {
                self.refresh();
                refreshed = Some(Instant::now());
            }
        }
    }

    fn refresh(&mut self) {
        let title = self.title.lock().map(|t| t.chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect::<String>()).unwrap_or_default();
        if title != self.me.title || self.written.elapsed() >= HEARTBEAT {
            self.me.title = title;
            self.written = Instant::now();
            if let Err(e) = write_entry(&self.dir, &self.me, &self.closed) {
                log::warn!("window registry: {e}");
            }
        }
        self.known = live_entries(&self.dir, self.me.id);
        if let Ok(mut o) = self.others.lock() {
            *o = self.known.iter().map(Entry::other).collect();
        }
    }
}

/// Queued messages in order, with runs of scrolls to the same window added up.
fn coalesce(batch: Vec<(u64, PeerMessage)>) -> Vec<(u64, PeerMessage)> {
    let mut out: Vec<(u64, PeerMessage)> = Vec::new();
    for (to, msg) in batch {
        if let (Some((last_to, PeerMessage::Scroll { dy: sum })), PeerMessage::Scroll { dy }) = (out.last_mut(), &msg)
            && *last_to == to
        {
            *sum += dy;
            continue;
        }
        out.push((to, msg));
    }
    out
}

fn serve_on(listener: TcpListener, key: String, tx: Sender<Envelope>, ctx: egui::Context) {
    let key: Arc<str> = key.into();
    let busy = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if busy.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                busy.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let (key, tx, ctx, busy) = (key.clone(), tx.clone(), ctx.clone(), busy.clone());
            std::thread::spawn(move || {
                serve(stream, &key, &tx, &ctx);
                busy.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
}

/// One connection: one line in, one reply out.
fn serve(stream: TcpStream, key: &str, tx: &Sender<Envelope>, ctx: &egui::Context) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    let mut line = Vec::new();
    let mut reader = BufReader::new(Timed::new(read, Instant::now() + FIRST_LINE));
    if reader.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut line).is_err() {
        return;
    }
    let reply = match handle(&line, key) {
        Ok(Some(env)) => {
            let sent = tx.send(env).is_ok();
            ctx.request_repaint();
            if sent { json!({"ok": true}) } else { json!({"ok": false, "error": "closing"}) }
        }
        Ok(None) => json!({"ok": true}),
        Err(e) => json!({"ok": false, "error": e}),
    };
    let _ = writeln!(out, "{reply}");
}

/// A request line: the message for the UI, `None` for a ping, or why it was refused.
fn handle(line: &[u8], key: &str) -> Result<Option<Envelope>, &'static str> {
    if line.len() > MAX_LINE {
        return Err("line too long");
    }
    let v: Value = serde_json::from_slice(line).map_err(|_| "bad JSON")?;
    if !keys_match(v.get("key").and_then(Value::as_str).unwrap_or(""), key) {
        return Err("unauthorized");
    }
    let from = v.get("from").and_then(Value::as_u64).ok_or("missing `from`")?;
    let msg =
        v.get("msg").cloned().and_then(|m| serde_json::from_value::<PeerMessage>(m).ok()).and_then(PeerMessage::sanitized).ok_or("bad message")?;
    Ok((msg != PeerMessage::Ping).then_some(Envelope { from, msg }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wordcraft-windows-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn registry_ignores_hostile_entries_and_drops_stale_ones() {
        let dir = temp_dir("registry");
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        let entry = |id: u64, port: u16| Entry { id, port, key: KEY.into(), title: "Notes\n\u{7}.docx".into(), modified: None }.to_json().to_string();
        // A window that is gone: nothing listens on its port any more, and it stopped rewriting
        // its entry (Windows doesn't refuse a closed port right away).
        let closed_port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        write("11.json", &entry(11, closed_port));
        let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options().write(true).open(dir.join("11.json")).unwrap().set_modified(hour_ago).unwrap();
        // Malformed or hostile entries.
        write("12.json", &entry(99, 4000)); // the id doesn't match the file name
        write("13.json", r#"{"id": 13, "port": 0, "key": "0123"}"#);
        write("14.json", r#"{"id": 14, "port": 70000, "key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}"#);
        write("15.json", &format!(r#"{{"id": 15, "port": 4000, "key": "{}"}}"#, "z".repeat(64)));
        write("16.json", "not json");
        write("17.json", &" ".repeat(MAX_LINE + 1));
        write("notes.txt", &entry(18, 4000));
        std::fs::create_dir_all(dir.join("19.json")).unwrap();
        let listed = read_entries(&dir, 1);
        assert_eq!(listed.iter().map(|e| e.id).collect::<Vec<_>>(), [11], "only the well-formed entry: {listed:?}");
        assert_eq!(listed[0].title, "Notes.docx", "control characters are dropped");
        assert!(
            Entry::parse("20", &Entry { id: 20, port: 1, key: KEY.into(), title: "x".repeat(10_000), modified: None }.to_json().to_string())
                .is_some_and(|e| e.title.chars().count() == MAX_TITLE)
        );
        assert!(live_entries(&dir, 1).is_empty());
        assert!(!dir.join("11.json").exists(), "the stale entry is removed");
        assert!(dir.join("16.json").exists(), "entries that aren't ours to judge are left alone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_find_and_message_each_other() {
        let dir = temp_dir("peers");
        let ctx = egui::Context::default();
        let (a, a_file) = start(&dir, ctx.clone()).unwrap();
        let (b, b_file) = start(&dir, ctx).unwrap();
        a.publish("Notes.docx");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !b.others().iter().any(|o| o.id == a.id() && o.title == "Notes.docx") {
            assert!(Instant::now() < deadline, "b never saw a: {:?}", b.others());
            std::thread::sleep(Duration::from_millis(50));
        }
        b.send(a.id(), PeerMessage::Scroll { dy: 10.0 });
        b.send(a.id(), PeerMessage::Scroll { dy: 5.0 });
        b.send(a.id(), PeerMessage::Focus);
        let mut got = Vec::new();
        while got.iter().all(|e: &Envelope| e.msg != PeerMessage::Focus) {
            assert!(Instant::now() < deadline, "a got {got:?}");
            got.extend(a.incoming());
            std::thread::sleep(Duration::from_millis(20));
        }
        let scrolled: f32 = got.iter().map(|e| if let PeerMessage::Scroll { dy } = e.msg { dy } else { 0.0 }).sum();
        assert_eq!(scrolled, 15.0);
        assert!(got.iter().all(|e| e.from == b.id()));
        // Without the key, or with a bad message, nothing reaches the window.
        let port = read_entries(&dir, b.id()).iter().find(|e| e.id == a.id()).unwrap().port;
        let wrong = Entry { id: a.id(), port, key: "f".repeat(64), title: String::new(), modified: None };
        assert!(send_to(&wrong, 1, &PeerMessage::Focus).is_err());
        assert_eq!(handle(br#"{"key": "x", "from": 1, "msg": {"type": "focus"}}"#, KEY), Err("unauthorized"));
        let place = format!(r#"{{"key": "{KEY}", "from": 1, "msg": {{"type": "place", "x": 1e30, "y": 0, "width": 10, "height": 10}}}}"#);
        assert_eq!(handle(place.as_bytes(), KEY), Err("bad message"));
        assert_eq!(handle(&vec![b' '; MAX_LINE + 1], KEY), Err("line too long"));
        std::thread::sleep(Duration::from_millis(200));
        assert!(a.incoming().is_empty());
        // A closed window's entry is gone.
        a_file.remove();
        assert!(!dir.join(format!("{}.json", a.id())).exists());
        b_file.remove();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scrolls_add_up_in_order() {
        let s = |dy| PeerMessage::Scroll { dy };
        assert_eq!(
            coalesce(vec![(1, s(1.0)), (1, s(2.0)), (2, s(4.0)), (1, PeerMessage::Focus), (1, s(8.0))]),
            vec![(1, s(3.0)), (2, s(4.0)), (1, PeerMessage::Focus), (1, s(8.0))]
        );
    }
}

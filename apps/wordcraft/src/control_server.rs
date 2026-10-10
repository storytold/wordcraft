//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`wordcraft-cli mcp --connect`) wraps.
//!
//! Every request needs a key: the window's key (written by Start chat or `--control`, see Keys in
//! `docs/control-protocol.md`) or a chat member's key from `chat.join`. `chat.*` is answered on the
//! server thread, so a long poll never waits on the UI thread; everything else goes to the UI
//! thread with the caller's identity and a deadline.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wordcraft_chat::{Hub, Principal};
use wordcraft_ui_egui::ControlRequest;

/// Longer request lines are refused (and the connection closed).
const MAX_LINE: usize = 1 << 20;

/// The port Start chat tries first when no `--control` port is open (then any free port).
pub const CHAT_PORT: u16 = 7981;

/// The control server's time limits.
#[derive(Clone, Copy, Debug)]
struct Waits {
    /// A connection that sends no authorised request in this time is closed.
    first: Duration,
    /// The UI does not run a forwarded request after this (it answers `expired`).
    deadline: Duration,
    /// How long the server waits for the UI's answer (then `timeout`).
    reply: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        // Clients wait 60 s: a request a client gave up on never runs (55 s), and the server
        // answers before the client's own timeout (58 s).
        Waits { first: Duration::from_secs(30), deadline: Duration::from_secs(55), reply: Duration::from_secs(58) }
    }
}

/// The clock and the random source of this computer, for the chat hub.
pub struct SystemEnv;

impl wordcraft_chat::Env for SystemEnv {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
    }
    fn random(&self, buf: &mut [u8]) -> Result<(), String> {
        getrandom::fill(buf).map_err(|e| e.to_string())
    }
}

/// The key file of a running control server; [`KeyFile::remove`] it when the port closes.
pub struct KeyFile(PathBuf);

impl KeyFile {
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What every connection of one open port shares.
#[derive(Clone)]
struct Shared {
    tx: Sender<ControlRequest>,
    ctx: egui::Context,
    key: Arc<str>,
    hub: Arc<Hub>,
    waits: Waits,
    /// Set when the port closes: the accept loop ends, connections close at their next request.
    stop: Arc<AtomicBool>,
}

struct Running {
    addr: SocketAddr,
    /// Opened with `--control`: Stop chat leaves it open.
    by_flag: bool,
    stop: Arc<AtomicBool>,
    key_file: Option<KeyFile>,
}

impl Running {
    fn halt(self) {
        self.stop.store(true, SeqCst);
        // Wake the accept loop so it sees `stop` and drops the listener.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
        if let Some(f) = &self.key_file {
            f.remove();
        }
    }
}

/// This window's control port: opened by `--control` at start or by Start chat, closed by Stop
/// chat (only when Start chat opened it) and when the window closes.
pub struct ControlPort {
    ctx: egui::Context,
    hub: Arc<Hub>,
    settings: Option<PathBuf>,
    tx: Sender<ControlRequest>,
    waits: Waits,
    start_port: u16,
    running: Mutex<Option<Running>>,
}

impl ControlPort {
    pub fn new(ctx: egui::Context, hub: Arc<Hub>, settings: Option<PathBuf>) -> (Arc<ControlPort>, Receiver<ControlRequest>) {
        Self::with(ctx, hub, settings, CHAT_PORT, Waits::default())
    }

    /// [`ControlPort::new`] with the first port Start chat tries and the time limits (tests).
    fn with(
        ctx: egui::Context,
        hub: Arc<Hub>,
        settings: Option<PathBuf>,
        start_port: u16,
        waits: Waits,
    ) -> (Arc<ControlPort>, Receiver<ControlRequest>) {
        let (tx, rx) = channel();
        (Arc::new(ControlPort { ctx, hub, settings, tx, waits, start_port, running: Mutex::new(None) }), rx)
    }

    fn lock(&self) -> MutexGuard<'_, Option<Running>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `--control PORT`: open it now; it stays open until the window closes.
    pub fn open_flag(&self, port: u16) -> Result<String, String> {
        let mut g = self.lock();
        if let Some(r) = g.take() {
            r.halt();
        }
        let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("control server failed to bind 127.0.0.1:{port}: {e}"))?;
        let r = self.start_running(listener, true)?;
        let addr = r.addr.to_string();
        *g = Some(r);
        Ok(addr)
    }

    /// The window closes: stop serving and remove the key file (also for a `--control` port).
    pub fn shutdown(&self) {
        if let Some(r) = self.lock().take() {
            r.halt();
        }
    }

    /// Key, key file (see Keys in `docs/control-protocol.md`), accept thread.
    fn start_running(&self, listener: TcpListener, by_flag: bool) -> Result<Running, String> {
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let key = random_key().map_err(|e| format!("control server off: no random key ({e})"))?;
        let key_file = match self.settings.as_deref() {
            Some(dir) => {
                let p = wordcraft_control_key::key_path(dir, addr.port());
                match write_key_file(&p, &key) {
                    Ok(()) => Some(KeyFile(p)),
                    Err(e) => {
                        log::error!("control key file not written ({e}): only chat members can connect");
                        None
                    }
                }
            }
            None => {
                log::error!("no settings folder for the control key file: only chat members can connect");
                None
            }
        };
        match &key_file {
            Some(f) => log::info!("control server listening on {addr}, key in {}", f.path().display()),
            None => log::info!("control server listening on {addr}"),
        }
        let stop = Arc::new(AtomicBool::new(false));
        serve_on(
            listener,
            Shared { tx: self.tx.clone(), ctx: self.ctx.clone(), key: key.into(), hub: self.hub.clone(), waits: self.waits, stop: stop.clone() },
        );
        Ok(Running { addr, by_flag, stop, key_file })
    }
}

impl wordcraft_chat::Port for ControlPort {
    fn open(&self) -> Result<String, String> {
        let mut g = self.lock();
        if let Some(r) = g.as_ref() {
            return Ok(r.addr.to_string());
        }
        let listener = TcpListener::bind(("127.0.0.1", self.start_port))
            .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
            .map_err(|e| format!("no free port on 127.0.0.1: {e}"))?;
        let r = self.start_running(listener, false)?;
        let addr = r.addr.to_string();
        *g = Some(r);
        Ok(addr)
    }
    fn close(&self) {
        let mut g = self.lock();
        if g.as_ref().is_some_and(|r| !r.by_flag)
            && let Some(r) = g.take()
        {
            r.halt();
        }
    }
    fn address(&self) -> Option<String> {
        self.lock().as_ref().map(|r| r.addr.to_string())
    }
}

/// A new key: 32 random bytes from the operating system, as hex.
fn random_key() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Write the key file: a stale one (from a run that did not exit cleanly) is replaced; on Unix
/// the new file is created with mode 0600.
fn write_key_file(p: &Path, key: &str) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    let _ = std::fs::remove_file(p);
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let mut f = open.open(p).map_err(|e| format!("cannot create {}: {e}", p.display()))?;
    f.write_all(key.as_bytes()).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

/// Compare a key with the window's in time that does not depend on where they differ. An
/// empty key never matches.
fn keys_match(given: &str, key: &str) -> bool {
    let (a, b) = (given.as_bytes(), key.as_bytes());
    !b.is_empty() && a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `chat.poll`'s wait: `wait_s` seconds, at most 25 (also for junk), as ms.
fn poll_wait_ms(params: &Value) -> u64 {
    match params.get("wait_s").and_then(Value::as_f64) {
        Some(w) if w.is_finite() && w >= 0.0 => (w.min(25.0) * 1000.0) as u64,
        _ => 25_000,
    }
}

fn serve_on(listener: TcpListener, sh: Shared) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if sh.stop.load(SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let sh = sh.clone();
            std::thread::spawn(move || serve(stream, sh));
        }
    });
}

/// The read side of a connection with an optional overall deadline: each read waits only for
/// the time left, so blank lines, bad JSON or a slow drip of bytes cannot extend it.
struct Timed {
    s: TcpStream,
    deadline: Option<Instant>,
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(d) = self.deadline {
            let left = d.checked_duration_since(Instant::now()).filter(|l| !l.is_zero()).ok_or(std::io::ErrorKind::TimedOut)?;
            self.s.set_read_timeout(Some(left))?;
        }
        self.s.read(buf)
    }
}

impl Timed {
    fn set_deadline(&mut self, d: Option<Instant>) {
        self.deadline = d;
        if d.is_none() {
            let _ = self.s.set_read_timeout(None);
        }
    }
}

/// The next line (trimmed), `None` at the end of the stream or after a read error.
fn read_line(r: &mut BufReader<Timed>) -> Option<Result<String, &'static str>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf).ok()?;
    if n == 0 {
        return None;
    }
    if buf.len() > MAX_LINE {
        return Some(Err("line too long"));
    }
    Some(String::from_utf8(buf).map(|s| s.trim().to_string()).map_err(|_| "not UTF-8"))
}

/// Read and drop the rest of an oversize line, so closing the socket does not reset the
/// connection before the client has read the error. At most 64 MiB, within 2 s.
fn discard_rest_of_line(r: &mut BufReader<Timed>) {
    r.get_mut().set_deadline(Some(Instant::now() + Duration::from_secs(2)));
    let mut left: usize = 64 << 20;
    while left > 0 {
        let (used, done) = match r.fill_buf() {
            Ok([]) | Err(_) => return,
            Ok(b) => match b.iter().position(|&c| c == b'\n') {
                Some(i) => (i + 1, true),
                None => (b.len(), false),
            },
        };
        r.consume(used);
        if done {
            return;
        }
        left = left.saturating_sub(used);
    }
}

fn serve(stream: TcpStream, sh: Shared) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    // One overall deadline from accept to the first authorised request; after it, none (long
    // polls, pauses).
    let mut reader = BufReader::new(Timed { s: read, deadline: Some(Instant::now() + sh.waits.first) });
    let mut authorised = false;
    // The key an `auth` request gave this connection.
    let mut conn_key: Option<String> = None;
    while let Some(line) = read_line(&mut reader) {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": e}));
                if e == "line too long" {
                    discard_rest_of_line(&mut reader);
                }
                break;
            }
        };
        if line.is_empty() {
            continue;
        }
        let msg = match serde_json::from_str::<Value>(&line) {
            Ok(m) => m,
            // Close on a line that isn't JSON: an HTTP request (say, a web page's `fetch` to this
            // port) starts with a request line that never parses, so it can't carry a command in
            // its body, even before the key is checked.
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": format!("bad JSON: {e}; closing the connection")}));
                break;
            }
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        // The top-level `key`; `auth` may carry it in its params. For other methods `params.key`
        // is an ordinary parameter (`ui.key {"key": "Enter"}`).
        let given =
            msg.get("key").and_then(Value::as_str).or_else(|| if method == "auth" { params.get("key").and_then(Value::as_str) } else { None });
        let mut reply = |mut v: Value| -> bool {
            if let Some(o) = v.as_object_mut() {
                o.insert("id".into(), id.clone());
            }
            writeln!(out, "{v}").is_ok()
        };
        // The port closed (Stop chat, window closed): no more requests on this connection.
        if sh.stop.load(SeqCst) {
            let _ = reply(json!({"ok": false, "error": "unauthorized"}));
            break;
        }
        if method == "chat.join" {
            let code = params.get("code").and_then(Value::as_str).unwrap_or("");
            match sh.hub.join(code) {
                Ok((handle, key)) => {
                    if !reply(json!({"ok": true, "result": {"handle": handle, "key": key}})) {
                        break;
                    }
                    continue;
                }
                Err(e) => {
                    // One guess per connection.
                    let _ = reply(json!({"ok": false, "error": e.to_string()}));
                    break;
                }
            }
        }
        // Checked on every request: a member's key against the hub each time, so Remove and
        // Stop chat take effect at once.
        let k = given.or(conn_key.as_deref()).unwrap_or("");
        let who = if keys_match(k, &sh.key) { Some(Principal::Host) } else { sh.hub.member_for_key(k).map(Principal::Member) };
        let Some(who) = who else {
            let _ = reply(json!({"ok": false, "error": "unauthorized"}));
            break;
        };
        if !authorised {
            authorised = true;
            reader.get_mut().set_deadline(None);
        }
        if method == "auth" {
            conn_key = Some(k.to_string());
            if !reply(json!({"ok": true, "result": {}})) {
                break;
            }
            continue;
        }
        let r = match (method.as_str(), &who) {
            ("chat.poll", _) => {
                let after = params.get("after").and_then(Value::as_u64).unwrap_or(0);
                json!({"ok": true, "result": sh.hub.poll(after, poll_wait_ms(&params))})
            }
            ("chat.members", _) => json!({"ok": true, "result": sh.hub.members()}),
            ("chat.post", Principal::Member(h)) => match sh.hub.post_agent(h, params.get("text").and_then(Value::as_str).unwrap_or("")) {
                Ok(m) => json!({"ok": true, "result": m}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            ("chat.leave", Principal::Member(h)) => {
                sh.hub.remove(h);
                json!({"ok": true, "result": {}})
            }
            ("chat.leave", Principal::Host) => json!({"ok": false, "error": "only a member can leave"}),
            // Everything else, the host's `chat.post` included (the `chat.post` command posts as
            // the owner): the UI thread runs it, unless the deadline passed first.
            _ => {
                let deadline = wordcraft_ui_egui::now_ms() + sh.waits.deadline.as_secs_f64() * 1000.0;
                let (req, rrx) = ControlRequest::new(method.clone(), params.clone());
                if sh.tx.send(req.with_principal(who.clone()).with_deadline_ms(deadline)).is_err() {
                    break;
                }
                sh.ctx.request_repaint();
                rrx.recv_timeout(sh.waits.reply).unwrap_or_else(|_| json!({"ok": false, "error": "timeout"}))
            }
        };
        if !reply(r) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use wordcraft_chat::testing::TestEnv;
    use wordcraft_chat::{Hub, Port, Principal};

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn open_hub() -> Arc<Hub> {
        let h = Hub::new(TestEnv::at(1_000));
        h.set_open(true);
        h
    }

    fn boot_full(waits: Waits) -> (u16, Receiver<ControlRequest>, Arc<Hub>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = channel();
        let hub = open_hub();
        let shared = Shared { tx, ctx: egui::Context::default(), key: KEY.into(), hub: hub.clone(), waits, stop: Arc::new(AtomicBool::new(false)) };
        serve_on(listener, shared);
        (port, rx, hub)
    }

    fn boot_with(first: Duration) -> (u16, Receiver<ControlRequest>) {
        let (p, rx, _) = boot_full(Waits { first, ..Waits::default() });
        (p, rx)
    }

    fn boot() -> (u16, Receiver<ControlRequest>) {
        boot_with(Waits::default().first)
    }

    fn member(hub: &Hub, handle: &str) -> String {
        let (_, code) = hub.invite(handle).unwrap();
        hub.join(&code).unwrap().1
    }

    fn port_of(addr: &str) -> u16 {
        addr.rsplit_once(':').and_then(|(_, p)| p.parse().ok()).unwrap()
    }

    /// Answer the next request that reaches the UI with `ok`, and return it.
    fn answer(rx: &Receiver<ControlRequest>) -> ControlRequest {
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let _ = req.reply.send(json!({"ok": true, "result": {}}));
        req
    }

    /// One connection, several lines.
    struct Conn(BufReader<TcpStream>, TcpStream);

    impl Conn {
        fn open(port: u16) -> Self {
            let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            Self(BufReader::new(s.try_clone().unwrap()), s)
        }
        fn send(&mut self, line: &str) {
            writeln!(self.1, "{line}").unwrap();
        }
        fn recv(&mut self) -> Value {
            let mut r = String::new();
            let _ = self.0.read_line(&mut r);
            serde_json::from_str(&r).unwrap_or_default()
        }
        fn call(&mut self, line: &str) -> Value {
            self.send(line);
            self.recv()
        }
        /// The server closed the connection.
        fn closed(&mut self) -> bool {
            let mut r = String::new();
            matches!(self.0.read_line(&mut r), Ok(0))
        }
    }

    #[test]
    fn a_request_without_the_key_is_refused_and_closed() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        let r = c.call(r#"{"id":1,"method":"document.inspect"}"#);
        assert_eq!((r["ok"].clone(), r["error"].clone(), r["id"].clone()), (json!(false), json!("unauthorized"), json!(1)));
        assert!(c.closed());
        assert!(rx.try_recv().is_err(), "nothing reached the UI");
    }

    #[test]
    fn a_wrong_key_is_refused_and_closed() {
        let (port, rx) = boot();
        for key in ["nope", &KEY[..63], &format!("{KEY}0"), &KEY.to_uppercase()] {
            let mut c = Conn::open(port);
            assert_eq!(c.call(&format!(r#"{{"id":1,"key":"{key}","method":"document.inspect"}}"#))["error"], "unauthorized", "{key}");
            assert!(c.closed());
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn the_key_on_each_request() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        c.send(&format!(r#"{{"id":7,"key":"{KEY}","method":"document.inspect","params":{{"text":true}}}}"#));
        let req = answer(&rx);
        assert_eq!((req.method.as_str(), &req.params), ("document.inspect", &json!({"text": true})));
        let r = c.recv();
        assert_eq!((r["ok"].clone(), r["id"].clone()), (json!(true), json!(7)));
        // Every request needs it: one without the key is refused, also after good ones.
        assert_eq!(c.call(r#"{"id":8,"method":"document.inspect"}"#)["error"], "unauthorized");
        assert!(c.closed());
    }

    #[test]
    fn auth_once_per_connection() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"auth","key":"{KEY}"}}"#))["ok"], true);
        for id in 2..4 {
            c.send(&format!(r#"{{"id":{id},"method":"document.inspect"}}"#));
            answer(&rx);
            assert_eq!(c.recv()["ok"], true);
        }
        // `params.key` works for `auth` too.
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"auth","params":{{"key":"{KEY}"}}}}"#))["ok"], true);
        // A wrong key on an authenticated connection is still refused, and closes it.
        assert_eq!(c.call(r#"{"id":2,"key":"nope","method":"document.inspect"}"#)["error"], "unauthorized");
        assert!(c.closed());
        // A failed auth closes the connection.
        let mut c = Conn::open(port);
        assert_eq!(c.call(r#"{"id":1,"method":"auth","key":"nope"}"#)["error"], "unauthorized");
        assert!(c.closed());
    }

    #[test]
    fn params_key_is_only_a_secret_for_auth() {
        let (port, rx) = boot();
        // `params.key` does not authenticate other methods.
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"document.inspect","params":{{"key":"{KEY}"}}}}"#))["error"], "unauthorized");
        // It is an ordinary parameter for them (`ui.key` presses a key).
        let mut c = Conn::open(port);
        c.send(&format!(r#"{{"id":1,"key":"{KEY}","method":"ui.key","params":{{"key":"Enter"}}}}"#));
        let req = answer(&rx);
        assert_eq!((req.method.as_str(), &req.params), ("ui.key", &json!({"key": "Enter"})));
        assert_eq!(c.recv()["ok"], true);
    }

    #[test]
    fn a_line_over_1_mib_is_refused_and_closed() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        let big = format!(r#"{{"id":1,"key":"{KEY}","method":"document.inspect","pad":"{}"}}"#, "x".repeat(MAX_LINE));
        assert_eq!(c.call(&big)["error"], "line too long");
        assert!(c.closed());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_silent_connection_is_closed() {
        let (port, rx) = boot_with(Duration::from_millis(300));
        let mut quiet = Conn::open(port);
        let t0 = Instant::now();
        assert!(quiet.closed());
        assert!(t0.elapsed() < Duration::from_secs(3));
        // After an authorised request the connection may stay idle.
        let mut busy = Conn::open(port);
        assert_eq!(busy.call(&format!(r#"{{"id":1,"method":"auth","key":"{KEY}"}}"#))["ok"], true);
        std::thread::sleep(Duration::from_millis(600));
        busy.send(r#"{"id":2,"method":"document.inspect"}"#);
        answer(&rx);
        assert_eq!(busy.recv()["ok"], true);
    }

    #[test]
    fn the_first_request_limit_is_one_overall_deadline() {
        // Blank lines and bad JSON do not keep a connection without the key open.
        let (port, _rx) = boot_with(Duration::from_millis(300));
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut w = s.try_clone().unwrap();
        let feeder = std::thread::spawn(move || {
            for i in 0..40 {
                if w.write_all(if i % 2 == 0 { b"\n" } else { b"not json\n" }).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let t0 = Instant::now();
        let mut r = BufReader::new(s);
        r.get_ref().set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut line = String::new();
        loop {
            line.clear();
            if matches!(r.read_line(&mut line), Ok(0) | Err(_)) {
                break;
            }
            assert!(line.contains("bad JSON"), "{line}");
        }
        let _ = feeder.join();
        assert!(t0.elapsed() < Duration::from_millis(1500), "{:?}", t0.elapsed());
    }

    #[test]
    fn bad_json_closes_the_connection() {
        let (port, rx) = boot();
        let mut c = Conn::open(port);
        assert_eq!(c.call("not json")["ok"], false);
        assert!(c.closed());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn http_request_cannot_smuggle_a_command() {
        // What a web page's `fetch("http://127.0.0.1:<port>/", {method: "POST", body})` sends,
        // even with the key in the body.
        let (port, rx) = boot();
        let body = format!("\n{{\"key\":\"{KEY}\",\"method\":\"file.saveAs\",\"params\":{{\"path\":\"/tmp/x.docx\"}}}}\n");
        let req = format!("POST / HTTP/1.1\r\nHost: 127.0.0.1:7981\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        let mut c = Conn::open(port);
        c.1.write_all(req.as_bytes()).unwrap();
        let _ = c.recv();
        assert!(c.closed());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn keys_compare_whole() {
        assert!(keys_match(KEY, KEY));
        assert!(!keys_match(KEY, &KEY[..63]));
        assert!(!keys_match(&KEY[..63], KEY));
        assert!(!keys_match(KEY, &KEY.replace('f', "e")));
        assert!(!keys_match("", ""), "an empty key never matches");
    }

    #[test]
    fn random_keys_are_256_bits() {
        let (a, b) = (random_key().unwrap(), random_key().unwrap());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-control-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_key_file_is_written_after_the_bind_and_removed_on_exit() {
        let dir = temp_dir("life");
        let (cp, rx) = ControlPort::with(egui::Context::default(), open_hub(), Some(dir.clone()), 0, Waits::default());
        // The port is taken: no key file.
        let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        assert!(cp.open_flag(port).is_err());
        assert!(cp.address().is_none());
        assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none(), "no key file after a failed bind");
        drop(taken);
        // Bound: the key file holds the key that opens the port.
        let addr = cp.open_flag(port).unwrap();
        assert_eq!(port_of(&addr), port);
        let key_file = wordcraft_control_key::key_path(&dir, port);
        let key = std::fs::read_to_string(&key_file).unwrap();
        assert_eq!(key.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&key_file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut c = Conn::open(port);
        c.send(&format!(r#"{{"id":1,"key":"{key}","method":"document.inspect"}}"#));
        assert_eq!(answer(&rx).principal, Principal::Host);
        assert_eq!(c.recv()["ok"], true);
        // The window closes.
        cp.shutdown();
        assert!(!key_file.exists());
        assert!(cp.address().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_key_file_is_replaced() {
        let dir = temp_dir("stale");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("control-key.test");
        std::fs::write(&p, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        write_key_file(&p, KEY).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), KEY);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn join_then_member_post_and_poll() {
        let (port, _rx, hub) = boot_full(Waits::default());
        let (_, code) = hub.invite("@claude").unwrap();
        let mut c = Conn::open(port);
        let j = c.call(&format!(r#"{{"id":1,"method":"chat.join","params":{{"code":"{code}"}}}}"#));
        assert_eq!(j["result"]["handle"], "@claude");
        let key = j["result"]["key"].as_str().unwrap().to_string();
        assert_eq!(c.call(&format!(r#"{{"id":2,"key":"{key}","method":"chat.post","params":{{"text":"hello"}}}}"#))["ok"], true);
        let q = c.call(&format!(r#"{{"id":3,"key":"{key}","method":"chat.poll","params":{{"after":0,"wait_s":0}}}}"#));
        assert!(q["result"].as_array().is_some_and(|a| a.iter().any(|m| m["text"] == "hello" && m["role"] == "agent")));
    }

    #[test]
    fn a_failed_join_closes_the_connection() {
        let (port, rx, _hub) = boot_full(Waits::default());
        let mut c = Conn::open(port);
        assert_eq!(c.call(r#"{"id":1,"method":"chat.join","params":{"code":"ABCD-EFGH-JKMN"}}"#)["error"], "invite_invalid");
        assert!(c.closed());
        assert!(rx.try_recv().is_err(), "nothing reached the UI");
    }

    #[test]
    fn forwarded_requests_carry_the_caller_and_the_deadline() {
        let (port, rx, hub) = boot_full(Waits::default());
        let key = member(&hub, "@claude");
        let t = std::thread::spawn(move || Conn::open(port).call(&format!(r#"{{"id":1,"key":"{key}","method":"document.inspect"}}"#)));
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(req.principal, Principal::Member("@claude".into()));
        let left = req.deadline_ms.map(|d| d - wordcraft_ui_egui::now_ms());
        assert!(left.is_some_and(|l| l > 50_000.0 && l <= 55_000.0), "{left:?}");
        let _ = req.reply.send(json!({"ok": true, "result": {}}));
        assert_eq!(t.join().unwrap()["ok"], true);
        // The window key is the host: its chat.post goes to the UI (the chat.post command posts as the owner).
        let t =
            std::thread::spawn(move || Conn::open(port).call(&format!(r#"{{"id":2,"key":"{KEY}","method":"chat.post","params":{{"text":"hi"}}}}"#)));
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((req.method.as_str(), &req.principal), ("chat.post", &Principal::Host));
        let _ = req.reply.send(json!({"ok": true, "result": {}}));
        let _ = t.join();
    }

    #[test]
    fn only_members_leave() {
        let (port, _rx, _hub) = boot_full(Waits::default());
        assert_eq!(Conn::open(port).call(&format!(r#"{{"id":1,"key":"{KEY}","method":"chat.leave"}}"#))["error"], "only a member can leave");
    }

    #[test]
    fn poll_waits_at_most_25_seconds() {
        assert_eq!(poll_wait_ms(&json!({})), 25_000);
        assert_eq!(poll_wait_ms(&json!({"wait_s": 3})), 3_000);
        assert_eq!(poll_wait_ms(&json!({"wait_s": 0})), 0);
        assert_eq!(poll_wait_ms(&json!({"wait_s": 1e308})), 25_000);
        assert_eq!(poll_wait_ms(&json!({"wait_s": -1})), 25_000);
        assert_eq!(poll_wait_ms(&json!({"wait_s": "x"})), 25_000);
    }

    #[test]
    fn start_chat_opens_the_port_and_stop_closes_it() {
        let dir = temp_dir("startstop");
        let (port, _rx) = ControlPort::with(egui::Context::default(), open_hub(), Some(dir.clone()), 0, Waits::default());
        assert!(port.address().is_none(), "no port before Start chat");
        let addr = port.open().unwrap();
        assert_eq!(port.open().unwrap(), addr, "a second start keeps the port");
        let sa: std::net::SocketAddr = addr.parse().unwrap();
        assert!(TcpStream::connect_timeout(&sa, Duration::from_millis(500)).is_ok());
        let key_file = wordcraft_control_key::key_path(&dir, sa.port());
        assert_eq!(std::fs::read_to_string(&key_file).unwrap().len(), 64, "Start chat writes the window key");
        port.close();
        assert!(port.address().is_none());
        let end = Instant::now() + Duration::from_secs(2);
        while TcpStream::connect_timeout(&sa, Duration::from_millis(200)).is_ok() {
            assert!(Instant::now() < end, "port still open after Stop chat");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!key_file.exists(), "the key file goes with the port");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn start_chat_takes_another_port_when_the_first_is_busy() {
        let busy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let taken = busy.local_addr().unwrap().port();
        let dir = temp_dir("busy");
        let (port, _rx) = ControlPort::with(egui::Context::default(), open_hub(), Some(dir.clone()), taken, Waits::default());
        let addr = port.open().unwrap();
        assert_ne!(port_of(&addr), taken);
        port.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stop_chat_disconnects_every_member() {
        let dir = temp_dir("stop-members");
        let hub = open_hub();
        let (port, _rx) = ControlPort::with(egui::Context::default(), hub.clone(), Some(dir.clone()), 0, Waits::default());
        let chat = wordcraft_chat::Chat::new(hub.clone(), port.clone());
        let p = port_of(&chat.start().unwrap());
        let key = member(&hub, "@claude");
        let mut c = Conn::open(p);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"auth","key":"{key}"}}"#))["ok"], true);
        chat.stop();
        assert_eq!(c.call(r#"{"id":2,"method":"chat.members"}"#)["error"], "unauthorized");
        assert!(c.closed());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_control_flag_port_stays_open_after_stop_chat() {
        let dir = temp_dir("flag");
        let hub = open_hub();
        let (port, _rx) = ControlPort::with(egui::Context::default(), hub.clone(), Some(dir.clone()), 0, Waits::default());
        let free = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        let addr = port.open_flag(free).unwrap();
        let chat = wordcraft_chat::Chat::new(hub, port.clone());
        assert_eq!(chat.start().unwrap(), addr);
        chat.stop();
        assert_eq!(port.address(), Some(addr));
        port.shutdown();
        assert!(!wordcraft_control_key::key_path(&dir, free).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removed_member_is_refused() {
        let (port, rx, hub) = boot_full(Waits::default());
        let key = member(&hub, "@pi");
        assert!(!key.is_empty());
        assert!(hub.remove("@pi"));
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"key":"{key}","method":"chat.members"}}"#))["error"], "unauthorized");
        assert!(c.closed());
        assert!(rx.try_recv().is_err(), "nothing reached the UI");
    }

    #[test]
    fn auth_connection_loses_access_after_remove() {
        let (port, rx, hub) = boot_full(Waits::default());
        let key = member(&hub, "@pi");
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"auth","key":"{key}"}}"#))["ok"], true);
        assert_eq!(c.call(r#"{"id":2,"method":"chat.members"}"#)["ok"], true);
        hub.remove("@pi");
        assert_eq!(c.call(r#"{"id":3,"method":"chat.members"}"#)["error"], "unauthorized");
        assert!(c.closed());
        assert!(rx.try_recv().is_err(), "nothing reached the UI");
    }

    #[test]
    fn auth_connection_not_revived_by_reinviting_same_handle() {
        let (port, _rx, hub) = boot_full(Waits::default());
        let key = member(&hub, "@pi");
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"auth","key":"{key}"}}"#))["ok"], true);
        hub.remove("@pi");
        let new_key = member(&hub, "@pi");
        assert!(!new_key.is_empty());
        assert_ne!(key, new_key);
        assert_eq!(c.call(r#"{"id":2,"method":"chat.members"}"#)["error"], "unauthorized");
        assert!(c.closed());
    }

    /// A member's `listen`: one long-lived connection polling like the client's stream mode
    /// (`chat.poll {after, wait_s: 25}` in a loop). Every message it gets is sent to the test with
    /// the time it arrived.
    fn stream(port: u16, key: String) -> Receiver<(u64, String, Instant)> {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let Ok(s) = TcpStream::connect(("127.0.0.1", port)) else { return };
            let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
            let Ok(r) = s.try_clone() else { return };
            let (mut r, mut w) = (BufReader::new(r), s);
            let mut after = 0u64;
            loop {
                let poll = json!({"id": 1, "key": key, "method": "chat.poll", "params": {"after": after, "wait_s": 25}});
                if writeln!(w, "{poll}").is_err() {
                    return;
                }
                let mut line = String::new();
                if !matches!(r.read_line(&mut line), Ok(n) if n > 0) {
                    return;
                }
                let v: Value = serde_json::from_str(&line).unwrap_or_default();
                if v["ok"] != true {
                    return;
                }
                for m in v["result"].as_array().cloned().unwrap_or_default() {
                    let seq = m["seq"].as_u64().unwrap_or(0);
                    after = after.max(seq);
                    if tx.send((seq, m["text"].as_str().unwrap_or("").to_string(), Instant::now())).is_err() {
                        return;
                    }
                }
            }
        });
        rx
    }

    /// Wait until `text` arrives on the stream; how long it took after `t0`.
    fn arrival(rx: &Receiver<(u64, String, Instant)>, text: &str, t0: Instant) -> Option<Duration> {
        let end = Instant::now() + Duration::from_secs(5);
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok((_, t, at)) if t == text => return Some(at.saturating_duration_since(t0)),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
        None
    }

    #[test]
    fn owner_messages_reach_a_listening_member_at_once() {
        let (port, rx_ui, hub) = boot_full(Waits::default());
        let key = member(&hub, "@claude");
        let msgs = stream(port, key.clone());
        assert!(arrival(&msgs, "@claude joined the chat", Instant::now()).is_some());
        // The stream is now waiting inside a 25 s poll; the owner posts from another thread (the UI).
        std::thread::sleep(Duration::from_millis(300));
        let owner_post = |text: &str| {
            let (h, text) = (hub.clone(), text.to_string());
            let t0 = Instant::now();
            let _ = std::thread::spawn(move || h.post_owner(&text)).join();
            t0
        };
        let t0 = owner_post("first from the owner");
        let d = arrival(&msgs, "first from the owner", t0);
        assert!(d.is_some_and(|d| d < Duration::from_secs(1)), "owner message 1: {d:?}");
        // A host-key UI request (a screenshot) in between, answered by the "UI thread".
        let host = std::thread::spawn(move || Conn::open(port).call(&format!(r#"{{"id":9,"key":"{KEY}","method":"ui.screenshot","params":{{}}}}"#)));
        let req = rx_ui.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(req.principal, Principal::Host);
        std::thread::sleep(Duration::from_millis(200));
        let _ = req.reply.send(json!({"ok": true, "result": {}}));
        assert_eq!(host.join().unwrap()["ok"], true);
        std::thread::sleep(Duration::from_millis(300));
        let t0 = owner_post("second from the owner");
        let d = arrival(&msgs, "second from the owner", t0);
        assert!(d.is_some_and(|d| d < Duration::from_secs(1)), "owner message after a UI request: {d:?}");
        // A member post (its own `send`, another connection), then the owner again.
        let p = Conn::open(port).call(&format!(r#"{{"id":3,"key":"{key}","method":"chat.post","params":{{"text":"from the agent"}}}}"#));
        assert_eq!(p["ok"], true);
        assert!(arrival(&msgs, "from the agent", Instant::now()).is_some());
        std::thread::sleep(Duration::from_millis(300));
        let t0 = owner_post("third from the owner");
        let d = arrival(&msgs, "third from the owner", t0);
        assert!(d.is_some_and(|d| d < Duration::from_secs(1)), "owner message after a member post: {d:?}");
        // Several owner messages in a row, with the stream between polls and inside one.
        for (i, pause) in [0u64, 5, 50, 400].iter().enumerate() {
            std::thread::sleep(Duration::from_millis(*pause));
            let text = format!("owner {i}");
            let t0 = owner_post(&text);
            let d = arrival(&msgs, &text, t0);
            assert!(d.is_some_and(|d| d < Duration::from_secs(1)), "owner message {text}: {d:?}");
        }
    }

    #[test]
    fn the_server_gives_up_before_the_client_does() {
        // The client waits 60 s; the UI must not run a request after 55 s, and the server answers
        // before the client's own timeout.
        let w = Waits::default();
        assert_eq!(w.deadline, Duration::from_secs(55));
        assert!(w.reply > w.deadline && w.reply < Duration::from_secs(60), "{w:?}");
        // Short waits: the request carries the deadline, and the server answers "timeout" when
        // the UI did not answer in time.
        let (port, rx, _hub) =
            boot_full(Waits { first: Duration::from_secs(5), deadline: Duration::from_millis(200), reply: Duration::from_millis(400) });
        let (t0, start) = (wordcraft_ui_egui::now_ms(), Instant::now());
        let t = std::thread::spawn(move || Conn::open(port).call(&format!(r#"{{"id":1,"key":"{KEY}","method":"document.inspect"}}"#)));
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let ahead = req.deadline_ms.map(|d| d - t0);
        assert!(ahead.is_some_and(|l| l > 150.0 && l < 400.0), "{ahead:?}");
        let r = t.join().unwrap();
        assert_eq!(r["error"], "timeout", "{r}");
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(req);
    }
}

//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`wordcraft-cli mcp --connect`) wraps.
//!
//! Every request needs the window's key (`docs/control-protocol.md`, Keys): once the port is
//! bound, the app writes a random key to `<settings>/control-key.<instance>` (mode 0600) and
//! removes the file when the window closes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wordcraft_ui_egui::ControlRequest;
use wordcraft_ui_egui::ask_agent::AgentHub;

/// Longer request lines are refused (and the connection closed).
const MAX_LINE: usize = 1 << 20;

/// A connection that sends no authorised request in this time is closed.
const FIRST_REQUEST: Duration = Duration::from_secs(30);

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// The key file of a running control server; [`KeyFile::remove`] it when the window closes.
pub struct KeyFile(PathBuf);

impl KeyFile {
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn peer_open(s: &TcpStream) -> bool {
    if s.set_nonblocking(true).is_err() {
        return true;
    }
    let mut b = [0u8; 1];
    let r = s.peek(&mut b);
    let _ = s.set_nonblocking(false);
    match r {
        Ok(0) => false,
        Ok(_) => true,
        Err(e) => e.kind() == std::io::ErrorKind::WouldBlock,
    }
}

/// Bind `127.0.0.1:port`, write the key file into `settings` and serve.
pub fn start(port: u16, ctx: egui::Context, settings: Option<&Path>, hub: AgentHub) -> Option<(Receiver<ControlRequest>, Option<KeyFile>)> {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server failed to bind 127.0.0.1:{port}: {e}");
            return None;
        }
    };
    let key = match random_key() {
        Ok(k) => k,
        Err(e) => {
            log::error!("control server off: no random key ({e})");
            return None;
        }
    };
    let key_file = match settings {
        Some(dir) => {
            let p = wordcraft_control_key::key_path(dir, port);
            match write_key_file(&p, &key) {
                Ok(()) => Some(KeyFile(p)),
                Err(e) => {
                    log::error!("control key file not written ({e}): requests will be refused");
                    None
                }
            }
        }
        None => {
            log::error!("no settings folder for the control key file: requests will be refused");
            None
        }
    };
    match &key_file {
        Some(f) => log::info!("control server listening on 127.0.0.1:{port}, key in {}", f.path().display()),
        None => log::info!("control server listening on 127.0.0.1:{port}"),
    }
    Some((serve_on(listener, ctx, key, FIRST_REQUEST, hub), key_file))
}

fn random_key() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

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

fn keys_match(given: &str, key: &str) -> bool {
    let (a, b) = (given.as_bytes(), key.as_bytes());
    !b.is_empty() && a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn serve_on(listener: TcpListener, ctx: egui::Context, key: String, first: Duration, hub: AgentHub) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let key: Arc<str> = key.into();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, ctx, key, hub) = (tx.clone(), ctx.clone(), key.clone(), hub.clone());
            let conn = NEXT_CONN.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || serve(stream, tx, ctx, &key, first, &hub, conn));
        }
    });
    rx
}

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

fn discard_rest_of_line(r: &mut BufReader<Timed>) {
    r.get_mut().deadline = Some(Instant::now() + Duration::from_secs(2));
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

fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context, key: &str, first: Duration, hub: &AgentHub, connection: u64) {
    let Ok(read) = stream.try_clone() else { return };
    let Ok(probe) = stream.try_clone() else { return };
    let mut out = stream;
    let mut reader = BufReader::new(Timed { s: read, deadline: Some(Instant::now() + first) });
    let mut authorised = false;
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
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": format!("bad JSON: {e}; closing the connection")}));
                break;
            }
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let given =
            msg.get("key").and_then(Value::as_str).or_else(|| if method == "auth" { params.get("key").and_then(Value::as_str) } else { None });
        let mut reply = |mut v: Value| -> bool {
            if let Some(o) = v.as_object_mut() {
                o.insert("id".into(), id.clone());
            }
            writeln!(out, "{v}").is_ok()
        };
        let k = given.or(conn_key.as_deref()).unwrap_or("");
        if !keys_match(k, key) {
            let _ = reply(json!({"ok": false, "error": "unauthorized"}));
            break;
        }
        if !authorised {
            authorised = true;
            reader.get_mut().deadline = None;
            let _ = reader.get_mut().s.set_read_timeout(None);
        }
        if method == "auth" {
            conn_key = Some(k.to_string());
            if !reply(json!({"ok": true, "result": {}})) {
                break;
            }
            continue;
        }

        // Ask Agent: answered here, on the connection's own thread.
        if let Some(r) = hub.handle(connection, &method, &params, || peer_open(&probe)) {
            ctx.request_repaint();
            if !reply(r) {
                break;
            }
            continue;
        }

        let (mut req, rrx) = ControlRequest::new(method, params);
        req.connection = Some(connection);
        if tx.send(req).is_err() {
            hub.disconnect(connection);
            break;
        }
        ctx.request_repaint();
        let r = rrx.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| json!({"ok": false, "error": "timeout"}));
        if !reply(r) {
            hub.disconnect(connection);
            break;
        }
    }
    hub.disconnect(connection);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use wordcraft_ui_egui::ask_agent::Request;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn boot_agents(first: Duration) -> (u16, Receiver<ControlRequest>, AgentHub) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let hub = AgentHub::default();
        (port, serve_on(listener, egui::Context::default(), KEY.to_string(), first, hub.clone()), hub)
    }

    fn boot() -> (u16, Receiver<ControlRequest>) {
        let (port, rx, _) = boot_agents(FIRST_REQUEST);
        (port, rx)
    }

    fn answer(rx: &Receiver<ControlRequest>) -> ControlRequest {
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let _ = req.reply.send(json!({"ok": true, "result": {}}));
        req
    }

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
        fn closed(&mut self) -> bool {
            let mut r = String::new();
            matches!(self.0.read_line(&mut r), Ok(0))
        }
    }

    fn authed(port: u16) -> Conn {
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":0,"method":"auth","key":"{KEY}"}}"#))["ok"], true);
        c
    }

    #[test]
    fn agent_listen_and_reply_over_socket() {
        let (port, _rx, hub) = boot_agents(FIRST_REQUEST);
        let id = hub.ask(Request {
            id: 0,
            instruction: "Fix style".into(),
            history: vec![],
            selection: json!({"text": "sample"}),
            document: json!({"title": "Doc.docx"}),
            agent: None,
        });

        let mut c = authed(port);
        let r = c.call(r#"{"id":1,"method":"agent.listen","params":{"timeoutMs":1000}}"#);
        assert_eq!(r["result"]["id"], id);
        assert_eq!(r["result"]["instruction"], "Fix style");

        // Progress note
        let r_note = c.call(&format!(r#"{{"id":2,"method":"agent.reply","params":{{"id":{id},"text":"Working","done":false}}}}"#));
        assert_eq!(r_note["result"]["done"], false);

        // Final answer
        let r_ans = c.call(&format!(r#"{{"id":3,"method":"agent.reply","params":{{"id":{id},"text":"Done"}}}}"#));
        assert_eq!(r_ans["result"]["done"], true);

        let snap = hub.snapshot(id).unwrap();
        assert_eq!(snap.answer.as_deref(), Some("Done"));
    }

    #[test]
    fn engine_requests_carry_their_connection_for_the_ownership_check() {
        let (port, rx, hub) = boot_agents(FIRST_REQUEST);
        let id = hub.ask(Request { id: 0, instruction: "Edit".into(), history: vec![], selection: Value::Null, document: Value::Null, agent: None });
        let mut c = authed(port);
        assert_eq!(c.call(r#"{"id":1,"method":"agent.listen","params":{"timeoutMs":1000}}"#)["result"]["id"], id);
        c.send(&format!(r#"{{"id":2,"method":"engine.execute","params":{{"command":"text.insert","params":{{"text":"a"}},"request":{id}}}}}"#));
        assert!(answer(&rx).connection.is_some(), "the window checks ownership against this connection");
        assert_eq!(c.recv()["ok"], true);
    }

    #[test]
    fn a_request_without_the_key_is_refused_and_closed() {
        let (port, _rx) = boot();
        let mut c = Conn::open(port);
        let r = c.call(r#"{"id":1,"method":"document.inspect"}"#);
        assert_eq!(r["error"], "unauthorized");
        assert!(c.closed());
    }

    #[test]
    fn a_silent_connection_is_closed() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = serve_on(listener, egui::Context::default(), KEY.to_string(), Duration::from_millis(100), AgentHub::default());
        let mut quiet = Conn::open(port);
        std::thread::sleep(Duration::from_millis(250));
        assert!(quiet.closed());
    }
}

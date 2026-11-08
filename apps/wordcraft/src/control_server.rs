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

/// Maximum unused idle time and absolute line-completion budget. A live working owner
/// may renew only an empty wait; partial frames and unused listeners retain this bound.
const FIRST_REQUEST: Duration = Duration::from_secs(30);

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// The key file of a running control server; [`KeyFile::remove`] it when the window closes.
pub struct KeyFile(PathBuf);

impl KeyFile {
    /// Return the key file's location without opening or disclosing its contents.
    pub fn path(&self) -> &Path {
        &self.0
    }
    /// Best-effort cleanup on app shutdown; a missing key file is already removed.
    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Probe EOF without consuming any request bytes. Restore blocking mode before the next read;
/// an inconclusive mode-change failure does not by itself establish a disconnect.
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

/// Generate a fresh window key from the OS random source, failing instead of using a fallback.
fn random_key() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Replace the stale instance key with an exclusively created file (0600 on Unix). A creation
/// race fails closed rather than following another process's file or symlink.
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

/// Require a nonempty, equal-length key and accumulate differences across all of its bytes.
fn keys_match(given: &str, key: &str) -> bool {
    let (a, b) = (given.as_bytes(), key.as_bytes());
    !b.is_empty() && a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Accept independent sockets and allocate a lifetime-unique connection id before spawning
/// their reader. Stop at counter exhaustion so a new peer can never inherit an old claim.
fn serve_on(listener: TcpListener, ctx: egui::Context, key: String, first: Duration, hub: AgentHub) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let key: Arc<str> = key.into();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, ctx, key, hub) = (tx.clone(), ctx.clone(), key.clone(), hub.clone());
            let mut conn = NEXT_CONN.load(Ordering::SeqCst);
            let allocated = loop {
                let Some(next) = conn.checked_add(1) else { break false };
                match NEXT_CONN.compare_exchange_weak(conn, next, Ordering::SeqCst, Ordering::SeqCst) {
                    Ok(_) => break true,
                    Err(current) => conn = current,
                }
            };
            if !allocated {
                break;
            }
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
    /// Apply the remaining absolute budget to this socket read; receiving data never renews it.
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(d) = self.deadline {
            let left = d.checked_duration_since(Instant::now()).filter(|l| !l.is_zero()).ok_or(std::io::ErrorKind::TimedOut)?;
            self.s.set_read_timeout(Some(left))?;
        }
        self.s.read(buf)
    }
}

/// Read one bounded UTF-8 line without discarding a prefix on a retry. Only a wholly empty
/// read may renew its idle deadline; `keep_waiting` checks authenticated live work ownership.
/// Once any byte arrives, one absolute frame deadline applies even to a working owner. Restore
/// the prior idle deadline on return so blank lines cannot keep an unused connection alive.
fn read_line(r: &mut BufReader<Timed>, period: Duration, keep_waiting: impl Fn() -> bool) -> Option<Result<String, &'static str>> {
    let mut idle_deadline = r.get_ref().deadline;
    let mut buf = Vec::new();
    loop {
        if !buf.is_empty() && r.get_ref().deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return None;
        }
        match r.fill_buf() {
            Ok([]) => break,
            Ok(bytes) => {
                let end = bytes.iter().position(|&byte| byte == b'\n').map_or(bytes.len(), |index| index + 1);
                let used = end.min(MAX_LINE + 1 - buf.len());
                let complete = bytes.get(used.saturating_sub(1)) == Some(&b'\n');
                let first = buf.is_empty();
                buf.extend_from_slice(&bytes[..used]);
                r.consume(used);
                if first {
                    let frame = Instant::now() + period;
                    r.get_mut().deadline = Some(if keep_waiting() { frame } else { idle_deadline.map_or(frame, |idle| idle.min(frame)) });
                }
                if buf.len() > MAX_LINE {
                    return Some(Err("line too long"));
                }
                if complete {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error)
                if buf.is_empty() && matches!(error.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) && keep_waiting() =>
            {
                idle_deadline = Some(Instant::now() + period);
                r.get_mut().deadline = idle_deadline;
            }
            Err(_) => return None,
        }
    }
    r.get_mut().deadline = idle_deadline;
    if buf.is_empty() {
        return None;
    }
    Some(String::from_utf8(buf).map(|s| s.trim().to_string()).map_err(|_| "not UTF-8"))
}

/// Drain a rejected oversized frame for at most two seconds and 64 MiB. This bounded cleanup
/// lets the error reply reach a writer without granting an unlimited read or allocation.
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

/// Authenticate every request, then keep only long-poll listen on this socket thread. Native
/// edits and replies use the window's serialized queue; idle grace queries hub ownership only.
/// EOF, frame failure or failed writes always release this exact connection's claims.
fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context, key: &str, first: Duration, hub: &AgentHub, connection: u64) {
    let Ok(read) = stream.try_clone() else { return };
    let Ok(probe) = stream.try_clone() else { return };
    let mut out = stream;
    if out.set_write_timeout(Some(first)).is_err() {
        return;
    }
    let mut reader = BufReader::new(Timed { s: read, deadline: Some(Instant::now() + first) });
    let mut authorised = false;
    let mut conn_key: Option<String> = None;
    while let Some(line) = read_line(&mut reader, first, || authorised && hub.owns_work(connection) && peer_open(&probe)) {
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
            reader.get_mut().deadline = Some(Instant::now() + first);
        }
        if method == "auth" {
            conn_key = Some(k.to_string());
            if !reply(json!({"ok": true, "result": {}})) {
                break;
            }
            reader.get_mut().deadline = Some(Instant::now() + first);
            continue;
        }

        // Only the long poll runs on the socket thread. Replies resolve and validate their
        // captured document on the UI/document owner's serialized command queue.
        if method == "agent.listen"
            && let Some(r) = hub.handle(connection, &method, &params, || peer_open(&probe), |_| None)
        {
            ctx.request_repaint();
            if !reply(r) {
                break;
            }
            reader.get_mut().deadline = Some(Instant::now() + first);
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
        reader.get_mut().deadline = Some(Instant::now() + first);
    }
    hub.disconnect(connection);
}

#[cfg(test)]
mod tests {
    mod idle;
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use wordcraft_ui_egui::ask_agent::{Request, Target};

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// Bind an ephemeral loopback listener with a synthetic key and an injectable deadline.
    /// Return the UI queue and the same hub used by its socket threads.
    fn boot_agents(first: Duration) -> (u16, Receiver<ControlRequest>, AgentHub) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let hub = AgentHub::default();
        (port, serve_on(listener, egui::Context::default(), KEY.to_string(), first, hub.clone()), hub)
    }

    /// Use the production deadline for transport cases that do not measure idle expiry.
    fn boot() -> (u16, Receiver<ControlRequest>) {
        let (port, rx, _) = boot_agents(FIRST_REQUEST);
        (port, rx)
    }

    /// A client that authenticates but never reads cannot retain a listener slot forever.
    #[test]
    fn authenticated_client_that_never_reads_is_disconnected_after_idle_timeout() {
        let (port, _rx, hub) = boot_agents(Duration::from_millis(200));
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        writeln!(stream, "{}", json!({"id":0,"method":"auth","key":KEY})).unwrap();
        writeln!(stream, "{}", json!({"id":1,"method":"agent.listen","params":{"timeoutMs":0,"agent":"Never reads"}})).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut registered = false;
        while Instant::now() < deadline {
            if hub.agents().is_empty() {
                if registered {
                    return;
                }
            } else {
                registered = true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(registered, "the authenticated long-poll connection was registered");
        assert!(hub.agents().is_empty(), "idle authenticated clients release their slot even when no reply was read");
    }

    /// Acknowledge one queued request while returning its original connection metadata to the test.
    fn answer(rx: &Receiver<ControlRequest>) -> ControlRequest {
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let _ = req.reply.send(json!({"ok": true, "result": {}}));
        req
    }

    struct Conn(BufReader<TcpStream>, TcpStream);

    impl Conn {
        /// Connect a real test socket with a bounded receive wait, retaining separate read/write handles.
        fn open(port: u16) -> Self {
            let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            Self(BufReader::new(s.try_clone().unwrap()), s)
        }
        /// Send exactly one newline-delimited request over the real TCP writer.
        fn send(&mut self, line: &str) {
            writeln!(self.1, "{line}").unwrap();
        }
        /// Decode one server response; EOF or an unreadable reply produces null for explicit assertions.
        fn recv(&mut self) -> Value {
            let mut r = String::new();
            let _ = self.0.read_line(&mut r);
            serde_json::from_str(&r).unwrap_or_default()
        }
        /// Perform a synchronous request/response exchange through the wire protocol.
        fn call(&mut self, line: &str) -> Value {
            self.send(line);
            self.recv()
        }
        /// Distinguish a clean server EOF from data or a receive timeout.
        fn closed(&mut self) -> bool {
            let mut r = String::new();
            matches!(self.0.read_line(&mut r), Ok(0))
        }
    }

    /// Authenticate once with the synthetic test key before exercising inherited per-socket credentials.
    fn authed(port: u16) -> Conn {
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":0,"method":"auth","key":"{KEY}"}}"#))["ok"], true);
        c
    }

    #[test]
    /// Progress and final replies cross the UI queue, while listen stays on the socket reader.
    /// The final answer must retain the original request identity and complete its ownership.
    fn agent_listen_and_reply_over_socket() {
        let (port, rx, hub) = boot_agents(FIRST_REQUEST);
        let id = hub
            .ask(Request {
                id: 0,
                instruction: "Fix style".into(),
                history: vec![],
                selection: json!({"text": "sample"}),
                document: json!({"title": "Doc.docx"}),
                target: Target { document: "test".into(), revision: 1 },
                agent: None,
            })
            .unwrap();

        // A real window dispatches replies on its document-owning thread. This fixture supplies
        // that same serialized hop with a known, current document token.
        let answer_hub = hub.clone();
        let worker = std::thread::spawn(move || {
            for _ in 0..2 {
                let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
                assert_eq!(request.method, "agent.reply");
                let response = answer_hub
                    .handle(
                        request.connection.unwrap(),
                        &request.method,
                        &request.params,
                        || true,
                        |_| Some(Target { document: "test".into(), revision: 1 }),
                    )
                    .unwrap();
                request.reply.send(response).unwrap();
            }
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
        worker.join().unwrap();
    }

    #[test]
    /// An engine request must carry the server's connection identity to the UI queue; a caller's
    /// request number alone cannot identify the socket that owns the claim.
    fn engine_requests_carry_their_connection_for_the_ownership_check() {
        let (port, rx, hub) = boot_agents(FIRST_REQUEST);
        let id = hub
            .ask(Request {
                id: 0,
                instruction: "Edit".into(),
                history: vec![],
                selection: Value::Null,
                document: Value::Null,
                target: Target { document: "test".into(), revision: 1 },
                agent: None,
            })
            .unwrap();
        let mut c = authed(port);
        assert_eq!(c.call(r#"{"id":1,"method":"agent.listen","params":{"timeoutMs":1000}}"#)["result"]["id"], id);
        c.send(&format!(r#"{{"id":2,"method":"engine.execute","params":{{"command":"text.insert","params":{{"text":"a"}},"request":{id}}}}}"#));
        assert!(answer(&rx).connection.is_some(), "the window checks ownership against this connection");
        assert_eq!(c.recv()["ok"], true);
    }

    #[test]
    /// Reject unauthenticated native access and close its socket instead of leaving a reusable session.
    fn a_request_without_the_key_is_refused_and_closed() {
        let (port, _rx) = boot();
        let mut c = Conn::open(port);
        let r = c.call(r#"{"id":1,"method":"document.inspect"}"#);
        assert_eq!(r["error"], "unauthorized");
        assert!(c.closed());
    }

    #[test]
    /// A peer that sends no first request cannot retain a socket thread indefinitely.
    fn a_silent_connection_is_closed() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = serve_on(listener, egui::Context::default(), KEY.to_string(), Duration::from_millis(100), AgentHub::default());
        let mut quiet = Conn::open(port);
        std::thread::sleep(Duration::from_millis(250));
        assert!(quiet.closed());
    }
}

//! Where MCP tool calls end up: a control-channel method call.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

/// Something that answers control-channel methods (`engine.execute`, `document.inspect`,
/// `ui.pointer`, `ui.render`, …). See `wordcraft_ui_egui::control` for the full list.
pub trait Backend {
    /// Call one method. `Ok` carries the `result`, `Err` the error message.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// True when a real UI is attached (`ui.screenshot`, `ui.click`, dialogs… work).
    fn has_ui(&self) -> bool;
    /// Short human description ("headless", "connected to 127.0.0.1:7981").
    fn describe(&self) -> String;
    /// The MCP client's name (`initialize`'s `clientInfo.name`): an app backend names the agent
    /// with it, so its Ask Agent edits are tracked as `Agent (name)`.
    fn set_client(&mut self, _name: &str) {}
}

/// Most Ask Agent requests the bridge keeps open at once (older ones are forgotten).
const MAX_OPEN: usize = 16;

/// What the bridge knows about Ask Agent: its client's name and the requests it took and hasn't
/// closed, oldest first. While one is open, the bridge's `engine.execute` calls carry its id
/// (`request`), so the app records those edits as the agent's tracked changes; `agent.listen`
/// carries the client's name. Shared by the bridge's connections (`--channel` listens on its own).
#[derive(Default, Debug)]
pub struct AgentState {
    client: Option<String>,
    open: Vec<u64>,
}

impl AgentState {
    /// The request ids taken and not yet closed, oldest first.
    pub fn open(&self) -> &[u64] {
        &self.open
    }

    fn before(&self, method: &str, params: &mut Value) {
        let Some(p) = params.as_object_mut() else { return };
        match method {
            "agent.listen" if !p.contains_key("agent") => {
                if let Some(c) = &self.client {
                    p.insert("agent".into(), json!(c));
                }
            }
            "engine.execute" if !p.contains_key("request") => {
                if let Some(id) = self.open.first() {
                    p.insert("request".into(), json!(id));
                }
            }
            _ => {}
        }
    }

    fn after(&mut self, method: &str, params: &Value, result: &Result<Value, String>) {
        match method {
            "agent.listen" => {
                if let Ok(id) = result.as_ref().map(|v| v.get("id").and_then(Value::as_u64)) {
                    self.open.extend(id);
                    if self.open.len() > MAX_OPEN {
                        self.open.remove(0);
                    }
                }
            }
            "agent.reply" => {
                let id = params.get("id").and_then(Value::as_u64);
                let done = params.get("done").and_then(Value::as_bool).unwrap_or(true);
                let cancelled = result.as_ref().is_ok_and(|v| v.get("cancelled").and_then(Value::as_bool) == Some(true));
                if done || cancelled || result.is_err() {
                    self.open.retain(|o| Some(*o) != id);
                }
            }
            // The app refused an edit because the request is over (cancelled, answered, gone):
            // later calls go without it.
            "engine.execute" => {
                if let (Err(e), Some(id)) = (result, params.get("request").and_then(Value::as_u64))
                    && (e.contains(&format!("Ask Agent request {id} ")) || e.starts_with("no Ask Agent request"))
                    && !e.contains("asks for an answer only")
                {
                    self.open.retain(|o| *o != id);
                }
            }
            _ => {}
        }
    }
}

/// Looks up the control key for an address (`HOST:PORT`).
type KeyFinder = Box<dyn Fn(&str) -> Result<String, String> + Send>;

/// A running WordCraft app, reached through its loopback control port.
pub struct Remote {
    addr: String,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
    /// The connection leads to a loopback address: requests carry the control key.
    send_key: bool,
    /// The control key, looked up once per connection (the app picks a new one at every start).
    key: Option<String>,
    find_key: KeyFinder,
    /// Ask Agent: the client's name and the requests taken ([`AgentState`]).
    agent: Arc<Mutex<AgentState>>,
}

/// The control key: `env` (`WORDCRAFT_CONTROL_KEY`) when set, else the app's key file for
/// `port` in `settings`. The error says where it looked.
fn lookup_key(env: Option<String>, settings: Option<&Path>, port: Option<u16>) -> Result<String, String> {
    if let Some(k) = env.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        return Ok(k);
    }
    let (Some(settings), Some(port)) = (settings, port) else {
        return Err("no control key: set WORDCRAFT_CONTROL_KEY (no settings folder or port to find the key file)".to_string());
    };
    let path = wordcraft_control_key::key_path(settings, port);
    match std::fs::read_to_string(&path).map(|k| k.trim().to_string()) {
        Ok(k) if !k.is_empty() => Ok(k),
        _ => Err(format!("no control key: set WORDCRAFT_CONTROL_KEY, or start the app with --control {port} (it writes {})", path.display())),
    }
}

/// The key for the app at `addr`, from the environment or the app's key file.
fn find_key(addr: &str) -> Result<String, String> {
    let port = addr.rsplit_once(':').and_then(|(_, p)| p.parse().ok());
    lookup_key(std::env::var("WORDCRAFT_CONTROL_KEY").ok(), wordcraft_control_key::settings_dir().as_deref(), port)
}

/// Whether `addr` (`HOST:PORT`, as given to `--connect`) names a loopback address: 127.0.0.0/8,
/// ::1 or `localhost`. Other host names are not resolved: they never count.
fn is_loopback_addr(addr: &str) -> bool {
    let loopback = |ip: IpAddr| ip.to_canonical().is_loopback();
    if let Ok(sa) = addr.parse::<SocketAddr>() {
        return loopback(sa.ip());
    }
    if let Ok(ip) = addr.parse::<IpAddr>() {
        return loopback(ip);
    }
    let host = addr.rsplit_once(':').map_or(addr, |(h, _)| h);
    let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    host.parse::<IpAddr>().map_or_else(|_| host.eq_ignore_ascii_case("localhost"), loopback)
}

/// The key goes only to a loopback `--connect` address, and only when the connection really
/// leads to one (`peer`, so a `localhost` that resolves elsewhere gets no key either).
fn may_send_key(addr: &str, peer: Option<SocketAddr>) -> bool {
    is_loopback_addr(addr) && peer.is_some_and(|p| p.ip().to_canonical().is_loopback())
}

impl Remote {
    /// Connect to `addr` (`127.0.0.1:7981`), failing fast when nothing is listening.
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        Self::connect_with(addr, Box::new(find_key))
    }

    fn connect_with(addr: &str, find_key: KeyFinder) -> std::io::Result<Self> {
        let mut r = Self { addr: addr.to_string(), conn: None, next_id: 1, send_key: false, key: None, find_key, agent: Default::default() };
        r.reconnect()?;
        if let Some(w) = r.key_warning() {
            log::warn!("{w}");
        }
        Ok(r)
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// This connection's Ask Agent state, to share with another connection of the same bridge.
    pub fn agent_state(&self) -> Arc<Mutex<AgentState>> {
        self.agent.clone()
    }

    /// Share display/tagging state only. This does not transfer a server claim; use SharedRemote for channel ownership.
    pub fn sharing(mut self, state: Arc<Mutex<AgentState>>) -> Self {
        self.agent = state;
        self
    }

    fn agent(&self) -> std::sync::MutexGuard<'_, AgentState> {
        self.agent.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Why requests go without the control key (the address is not loopback), if they do.
    pub fn key_warning(&self) -> Option<String> {
        (!self.send_key).then(|| {
            format!(
                "{} is not a loopback address, so the control key is not sent (it only goes to 127.0.0.0/8, ::1 and localhost) and the app refuses requests; reach the app through 127.0.0.1, for example with an SSH tunnel",
                self.addr
            )
        })
    }

    fn reconnect(&mut self) -> std::io::Result<()> {
        self.conn = None;
        self.key = None;
        let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {}", self.addr));
        for sa in self.addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&sa, Duration::from_millis(800)) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    // The app answers within 60 s (its own timeout); leave headroom.
                    s.set_read_timeout(Some(Duration::from_secs(90))).ok();
                    self.send_key = may_send_key(&self.addr, s.peer_addr().ok());
                    let read = s.try_clone()?;
                    self.conn = Some((BufReader::new(read), s));
                    return Ok(());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// The request line; it carries the key when this connection may (looked up once per
    /// connection).
    fn request_line(&mut self, id: u64, method: &str, params: &Value) -> Result<String, String> {
        let mut req = json!({"id": id, "method": method, "params": params});
        if self.send_key {
            let key = match &self.key {
                Some(k) => k.clone(),
                None => (self.find_key)(&self.addr)?,
            };
            req["key"] = json!(key);
            self.key = Some(key);
        }
        Ok(req.to_string())
    }

    fn roundtrip(&mut self, line: &str) -> std::io::Result<String> {
        let Some((reader, writer)) = self.conn.as_mut() else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "not connected"));
        };
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        let mut reply = String::new();
        if reader.read_line(&mut reply)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "control channel closed"));
        }
        Ok(reply)
    }

    /// One request and its reply. A lost response is ambiguous and is never replayed.
    fn exchange(&mut self, id: u64, method: &str, params: &Value) -> Result<Value, String> {
        let unreachable = |addr: &str, e: std::io::Error| format!("WordCraft app at {addr} is not reachable: {e}");
        if self.conn.is_none() {
            self.reconnect().map_err(|e| unreachable(&self.addr, e))?;
        }
        let line = self.request_line(id, method, params)?;
        let reply = self.roundtrip(&line).map_err(|e| {
            self.conn = None;
            format!("AMBIGUOUS_RESPONSE: request was not replayed; reconnect and inspect: {e}")
        })?;
        serde_json::from_str(reply.trim()).map_err(|e| format!("bad reply from app: {e}"))
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let mut params = params;
        self.agent().before(method, &mut params);
        let r = self.call_app(method, &params);
        self.agent().after(method, &params, &r);
        r
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        format!("connected to the WordCraft app at {}", self.addr)
    }

    fn set_client(&mut self, name: &str) {
        self.agent().client = Some(name.chars().filter(|c| !c.is_control()).take(64).collect());
    }
}

impl Remote {
    /// One method call to the app; `Ok` carries the `result`.
    fn call_app(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        let params = params.clone();
        let id = self.next_id;
        self.next_id += 1;
        let unauthorized = |v: &Value| v.get("error").and_then(Value::as_str) == Some("unauthorized");
        let mut v = self.exchange(id, method, &params)?;
        if unauthorized(&v) && self.send_key {
            // A stale key (the app restarted on this port): the app closed the connection; a new
            // one reads the key again. Once.
            self.conn = None;
            v = self.exchange(id, method, &params)?;
        }
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else if unauthorized(&v) {
            self.conn = None;
            Err(self.key_warning().map_or_else(
                || {
                    format!(
                        "unauthorized: the WordCraft app at {} refused the control key (from WORDCRAFT_CONTROL_KEY or the app's key file)",
                        self.addr
                    )
                },
                |w| format!("unauthorized: {w}"),
            ))
        } else {
            let error = v.get("error").and_then(Value::as_str).unwrap_or("unknown error");
            Err(v.get("errorCode").and_then(Value::as_str).map_or_else(|| error.to_string(), |code| format!("{code}: {error}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, TcpListener};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, channel};

    #[test]
    fn key_from_env_then_key_file() {
        let dir = std::env::temp_dir().join(format!("wordcraft-mcp-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = wordcraft_control_key::key_path(&dir, 7981);
        // Neither: the error names the variable and the file it looked for.
        let e = lookup_key(None, Some(&dir), Some(7981)).unwrap_err();
        assert!(e.contains("WORDCRAFT_CONTROL_KEY") && e.contains(&path.display().to_string()), "{e}");
        // The key file, trimmed; an empty variable does not count.
        std::fs::write(&path, "  file-key\n").unwrap();
        assert_eq!(lookup_key(None, Some(&dir), Some(7981)).unwrap(), "file-key");
        assert_eq!(lookup_key(Some(" ".into()), Some(&dir), Some(7981)).unwrap(), "file-key");
        // The variable wins.
        assert_eq!(lookup_key(Some(" env-key ".into()), Some(&dir), Some(7981)).unwrap(), "env-key");
        assert_eq!(lookup_key(Some("env-key".into()), None, None).unwrap(), "env-key");
        // Read again every time: the app writes a new key at every start.
        std::fs::write(&path, "new-key").unwrap();
        assert_eq!(lookup_key(None, Some(&dir), Some(7981)).unwrap(), "new-key");
        // Another port, another file.
        assert!(lookup_key(None, Some(&dir), Some(7982)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_only_for_loopback_addresses() {
        let peer = |s: &str| s.parse::<SocketAddr>().ok();
        for (addr, at) in [
            ("127.0.0.1:7981", "127.0.0.1:7981"),
            ("127.0.0.2:7981", "127.0.0.2:7981"),
            ("[::1]:7981", "[::1]:7981"),
            ("[::ffff:127.0.0.1]:7981", "[::ffff:127.0.0.1]:7981"),
            ("localhost:7981", "127.0.0.1:7981"),
            ("localhost:7981", "[::1]:7981"),
            ("LocalHost:7981", "127.0.0.1:7981"),
        ] {
            assert!(may_send_key(addr, peer(at)), "{addr} at {at}");
        }
        for (addr, at) in [
            ("192.0.2.20:7981", "192.0.2.20:7981"),
            ("10.0.0.7:7981", "10.0.0.7:7981"),
            ("[fe80::1]:7981", "[fe80::1]:7981"),
            ("0.0.0.0:7981", "127.0.0.1:7981"),
            // Host names other than localhost: even when they lead to this machine.
            ("workstation:7981", "127.0.1.1:7981"),
            ("example.org:7981", "93.184.215.14:7981"),
            ("localhost.example.org:7981", "127.0.0.1:7981"),
            // `localhost` that does not lead to a loopback address.
            ("localhost:7981", "192.0.2.20:7981"),
        ] {
            assert!(!may_send_key(addr, peer(at)), "{addr} at {at}");
        }
        // No peer address: no key.
        assert!(!may_send_key("127.0.0.1:7981", None));
    }

    /// A stand-in for the app on 127.0.0.1: every request line goes to the returned channel and
    /// gets `answer(request)`: a reply and whether to close the connection after it, or `None`
    /// to close it without a reply (the app quit).
    fn fake_app(answer: impl Fn(&Value) -> Option<(Value, bool)> + Send + Sync + 'static) -> (u16, Receiver<Value>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = channel();
        let answer = Arc::new(answer);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tx, answer) = (tx.clone(), answer.clone());
                std::thread::spawn(move || {
                    let mut out = stream.try_clone().unwrap();
                    for line in BufReader::new(stream).lines() {
                        let Ok(req) = serde_json::from_str::<Value>(&line.unwrap_or_default()) else { return };
                        let _ = tx.send(req.clone());
                        let Some((mut reply, close)) = answer(&req) else { return };
                        reply["id"] = req["id"].clone();
                        if writeln!(out, "{reply}").is_err() || close {
                            return;
                        }
                    }
                });
            }
        });
        (port, rx)
    }

    fn fixed_key(key: &'static str) -> KeyFinder {
        Box::new(move |_| Ok(key.to_string()))
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn sends_the_key_to_loopback() {
        let (port, seen) = fake_app(|_| Some((json!({"ok": true, "result": 7}), false)));
        for host in ["127.0.0.1", "localhost"] {
            let mut r = Remote::connect_with(&format!("{host}:{port}"), fixed_key("k1")).unwrap();
            assert!(r.key_warning().is_none(), "{host}");
            assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(7));
            assert_eq!(seen.recv_timeout(WAIT).unwrap()["key"], "k1", "{host}");
        }
    }

    #[test]
    fn reads_the_key_again_on_a_new_connection() {
        // The app restarted with a new key: the old key is refused (and the connection closed).
        let (port, seen) = fake_app(|req| {
            Some(if req["key"] == "new" { (json!({"ok": true, "result": 1}), false) } else { (json!({"ok": false, "error": "unauthorized"}), true) })
        });
        let reads = Arc::new(AtomicUsize::new(0));
        let n = reads.clone();
        let finder: KeyFinder = Box::new(move |_| Ok(if n.fetch_add(1, Ordering::SeqCst) == 0 { "old" } else { "new" }.to_string()));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{port}"), finder).unwrap();
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(1));
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["key"], "old");
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["key"], "new");
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        // The new key is kept for the next requests on the same connection.
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(1));
        assert_eq!(reads.load(Ordering::SeqCst), 2);

        // An ambiguous lost response is never replayed. The next explicit call reconnects.
        let (port, seen) = fake_app(|req| if req["id"] == 2 { None } else { Some((json!({"ok": true, "result": 1}), false)) });
        let reads = Arc::new(AtomicUsize::new(0));
        let n = reads.clone();
        let finder: KeyFinder = Box::new(move |_| Ok(format!("key-{}", n.fetch_add(1, Ordering::SeqCst))));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{port}"), finder).unwrap();
        r.call("document.inspect", json!({})).unwrap();
        r.call("document.inspect", json!({})).unwrap_err();
        let keys: Vec<Value> = seen.try_iter().map(|v| v["key"].clone()).collect();
        assert_eq!(keys.first(), Some(&json!("key-0")), "{keys:?}");
        assert_eq!(keys.len(), 2, "the failed call was sent exactly once");
        assert_eq!(keys.last(), Some(&json!("key-0")), "{keys:?}");
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(1));
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["key"], "key-1");
    }

    #[test]
    fn no_key_for_other_hosts_and_a_clear_error() {
        // There is no LAN address to test with here: same stand-in, with the decision a LAN
        // address gets.
        let (port, seen) = fake_app(|req| {
            Some(if req.get("key").is_some() {
                (json!({"ok": true, "result": 1}), false)
            } else {
                (json!({"ok": false, "error": "unauthorized"}), true)
            })
        });
        let finder: KeyFinder = Box::new(|_| Err("the key must not be read for another host".to_string()));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{port}"), finder).unwrap();
        r.send_key = false;
        assert!(r.key_warning().is_some_and(|w| w.contains("not a loopback address")));
        let e = r.call("document.inspect", json!({})).unwrap_err();
        assert!(e.contains("not a loopback address") && e.contains("not sent"), "{e}");
        assert!(seen.recv_timeout(WAIT).unwrap().get("key").is_none());
    }

    /// Ask Agent: the bridge names its agent on `agent.listen`, tags its edits with the open
    /// request until it is answered, and stops when the app says the request is over.
    #[test]
    fn edits_carry_the_open_request_until_it_is_closed() {
        let (port, seen) = fake_app(|req| {
            let method = req["method"].as_str().unwrap_or("");
            let reply = match method {
                "agent.listen" => json!({"ok": true, "result": {"id": 5, "instruction": "shorten"}}),
                "engine.execute" if req["params"]["request"] == 6 => {
                    json!({"ok": false, "error": "Ask Agent request 6 was cancelled by the person: `text.insert` was not run"})
                }
                "agent.reply" => json!({"ok": true, "result": {"id": req["params"]["id"], "shown": true}}),
                _ => json!({"ok": true, "result": {}}),
            };
            Some((reply, false))
        });
        let mut r = Remote::connect_with(&format!("127.0.0.1:{port}"), fixed_key("k")).unwrap();
        r.set_client("test-client\u{7}");
        r.call("engine.execute", json!({"command": "text.insert", "params": {"text": "a"}})).unwrap();
        assert!(seen.recv_timeout(WAIT).unwrap()["params"].get("request").is_none(), "no request open yet");
        r.call("agent.listen", json!({"timeoutMs": 10})).unwrap();
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["params"]["agent"], "test-client", "named after the MCP client");
        r.call("engine.execute", json!({"command": "text.insert", "params": {"text": "b"}})).unwrap();
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["params"]["request"], 5, "edits carry the open request");
        // A progress note keeps it open; the answer closes it.
        r.call("agent.reply", json!({"id": 5, "text": "half way", "done": false})).unwrap();
        let _ = seen.recv_timeout(WAIT);
        assert_eq!(r.agent_state().lock().unwrap().open(), &[5]);
        r.call("agent.reply", json!({"id": 5, "text": "Done."})).unwrap();
        let _ = seen.recv_timeout(WAIT);
        r.call("engine.execute", json!({"command": "text.insert", "params": {"text": "c"}})).unwrap();
        assert!(seen.recv_timeout(WAIT).unwrap()["params"].get("request").is_none(), "closed: plain edits again");
        // The person cancelled: the app refuses, and the bridge lets the request go.
        r.agent_state().lock().unwrap().open.push(6);
        assert!(r.call("engine.execute", json!({"command": "text.insert", "params": {"text": "d"}})).unwrap_err().contains("cancelled"));
        let _ = seen.recv_timeout(WAIT);
        assert!(r.agent_state().lock().unwrap().open().is_empty());
        // Another connection of the same bridge (`--channel`) shares the state.
        let other = Remote::connect_with(&format!("127.0.0.1:{port}"), fixed_key("k")).unwrap().sharing(r.agent_state());
        other.agent_state().lock().unwrap().open.push(7);
        r.call("engine.execute", json!({"command": "text.insert", "params": {"text": "e"}})).unwrap();
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["params"]["request"], 7);
    }
}

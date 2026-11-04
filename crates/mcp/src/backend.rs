//! Where MCP tool calls end up: a control-channel method call.

use std::io::{BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::server::{BoundedLine, read_bounded_line};

/// Longest reply line accepted from the app (screenshots and `document.inspect` are large).
const MAX_REPLY_LINE: usize = 256 << 20;

/// Something that answers control-channel methods (`engine.execute`, `document.inspect`,
/// `ui.pointer`, `ui.render`, …). See `wordcraft_ui_egui::control` for the full list.
pub trait Backend {
    /// Call one method. `Ok` carries the `result`, `Err` the error message.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// True when a real UI is attached (`ui.screenshot`, `ui.click`, dialogs… work).
    fn has_ui(&self) -> bool;
    /// Short human description ("headless", "connected to 127.0.0.1:7981").
    fn describe(&self) -> String;
}

/// Looks up the control key for an address (`HOST:PORT`).
type KeyFinder = Box<dyn Fn(&str) -> Result<String, String> + Send>;

/// A running WordCraft app, reached through its loopback control port.
pub struct Remote {
    addr: String,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
    /// Whether a connection ever succeeded (`lazy` starts without one; the app may open later).
    connected: bool,
    /// The connection leads to the app's 127.0.0.1: requests carry the control key.
    send_key: bool,
    /// The control key, looked up once per connection (the app picks a new one at every start).
    key: Option<String>,
    find_key: KeyFinder,
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

/// The only address the app's control server binds (`apps/wordcraft/src/control_server.rs`).
const APP_IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// The host part of `addr` (`HOST:PORT`, `[V6]:PORT`), without brackets.
fn host_of(addr: &str) -> &str {
    let host = addr.rsplit_once(':').map_or(addr, |(h, _)| h);
    host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host)
}

/// Whether `addr` (`HOST:PORT`, as given to `--connect`) names the app's own address:
/// 127.0.0.1 or `localhost`. Other loopback addresses (127.0.0.2, ::1) don't count: the app never
/// listens there, so whatever does is not the app. Other host names are not resolved.
fn is_app_addr(addr: &str) -> bool {
    let app = |ip: IpAddr| ip.to_canonical() == APP_IP;
    if let Ok(sa) = addr.parse::<SocketAddr>() {
        return app(sa.ip());
    }
    if let Ok(ip) = addr.parse::<IpAddr>() {
        return app(ip);
    }
    let host = host_of(addr);
    host.parse::<IpAddr>().map_or_else(|_| host.eq_ignore_ascii_case("localhost"), app)
}

/// The key goes only to the app's `--connect` address, and only when the connection really
/// leads to 127.0.0.1 (`peer`): a listener on another loopback address (::1, 127.0.0.2), which
/// any local user can open, never sees it.
fn may_send_key(addr: &str, peer: Option<SocketAddr>) -> bool {
    is_app_addr(addr) && peer.is_some_and(|p| p.ip().to_canonical() == APP_IP)
}

/// Where to connect for `addr`: `localhost` means the app's 127.0.0.1 (not whatever the resolver
/// lists first, which may be ::1); anything else as resolved.
fn targets(addr: &str) -> std::io::Result<Vec<SocketAddr>> {
    if host_of(addr).eq_ignore_ascii_case("localhost")
        && let Some(port) = addr.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok())
    {
        return Ok(vec![SocketAddr::new(APP_IP, port)]);
    }
    Ok(addr.to_socket_addrs()?.collect())
}

impl Remote {
    /// Connect to `addr` (`127.0.0.1:7981`), failing fast when nothing is listening.
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        Self::connect_with(addr, Box::new(find_key))
    }

    fn connect_with(addr: &str, find_key: KeyFinder) -> std::io::Result<Self> {
        let mut r = Self { addr: addr.to_string(), conn: None, next_id: 1, connected: false, send_key: false, key: None, find_key };
        r.reconnect()?;
        if let Some(w) = r.key_warning() {
            log::warn!("{w}");
        }
        Ok(r)
    }

    /// A remote that connects on first use: the app may start after the MCP server, and a
    /// server that exits at once reads as failed for the client's whole session (#4078).
    pub fn lazy(addr: &str) -> Self {
        Self { addr: addr.to_string(), conn: None, next_id: 1, connected: false, send_key: false, key: None, find_key: Box::new(find_key) }
    }

    /// The address the app is reached at, as given.
    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// Why requests go without the control key (the address is not the app's 127.0.0.1), if they
    /// do.
    pub fn key_warning(&self) -> Option<String> {
        // Nothing is known about the address until a connection succeeds.
        if !self.connected {
            return None;
        }
        (!self.send_key).then(|| {
            format!(
                "{} is not the app's address, so the control key is not sent (it only goes to 127.0.0.1 and localhost, where the app listens) and the app refuses requests; reach the app through 127.0.0.1, for example with an SSH tunnel",
                self.addr
            )
        })
    }

    /// Drop the connection and connect again to the first of the address's targets that answers,
    /// deciding afresh whether the control key may go to it.
    fn reconnect(&mut self) -> std::io::Result<()> {
        self.conn = None;
        self.key = None;
        let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {}", self.addr));
        for sa in targets(&self.addr)? {
            match TcpStream::connect_timeout(&sa, Duration::from_millis(800)) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    // The app answers within 60 s (its own timeout); leave headroom.
                    s.set_read_timeout(Some(Duration::from_secs(90))).ok();
                    self.send_key = may_send_key(&self.addr, s.peer_addr().ok());
                    let read = s.try_clone()?;
                    self.conn = Some((BufReader::new(read), s));
                    self.connected = true;
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
        match read_bounded_line(reader, MAX_REPLY_LINE)? {
            BoundedLine::Line(reply) => Ok(reply),
            BoundedLine::Eof => Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "control channel closed")),
            BoundedLine::TooLong => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "reply from app too long")),
        }
    }

    /// One request and its reply, with one retry on a new connection (the app may have
    /// restarted; a new connection also reads the key again).
    fn exchange(&mut self, id: u64, method: &str, params: &Value) -> Result<Value, String> {
        let unreachable = |addr: &str, e: std::io::Error, connected: bool| {
            let retry = if connected { String::new() } else { "; will retry on each call".to_string() };
            format!("WordCraft app at {addr} is not reachable: {e}{retry}")
        };
        let mut first = true;
        let reply = loop {
            let tried = if self.conn.is_some() { Ok(()) } else { self.reconnect() };
            let result = match tried {
                Ok(()) => {
                    let line = self.request_line(id, method, params)?;
                    self.roundtrip(&line)
                }
                Err(e) => Err(e),
            };
            match result {
                Ok(r) => break r,
                Err(e) => {
                    self.conn = None;
                    // A reply that arrived but can't be read (too long, not UTF-8): the app has
                    // run the request, so sending it again could run a state-changing one twice.
                    if e.kind() == std::io::ErrorKind::InvalidData {
                        return Err(format!("bad reply from app: {e}"));
                    }
                    if !first {
                        return Err(unreachable(&self.addr, e, self.connected));
                    }
                    first = false;
                }
            }
        };
        serde_json::from_str(reply.trim()).map_err(|e| format!("bad reply from app: {e}"))
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
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
            Err(v.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_string())
        }
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        if self.connected {
            format!("connected to the WordCraft app at {}", self.addr)
        } else {
            format!("will connect to the WordCraft app at {} when it opens", self.addr)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
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
    fn key_only_for_the_apps_address() {
        let peer = |s: &str| s.parse::<SocketAddr>().ok();
        for (addr, at) in [
            ("127.0.0.1:7981", "127.0.0.1:7981"),
            ("[::ffff:127.0.0.1]:7981", "[::ffff:127.0.0.1]:7981"),
            ("localhost:7981", "127.0.0.1:7981"),
            ("LocalHost:7981", "127.0.0.1:7981"),
        ] {
            assert!(may_send_key(addr, peer(at)), "{addr} at {at}");
        }
        for (addr, at) in [
            ("192.0.2.20:7981", "192.0.2.20:7981"),
            ("10.0.0.7:7981", "10.0.0.7:7981"),
            ("[fe80::1]:7981", "[fe80::1]:7981"),
            ("0.0.0.0:7981", "127.0.0.1:7981"),
            // Loopback addresses the app never binds: another local user's listener (#3699).
            ("127.0.0.2:7981", "127.0.0.2:7981"),
            ("[::1]:7981", "[::1]:7981"),
            ("localhost:7981", "[::1]:7981"),
            ("127.0.0.1:7981", "127.0.0.2:7981"),
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

    #[test]
    fn localhost_connects_to_the_apps_address_only() {
        // Not the resolver's list, which may put ::1 (where anyone can listen) first.
        let at: SocketAddr = "127.0.0.1:7981".parse().unwrap();
        assert_eq!(targets("localhost:7981").unwrap(), vec![at]);
        assert_eq!(targets("LOCALHOST:7981").unwrap(), vec![at]);
        assert_eq!(targets("127.0.0.1:7981").unwrap(), vec![at]);
        assert_eq!(targets("[::1]:7981").unwrap(), vec!["[::1]:7981".parse::<SocketAddr>().unwrap()]);
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

    #[test]
    fn an_unreadable_reply_is_not_retried() {
        // The app answered something that can't be read (here not UTF-8; an over-long reply
        // takes the same path): the request ran, so it must not be sent again.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(AtomicUsize::new(0));
        let n = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let n = n.clone();
                std::thread::spawn(move || {
                    let mut out = stream.try_clone().unwrap();
                    for line in BufReader::new(stream).lines() {
                        if line.is_err() {
                            return;
                        }
                        n.fetch_add(1, Ordering::SeqCst);
                        if out.write_all(b"\xff\xfe\n").is_err() {
                            return;
                        }
                    }
                });
            }
        });
        let mut r = Remote::connect_with(&format!("127.0.0.1:{port}"), fixed_key("k1")).unwrap();
        let e = r.call("engine.execute", json!({"command": "text.insert", "params": {"text": "x"}})).unwrap_err();
        assert!(e.contains("bad reply"), "{e}");
        assert_eq!(seen.load(Ordering::SeqCst), 1, "the request was sent once");
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
    fn a_lazy_remote_serves_once_the_app_opens() {
        // Nothing is listening: the call fails, but the server stays up and says it will retry.
        let mut r = Remote::lazy("127.0.0.1:1");
        r.find_key = fixed_key("k1");
        let e = r.call("document.inspect", json!({})).unwrap_err();
        assert!(e.contains("not reachable") && e.contains("will retry"), "{e}");
        assert!(r.describe().contains("will connect to"));
        // The app opens later: the next call connects and answers.
        let (port, seen) = fake_app(|_| Some((json!({"ok": true, "result": 7}), false)));
        r.addr = format!("127.0.0.1:{port}");
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(7));
        assert!(r.key_warning().is_none());
        assert_eq!(seen.recv_timeout(WAIT).unwrap()["key"], "k1");
        assert!(r.describe().contains("connected to"));
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

        // The app quits mid-session (connection closed): the retry reads the key again.
        let (port, seen) = fake_app(|req| if req["id"] == 2 { None } else { Some((json!({"ok": true, "result": 1}), false)) });
        let reads = Arc::new(AtomicUsize::new(0));
        let n = reads.clone();
        let finder: KeyFinder = Box::new(move |_| Ok(format!("key-{}", n.fetch_add(1, Ordering::SeqCst))));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{port}"), finder).unwrap();
        r.call("document.inspect", json!({})).unwrap();
        r.call("document.inspect", json!({})).unwrap_err();
        let keys: Vec<Value> = seen.try_iter().map(|v| v["key"].clone()).collect();
        assert_eq!(keys.first(), Some(&json!("key-0")), "{keys:?}");
        assert_eq!(keys.last(), Some(&json!("key-1")), "{keys:?}");
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
        assert!(r.key_warning().is_some_and(|w| w.contains("not the app's address")));
        let e = r.call("document.inspect", json!({})).unwrap_err();
        assert!(e.contains("not the app's address") && e.contains("not sent"), "{e}");
        assert!(seen.recv_timeout(WAIT).unwrap().get("key").is_none());
    }
}

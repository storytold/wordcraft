//! The bridge to a running WordCraft app: a control-channel client over loopback TCP. Native only
//! (the control channel and its client do not build for wasm32).

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::Path;

use craft::automation::args::{EnvNames, FlagNames};
use craft::automation::client::{ConnectError, ControlClient};
use craft::automation::security::client_token;
use serde_json::Value;

use crate::backend::Backend;

/// A control token and where it came from: a token read from the app's file may be stale after
/// an app restart, one given by the user is not re-read.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    value: String,
    from_file: bool,
}

/// Looks up the control token for an address (`HOST:PORT`).
type TokenFinder = Box<dyn Fn(&str) -> Result<Token, Connect> + Send>;

/// The environment variable names of the control channel (`WORDCRAFT_CONTROL_TOKEN`, …).
fn env_names() -> EnvNames {
    EnvNames::for_app("WORDCRAFT")
}

/// The control token: `env` (`WORDCRAFT_CONTROL_TOKEN`) when set and not empty, else the app's
/// token file for `port` in `settings`. The error says where it looked. A missing token file means
/// the app has not started (it writes the file before it serves, and removes it on exit), so it
/// reads as unreachable and a lazy bridge keeps retrying (#4078).
fn lookup_token(env: Option<String>, settings: Option<&Path>, port: Option<u16>) -> Result<Token, Connect> {
    let names = env_names();
    let supplied = env.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let file = match (&supplied, settings.zip(port)) {
        (Some(_), _) => None,
        (None, Some((dir, port))) => {
            let path = wordcraft_control_key::key_path(dir, port);
            if !path.is_file() {
                return Err(Connect::Unreachable(format!(
                    "no control token yet: set {}, or start the app with --control {port} (it writes {})",
                    names.token,
                    path.display()
                )));
            }
            Some(path)
        }
        (None, None) => {
            return Err(Connect::Token(format!("no control token: set {} (no settings folder or port to find the token file)", names.token)));
        }
    };
    let from_file = file.is_some();
    client_token(supplied.as_deref(), file.as_deref(), &FlagNames::default(), &names)
        .map(|value| Token { value, from_file })
        .map_err(|e| Connect::Token(e.to_string()))
}

/// The port of `addr` (`HOST:PORT`).
fn port_of(addr: &str) -> Option<u16> {
    addr.rsplit_once(':').and_then(|(_, p)| p.parse().ok())
}

/// The token for the app at `addr`, from the environment or the app's token file.
fn find_token(addr: &str) -> Result<Token, Connect> {
    lookup_token(std::env::var(env_names().token).ok(), wordcraft_control_key::settings_dir().as_deref(), port_of(addr))
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

/// The token goes only to the app's `--connect` address, and only when the connection really
/// leads to 127.0.0.1 (`peer`): a listener on another loopback address (::1, 127.0.0.2), which
/// any local user can open, never sees it.
fn may_send_key(addr: &str, peer: SocketAddr) -> bool {
    is_app_addr(addr) && peer.ip().to_canonical() == APP_IP
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

/// Why `addr` gets no token.
fn not_app_address(addr: &str) -> String {
    format!(
        "{addr} is not the app's address, so the control token is not sent (it only goes to 127.0.0.1 and localhost, where the app listens) and the app refuses requests; reach the app through 127.0.0.1, for example with an SSH tunnel"
    )
}

/// Why a connection could not be made.
#[derive(Debug)]
enum Connect {
    /// The app was not reached.
    Unreachable(String),
    /// The app refused the token.
    Refused(String),
    /// No token could be found.
    Token(String),
}

/// A running WordCraft app, reached through its loopback control port.
pub struct Remote {
    addr: String,
    client: Option<ControlClient>,
    /// Whether a connection ever succeeded (`lazy` starts without one; the app may open later).
    connected: bool,
    /// The connection leads to the app's 127.0.0.1: the control token may be sent.
    send_key: bool,
    find_token: TokenFinder,
}

impl Remote {
    /// Connect to `addr` (`127.0.0.1:7981`), failing fast when nothing is listening.
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        Self::connect_with(addr, Box::new(find_token))
    }

    fn connect_with(addr: &str, find_token: TokenFinder) -> std::io::Result<Self> {
        let mut r = Self { addr: addr.to_string(), client: None, connected: false, send_key: false, find_token };
        r.reconnect().map_err(|c| std::io::Error::other(r.connect_error(c, false)))?;
        if let Some(w) = r.key_warning() {
            log::warn!("{w}");
        }
        Ok(r)
    }

    /// A remote that connects on first use: the app may start after the MCP server, and a
    /// server that exits at once reads as failed for the client's whole session (#4078).
    pub fn lazy(addr: &str) -> Self {
        Self { addr: addr.to_string(), client: None, connected: false, send_key: false, find_token: Box::new(find_token) }
    }

    /// The address the app is reached at, as given.
    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// Why requests go without the control token (the address is not the app's 127.0.0.1), if
    /// they do.
    pub fn key_warning(&self) -> Option<String> {
        // Nothing is known about the address until a connection succeeds.
        if !self.connected {
            return None;
        }
        (!self.send_key).then(|| not_app_address(&self.addr))
    }

    /// Drop the connection and connect again to the first of the address's targets that answers,
    /// deciding afresh whether the token may go to it. An address that is not the app's gets no
    /// connection: nothing could authenticate on it.
    fn reconnect(&mut self) -> Result<(), Connect> {
        self.client = None;
        let mut last = Connect::Unreachable(format!("cannot resolve {}", self.addr));
        for sa in targets(&self.addr).map_err(|e| Connect::Unreachable(e.to_string()))? {
            self.send_key = may_send_key(&self.addr, sa);
            if !self.send_key {
                self.connected = true;
                return Ok(());
            }
            match self.authenticate(sa) {
                Ok(client) => {
                    self.client = Some(client);
                    self.connected = true;
                    return Ok(());
                }
                Err(e @ Connect::Unreachable(_)) => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Connect to `sa` and authenticate. A token read from the app's file that is refused may be
    /// stale (the app restarted with a new one), so it is read again once.
    fn authenticate(&self, sa: SocketAddr) -> Result<ControlClient, Connect> {
        let mut refused_before = false;
        loop {
            let token = (self.find_token)(&self.addr)?;
            match ControlClient::connect(sa, &token.value) {
                Ok(client) => return Ok(client),
                Err(ConnectError::Refused(_)) if token.from_file && !refused_before => refused_before = true,
                Err(ConnectError::Refused(e)) => return Err(Connect::Refused(e)),
                Err(ConnectError::Transport(e)) => return Err(Connect::Unreachable(e)),
                // `ConnectError` is non-exhaustive: a failure it adds later did not reach the app either.
                Err(other) => return Err(Connect::Unreachable(other.to_string())),
            }
        }
    }

    /// The error a failed connection reports; `will_retry` says the next call connects again.
    fn connect_error(&self, why: Connect, will_retry: bool) -> String {
        match why {
            Connect::Unreachable(e) => {
                let retry = if will_retry { "; will retry on each call" } else { "" };
                format!("WordCraft app at {} is not reachable: {e}{retry}", self.addr)
            }
            Connect::Refused(e) => format!(
                "unauthorized: the WordCraft app at {} refused the control token ({e}); it came from {} or the app's token file",
                self.addr,
                env_names().token
            ),
            Connect::Token(e) => e,
        }
    }

    /// One request and its reply. A request that was not sent, or one that only reads, is tried
    /// once more on a new connection (the app may have restarted; a new connection also reads
    /// the token again). A mutation whose outcome is unknown is never sent again: it may have run.
    fn exchange(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        let mut replayed = false;
        loop {
            if self.client.is_none() {
                self.reconnect().map_err(|c| self.connect_error(c, !self.connected))?;
            }
            if !self.send_key {
                return Err(not_app_address(&self.addr));
            }
            let Some(client) = self.client.as_mut() else {
                return Err(format!("WordCraft app at {} is not connected", self.addr));
            };
            match client.call(method, params.clone()) {
                Ok(v) => return Ok(v),
                // An error reply: the connection is intact and the app answered.
                Err(e) if !client.is_broken() => return Err(e),
                Err(e) => {
                    self.client = None;
                    if !is_read_only(method, params) {
                        return Err(format!(
                            "outcome unknown: the connection to the WordCraft app at {} failed while sending `{method}` or waiting for its reply ({e}); it may have been executed, so it is not sent again. Inspect the document to check",
                            self.addr
                        ));
                    }
                    if replayed {
                        return Err(format!("WordCraft app at {} is not reachable: {e}", self.addr));
                    }
                    replayed = true;
                }
            }
        }
    }
}

/// Methods that only read: sending them twice does no harm.
fn is_read_only(method: &str, params: &Value) -> bool {
    match method {
        "document.inspect" | "engine.commands" | "ui.inspect" | "ui.parity" => true,
        "ui.render" | "ui.screenshot" => params.get("path").and_then(Value::as_str).is_none(),
        _ => false,
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.exchange(method, &params)
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
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);

    /// A valid control token (64 hex digits) made of one digit.
    fn tok(c: char) -> String {
        c.to_string().repeat(64)
    }

    /// Writes a token file as the app does: readable by its owner alone (the control-key reader refuses
    /// a file that group or others can read).
    fn write_token_file(path: &std::path::Path, contents: impl AsRef<[u8]>) {
        std::fs::write(path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn fixed_token(t: String) -> TokenFinder {
        Box::new(move |_| Ok(Token { value: t.clone(), from_file: false }))
    }

    #[test]
    fn token_from_env_then_token_file() {
        let dir = std::env::temp_dir().join(format!("wordcraft-mcp-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = wordcraft_control_key::key_path(&dir, 7981);
        // Neither: the app has not started; the error names the variable and the file it looked for.
        let Connect::Unreachable(e) = lookup_token(None, Some(&dir), Some(7981)).unwrap_err() else {
            panic!("a missing token file must read as unreachable");
        };
        assert!(e.contains("WORDCRAFT_CONTROL_TOKEN") && e.contains(&path.display().to_string()), "{e}");
        // The token file, trimmed; an empty variable does not count.
        write_token_file(&path, format!("  {}\n", tok('a')));
        let file = Token { value: tok('a'), from_file: true };
        assert_eq!(lookup_token(None, Some(&dir), Some(7981)).unwrap(), file);
        assert_eq!(lookup_token(Some(" ".into()), Some(&dir), Some(7981)).unwrap(), file);
        // The variable wins.
        let env = Token { value: tok('b'), from_file: false };
        assert_eq!(lookup_token(Some(format!(" {} ", tok('b'))), Some(&dir), Some(7981)).unwrap(), env);
        assert_eq!(lookup_token(Some(tok('b')), None, None).unwrap(), env);
        // Read again every time: the app writes a new token at every start.
        write_token_file(&path, tok('c'));
        assert_eq!(lookup_token(None, Some(&dir), Some(7981)).unwrap().value, tok('c'));
        // Another port, another file.
        assert!(lookup_token(None, Some(&dir), Some(7982)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_only_for_the_apps_address() {
        let peer = |s: &str| s.parse::<SocketAddr>().unwrap();
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

    /// A stand-in for the app on 127.0.0.1.
    struct Fake {
        port: u16,
        /// Every request line except `auth`.
        seen: Receiver<Value>,
        /// The token of every `auth` frame.
        tokens: Receiver<String>,
    }

    /// An `auth` frame is accepted when `accept(token)`; refused, the connection closes. Every
    /// other request line gets `answer(request)`: a reply and whether to close the connection
    /// after it, or `None` to close it without a reply (the app quit).
    fn fake_app(
        accept: impl Fn(&str) -> bool + Send + Sync + 'static,
        answer: impl Fn(&Value) -> Option<(Value, bool)> + Send + Sync + 'static,
    ) -> Fake {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen_tx, seen) = channel();
        let (token_tx, tokens) = channel();
        let (accept, answer) = (Arc::new(accept), Arc::new(answer));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (seen_tx, token_tx, accept, answer) = (seen_tx.clone(), token_tx.clone(), accept.clone(), answer.clone());
                std::thread::spawn(move || {
                    let mut out = stream.try_clone().unwrap();
                    for line in BufReader::new(stream).lines() {
                        let Ok(req) = serde_json::from_str::<Value>(&line.unwrap_or_default()) else { return };
                        let (mut reply, close) = if req["method"] == "auth" {
                            let token = req["params"]["token"].as_str().unwrap_or("").to_string();
                            let ok = accept(&token);
                            let _ = token_tx.send(token);
                            if ok {
                                (json!({"ok": true, "result": {"authenticated": true}}), false)
                            } else {
                                (json!({"ok": false, "error": "authentication required"}), true)
                            }
                        } else {
                            let _ = seen_tx.send(req.clone());
                            let Some(r) = answer(&req) else { return };
                            r
                        };
                        reply["id"] = req["id"].clone();
                        if writeln!(out, "{reply}").is_err() || close {
                            return;
                        }
                    }
                });
            }
        });
        Fake { port, seen, tokens }
    }

    fn ok(result: i64) -> Option<(Value, bool)> {
        Some((json!({"ok": true, "result": result}), false))
    }

    #[test]
    fn sends_the_token_to_loopback() {
        let app = fake_app(|_| true, |_| ok(7));
        for host in ["127.0.0.1", "localhost"] {
            let mut r = Remote::connect_with(&format!("{host}:{}", app.port), fixed_token(tok('1'))).unwrap();
            assert!(r.key_warning().is_none(), "{host}");
            assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(7));
            assert_eq!(app.tokens.recv_timeout(WAIT).unwrap(), tok('1'), "{host}");
        }
    }

    #[test]
    fn a_lazy_remote_serves_once_the_app_opens() {
        // The app is not running, so it has written no token file: the call fails, but the server
        // stays up and says it will retry.
        let dir = std::env::temp_dir().join(format!("wordcraft-mcp-lazy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = Remote::lazy("127.0.0.1:1");
        let settings = dir.clone();
        r.find_token = Box::new(move |addr| lookup_token(None, Some(&settings), port_of(addr)));
        let e = r.call("document.inspect", json!({})).unwrap_err();
        assert!(e.contains("not reachable") && e.contains("will retry"), "{e}");
        assert!(r.describe().contains("will connect to"));
        // The app opens later and writes its token file: the next call connects and answers.
        let app = fake_app(|_| true, |_| ok(7));
        write_token_file(&wordcraft_control_key::key_path(&dir, app.port), tok('1'));
        r.addr = format!("127.0.0.1:{}", app.port);
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(7));
        assert!(r.key_warning().is_none());
        assert_eq!(app.tokens.recv_timeout(WAIT).unwrap(), tok('1'));
        assert!(r.describe().contains("connected to"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_the_token_again_on_a_new_connection() {
        // The app restarted with a new token: the old one is refused, the file is read again.
        let app = fake_app(|t| t == tok('b'), |_| ok(1));
        let reads = Arc::new(AtomicUsize::new(0));
        let n = reads.clone();
        let finder: TokenFinder =
            Box::new(move |_| Ok(Token { value: if n.fetch_add(1, Ordering::SeqCst) == 0 { tok('a') } else { tok('b') }, from_file: true }));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{}", app.port), finder).unwrap();
        assert_eq!(app.tokens.recv_timeout(WAIT).unwrap(), tok('a'));
        assert_eq!(app.tokens.recv_timeout(WAIT).unwrap(), tok('b'));
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(1));
        // The new token is kept for the next requests on the same connection.
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(1));
        assert_eq!(reads.load(Ordering::SeqCst), 2);

        // The app quits mid-session (connection closed): the retry authenticates with the token
        // read again.
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let app = fake_app(|_| true, move |_| if c.fetch_add(1, Ordering::SeqCst) == 1 { None } else { ok(1) });
        let reads = Arc::new(AtomicUsize::new(0));
        let n = reads.clone();
        let finder: TokenFinder = Box::new(move |_| Ok(Token { value: tok(['a', 'b', 'c'][n.fetch_add(1, Ordering::SeqCst) % 3]), from_file: true }));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{}", app.port), finder).unwrap();
        r.call("document.inspect", json!({})).unwrap();
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(1));
        let tokens: Vec<String> = app.tokens.try_iter().collect();
        assert_eq!(tokens, vec![tok('a'), tok('b')]);
    }

    #[test]
    fn no_token_for_other_hosts_and_a_clear_error() {
        // A LAN address: the token must not be read, and nothing is sent.
        let finder: TokenFinder = Box::new(|_| Err(Connect::Token("the token must not be read for another host".to_string())));
        let mut r = Remote::connect_with("192.0.2.20:7981", finder).unwrap();
        assert!(r.key_warning().is_some_and(|w| w.contains("not the app's address")));
        let e = r.call("document.inspect", json!({})).unwrap_err();
        assert!(e.contains("not the app's address") && e.contains("not sent"), "{e}");
    }

    #[test]
    fn a_mutation_with_a_lost_reply_is_not_sent_again() {
        // The app runs the request, then the connection drops without a reply.
        let app = fake_app(|_| true, |_| None);
        let mut r = Remote::connect_with(&format!("127.0.0.1:{}", app.port), fixed_token(tok('1'))).unwrap();
        let e = r.call("engine.execute", json!({"command": "text.insert", "params": {"text": "x"}})).unwrap_err();
        assert!(e.contains("outcome unknown"), "{e}");
        assert_eq!(app.seen.recv_timeout(WAIT).unwrap()["method"], "engine.execute");
        std::thread::sleep(Duration::from_millis(200));
        assert!(app.seen.try_recv().is_err(), "the mutation was sent a second time");
    }

    #[test]
    fn a_read_with_a_lost_reply_is_sent_again() {
        let n = Arc::new(AtomicUsize::new(0));
        let app = fake_app(|_| true, move |_| if n.fetch_add(1, Ordering::SeqCst) == 0 { None } else { ok(3) });
        let mut r = Remote::connect_with(&format!("127.0.0.1:{}", app.port), fixed_token(tok('1'))).unwrap();
        assert_eq!(r.call("document.inspect", json!({})).unwrap(), json!(3));
        assert_eq!(app.seen.try_iter().count(), 2);
    }

    #[test]
    fn image_exports_with_a_lost_reply_are_not_replayed_but_in_memory_reads_are() {
        for method in ["ui.render", "ui.screenshot"] {
            for path in [json!("out.png"), json!(""), Value::Null] {
                let count = Arc::new(AtomicUsize::new(0));
                let n = count.clone();
                let app = fake_app(|_| true, move |_| if n.fetch_add(1, Ordering::SeqCst) == 0 { None } else { ok(3) });
                let mut r = Remote::connect_with(&format!("127.0.0.1:{}", app.port), fixed_token(tok('1'))).unwrap();
                let result = r.call(method, json!({"path": path}));
                if path.is_string() {
                    assert!(result.unwrap_err().contains("outcome unknown"));
                    assert_eq!(count.load(Ordering::SeqCst), 1);
                } else {
                    assert_eq!(result.unwrap(), json!(3));
                    assert_eq!(count.load(Ordering::SeqCst), 2);
                }
                assert_eq!(app.seen.try_iter().count(), if path.is_string() { 1 } else { 2 });
            }
            assert!(is_read_only(method, &json!({})), "no path means PNG in the reply");
        }
    }

    #[test]
    fn a_mutation_not_sent_goes_out_on_a_new_connection() {
        let app = fake_app(|_| true, |_| ok(1));
        let mut r = Remote::connect_with(&format!("127.0.0.1:{}", app.port), fixed_token(tok('1'))).unwrap();
        r.client = None;
        assert_eq!(r.call("engine.execute", json!({"command": "file.new"})).unwrap(), json!(1));
        assert_eq!(app.seen.try_iter().count(), 1);
    }
}

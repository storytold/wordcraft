//! Local control channel: JSON lines on 127.0.0.1 (inside the Flatpak sandbox network namespace
//! this port is invisible to the host). Every request needs a key; `chat.*` is answered here so a
//! long poll never waits on the UI thread.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wordcraft_chat::{Hub, Principal, hub};
use wordcraft_ui_egui::ControlRequest;

const MAX_LINE: usize = 1 << 20;

/// The server's time limits.
#[derive(Clone, Copy, Debug)]
struct Waits {
    /// A connection that sends no authorised request in this time is closed.
    first: Duration,
    /// The UI does not run a request after this (it answers `expired`).
    deadline: Duration,
    /// How long the server waits for the UI's answer (then `timeout`).
    reply: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        // The client waits 60 s: a request it gave up on is never run (55 s), and the server
        // answers before the client's own timeout (58 s).
        Waits { first: Duration::from_secs(30), deadline: Duration::from_secs(55), reply: Duration::from_secs(58) }
    }
}

pub fn start_with(listener: TcpListener, ctx: egui::Context, hub: Arc<Hub>) -> Receiver<ControlRequest> {
    start_with_waits(listener, ctx, hub, Waits::default())
}

fn start_with_waits(listener: TcpListener, ctx: egui::Context, hub: Arc<Hub>, waits: Waits) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, ctx, hub) = (tx.clone(), ctx.clone(), hub.clone());
            std::thread::spawn(move || serve(stream, tx, ctx, hub, waits));
        }
    });
    rx
}

/// The read side of a connection with an optional overall deadline: each read waits only for the
/// time left, so blank lines, bad JSON or a slow drip of bytes cannot extend it.
struct Timed {
    s: TcpStream,
    deadline: Option<Instant>,
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(d) = self.deadline {
            let left = d.checked_duration_since(Instant::now()).filter(|l| !l.is_zero()).ok_or_else(|| std::io::Error::from(std::io::ErrorKind::TimedOut))?;
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
/// connection before the client has read our error reply. Bounded by bytes (64 MiB) and by one
/// overall deadline of 2 s for the whole drain.
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

fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context, hub: Arc<Hub>, waits: Waits) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    // One overall deadline from accept to the first authorised request; then no limit (long
    // polls, pauses).
    let mut authorised = false;
    let mut reader = BufReader::new(Timed { s: read, deadline: Some(Instant::now() + waits.first) });
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
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = writeln!(out, "{}", json!({"ok": false, "error": "bad JSON"}));
            continue;
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        // Only the top-level `key` counts, except for `auth`, whose params may carry it.
        // (`ui.key {"key":"Enter"}` is a key press, not a secret.)
        let key = msg
            .get("key")
            .and_then(Value::as_str)
            .or_else(|| if method == "auth" { params.get("key").and_then(Value::as_str) } else { None })
            .unwrap_or("");
        let mut reply = |v: Value| -> bool {
            let mut v = v;
            if let Some(o) = v.as_object_mut() {
                o.insert("id".into(), id.clone());
            }
            writeln!(out, "{v}").is_ok()
        };
        if method == "chat.join" {
            let code = params.get("code").and_then(Value::as_str).unwrap_or("");
            let r = match hub.join(code, hub::now_ms()) {
                Ok((handle, key)) => json!({"ok": true, "result": {"handle": handle, "key": key, "lang": hub.lang().code()}}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            };
            if !reply(r) {
                break;
            }
            continue;
        }
        // Always re-validate the key against the hub (Remove revokes it at once, even when the
        // same handle is invited again with a new key).
        let k = if key.is_empty() { conn_key.as_deref().unwrap_or("") } else { key };
        let who = if k.is_empty() { None } else { hub.authorize(k) };
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
            ("chat.post", Principal::Member(h)) => match hub.post_agent(h, params.get("text").and_then(Value::as_str).unwrap_or("")) {
                Ok(m) => json!({"ok": true, "result": m}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            ("chat.post", Principal::Host) => json!({"ok": false, "error": "owner_messages_come_from_the_pane"}),
            ("chat.poll", _) => {
                let after = params.get("after").and_then(Value::as_u64).unwrap_or(0);
                let wait = params.get("wait_s").and_then(Value::as_u64).unwrap_or(25).min(25);
                json!({"ok": true, "result": hub.poll(after, Duration::from_secs(wait))})
            }
            ("chat.members", _) => json!({"ok": true, "result": hub.members()}),
            ("chat.leave", Principal::Member(h)) => json!({"ok": hub.remove(h), "result": {}}),
            ("chat.leave", Principal::Host) => json!({"ok": false, "error": "host_cannot_leave"}),
            _ => {
                // The UI does not run it after the deadline: we stop waiting then.
                let (req, rrx) = ControlRequest::new(method.clone(), params.clone());
                if tx.send(req.with_principal(who.clone()).with_deadline(Instant::now() + waits.deadline)).is_err() {
                    break;
                }
                ctx.request_repaint();
                rrx.recv_timeout(waits.reply).unwrap_or_else(|_| json!({"ok": false, "error": "timeout"}))
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

    fn boot() -> (u16, Arc<Hub>, std::sync::mpsc::Receiver<ControlRequest>) {
        let hub = Hub::new("host-key".into());
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|e| panic!("{e}"));
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let rx = start_with(listener, egui::Context::default(), hub.clone());
        (port, hub, rx)
    }

    fn call(port: u16, line: &str) -> serde_json::Value {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|e| panic!("{e}"));
        writeln!(s, "{line}").unwrap_or_else(|e| panic!("{e}"));
        let mut r = String::new();
        let _ = BufReader::new(s).read_line(&mut r);
        serde_json::from_str(&r).unwrap_or_default()
    }

    #[test]
    fn no_key_is_refused() {
        let (port, _, _) = boot();
        let r = call(port, r#"{"id":1,"method":"document.inspect"}"#);
        assert_eq!(r["error"], "unauthorized");
    }

    #[test]
    fn join_then_member_post_and_poll() {
        let (port, hub, _) = boot();
        let (_, code) = hub.invite("@claude", hub::now_ms()).unwrap_or_default();
        let j = call(port, &format!(r#"{{"id":1,"method":"chat.join","params":{{"code":"{code}"}}}}"#));
        let key = j["result"]["key"].as_str().unwrap_or("").to_string();
        assert_eq!(j["result"]["handle"], "@claude");
        let p = call(port, &format!(r#"{{"id":2,"key":"{key}","method":"chat.post","params":{{"text":"olá"}}}}"#));
        assert_eq!(p["ok"], true);
        let q = call(port, &format!(r#"{{"id":3,"key":"{key}","method":"chat.poll","params":{{"after":0,"wait_s":0}}}}"#));
        assert!(q["result"].as_array().is_some_and(|a| a.iter().any(|m| m["text"] == "olá")));
    }

    #[test]
    fn join_tells_the_chat_language() {
        let (port, hub, _) = boot();
        let (_, code) = hub.invite("@claude", hub::now_ms()).unwrap_or_default();
        let j = call(port, &format!(r#"{{"id":1,"method":"chat.join","params":{{"code":"{code}"}}}}"#));
        assert_eq!(j["result"]["lang"], "en", "{j}");
        let pt = Hub::with_lang("host-key".into(), wordcraft_chat::Lang::Pt);
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|e| panic!("{e}"));
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let _rx = start_with(listener, egui::Context::default(), pt.clone());
        let (_, code) = pt.invite("@claude", hub::now_ms()).unwrap_or_default();
        let j = call(port, &format!(r#"{{"id":1,"method":"chat.join","params":{{"code":"{code}"}}}}"#));
        assert_eq!(j["result"]["lang"], "pt", "{j}");
    }

    #[test]
    fn host_cannot_post_as_owner() {
        let (port, _, _) = boot();
        let r = call(port, r#"{"id":1,"key":"host-key","method":"chat.post","params":{"text":"x"}}"#);
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"], "owner_messages_come_from_the_pane");
    }

    #[test]
    fn host_cannot_leave() {
        let (port, _, _) = boot();
        let r = call(port, r#"{"id":1,"key":"host-key","method":"chat.leave"}"#);
        assert_eq!(r["error"], "host_cannot_leave");
    }

    #[test]
    fn removed_member_is_refused() {
        let (port, hub, _) = boot();
        let (_, code) = hub.invite("@pi", hub::now_ms()).unwrap_or_default();
        let (_, key) = hub.join(&code, hub::now_ms()).unwrap_or_default();
        assert!(!key.is_empty());
        hub.remove("@pi");
        let r = call(port, &format!(r#"{{"id":1,"key":"{key}","method":"chat.members"}}"#));
        assert_eq!(r["error"], "unauthorized");
    }

    #[test]
    fn forwarded_request_carries_principal() {
        let (port, hub, rx) = boot();
        let (_, code) = hub.invite("@claude", hub::now_ms()).unwrap_or_default();
        let (_, key) = hub.join(&code, hub::now_ms()).unwrap_or_default();
        let t = std::thread::spawn(move || call(port, &format!(r#"{{"id":1,"key":"{key}","method":"document.inspect"}}"#)));
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(req.principal, Principal::Member("@claude".into()));
        let left = req.deadline.map(|d| d.saturating_duration_since(Instant::now()));
        assert!(left.is_some_and(|l| l > Duration::from_secs(50) && l <= Duration::from_secs(55)), "{left:?}");
        let _ = req.reply.send(serde_json::json!({"ok": true, "result": {}}));
        assert_eq!(t.join().unwrap_or_default()["ok"], true);
    }

    #[test]
    fn huge_line_is_refused() {
        let (port, _, _) = boot();
        let big = format!(r#"{{"id":1,"key":"host-key","method":"chat.members","pad":"{}"}}"#, "x".repeat(1_100_000));
        let r = call(port, &big);
        assert_eq!(r["error"], "line too long");
    }

    /// One connection kept open across several lines.
    struct Conn(BufReader<TcpStream>, TcpStream);

    impl Conn {
        fn open(port: u16) -> Self {
            let s = TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|e| panic!("{e}"));
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let r = s.try_clone().unwrap_or_else(|e| panic!("{e}"));
            Self(BufReader::new(r), s)
        }
        fn send(&mut self, line: &str) {
            writeln!(self.1, "{line}").unwrap_or_else(|e| panic!("{e}"));
        }
        fn recv(&mut self) -> serde_json::Value {
            let mut r = String::new();
            let _ = self.0.read_line(&mut r);
            serde_json::from_str(&r).unwrap_or_default()
        }
        fn call(&mut self, line: &str) -> serde_json::Value {
            self.send(line);
            self.recv()
        }
        fn is_eof(&mut self) -> bool {
            let mut r = String::new();
            matches!(self.0.read_line(&mut r), Ok(0))
        }
    }

    fn member(hub: &Arc<Hub>, handle: &str) -> String {
        let (_, code) = hub.invite(handle, hub::now_ms()).unwrap_or_default();
        hub.join(&code, hub::now_ms()).unwrap_or_default().1
    }

    #[test]
    fn auth_connection_loses_access_after_remove() {
        let (port, hub, _) = boot();
        let key = member(&hub, "@pi");
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"auth","key":"{key}"}}"#))["ok"], true);
        assert_eq!(c.call(r#"{"id":2,"method":"chat.members"}"#)["ok"], true);
        hub.remove("@pi");
        assert_eq!(c.call(r#"{"id":3,"method":"chat.members"}"#)["error"], "unauthorized");
        assert!(c.is_eof());
    }

    #[test]
    fn auth_connection_not_revived_by_reinviting_same_handle() {
        let (port, hub, _) = boot();
        let key = member(&hub, "@pi");
        let mut c = Conn::open(port);
        assert_eq!(c.call(&format!(r#"{{"id":1,"method":"auth","key":"{key}"}}"#))["ok"], true);
        hub.remove("@pi");
        let new_key = member(&hub, "@pi");
        assert!(!new_key.is_empty());
        assert_ne!(key, new_key);
        assert_eq!(c.call(r#"{"id":2,"method":"chat.members"}"#)["error"], "unauthorized");
    }

    #[test]
    fn params_key_is_not_a_secret_for_forwarded_methods() {
        let (port, _, rx) = boot();
        let mut c = Conn::open(port);
        assert_eq!(c.call(r#"{"id":1,"method":"auth","key":"host-key"}"#)["ok"], true);
        c.send(r#"{"id":2,"method":"ui.key","params":{"key":"Enter"}}"#);
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(req.method, "ui.key");
        assert_eq!(req.params["key"], "Enter");
        assert_eq!(req.principal, Principal::Host);
        let _ = req.reply.send(serde_json::json!({"ok": true, "result": {}}));
        assert_eq!(c.recv()["ok"], true);
    }

    #[test]
    fn params_key_does_not_authenticate_other_methods() {
        let (port, _, _) = boot();
        let r = call(port, r#"{"id":1,"method":"chat.members","params":{"key":"host-key"}}"#);
        assert_eq!(r["error"], "unauthorized");
    }

    #[test]
    fn silent_connection_is_closed() {
        let hub = Hub::new("host-key".into());
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|e| panic!("{e}"));
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let _rx = start_with_waits(listener, egui::Context::default(), hub, Waits { first: Duration::from_millis(300), ..Waits::default() });
        let mut quiet = Conn::open(port);
        let t0 = Instant::now();
        assert!(quiet.is_eof(), "a connection without a request is closed");
        assert!(t0.elapsed() < Duration::from_secs(4));
        // After an authorised request the connection may stay idle (long polls, pauses).
        let mut busy = Conn::open(port);
        assert_eq!(busy.call(r#"{"id":1,"method":"auth","key":"host-key"}"#)["ok"], true);
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(busy.call(r#"{"id":2,"method":"chat.members"}"#)["ok"], true);
    }

    /// A member's `listen`: one long-lived connection polling like the client's stream mode
    /// (`chat.poll {after, wait_s: 25}` in a loop). Every message it gets is sent to the test with
    /// the time it arrived.
    fn stream(port: u16, key: String) -> std::sync::mpsc::Receiver<(u64, String, Instant)> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Ok(s) = TcpStream::connect(("127.0.0.1", port)) else { return };
            let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
            let Ok(r) = s.try_clone() else { return };
            let (mut r, mut w) = (BufReader::new(r), s);
            let mut after = 0u64;
            loop {
                let poll = serde_json::json!({"id": 1, "key": key, "method": "chat.poll", "params": {"after": after, "wait_s": 25}});
                if writeln!(w, "{poll}").is_err() {
                    return;
                }
                let mut line = String::new();
                if !matches!(r.read_line(&mut line), Ok(n) if n > 0) {
                    return;
                }
                let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
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
    fn arrival(rx: &std::sync::mpsc::Receiver<(u64, String, Instant)>, text: &str, t0: Instant) -> Option<Duration> {
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
        let (port, hub, rx_ui) = boot();
        let key = member(&hub, "@claude");
        let msgs = stream(port, key.clone());
        assert!(arrival(&msgs, "@claude joined the chat", Instant::now()).is_some());
        // The stream is now waiting inside a 25 s poll; the owner posts from another thread (the UI).
        std::thread::sleep(Duration::from_millis(300));
        let owner_post = |text: &str| {
            let (h, text) = (hub.clone(), text.to_string());
            let t0 = Instant::now();
            let _ = std::thread::spawn(move || h.post_owner("Owner", &text)).join();
            t0
        };
        let t0 = owner_post("primeira do dono");
        let d = arrival(&msgs, "primeira do dono", t0);
        assert!(d.is_some_and(|d| d < Duration::from_secs(1)), "owner message 1: {d:?}");
        // A host-key UI request (a screenshot) in between, answered by the "UI thread".
        let host = std::thread::spawn(move || call(port, r#"{"id":9,"key":"host-key","method":"ui.screenshot","params":{}}"#));
        let req = rx_ui.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(req.principal, Principal::Host);
        std::thread::sleep(Duration::from_millis(200));
        let _ = req.reply.send(serde_json::json!({"ok": true, "result": {}}));
        assert_eq!(host.join().unwrap_or_default()["ok"], true);
        std::thread::sleep(Duration::from_millis(300));
        let t0 = owner_post("segunda do dono");
        let d = arrival(&msgs, "segunda do dono", t0);
        assert!(d.is_some_and(|d| d < Duration::from_secs(1)), "owner message after a UI request: {d:?}");
        // A member post (its own `send`, another connection), then the owner again.
        let p = call(port, &format!(r#"{{"id":3,"key":"{key}","method":"chat.post","params":{{"text":"do agente"}}}}"#));
        assert_eq!(p["ok"], true);
        assert!(arrival(&msgs, "do agente", Instant::now()).is_some());
        std::thread::sleep(Duration::from_millis(300));
        let t0 = owner_post("terceira do dono");
        let d = arrival(&msgs, "terceira do dono", t0);
        assert!(d.is_some_and(|d| d < Duration::from_secs(1)), "owner message after a member post: {d:?}");
        // Several owner messages in a row, with the stream between polls and inside one.
        for (i, pause) in [0u64, 5, 50, 400].iter().enumerate() {
            std::thread::sleep(Duration::from_millis(*pause));
            let text = format!("dono {i}");
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
        let hub = Hub::new("host-key".into());
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|e| panic!("{e}"));
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let waits = Waits { first: Duration::from_secs(5), deadline: Duration::from_millis(200), reply: Duration::from_millis(400) };
        let rx = start_with_waits(listener, egui::Context::default(), hub, waits);
        let t0 = Instant::now();
        let t = std::thread::spawn(move || call(port, r#"{"id":1,"key":"host-key","method":"document.inspect"}"#));
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|e| panic!("{e}"));
        let left = req.deadline.map(|d| d.saturating_duration_since(t0));
        assert!(left.is_some_and(|l| l > Duration::from_millis(150) && l < Duration::from_millis(400)), "{left:?}");
        let r = t.join().unwrap_or_default();
        assert_eq!(r["error"], "timeout", "{r}");
        assert!(t0.elapsed() < Duration::from_secs(2));
        drop(req);
    }

    #[test]
    fn first_request_deadline_is_one_overall_limit() {
        // Blank lines and bad JSON do not keep an unauthorised connection open.
        let hub = Hub::new("host-key".into());
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|e| panic!("{e}"));
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let waits = Waits { first: Duration::from_millis(300), ..Waits::default() };
        let _rx = start_with_waits(listener, egui::Context::default(), hub, waits);
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|e| panic!("{e}"));
        let mut w = s.try_clone().unwrap_or_else(|e| panic!("{e}"));
        let t0 = Instant::now();
        let feeder = std::thread::spawn(move || {
            for i in 0..40 {
                let line = if i % 2 == 0 { "\n" } else { "not json\n" };
                if w.write_all(line.as_bytes()).is_err() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            false
        });
        let mut r = BufReader::new(s);
        let _ = r.get_ref().set_read_timeout(Some(Duration::from_secs(5)));
        let mut closed_at = None;
        let mut line = String::new();
        while closed_at.is_none() {
            line.clear();
            match r.read_line(&mut line) {
                Ok(0) | Err(_) => closed_at = Some(t0.elapsed()),
                Ok(_) => {}
            }
        }
        let _ = feeder.join();
        assert!(closed_at.is_some_and(|d| d < Duration::from_millis(1200)), "{closed_at:?}");
    }

    #[test]
    fn unknown_key_is_refused_and_closed() {
        let (port, _, _) = boot();
        let mut c = Conn::open(port);
        assert_eq!(c.call(r#"{"id":1,"key":"nope","method":"chat.members"}"#)["error"], "unauthorized");
        assert!(c.is_eof());
    }
}

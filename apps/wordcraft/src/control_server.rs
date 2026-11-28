//! The control server: craft-automation's loopback JSON-lines transport, started on the port the
//! user asked for.
//!
//! A connection authenticates once with the token (`docs/control-protocol.md`, Token). When no
//! token or token file is supplied, the app generates a token and writes it to
//! `<settings>/control-key.<instance>` (mode 0600) after the port is bound, and removes the file
//! when the window closes.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use craft::automation::args::ControlOptions;
use craft::automation::control::ControlRequest;
use craft::automation::server::{Listening, serve};
use craft::ui::runtime::windows::WindowHost;

/// A running control server. [`Control::shut_down`] it when the window closes.
pub struct Control {
    /// Dropping it stops accepting connections and frees the port.
    listening: Listening,
    /// The default token file this run wrote; a token file the user supplied is never removed.
    created: Option<PathBuf>,
}

impl Control {
    /// Stop accepting connections and remove the token file this run wrote, if any.
    pub fn shut_down(self) {
        let Self { listening, created } = self;
        drop(listening);
        remove(created.as_deref());
    }
}

fn remove(path: Option<&Path>) {
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

/// Bind `127.0.0.1:port`, settle the token and serve. `None` when the port cannot be bound or
/// the token cannot be set up (the control channel is off); a taken port leaves no token file.
///
/// With no token and no token file in `options`, the token is kept in the default file for
/// `port` in `settings`; a file left by a run that did not exit cleanly is replaced. Without a
/// settings folder a fresh token is generated and reported by the log.
pub fn start(options: &ControlOptions, port: u16, host: WindowHost, settings: Option<&Path>) -> Option<(Receiver<ControlRequest>, Control)> {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server failed to bind 127.0.0.1:{port}: {e}");
            return None;
        }
    };
    let created = settings.filter(|_| options.token.is_none() && options.token_file.is_none()).map(|dir| wordcraft_control_key::key_path(dir, port));
    // The port was free, so no other run owns a file left here.
    remove(created.as_deref());
    let effective = ControlOptions { token_file: created.clone().or_else(|| options.token_file.clone()), ..options.clone() };
    let token = match effective.server_token() {
        Ok(t) => t,
        Err(e) => {
            remove(created.as_deref());
            log::error!("control server off: {e}");
            return None;
        }
    };
    let notice = effective.token_notice(&token);
    match serve(listener, token, host) {
        Ok((rx, listening)) => {
            log::info!("control server listening on {}, {notice}", listening.addr);
            Some((rx, Control { listening, created }))
        }
        Err(e) => {
            remove(created.as_deref());
            log::error!("control server off: cannot start the accept thread ({e})");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::automation::client::ControlClient;

    fn host() -> WindowHost {
        WindowHost::of(&egui::Context::default())
    }

    #[test]
    fn the_token_file_is_written_after_the_bind_and_removed_on_exit() {
        let dir = std::env::temp_dir().join(format!("wordcraft-control-life-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let options = ControlOptions::default();

        // The port is taken: no token file.
        let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        assert!(start(&options, port, host(), Some(&dir)).is_none());
        assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none(), "no token file after a failed bind");
        drop(taken);

        // Bound: the default file holds the token that opens the port.
        let (_rx, control) = start(&options, 0, host(), Some(&dir)).unwrap();
        let path = control.created.clone().unwrap();
        assert_eq!(path, wordcraft_control_key::key_path(&dir, 0));
        let token = std::fs::read_to_string(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(ControlClient::connect(control.listening.addr, token.trim()).is_ok());
        control.shut_down();
        assert!(!path.exists());

        // A token file the user supplied stays.
        let supplied = dir.join("supplied-token");
        let options = ControlOptions { token_file: Some(supplied.clone()), ..ControlOptions::default() };
        let (_rx, control) = start(&options, 0, host(), Some(&dir)).unwrap();
        assert!(control.created.is_none());
        control.shut_down();
        assert!(supplied.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn boot() -> (u16, Receiver<ControlRequest>, Control) {
        let options = ControlOptions { token: Some(TOKEN.to_string()), ..ControlOptions::default() };
        let (rx, control) = start(&options, 0, host(), None).unwrap();
        let port = control.listening.addr.port();
        (port, rx, control)
    }

    /// Answer the next request that reaches the UI with `ok`, and return it.
    fn answer(rx: &Receiver<ControlRequest>) -> ControlRequest {
        let req = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let _ = req.reply.send(serde_json::json!({"ok": true, "result": {}}));
        req
    }

    /// One connection, several lines.
    struct Conn(std::io::BufReader<std::net::TcpStream>, std::net::TcpStream);

    impl Conn {
        fn open(port: u16) -> Self {
            let s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            Self(std::io::BufReader::new(s.try_clone().unwrap()), s)
        }
        fn send(&mut self, line: &str) {
            use std::io::Write;
            writeln!(self.1, "{line}").unwrap();
        }
        fn recv(&mut self) -> serde_json::Value {
            use std::io::BufRead;
            let mut r = String::new();
            let n = self.0.read_line(&mut r).expect("read reply line");
            assert!(n > 0, "unexpected EOF while waiting for reply");
            serde_json::from_str(&r).expect("reply is valid JSON")
        }
        fn call(&mut self, line: &str) -> serde_json::Value {
            self.send(line);
            self.recv()
        }
        /// The server closed the connection.
        fn closed(&mut self) -> bool {
            use std::io::BufRead;
            let mut r = String::new();
            match self.0.read_line(&mut r) {
                Ok(0) => true,
                Err(e) => matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::NotConnected
                        | std::io::ErrorKind::BrokenPipe
                ),
                _ => false,
            }
        }
    }

    /// Post-#7101 adaptation of `the_key_on_each_request` (#124, tracked under #7317 / #7720):
    /// `craft::automation::server` replaced the legacy per-request `"key"` field with one-time
    /// per-connection token authentication (`docs/control-protocol.md`, Token).
    #[test]
    fn a_connection_authenticates_once_with_the_token_and_unauthenticated_requests_are_refused() {
        let (port, rx, control) = boot();

        // Authenticate once per connection with `params.token`.
        let mut c = Conn::open(port);
        let auth_resp = c.call(&format!(r#"{{"id":1,"method":"auth","params":{{"token":"{TOKEN}"}}}}"#));
        assert_eq!(auth_resp, serde_json::json!({"id": 1, "ok": true, "result": {"authenticated": true}}));

        // Subsequent requests on the authenticated connection carry no token and reach the UI.
        c.send(r#"{"id":7,"method":"document.inspect","params":{"text":true}}"#);
        let req = answer(&rx);
        assert_eq!((req.method.as_str(), &req.params), ("document.inspect", &serde_json::json!({"text": true})));
        let r = c.recv();
        assert_eq!((&r["ok"], &r["id"]), (&serde_json::json!(true), &serde_json::json!(7)));

        c.send(r#"{"id":8,"method":"document.inspect","params":{"text":false}}"#);
        let req = answer(&rx);
        assert_eq!((req.method.as_str(), &req.params), ("document.inspect", &serde_json::json!({"text": false})));
        let r = c.recv();
        assert_eq!((&r["ok"], &r["id"]), (&serde_json::json!(true), &serde_json::json!(8)));
        drop(c);

        // An unauthenticated connection is refused and closed without reaching the UI.
        let mut unauth = Conn::open(port);
        let r = unauth.call(r#"{"id":9,"method":"document.inspect"}"#);
        assert_eq!(r, serde_json::json!({"id": 9, "ok": false, "error": "authentication required"}));
        assert!(unauth.closed());
        assert!(rx.try_recv().is_err(), "unauthenticated request must not reach the UI");
        drop(unauth);

        // Legacy pre-#7101 per-request `"key"` does not bypass `auth` and closes the connection.
        let mut legacy = Conn::open(port);
        let r = legacy.call(&format!(r#"{{"id":10,"key":"{TOKEN}","method":"document.inspect","params":{{"text":true}}}}"#));
        assert_eq!(r, serde_json::json!({"id": 10, "ok": false, "error": "authentication required"}));
        assert!(legacy.closed());
        assert!(rx.try_recv().is_err(), "legacy per-request key must not reach the UI");
        drop(legacy);

        control.shut_down();
    }

    /// Post-#7101 adaptation of `params_key_is_only_a_secret_for_auth` (#124, tracked under #7317 / #7729):
    /// `craft::automation::server` replaced legacy `"key"` / `params.key` secrets with one-time
    /// per-connection `auth` using `params.token` (`docs/control-protocol.md`, Token).
    #[test]
    fn params_token_is_only_a_secret_for_auth() {
        let (port, rx, control) = boot();

        // Neither `params.token` on a non-`auth` method nor legacy `params.key` on
        // `document.inspect` or `auth` authenticates.
        for (id, line) in [
            (1, format!(r#"{{"id":1,"method":"document.inspect","params":{{"token":"{TOKEN}"}}}}"#)),
            (2, format!(r#"{{"id":2,"method":"document.inspect","params":{{"key":"{TOKEN}"}}}}"#)),
            (3, format!(r#"{{"id":3,"method":"auth","params":{{"key":"{TOKEN}"}}}}"#)),
        ] {
            let mut c = Conn::open(port);
            assert_eq!(c.call(&line), serde_json::json!({"id": id, "ok": false, "error": "authentication required"}));
            assert!(c.closed());
        }
        assert!(rx.try_recv().is_err(), "refused requests must not reach the UI");

        // Authenticate once with `params.token`; `params.key` and `params.token` on subsequent
        // non-`auth` methods are forwarded untouched as ordinary parameters.
        let mut c = Conn::open(port);
        let auth_resp = c.call(&format!(r#"{{"id":1,"method":"auth","params":{{"token":"{TOKEN}"}}}}"#));
        assert_eq!(auth_resp, serde_json::json!({"id": 1, "ok": true, "result": {"authenticated": true}}));

        c.send(r#"{"id":2,"method":"ui.key","params":{"key":"Enter"}}"#);
        let req = answer(&rx);
        assert_eq!((req.method.as_str(), &req.params), ("ui.key", &serde_json::json!({"key": "Enter"})));
        let r = c.recv();
        assert_eq!((&r["ok"], &r["id"]), (&serde_json::json!(true), &serde_json::json!(2)));

        c.send(r#"{"id":3,"method":"document.inspect","params":{"token":"ordinary"}}"#);
        let req = answer(&rx);
        assert_eq!((req.method.as_str(), &req.params), ("document.inspect", &serde_json::json!({"token": "ordinary"})));
        let r = c.recv();
        assert_eq!((&r["ok"], &r["id"]), (&serde_json::json!(true), &serde_json::json!(3)));
        drop(c);

        control.shut_down();
    }

    /// Post-#7101 adaptation of `a_wrong_key_is_refused_and_closed` (#124, tracked under #7317):
    /// wrong, short, and long `params.token` values are refused and close the connection without
    /// reaching the UI.
    #[test]
    fn a_wrong_token_is_refused_and_closed() {
        let (port, rx, control) = boot();
        let wrong = TOKEN.replace('0', "1");
        let long = format!("{TOKEN}0");
        for token in ["nope", wrong.as_str(), &TOKEN[..63], long.as_str()] {
            let mut c = Conn::open(port);
            let resp = c.call(&format!(r#"{{"id":1,"method":"auth","params":{{"token":"{token}"}}}}"#));
            assert_eq!(resp, serde_json::json!({"id": 1, "ok": false, "error": "authentication required"}), "{token}");
            assert!(c.closed(), "{token}");
        }
        assert!(rx.try_recv().is_err(), "refused auth attempts must not reach the UI");
        control.shut_down();
    }
}

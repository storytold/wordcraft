//! Where MCP tool calls end up: a control-channel method call.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
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
}

/// A running WordCraft app, reached through its loopback control port.
pub struct Remote {
    addr: String,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
    /// Control key (env `WORDCRAFT_CONTROL_KEY` or the per-instance key file), read lazily.
    key: Option<String>,
}

/// Key lookup order: `env` (`WORDCRAFT_CONTROL_KEY`), then the per-instance key file in
/// `config_dir`. The error names the path that was tried.
fn lookup_key(env: Option<String>, config_dir: Option<&std::path::Path>, port: Option<u16>) -> Result<String, String> {
    if let Some(k) = env.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        return Ok(k);
    }
    let (Some(dir), Some(port)) = (config_dir, port) else {
        return Err("no control key: set WORDCRAFT_CONTROL_KEY (no config directory or port to look in)".to_string());
    };
    let path = wordcraft_chat::control_key_path(dir, port);
    match std::fs::read_to_string(&path).map(|k| k.trim().to_string()) {
        Ok(k) if !k.is_empty() => Ok(k),
        _ => Err(format!("no control key: set WORDCRAFT_CONTROL_KEY (looked in {})", path.display())),
    }
}

/// The control key for the app on `addr`, looked up fresh (the app picks a new key at every start).
fn find_key(addr: &str) -> Result<String, String> {
    let port = addr.rsplit(':').next().and_then(|p| p.parse().ok());
    lookup_key(std::env::var("WORDCRAFT_CONTROL_KEY").ok(), wordcraft_chat::config_dir().as_deref(), port)
}

impl Remote {
    /// Connect to `addr` (`127.0.0.1:7981`), failing fast when nothing is listening.
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        let mut r = Self { addr: addr.to_string(), conn: None, next_id: 1, key: None };
        r.reconnect()?;
        Ok(r)
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    fn reconnect(&mut self) -> std::io::Result<()> {
        self.conn = None;
        let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {}", self.addr));
        for sa in self.addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&sa, Duration::from_millis(800)) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    // The app answers within 60 s (its own timeout); leave headroom.
                    s.set_read_timeout(Some(Duration::from_secs(90))).ok();
                    let read = s.try_clone()?;
                    self.conn = Some((BufReader::new(read), s));
                    return Ok(());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn roundtrip(&mut self, line: &str) -> std::io::Result<String> {
        if self.conn.is_none() {
            self.reconnect()?;
        }
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
}

impl Remote {
    /// Send one request and parse the reply. Looks the key up when none is cached, and retries
    /// once on a new connection (with a new key lookup) when the connection fails.
    fn exchange(&mut self, id: u64, method: &str, params: &Value) -> Result<Value, String> {
        let attempt = |this: &mut Self| -> std::io::Result<Result<String, String>> {
            if this.key.is_none() {
                match find_key(&this.addr) {
                    Ok(k) => this.key = Some(k),
                    Err(e) => return Ok(Err(e)),
                }
            }
            let key = this.key.clone().unwrap_or_default();
            let line = json!({"id": id, "key": key, "method": method, "params": params}).to_string();
            this.roundtrip(&line).map(Ok)
        };
        let reply = match attempt(self) {
            Ok(r) => r,
            Err(_) => {
                self.conn = None;
                self.key = None;
                attempt(self).map_err(|e| {
                    self.conn = None;
                    format!("WordCraft app at {} is not reachable: {e}", self.addr)
                })?
            }
        };
        let reply = reply?;
        serde_json::from_str(reply.trim()).map_err(|e| format!("bad reply from app: {e}"))
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        // One retry with a fresh connection (the app may have restarted); a fresh connection
        // also means a fresh key lookup.
        let mut v = self.exchange(id, method, &params)?;
        if v.get("error").and_then(Value::as_str) == Some("unauthorized") {
            // Stale key (app restarted): drop it, reconnect, look it up again, retry once.
            self.key = None;
            self.conn = None;
            v = self.exchange(id, method, &params)?;
        }
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(v.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_string())
        }
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        format!("connected to the WordCraft app at {}", self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_lookup_order() {
        let dir = std::env::temp_dir().join(format!("wc-mcp-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = wordcraft_chat::control_key_path(&dir, 7981);

        // Neither: the error names the path it looked for.
        let e = lookup_key(None, Some(&dir), Some(7981)).unwrap_err();
        assert!(e.contains(&path.display().to_string()), "{e}");
        assert!(e.contains("WORDCRAFT_CONTROL_KEY"));

        // File only: content is trimmed.
        std::fs::write(&path, "  filekey\n").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lookup_key(None, Some(&dir), Some(7981)).unwrap_or_default(), "filekey");
        assert_eq!(lookup_key(Some("  ".into()), Some(&dir), Some(7981)).unwrap_or_default(), "filekey");

        // Env wins over the file.
        assert_eq!(lookup_key(Some(" envkey ".into()), Some(&dir), Some(7981)).unwrap_or_default(), "envkey");

        // A new file content is seen on the next lookup (app restarted with a new key).
        std::fs::write(&path, "newkey").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lookup_key(None, Some(&dir), Some(7981)).unwrap_or_default(), "newkey");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

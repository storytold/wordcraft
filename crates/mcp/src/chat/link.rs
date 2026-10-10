//! One TCP connection to a window's control port, with a member key on every request.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{Value, json};

/// Longest reply line read (a page PNG in base64 fits).
const MAX_REPLY: u64 = 64 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Nothing listens: the window is closed.
    Refused,
    /// The connection broke or the window closed it.
    Closed,
    /// No answer in time.
    Timeout,
    /// The key was refused (removed, or the chat stopped).
    Unauthorized,
    /// The window answered with an error.
    Remote(String),
    /// Not an answer the client understands.
    Bad(String),
}

/// Something that answers control methods: a [`Link`], or an MCP backend.
pub trait Caller {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, LinkError>;
}

pub struct Link {
    addr: String,
    key: Option<String>,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
    timeout: Duration,
}

impl Link {
    /// To the window at `addr`, sending `key` with every request; connects on the first call.
    pub fn new(addr: &str, key: Option<&str>) -> Link {
        Link { addr: addr.to_string(), key: key.map(str::to_string), conn: None, next_id: 1, timeout: Duration::from_secs(60) }
    }

    fn connect(&mut self) -> Result<(), LinkError> {
        let sa: SocketAddr = self
            .addr
            .to_socket_addrs()
            .map_err(|e| LinkError::Bad(format!("{}: {e}", self.addr)))?
            .find(|a| a.ip().is_loopback())
            .ok_or_else(|| LinkError::Bad(format!("{}: not an address on this computer", self.addr)))?;
        let s = TcpStream::connect_timeout(&sa, Duration::from_secs(2))
            .map_err(|e| if e.kind() == std::io::ErrorKind::ConnectionRefused { LinkError::Refused } else { LinkError::Closed })?;
        let _ = s.set_nodelay(true);
        let r = s.try_clone().map_err(|_| LinkError::Closed)?;
        self.conn = Some((BufReader::new(r), s));
        Ok(())
    }

    /// One request and its reply within `t`.
    pub fn call_within(&mut self, method: &str, params: Value, t: Duration) -> Result<Value, LinkError> {
        if self.conn.is_none() {
            self.connect()?;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let mut req = json!({"id": id, "method": method, "params": params});
        if let Some(k) = &self.key {
            req["key"] = json!(k);
        }
        let Some((r, w)) = self.conn.as_mut() else { return Err(LinkError::Closed) };
        let io = (|| -> std::io::Result<String> {
            r.get_ref().set_read_timeout(Some(t.max(Duration::from_millis(1))))?;
            writeln!(w, "{req}")?;
            w.flush()?;
            let mut line = String::new();
            if r.by_ref().take(MAX_REPLY).read_line(&mut line)? == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            Ok(line)
        })();
        let line = match io {
            Ok(l) => l,
            Err(e) => {
                self.conn = None;
                return Err(match e.kind() {
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => LinkError::Timeout,
                    _ => LinkError::Closed,
                });
            }
        };
        let v: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(e) => {
                self.conn = None;
                return Err(LinkError::Bad(format!("malformed reply from the window: {e}")));
            }
        };
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
        match v.get("error").and_then(Value::as_str).unwrap_or("error") {
            "unauthorized" => {
                self.conn = None;
                Err(LinkError::Unauthorized)
            }
            e => Err(LinkError::Remote(e.to_string())),
        }
    }
}

impl Caller for Link {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, LinkError> {
        let t = self.timeout;
        self.call_within(method, params, t)
    }
}

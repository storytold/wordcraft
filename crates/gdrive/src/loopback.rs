//! The one-shot listener on `127.0.0.1` that receives Google's sign-in redirect.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::oauth::parse_callback;
use crate::{Error, Result};

/// The most bytes read from one request: the redirect is a few hundred.
const MAX_REQUEST: usize = 16 * 1024;
/// The most connections served before giving up (browsers also ask for a favicon).
const MAX_CONNECTIONS: usize = 32;

pub struct Listener {
    inner: TcpListener,
    addr: SocketAddr,
}

impl Listener {
    /// Binds an ephemeral port on the IPv4 loopback address only.
    pub fn bind() -> Result<Self> {
        let inner = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| Error::SignIn(format!("couldn't listen on 127.0.0.1: {e}")))?;
        let addr = inner.local_addr().map_err(|e| Error::SignIn(e.to_string()))?;
        inner.set_nonblocking(true).map_err(|e| Error::SignIn(e.to_string()))?;
        Ok(Self { inner, addr })
    }

    /// `http://127.0.0.1:<port>`, the redirect URI sent to Google and back in the exchange.
    pub fn redirect_uri(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Waits for the redirect and returns the authorization code.
    pub fn wait(&self, state: &str, cancel: &AtomicBool, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        let mut served = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::SignIn("cancelled".into()));
            }
            if Instant::now() >= deadline {
                return Err(Error::SignIn("timed out waiting for the browser".into()));
            }
            match self.inner.accept() {
                Ok((stream, peer)) => {
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    served += 1;
                    if let Some(result) = serve(stream, state) {
                        return result;
                    }
                    if served >= MAX_CONNECTIONS {
                        return Err(Error::SignIn("too many requests on the sign-in listener".into()));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(Error::SignIn(format!("listener failed: {e}"))),
            }
        }
    }
}

/// Answers one connection. `None` when it wasn't the redirect (keep waiting).
fn serve(mut stream: TcpStream, state: &str) -> Option<Result<String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let target = read_target(&mut stream)?;
    let outcome = match parse_callback(&target, state) {
        Ok(None) => {
            respond(&mut stream, "404 Not Found", "");
            return None;
        }
        Ok(Some(code)) => Ok(code),
        Err(e) => Err(e),
    };
    let body = match &outcome {
        Ok(_) => "Signed in to Google Drive. You can close this tab and go back to WordCraft.",
        Err(_) => "WordCraft couldn't sign in to Google Drive. Go back to WordCraft for details.",
    };
    respond(&mut stream, "200 OK", body);
    Some(outcome)
}

/// The request target of `GET <target> HTTP/1.1`, reading at most [`MAX_REQUEST`] bytes.
fn read_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(2).any(|w| w == b"\r\n") {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 || buf.len() + n > MAX_REQUEST {
            return None;
        }
        buf.extend_from_slice(chunk.get(..n)?);
    }
    request_target(&buf)
}

/// Parses the request line. Only `GET` is accepted.
pub fn request_target(head: &[u8]) -> Option<String> {
    let line = head.split(|&b| b == b'\r' || b == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split(' ');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("GET"), Some(target), Some(v)) if v.starts_with("HTTP/") => Some(target.to_owned()),
        _ => None,
    }
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let msg =
        format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(msg.as_bytes());
    let _ = stream.flush();
}

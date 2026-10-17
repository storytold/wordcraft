//! Sending a [`Request`]: HTTPS on the desktop; the browser build can't check (it has no HTTP
//! client of its own, and the public server's cross-origin rules aren't ours to rely on).

use std::time::Duration;

use crate::{LtError, Request};

/// Why the browser build has no LanguageTool checking.
pub const UNSUPPORTED_REASON: &str = "LanguageTool checking needs the desktop app; the browser version can't send text to a checking server yet.";

/// Whether this build can send requests.
pub const fn available() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

/// Send `req`, waiting at most `timeout` for the answer, and return its body.
#[cfg(not(target_arch = "wasm32"))]
pub fn post(req: &Request, timeout: Duration) -> Result<String, LtError> {
    use std::io::Read;

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5).min(timeout))
        .timeout(timeout)
        .redirects(0)
        .user_agent(concat!("WordCraft/", env!("CARGO_PKG_VERSION")))
        .build();
    let sent = agent
        .post(&req.url)
        .set("Accept", "application/json")
        .set("Content-Type", "application/x-www-form-urlencoded; charset=utf-8")
        .send_string(&req.body);
    match sent {
        Ok(resp) => {
            let mut bytes = Vec::new();
            let limit = u64::try_from(crate::response::MAX_RESPONSE_BYTES).unwrap_or(u64::MAX).saturating_add(1);
            resp.into_reader().take(limit).read_to_end(&mut bytes).map_err(|e| io_error(&e))?;
            utf8_body(bytes)
        }
        Err(ureq::Error::Status(429, _)) => Err(LtError::RateLimited),
        Err(ureq::Error::Status(status, resp)) => {
            // The server's explanation, shortened (it never contains what we sent).
            let mut text = String::new();
            let _ = resp.into_reader().take(4096).read_to_string(&mut text);
            let message: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
            Err(LtError::Http { status, message: message.trim().to_string() })
        }
        Err(ureq::Error::Transport(t)) => {
            let s = t.to_string();
            if s.contains("timed out") || s.contains("Timeout") { Err(LtError::Timeout) } else { Err(LtError::Network(s)) }
        }
    }
}

/// An answer's body as text. One that isn't UTF-8 isn't LanguageTool's JSON: a protocol error, so
/// the paragraph is skipped rather than sent again every 30 seconds (#1283).
pub fn utf8_body(bytes: Vec<u8>) -> Result<String, LtError> {
    String::from_utf8(bytes).map_err(|e| LtError::Protocol(format!("the answer isn't UTF-8 text ({e})")))
}

#[cfg(not(target_arch = "wasm32"))]
fn io_error(e: &std::io::Error) -> LtError {
    if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) { LtError::Timeout } else { LtError::Network(e.to_string()) }
}

/// The browser build: no requests.
#[cfg(target_arch = "wasm32")]
pub fn post(_req: &Request, _timeout: Duration) -> Result<String, LtError> {
    Err(LtError::Unsupported(UNSUPPORTED_REASON))
}

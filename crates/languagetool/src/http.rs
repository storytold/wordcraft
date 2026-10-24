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
            let mut body = String::new();
            let limit = u64::try_from(crate::response::MAX_RESPONSE_BYTES).unwrap_or(u64::MAX).saturating_add(1);
            resp.into_reader().take(limit).read_to_string(&mut body).map_err(|e| io_error(&e))?;
            Ok(body)
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

#[cfg(not(target_arch = "wasm32"))]
fn io_error(e: &std::io::Error) -> LtError {
    if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) { LtError::Timeout } else { LtError::Network(e.to_string()) }
}

/// The browser build: no requests.
#[cfg(target_arch = "wasm32")]
pub fn post(_req: &Request, _timeout: Duration) -> Result<String, LtError> {
    Err(LtError::Unsupported(UNSUPPORTED_REASON))
}

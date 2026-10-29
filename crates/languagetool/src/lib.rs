//! LanguageTool integration (#575): spelling and grammar checking by a LanguageTool server.
//!
//! Opt-in: it is off until the user turns it on, because the text of each paragraph is sent to
//! the server. The server is the public `https://api.languagetool.org` by default or any
//! self-hosted one; a username and API key unlock a premium account.
//!
//! - [`Settings`]: what the user chose (kept with the app's preferences).
//! - [`request`]: the `POST /v2/check` request for one paragraph (form-encoded).
//! - [`response`]: LanguageTool's JSON answer → [`wordcraft_proof::Issue`]s, with its offsets
//!   (Java `char`s, i.e. UTF-16 code units) mapped to byte offsets in the paragraph ([`offsets`]).
//! - [`http`]: the HTTP call (desktop only; the browser build reports why it can't check).
//! - [`worker`]: a background thread that checks one paragraph at a time, paced so the public
//!   server's rate limit holds; the app feeds it and stores results in
//!   [`wordcraft_proof::external`], where layout and the Review commands pick them up.
//!
//! Only LanguageTool's public HTTP API is used; no LanguageTool code is.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod http;
pub mod offsets;
pub mod request;
pub mod response;
mod settings;
pub mod worker;

pub use request::Request;
pub use settings::{DEFAULT_SERVER, Settings, is_language_code};
pub use worker::{Done, Job, Worker};

/// Things that go wrong checking text.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum LtError {
    /// The server address isn't an `http://` or `https://` URL.
    #[error("the LanguageTool server address isn't a web address: {0}")]
    BadServer(String),
    /// The paragraph is longer than one request may carry.
    #[error("the paragraph is too long to check ({bytes} bytes; at most {max})")]
    TooLong { bytes: usize, max: usize },
    /// No connection (offline, wrong address, server down, TLS failure).
    #[error("couldn't reach the LanguageTool server: {0}")]
    Network(String),
    /// The server took too long.
    #[error("the LanguageTool server didn't answer in time")]
    Timeout,
    /// Too many requests: the server asked us to slow down.
    #[error("the LanguageTool server is limiting requests; checking resumes in a minute")]
    RateLimited,
    /// The server turned the request down (bad API key, unknown language…).
    #[error("the LanguageTool server answered {status}: {message}")]
    Http { status: u16, message: String },
    /// The answer wasn't LanguageTool's JSON.
    #[error("unexpected answer from the LanguageTool server: {0}")]
    Protocol(String),
    /// This build can't make the request.
    #[error("{0}")]
    Unsupported(&'static str),
}

impl LtError {
    /// The server rejected our credentials: checking stops until the settings change.
    pub fn is_auth(&self) -> bool {
        matches!(self, LtError::Http { status: 401 | 403, .. })
    }
}

/// Check one paragraph: build the request, send it with `transport`, read the answer. When the
/// server rejects a specific language (HTTP 400), the text is checked once more with automatic
/// language detection.
pub fn check_with(
    settings: &Settings,
    transport: &mut dyn FnMut(&Request) -> Result<String, LtError>,
    text: &str,
    language: &str,
) -> Result<Vec<wordcraft_proof::Issue>, LtError> {
    let req = request::build(settings, text, language)?;
    let body = match transport(&req) {
        Err(LtError::Http { status: 400, message }) if language != request::AUTO => {
            log::info!("LanguageTool rejected language {language:?} ({message}); detecting it instead");
            transport(&request::build(settings, text, request::AUTO)?)?
        }
        r => r?,
    };
    response::parse(&body, text)
}

#[cfg(test)]
mod tests;

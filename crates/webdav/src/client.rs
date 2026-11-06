//! The HTTP side: one document at one URL, through `ureq`.
use std::time::Duration;

use ureq::http::{Method, Request, StatusCode, Uri};

use crate::lock::{self, Lock};
use crate::{Error, Result};

/// The default size cap for a loaded document: 512 MiB, far above any real .docx.
pub const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;
/// How long one request may take, from connecting to the last byte.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// One document on a WebDAV server.
///
/// A document management system typically hands out one URL per editing session, often with the
/// session's credential in the path, and refuses everything but GET, LOCK, PUT and UNLOCK on it.
/// So the client never creates, moves or deletes anything, and never follows a redirect: a lock
/// and the save it guards must go to exactly this URL.
#[derive(Debug, Clone)]
pub struct Resource {
    url: String,
    agent: ureq::Agent,
    max_bytes: u64,
}

impl Resource {
    /// The document at `url`, which must be an `http://` or `https://` URL with a host.
    pub fn new(url: &str) -> Result<Self> {
        let url = url.trim();
        let uri: Uri = url.parse().map_err(|_| Error::InvalidUrl(redact(url)))?;
        let scheme_ok = uri.scheme_str().is_some_and(|s| s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https"));
        let host_ok = uri.host().is_some_and(|h| !h.is_empty());
        if !scheme_ok || !host_ok {
            return Err(Error::InvalidUrl(redact(url)));
        }
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let agent = ureq::Agent::config_builder()
            // Statuses are answers this client interprets (423 Locked, 412…), not transport errors.
            .http_status_as_error(false)
            // LOCK and UNLOCK are WebDAV methods, not HTTP/1.1 ones.
            .allow_non_standard_methods(true)
            .max_redirects(0)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .tls_config(tls)
            .user_agent(concat!("WordCraft/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        Ok(Self { url: url.to_owned(), agent, max_bytes: DEFAULT_MAX_BYTES })
    }

    /// The same document with a different size cap for [`Resource::get`].
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// The document's URL, for messages; any credential in it is replaced by `…`.
    pub fn display_url(&self) -> String {
        redact(&self.url)
    }

    /// Loads the document (GET).
    pub fn get(&self) -> Result<Vec<u8>> {
        let request = self.request(Method::GET).body(()).map_err(transport)?;
        let mut response = self.agent.run(request).map_err(transport)?;
        expect(response.status(), "GET", &[StatusCode::OK])?;
        response.body_mut().with_config().limit(self.max_bytes).read_to_vec().map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => Error::TooLarge { limit: self.max_bytes },
            e => transport(e),
        })
    }

    /// Takes an exclusive write lock for `owner` (a name the server may show to others), asking
    /// to keep it for `timeout` (LOCK, RFC 4918 §9.10). The server may grant a different timeout;
    /// the returned [`Lock`] carries what it granted.
    pub fn lock(&self, owner: &str, timeout: Duration) -> Result<Lock> {
        let request = self
            .request(lock_method())
            .header("Depth", "0")
            .header("Timeout", lock::timeout_header(timeout))
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(lock::lockinfo(owner))
            .map_err(transport)?;
        let response = self.agent.run(request).map_err(transport)?;
        expect(response.status(), "LOCK", &[StatusCode::OK, StatusCode::CREATED])?;
        let token = header(&response, "Lock-Token").and_then(lock::parse_lock_token).ok_or(Error::NoLockToken)?;
        let granted = header(&response, "Timeout").and_then(lock::parse_timeout).unwrap_or(Some(timeout));
        Ok(Lock::new(token, granted))
    }

    /// Keeps `held` alive for another `timeout` (a LOCK with only an `If` header, RFC 4918
    /// §9.10.2). Returns the lock with the timeout the server granted this time.
    pub fn refresh(&self, held: &Lock, timeout: Duration) -> Result<Lock> {
        let request = self
            .request(lock_method())
            .header("If", held.if_header())
            .header("Timeout", lock::timeout_header(timeout))
            .body(())
            .map_err(transport)?;
        let response = self.agent.run(request).map_err(transport)?;
        expect(response.status(), "LOCK", &[StatusCode::OK])?;
        let granted = header(&response, "Timeout").and_then(lock::parse_timeout).unwrap_or(Some(timeout));
        Ok(Lock::new(held.token(), granted))
    }

    /// Saves `bytes` as the document (PUT). With `held`, the save only goes through while the
    /// server still holds that lock for this client.
    pub fn put(&self, bytes: &[u8], content_type: &str, held: Option<&Lock>) -> Result<()> {
        let mut builder = self.request(Method::PUT).header("Content-Type", content_type);
        if let Some(held) = held {
            builder = builder.header("If", held.if_header());
        }
        let request = builder.body(bytes).map_err(transport)?;
        let response = self.agent.run(request).map_err(transport)?;
        expect(response.status(), "PUT", &[StatusCode::OK, StatusCode::CREATED, StatusCode::NO_CONTENT])
    }

    /// Releases `held` (UNLOCK, RFC 4918 §9.11).
    pub fn unlock(&self, held: Lock) -> Result<()> {
        let request = self.request(unlock_method()).header("Lock-Token", held.lock_token_header()).body(()).map_err(transport)?;
        let response = self.agent.run(request).map_err(transport)?;
        expect(response.status(), "UNLOCK", &[StatusCode::NO_CONTENT, StatusCode::OK])
    }

    fn request(&self, method: Method) -> ureq::http::request::Builder {
        Request::builder().method(method).uri(&self.url)
    }
}

fn lock_method() -> Method {
    Method::from_bytes(b"LOCK").unwrap_or(Method::POST)
}

fn unlock_method() -> Method {
    Method::from_bytes(b"UNLOCK").unwrap_or(Method::POST)
}

/// The value of response header `name`, if it is text.
fn header<'a>(response: &'a ureq::http::Response<ureq::Body>, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

/// `Ok` for an expected status, otherwise the [`Error`] it stands for.
fn expect(status: StatusCode, method: &'static str, ok: &[StatusCode]) -> Result<()> {
    if ok.contains(&status) {
        return Ok(());
    }
    let code = status.as_u16();
    Err(match code {
        401 | 403 => Error::Forbidden { method, status: code },
        404 | 410 => Error::NotFound { status: code },
        423 => Error::Locked,
        412 => Error::LockLost { method },
        _ => Error::Status { method, status: code },
    })
}

fn transport(e: impl std::fmt::Display) -> Error {
    Error::Transport(e.to_string())
}

/// `url` for messages and logs: user info and the path are dropped, because a document management
/// system's session credential often sits in one of them.
fn redact(url: &str) -> String {
    match url.parse::<Uri>() {
        Ok(uri) => match (uri.scheme_str(), uri.host()) {
            (Some(scheme), Some(host)) => match uri.port_u16() {
                Some(port) => format!("{scheme}://{host}:{port}/…"),
                None => format!("{scheme}://{host}/…"),
            },
            _ => "…".to_owned(),
        },
        Err(_) => "…".to_owned(),
    }
}

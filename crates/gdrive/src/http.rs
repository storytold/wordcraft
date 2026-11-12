//! The HTTP seam. [`Transport`] sends one request and returns the whole (bounded) response.

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Patch,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Patch => "PATCH",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The most response bytes to accept.
    pub limit: u64,
}

impl std::fmt::Debug for Request {
    // Headers carry bearer tokens and bodies carry codes and refresh tokens: log neither.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("method", &self.method).field("url", &self.url).field("body_len", &self.body.len()).finish_non_exhaustive()
    }
}

impl Request {
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self { method, url: url.into(), headers: Vec::new(), body: Vec::new(), limit: JSON_LIMIT }
    }
    pub fn header(mut self, k: &str, v: impl Into<String>) -> Self {
        self.headers.push((k.to_owned(), v.into()));
        self
    }
    pub fn body(mut self, content_type: &str, body: Vec<u8>) -> Self {
        self.body = body;
        self.header("Content-Type", content_type)
    }
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = limit;
        self
    }
    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", format!("Bearer {token}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    /// The response as JSON when the status is 2xx, otherwise the API error it describes.
    pub fn json(&self) -> Result<serde_json::Value> {
        if !self.ok() {
            return Err(self.api_error());
        }
        serde_json::from_slice(&self.body).map_err(|e| Error::Protocol(format!("not JSON: {e}")))
    }
    /// Google's error JSON (`{"error":{"message":…}}` or `{"error":"…","error_description":…}`)
    /// turned into [`Error::Api`]; falls back to a short prefix of the body.
    pub fn api_error(&self) -> Error {
        let v: serde_json::Value = serde_json::from_slice(&self.body).unwrap_or_default();
        let err = v.get("error");
        let message = err
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .or_else(|| v.get("error_description").and_then(|m| m.as_str()))
            .or_else(|| err.and_then(|e| e.as_str()))
            .map(str::to_owned)
            .unwrap_or_else(|| String::from_utf8_lossy(self.body.get(..200).unwrap_or(&self.body)).into_owned());
        Error::Api { status: self.status, message }
    }
}

/// Default limit for JSON responses.
pub const JSON_LIMIT: u64 = 4 << 20;

pub trait Transport: Send + Sync {
    fn send(&self, req: Request) -> Result<Response>;
}

/// `ureq` with rustls and the platform's certificate verifier; HTTP error statuses are returned
/// as responses so [`Response::api_error`] can read Google's message.
#[cfg(not(target_arch = "wasm32"))]
pub struct Ureq {
    agent: ureq::Agent,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for Ureq {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // Limits on a stalled connection, not on the whole call: a large document on a slow
            // link takes as long as it takes while bytes keep moving.
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_per_read(Some(std::time::Duration::from_secs(60)))
            .timeout_per_write(Some(std::time::Duration::from_secs(60)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(120)))
            .user_agent(concat!("WordCraft/", env!("CARGO_PKG_VERSION")))
            .build();
        Self { agent: config.into() }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Transport for Ureq {
    fn send(&self, req: Request) -> Result<Response> {
        let mut b = ureq::http::Request::builder().method(req.method.as_str()).uri(&req.url);
        for (k, v) in &req.headers {
            b = b.header(k.as_str(), v.as_str());
        }
        let r = b.body(req.body).map_err(|e| Error::Invalid(format!("bad request: {e}")))?;
        let mut resp = self.agent.run(r).map_err(|e| Error::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(req.limit).read_to_vec().map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(n) => Error::Protocol(format!("response larger than {n} bytes")),
            e => Error::Network(e.to_string()),
        })?;
        Ok(Response { status, body })
    }
}

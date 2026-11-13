//! The cabinet commit callback (M87, `INTEGRATION.md` §5.2).
//!
//! The room builds the final `.docx` and posts it to the cabinet, which owns
//! the file version, the audit and the antivirus scan. The file never travels
//! through a browser, so a closed tab cannot lose a save. The body is the
//! cabinet contract:
//!
//! ```json
//! { "fileId", "name"?, "bytes": "<base64>", "sha256", "version",
//!   "expectedVersion"? }
//! ```
//!
//! `version` is the room's document version (the cabinet records it in its
//! error text only); `expectedVersion` is the cabinet etag the previous commit
//! returned, so a file changed outside the session is refused with `409`
//! instead of overwritten. The cabinet answers `{ "version", "etag" }`, where
//! `etag` is what the next commit must send as `expectedVersion`.
//!
//! The client is deliberately plain: `http://` only, `Connection: close`, one
//! request per attempt. The server is deployed beside the cabinet behind the
//! ingress (RFC §9), so TLS is the proxy's job; an `https://` endpoint is
//! refused with a message that says so rather than failing the handshake.
//! Neither the token nor the document ever reaches the logs.

use std::fmt;
use std::time::Duration;

use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Deadline for one request (connect, write and the whole answer).
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
/// Attempts for one commit: a lost answer or a busy cabinet is retried with the
/// same body; a `409`/`401`/`413` is not retried at all.
pub const DEFAULT_ATTEMPTS: u32 = 3;
/// A response head bigger than this is not the cabinet talking.
const MAX_HEAD: usize = 64 * 1024;
/// The answer is a small JSON object; anything larger is drained, not buffered.
const MAX_ANSWER: usize = 1 << 20;

/// Where and how the cabinet takes a committed file.
#[derive(Clone)]
pub struct CabinetConfig {
    /// The commit endpoint. `{sessionId}` and `{fileId}` are substituted.
    pub url: String,
    /// The service token the callback is authenticated with. Never logged.
    pub token: String,
    /// Deadline for one request.
    pub timeout: Duration,
    /// Attempts before a network failure is reported.
    pub attempts: u32,
}

impl fmt::Debug for CabinetConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CabinetConfig")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .field("timeout", &self.timeout)
            .field("attempts", &self.attempts)
            .finish()
    }
}

/// The cabinet's copy of an accepted commit.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Published {
    /// The cabinet's file version.
    pub version: u64,
    /// What the next commit must send as `expectedVersion`.
    pub etag: String,
}

/// Why the cabinet did not take the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishError {
    /// `409`: the cabinet holds another version (edited outside the session).
    /// The room must not overwrite it; clients resync.
    Conflict(String),
    /// `401`/`403`: the service token is wrong. Retrying cannot help.
    Unauthorized(String),
    /// `413`: the cabinet refused the body size.
    TooLarge(String),
    /// The network or the cabinet failed and every attempt is spent: the file
    /// is built but is not a cabinet version yet. The next commit retries with
    /// the same `expectedVersion`.
    Unavailable(String),
    /// Any other answer; retrying would get it again.
    Refused(String),
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PublishError::Conflict(message)
            | PublishError::Unauthorized(message)
            | PublishError::TooLarge(message)
            | PublishError::Unavailable(message)
            | PublishError::Refused(message) => f.write_str(message),
        }
    }
}

/// What one HTTP status means for the commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Accepted,
    Conflict,
    Unauthorized,
    TooLarge,
    /// Another attempt with the same body may succeed.
    Retryable,
    Refused,
}

/// Classify the cabinet's status. The three named codes are the contract's
/// (`200 {version, etag}`, `409`, `401`, `413`); a proxy's `408`/`429` and the
/// `5xx` family are transient, everything else is a refusal.
pub fn classify(status: u16) -> Answer {
    match status {
        200..=299 => Answer::Accepted,
        401 | 403 => Answer::Unauthorized,
        409 => Answer::Conflict,
        413 => Answer::TooLarge,
        408 | 425 | 429 | 500..=599 => Answer::Retryable,
        _ => Answer::Refused,
    }
}

/// One file to publish.
pub struct Publication<'a> {
    /// Cabinet session the room belongs to (the callback path).
    pub session_id: &'a str,
    /// Cabinet file id: the JWT `doc` claim.
    pub file_id: &'a str,
    /// The built `.docx`.
    pub bytes: &'a [u8],
    /// Hex SHA-256 of `bytes`.
    pub sha256: &'a str,
    /// The room's document version.
    pub version: u64,
    /// The etag the cabinet returned for the previous commit, if any.
    pub expected: Option<&'a str>,
}

/// The cabinet callback client. One per server process; a slow cabinet blocks
/// one commit, not the room.
#[derive(Clone)]
pub struct Cabinet {
    endpoint: String,
    token: String,
    timeout: Duration,
    attempts: u32,
}

impl Cabinet {
    pub fn new(config: &CabinetConfig) -> Cabinet {
        Cabinet { endpoint: config.url.clone(), token: config.token.clone(), timeout: config.timeout, attempts: config.attempts.max(1) }
    }

    /// The endpoint for one room, with `{sessionId}`/`{fileId}` substituted.
    pub fn endpoint_for(&self, session_id: &str, file_id: &str) -> String {
        self.endpoint.replace("{sessionId}", session_id).replace("{fileId}", file_id)
    }

    /// The request body of the contract. Public so a test can compare the
    /// retried attempts byte for byte (the retry must be idempotent).
    pub fn body(publication: &Publication<'_>) -> String {
        let mut body = serde_json::Map::new();
        body.insert("fileId".into(), publication.file_id.into());
        body.insert("bytes".into(), base64::engine::general_purpose::STANDARD.encode(publication.bytes).into());
        body.insert("sha256".into(), publication.sha256.into());
        body.insert("version".into(), publication.version.into());
        if let Some(expected) = publication.expected {
            body.insert("expectedVersion".into(), expected.into());
        }
        serde_json::Value::Object(body).to_string()
    }

    /// Post one committed file, retrying a transient failure with the same body
    /// and the same `expectedVersion`.
    pub async fn publish(&self, publication: &Publication<'_>) -> Result<Published, PublishError> {
        let url = self.endpoint_for(publication.session_id, publication.file_id);
        let body = Cabinet::body(publication);
        let target = Target::parse(&url).map_err(PublishError::Refused)?;
        let mut last = String::new();
        for attempt in 0..self.attempts {
            if attempt > 0 {
                // 200 ms, then 800 ms: short enough for a commit request,
                // long enough for a proxy to come back.
                tokio::time::sleep(Duration::from_millis(200u64 << (attempt - 1))).await;
            }
            match self.post(&target, body.as_bytes()).await {
                Ok((status, answer)) => match classify(status) {
                    Answer::Accepted => return accepted(&answer),
                    Answer::Conflict => {
                        return Err(PublishError::Conflict("the cabinet holds another version of the file".into()));
                    }
                    Answer::Unauthorized => {
                        return Err(PublishError::Unauthorized("the cabinet refused the callback token".into()));
                    }
                    Answer::TooLarge => {
                        return Err(PublishError::TooLarge("the cabinet refused the committed file: too large".into()));
                    }
                    Answer::Retryable => last = format!("the cabinet answered HTTP {status}"),
                    Answer::Refused => return Err(PublishError::Refused(format!("the cabinet answered HTTP {status}"))),
                },
                Err(message) => last = message,
            }
        }
        Err(PublishError::Unavailable(last))
    }

    async fn post(&self, target: &Target, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
        let mut stream = tokio::time::timeout(self.timeout, TcpStream::connect(&target.authority))
            .await
            .map_err(|_| "the cabinet callback timed out connecting".to_string())?
            .map_err(|err| format!("the cabinet callback could not connect: {err}"))?;
        let head = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            target.path,
            target.host_header,
            self.token,
            body.len()
        );
        let sent = async {
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(body).await?;
            stream.flush().await
        };
        tokio::time::timeout(self.timeout, sent)
            .await
            .map_err(|_| "the cabinet callback timed out sending".to_string())?
            .map_err(|err| format!("the cabinet callback could not be sent: {err}"))?;

        let mut response = Vec::new();
        let read = async {
            let mut chunk = [0u8; 4096];
            loop {
                match stream.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => {
                        response.extend_from_slice(&chunk[..n]);
                        if response.len() > MAX_ANSWER {
                            // The answer is a small JSON object; keep draining
                            // and bound what is kept.
                            break;
                        }
                    }
                    Err(err) => {
                        // A peer that closes without `close_notify` is common
                        // for `Connection: close`; the bytes that arrived are
                        // still the answer.
                        if response.is_empty() {
                            return Err(err);
                        }
                        break;
                    }
                }
            }
            Ok::<(), std::io::Error>(())
        };
        tokio::time::timeout(self.timeout, read)
            .await
            .map_err(|_| "the cabinet callback timed out waiting for an answer".to_string())?
            .map_err(|err| format!("the cabinet callback got no answer: {err}"))?;
        split_response(&response)
    }
}

/// The cabinet's answer of a `2xx`. The contract names both fields; a body
/// without them is not a stored version the room can compare against.
fn accepted(answer: &[u8]) -> Result<Published, PublishError> {
    let value: serde_json::Value =
        serde_json::from_slice(answer).map_err(|_| PublishError::Refused("the cabinet answered 2xx without a JSON body".into()))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| PublishError::Refused("the cabinet answered 2xx without a version".into()))?;
    let etag = value
        .get("etag")
        .and_then(serde_json::Value::as_str)
        .filter(|etag| !etag.is_empty())
        .ok_or_else(|| PublishError::Refused("the cabinet answered 2xx without an etag".into()))?
        .to_string();
    Ok(Published { version, etag })
}

/// Status line and body of one HTTP answer (`Content-Length` or chunked).
fn split_response(response: &[u8]) -> Result<(u16, Vec<u8>), String> {
    if response.len() > MAX_HEAD + MAX_ANSWER {
        return Err("the cabinet answer is too large".into());
    }
    let split = response.windows(4).position(|window| window == b"\r\n\r\n").map(|at| at + 4).ok_or("the cabinet sent no HTTP headers")?;
    let head = String::from_utf8_lossy(&response[..split]);
    let status = head.split_whitespace().nth(1).and_then(|status| status.parse::<u16>().ok()).ok_or("the cabinet sent no status line")?;
    let chunked = head.lines().any(|line| {
        let mut parts = line.splitn(2, ':');
        parts.next().is_some_and(|name| name.eq_ignore_ascii_case("transfer-encoding"))
            && parts.next().is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    });
    let body = &response[split..];
    if chunked { Ok((status, decode_chunked(body)?)) } else { Ok((status, body.to_vec())) }
}

/// Minimal chunked transfer decoding for the callback's JSON answer.
fn decode_chunked(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|window| window == b"\r\n").ok_or("a chunk has no size line")?;
        let size = std::str::from_utf8(&body[..line_end])
            .ok()
            .and_then(|line| line.split(';').next())
            .and_then(|size| usize::from_str_radix(size.trim(), 16).ok())
            .ok_or("a chunk size is not hex")?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size {
            return Err("a chunk is shorter than its size".into());
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size..];
        if let Some(rest) = body.strip_prefix(b"\r\n") {
            body = rest;
        }
    }
}

#[derive(Clone, Debug)]
struct Target {
    /// `host:port` for the connection.
    authority: String,
    /// The `Host` header (the port is omitted for 80).
    host_header: String,
    path: String,
}

impl Target {
    /// Only `http://`: TLS is terminated by the ingress (deployment note in
    /// `production.md`). Credentials in the URL are refused.
    fn parse(url: &str) -> Result<Target, String> {
        let rest = url
            .strip_prefix("http://")
            .ok_or("the cabinet callback URL must start with http:// (TLS is terminated by the ingress; an https:// endpoint is not supported)")?;
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty() {
            return Err("the cabinet callback URL names no host".into());
        }
        if authority.contains('@') {
            return Err("the cabinet callback URL takes no credentials; the service token goes in the Authorization header".into());
        }
        let (host, port) = split_host_port(authority)?;
        let port = port.unwrap_or(80);
        let authority = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
        let host_header = if port == 80 { if host.contains(':') { format!("[{host}]") } else { host.clone() } } else { authority.clone() };
        Ok(Target { authority, host_header, path })
    }
}

fn split_host_port(authority: &str) -> Result<(String, Option<u16>), String> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, rest) = rest.split_once(']').ok_or("the cabinet callback URL has an unclosed IPv6 address")?;
        let port = match rest.strip_prefix(':') {
            Some(port) => Some(port.parse::<u16>().map_err(|_| "the cabinet callback port is not a number".to_string())?),
            None if rest.is_empty() => None,
            None => return Err("the cabinet callback URL has trailing text after the IPv6 address".into()),
        };
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if !port.is_empty() => {
                (host, Some(port.parse::<u16>().map_err(|_| "the cabinet callback port is not a number".to_string())?))
            }
            _ => (authority, None),
        }
    };
    if host.is_empty() {
        return Err("the cabinet callback URL names no host".into());
    }
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::TcpListener;

    /// A cabinet on loopback that answers each accepted connection with the
    /// next reply and keeps every request it read.
    async fn fake_cabinet(replies: Vec<Option<&'static str>>) -> (String, tokio::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let mut request = Vec::new();
                let mut chunk = [0u8; 4096];
                while let Ok(n) = stream.read(&mut chunk).await {
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(split) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..split]).to_ascii_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:").map(|value| value.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if request.len() >= split + 4 + length {
                            break;
                        }
                    }
                }
                let _ = sender.send(String::from_utf8_lossy(&request).to_string()).await;
                if let Some(reply) = reply {
                    let _ = stream.write_all(reply.as_bytes()).await;
                }
                // `None` closes without an answer: a lost response.
            }
        });
        (format!("http://{address}/api/v1/internal/office/sessions/{{sessionId}}/commit"), receiver)
    }

    fn cabinet(url: &str, attempts: u32) -> Cabinet {
        Cabinet::new(&CabinetConfig { url: url.to_string(), token: "service-token".into(), timeout: Duration::from_secs(5), attempts })
    }

    fn publication<'a>(bytes: &'a [u8]) -> Publication<'a> {
        Publication { session_id: "session-1", file_id: "42", bytes, sha256: "a2e1", version: 7, expected: Some("1794186677") }
    }

    #[test]
    fn the_contract_codes_are_classified() {
        assert_eq!(classify(200), Answer::Accepted);
        assert_eq!(classify(201), Answer::Accepted);
        assert_eq!(classify(401), Answer::Unauthorized);
        assert_eq!(classify(403), Answer::Unauthorized);
        assert_eq!(classify(409), Answer::Conflict);
        assert_eq!(classify(413), Answer::TooLarge);
        assert_eq!(classify(429), Answer::Retryable);
        assert_eq!(classify(500), Answer::Retryable);
        assert_eq!(classify(503), Answer::Retryable);
        assert_eq!(classify(400), Answer::Refused);
        assert_eq!(classify(422), Answer::Refused);
        assert_eq!(classify(418), Answer::Refused);
    }

    #[test]
    fn only_plain_http_urls_with_a_host_parse() {
        let target = Target::parse("http://cabinet.example/api/commit").expect("valid");
        assert_eq!(target.host_header, "cabinet.example");
        assert_eq!(target.path, "/api/commit");
        assert_eq!(target.authority, "cabinet.example:80");
        assert_eq!(Target::parse("http://127.0.0.1:8788/commit").expect("port").authority, "127.0.0.1:8788");
        assert_eq!(Target::parse("http://[::1]:9/x").expect("IPv6").host_header, "[::1]:9");
        assert_eq!(Target::parse("http://example.test").expect("no path").path, "/");
        assert!(Target::parse("https://mail.example/commit").is_err(), "TLS is the ingress's job");
        assert!(Target::parse("http://user:pass@example.test/").is_err());
        assert!(Target::parse("http:///nohost").is_err());
    }

    #[test]
    fn the_body_carries_the_contract_fields() {
        let body = Cabinet::body(&publication(b"PK\x03\x04docx"));
        assert!(body.contains(r#""fileId":"42""#), "{body}");
        assert!(body.contains(r#""sha256":"a2e1""#), "{body}");
        assert!(body.contains(r#""version":7"#), "{body}");
        assert!(body.contains(r#""expectedVersion":"1794186677""#), "{body}");
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"PK\x03\x04docx");
        assert!(body.contains(&format!(r#""bytes":"{encoded}""#)), "{body}");
        let first = Cabinet::body(&Publication { session_id: "session-1", file_id: "42", bytes: b"PK", sha256: "a2e1", version: 1, expected: None });
        assert!(!first.contains("expectedVersion"), "the first commit expects nothing: {first}");
    }

    #[test]
    fn the_config_never_prints_the_token() {
        let config = CabinetConfig { url: "http://cabinet/api".into(), token: "super-secret".into(), timeout: Duration::from_secs(1), attempts: 1 };
        let debug = format!("{config:?}");
        assert!(!debug.contains("super-secret"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }

    #[tokio::test]
    async fn an_accepted_commit_names_the_cabinet_version_and_etag() {
        let body = r#"{"version":42,"etag":"1794186677"}"#;
        let reply =
            Box::leak(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_boxed_str());
        let (url, mut requests) = fake_cabinet(vec![Some(reply)]).await;
        let published = cabinet(&url, 1).publish(&publication(b"PK\x03\x04docx")).await.expect("published");
        assert_eq!(published, Published { version: 42, etag: "1794186677".into() });
        let request = requests.recv().await.expect("one request");
        assert!(request.starts_with("POST /api/v1/internal/office/sessions/session-1/commit HTTP/1.1"), "{request}");
        assert!(request.contains("Authorization: Bearer service-token"), "{request}");
        assert!(request.contains(r#""fileId":"42""#), "{request}");
        assert!(request.contains(r#""expectedVersion":"1794186677""#), "{request}");
    }

    #[tokio::test]
    async fn a_conflict_is_not_retried() {
        let (url, mut requests) = fake_cabinet(vec![Some("HTTP/1.1 409 Conflict\r\nContent-Length: 2\r\n\r\n{}")]).await;
        let error = cabinet(&url, 3).publish(&publication(b"PK")).await.expect_err("conflict");
        assert!(matches!(error, PublishError::Conflict(_)), "{error:?}");
        let _ = requests.recv().await.expect("one request");
        assert!(requests.try_recv().is_err(), "409 gets exactly one attempt");
    }

    #[tokio::test]
    async fn a_bad_token_is_not_retried() {
        let (url, mut requests) = fake_cabinet(vec![Some("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")]).await;
        let error = cabinet(&url, 3).publish(&publication(b"PK")).await.expect_err("unauthorized");
        assert!(matches!(error, PublishError::Unauthorized(_)), "{error:?}");
        let _ = requests.recv().await.expect("one request");
        assert!(requests.try_recv().is_err(), "401 gets exactly one attempt");
    }

    #[tokio::test]
    async fn a_too_large_body_is_not_retried() {
        let (url, mut requests) = fake_cabinet(vec![Some("HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")]).await;
        let error = cabinet(&url, 3).publish(&publication(b"PK")).await.expect_err("too large");
        assert!(matches!(error, PublishError::TooLarge(_)), "{error:?}");
        let _ = requests.recv().await.expect("one request");
        assert!(requests.try_recv().is_err(), "413 gets exactly one attempt");
    }

    #[tokio::test]
    async fn a_lost_answer_is_retried_with_the_same_body() {
        let body = r#"{"version":42,"etag":"1794186677"}"#;
        let reply = Box::leak(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_boxed_str());
        // The first connection is closed without an answer; the second succeeds.
        let (url, mut requests) = fake_cabinet(vec![None, Some(reply)]).await;
        let published = cabinet(&url, 3).publish(&publication(b"PK\x03\x04docx")).await.expect("the retry publishes");
        assert_eq!(published.etag, "1794186677");
        let first = requests.recv().await.expect("first attempt");
        let second = requests.recv().await.expect("second attempt");
        let body_of = |request: &str| request.split_once("\r\n\r\n").map(|(_, body)| body.to_string()).unwrap_or_default();
        assert_eq!(body_of(&first), body_of(&second), "a retry is the same body, so it is idempotent");
        assert!(second.contains("Authorization: Bearer service-token"), "{second}");
    }

    #[tokio::test]
    async fn an_unreachable_cabinet_gives_up_after_the_attempts() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        drop(listener);
        let url = format!("http://{address}/api/v1/internal/office/sessions/{{sessionId}}/commit");
        let error = cabinet(&url, 2).publish(&publication(b"PK")).await.expect_err("unreachable");
        assert!(matches!(error, PublishError::Unavailable(_)), "{error:?}");
    }

    #[tokio::test]
    async fn a_cabinet_that_refuses_the_body_is_reported_without_a_retry() {
        let (url, mut requests) = fake_cabinet(vec![Some("HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")]).await;
        let error = cabinet(&url, 3).publish(&publication(b"PK")).await.expect_err("refused");
        assert!(matches!(error, PublishError::Refused(_)), "{error:?}");
        let _ = requests.recv().await.expect("one request");
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_chunked_answer_is_decoded() {
        let (url, _requests) =
            fake_cabinet(vec![Some("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1a\r\n{\"version\":5,\"etag\":\"5\"}\r\n0\r\n\r\n")]).await;
        let published = cabinet(&url, 1).publish(&publication(b"PK")).await.expect("published");
        assert_eq!(published, Published { version: 5, etag: "5".into() });
    }

    #[tokio::test]
    async fn an_accepted_answer_without_the_contract_fields_is_refused() {
        let (url, _requests) = fake_cabinet(vec![Some("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")]).await;
        let error = cabinet(&url, 1).publish(&publication(b"PK")).await.expect_err("no etag");
        assert!(matches!(error, PublishError::Refused(_)), "{error:?}");
    }
}

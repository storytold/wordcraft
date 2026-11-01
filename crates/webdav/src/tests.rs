//! The client against a scripted HTTP server on 127.0.0.1: no network, no real WebDAV server.
#![cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::{Error, Lock, Resource};

/// One request as the server saw it.
#[derive(Debug, Default)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// A server that answers each connection with the next scripted response (`status line`,
/// headers, body) and records what it was sent. Every answer closes its connection, so each
/// request arrives on a connection of its own.
struct Server {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn start(responses: Vec<(&'static str, Vec<(&'static str, String)>, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/dav/session-secret/Contract.docx", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        let thread = std::thread::spawn(move || {
            for (status, headers, body) in responses {
                let Ok((stream, _)) = listener.accept() else { return };
                stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = Seen::default();
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                request.method = parts.next().unwrap_or_default().to_owned();
                request.path = parts.next().unwrap_or_default().to_owned();
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = trimmed.split_once(':') {
                        request.headers.push((name.trim().to_owned(), value.trim().to_owned()));
                    }
                }
                let length: usize = request.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
                request.body = vec![0; length];
                reader.read_exact(&mut request.body).unwrap();
                record.lock().unwrap().push(request);

                let mut stream = reader.into_inner();
                let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                for (name, value) in headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("\r\n");
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&body).unwrap();
                stream.flush().unwrap();
            }
        });
        Self { url, seen, thread: Some(thread) }
    }

    /// Waits for the script to run out and returns what the server was sent.
    fn finish(mut self) -> Vec<Seen> {
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
        std::mem::take(&mut *self.seen.lock().unwrap())
    }
}

fn ok(status: &'static str) -> (&'static str, Vec<(&'static str, String)>, Vec<u8>) {
    (status, Vec::new(), Vec::new())
}

#[test]
fn get_loads_the_document() {
    let server = Server::start(vec![("200 OK", Vec::new(), b"PK\x03\x04docx bytes".to_vec())]);
    let bytes = Resource::new(&server.url).unwrap().get().unwrap();
    assert_eq!(bytes, b"PK\x03\x04docx bytes");
    let seen = server.finish();
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/dav/session-secret/Contract.docx");
}

#[test]
fn get_refuses_a_document_over_the_size_cap() {
    let server = Server::start(vec![("200 OK", Vec::new(), vec![b'x'; 64])]);
    let err = Resource::new(&server.url).unwrap().with_max_bytes(16).get().unwrap_err();
    assert_eq!(err, Error::TooLarge { limit: 16 });
    server.finish();
}

#[test]
fn lock_asks_for_an_exclusive_write_lock_and_keeps_the_token() {
    let server = Server::start(vec![(
        "200 OK",
        vec![("Lock-Token", "<opaquelocktoken:e71d4fae-5dec-22d6>".to_owned()), ("Timeout", "Second-180".to_owned())],
        b"<?xml version=\"1.0\"?><D:prop xmlns:D=\"DAV:\"/>".to_vec(),
    )]);
    let lock = Resource::new(&server.url).unwrap().lock("Ana & Bo", Duration::from_secs(600)).unwrap();
    assert_eq!(lock.token(), "opaquelocktoken:e71d4fae-5dec-22d6");
    assert_eq!(lock.timeout(), Some(Duration::from_secs(180)));
    assert_eq!(lock.refresh_interval(), Duration::from_secs(90));

    let seen = server.finish();
    let request = &seen[0];
    assert_eq!(request.method, "LOCK");
    assert_eq!(request.header("Depth"), Some("0"));
    assert_eq!(request.header("Timeout"), Some("Second-600"));
    let body = String::from_utf8_lossy(&request.body);
    assert!(body.contains("<D:exclusive/>") && body.contains("<D:write/>"), "{body}");
    assert!(body.contains("<D:owner>Ana &amp; Bo</D:owner>"), "{body}");
}

#[test]
fn lock_keeps_the_requested_timeout_when_the_server_names_none() {
    let server = Server::start(vec![("201 Created", vec![("Lock-Token", "<urn:uuid:1>".to_owned())], Vec::new())]);
    let lock = Resource::new(&server.url).unwrap().lock("me", Duration::from_secs(120)).unwrap();
    assert_eq!(lock.timeout(), Some(Duration::from_secs(120)));
    server.finish();
}

#[test]
fn a_lock_without_a_token_is_an_error() {
    let server = Server::start(vec![ok("200 OK")]);
    let err = Resource::new(&server.url).unwrap().lock("me", Duration::from_secs(60)).unwrap_err();
    assert_eq!(err, Error::NoLockToken);
    server.finish();
}

#[test]
fn a_document_locked_by_someone_else_is_reported() {
    let server = Server::start(vec![ok("423 Locked")]);
    let err = Resource::new(&server.url).unwrap().lock("me", Duration::from_secs(60)).unwrap_err();
    assert_eq!(err, Error::Locked);
    server.finish();
}

#[test]
fn refresh_names_the_lock_in_an_if_header() {
    let server = Server::start(vec![("200 OK", vec![("Timeout", "Second-300".to_owned())], Vec::new())]);
    let held = Lock::new("opaquelocktoken:abc", Some(Duration::from_secs(60)));
    let fresh = Resource::new(&server.url).unwrap().refresh(&held, Duration::from_secs(600)).unwrap();
    assert_eq!(fresh.token(), "opaquelocktoken:abc");
    assert_eq!(fresh.timeout(), Some(Duration::from_secs(300)));
    let seen = server.finish();
    assert_eq!(seen[0].method, "LOCK");
    assert_eq!(seen[0].header("If"), Some("(<opaquelocktoken:abc>)"));
    assert!(seen[0].body.is_empty());
}

#[test]
fn put_saves_under_the_lock() {
    let server = Server::start(vec![ok("204 No Content")]);
    let held = Lock::new("opaquelocktoken:abc", None);
    let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    Resource::new(&server.url).unwrap().put(b"new bytes", docx, Some(&held)).unwrap();
    let seen = server.finish();
    assert_eq!(seen[0].method, "PUT");
    assert_eq!(seen[0].header("If"), Some("(<opaquelocktoken:abc>)"));
    assert_eq!(seen[0].header("Content-Type"), Some(docx));
    assert_eq!(seen[0].body, b"new bytes");
}

#[test]
fn a_save_after_the_lock_was_lost_is_refused() {
    let server = Server::start(vec![ok("412 Precondition Failed")]);
    let held = Lock::new("opaquelocktoken:gone", None);
    let err = Resource::new(&server.url).unwrap().put(b"x", "application/octet-stream", Some(&held)).unwrap_err();
    assert_eq!(err, Error::LockLost { method: "PUT" });
    server.finish();
}

#[test]
fn unlock_names_the_lock_token() {
    let server = Server::start(vec![ok("204 No Content")]);
    Resource::new(&server.url).unwrap().unlock(Lock::new("opaquelocktoken:abc", None)).unwrap();
    let seen = server.finish();
    assert_eq!(seen[0].method, "UNLOCK");
    assert_eq!(seen[0].header("Lock-Token"), Some("<opaquelocktoken:abc>"));
}

#[test]
fn a_whole_editing_session_runs_in_order() {
    let server = Server::start(vec![
        ("200 OK", Vec::new(), b"v1".to_vec()),
        ("200 OK", vec![("Lock-Token", "<opaquelocktoken:s>".to_owned()), ("Timeout", "Second-60".to_owned())], Vec::new()),
        ok("204 No Content"),
        ("200 OK", vec![("Timeout", "Second-60".to_owned())], Vec::new()),
        ok("204 No Content"),
    ]);
    let doc = Resource::new(&server.url).unwrap();
    assert_eq!(doc.get().unwrap(), b"v1");
    let lock = doc.lock("me", Duration::from_secs(60)).unwrap();
    doc.put(b"v2", "application/octet-stream", Some(&lock)).unwrap();
    let lock = doc.refresh(&lock, Duration::from_secs(60)).unwrap();
    doc.unlock(lock).unwrap();
    let methods: Vec<String> = server.finish().into_iter().map(|s| s.method).collect();
    assert_eq!(methods, ["GET", "LOCK", "PUT", "LOCK", "UNLOCK"]);
}

#[test]
fn errors_map_from_statuses() {
    let server = Server::start(vec![ok("401 Unauthorized"), ok("404 Not Found"), ok("500 Internal Server Error"), ok("302 Found")]);
    let doc = Resource::new(&server.url).unwrap();
    assert_eq!(doc.get().unwrap_err(), Error::Forbidden { method: "GET", status: 401 });
    assert_eq!(doc.get().unwrap_err(), Error::NotFound { status: 404 });
    assert_eq!(doc.get().unwrap_err(), Error::Status { method: "GET", status: 500 });
    // A redirect is never followed: the lock and the save must go to the same URL.
    assert!(doc.get().is_err());
    server.finish();
}

#[test]
fn only_http_urls_with_a_host_are_accepted() {
    for bad in ["", "Contract.docx", "/dav/Contract.docx", "ftp://example.com/a.docx", "file:///Users/a.docx", "http://", "mailto:a@b.c"] {
        assert!(matches!(Resource::new(bad), Err(Error::InvalidUrl(_))), "{bad:?}");
    }
    assert!(Resource::new("https://dms.example/dav/abc/Contract.docx").is_ok());
    assert!(Resource::new("HTTP://dms.example:8080/x.docx").is_ok());
}

#[test]
fn messages_never_show_the_session_credential() {
    let doc = Resource::new("https://user:pw@dms.example:8443/dav/session-secret/Contract.docx").unwrap();
    let shown = doc.display_url();
    assert_eq!(shown, "https://dms.example:8443/…");
    let Err(Error::InvalidUrl(shown)) = Resource::new("ftp://dms.example/session-secret/x") else { panic!("not refused") };
    assert!(!shown.contains("session-secret"), "{shown}");
}

#[test]
fn an_unreachable_server_is_a_transport_error() {
    // Bind and drop a listener so the port is very likely free and refuses the connection.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let err = Resource::new(&format!("http://127.0.0.1:{port}/x.docx")).unwrap().get().unwrap_err();
    assert!(matches!(err, Error::Transport(_)), "{err:?}");
}

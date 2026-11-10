use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proptest::prelude::*;
use serde_json::json;

use crate::Error;
use crate::drive::{self, Drive, DriveFile, SaveTarget};
use crate::encode;
use crate::http::{Method, Request, Response, Transport};
use crate::oauth::{self, Auth, ClientConfig};
use crate::pkce::{self, Pkce};
use crate::store::{Memory, TokenStore};

/// Answers scripted responses in order and records every request.
#[derive(Default)]
struct Fake {
    replies: Mutex<VecDeque<Response>>,
    seen: Mutex<Vec<Request>>,
}

impl Fake {
    fn with(replies: Vec<(u16, serde_json::Value)>) -> Arc<Self> {
        let replies = replies.into_iter().map(|(status, v)| Response { status, body: v.to_string().into_bytes() }).collect();
        Arc::new(Self { replies: Mutex::new(replies), seen: Mutex::default() })
    }
    fn push_raw(&self, status: u16, body: &[u8]) {
        self.replies.lock().unwrap().push_back(Response { status, body: body.to_vec() });
    }
    fn seen(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
}

impl Transport for Fake {
    fn send(&self, req: Request) -> crate::Result<Response> {
        self.seen.lock().unwrap().push(req);
        self.replies.lock().unwrap().pop_front().ok_or_else(|| Error::Network("no scripted reply".into()))
    }
}

fn cfg() -> ClientConfig {
    ClientConfig::new("123-abc.apps.googleusercontent.com", Some("not-really-secret"), None).unwrap()
}

fn signed_in(fake: &Arc<Fake>) -> Auth {
    let store = Memory::default();
    store.save("refresh-1").unwrap();
    let mut auth = Auth::new(cfg(), fake.clone(), Box::new(store));
    assert!(auth.restore().unwrap());
    auth
}

fn token_reply() -> (u16, serde_json::Value) {
    (200, json!({"access_token": "access-1", "expires_in": 3599, "token_type": "Bearer"}))
}

fn file_json(id: &str, name: &str, mime: &str, version: &str) -> serde_json::Value {
    json!({"id": id, "name": name, "mimeType": mime, "modifiedTime": "2026-11-09T10:00:00.000Z", "version": version, "size": "1234"})
}

fn header<'a>(r: &'a Request, k: &str) -> Option<&'a str> {
    r.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
}

// ---- PKCE and encoding

#[test]
fn pkce_matches_rfc7636_appendix_b() {
    let p = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
    assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
}

#[test]
fn generated_verifiers_are_43_unreserved_chars_and_differ() {
    let a = Pkce::generate().unwrap();
    let b = Pkce::generate().unwrap();
    assert_eq!(a.verifier.len(), 43);
    assert!(a.verifier.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
    assert_ne!(a.verifier, b.verifier);
    assert!(!format!("{a:?}").contains(&a.verifier), "the verifier must not appear in Debug output");
}

#[test]
fn percent_encoding_round_trips() {
    assert_eq!(encode::pct("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    assert_eq!(encode::unpct("a%20b+c%2"), "a b c%2");
    assert_eq!(encode::drive_literal(r"it's a\b"), r"'it\'s a\\b'");
}

proptest! {
    #[test]
    fn pct_then_unpct_is_identity(s in ".*") {
        prop_assert_eq!(encode::unpct(&encode::pct(&s)), s);
    }

    #[test]
    fn hostile_callbacks_never_panic(target in ".{0,300}", state in "[a-z]{0,10}") {
        let _ = oauth::parse_callback(&target, &state);
        let _ = crate::loopback::request_target(target.as_bytes());
        let _ = encode::parse_query(&target, 32);
    }

    #[test]
    fn hostile_ids_never_reach_a_url(id in ".{0,300}") {
        if let Ok(ok) = drive::check_id(&id) {
            prop_assert!(ok.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        }
    }
}

// ---- configuration

#[test]
fn config_accepts_our_file_and_googles_download() {
    let ours = ClientConfig::from_json(br#"{"client_id":"x.apps.googleusercontent.com","scope":"file"}"#).unwrap();
    assert_eq!(ours.scope, oauth::SCOPE_DRIVE_FILE);
    assert_eq!(ours.client_secret, None);
    let google = ClientConfig::from_json(br#"{"installed":{"client_id":"y.apps.googleusercontent.com","client_secret":"s"}}"#).unwrap();
    assert_eq!(google.client_id, "y.apps.googleusercontent.com");
    assert_eq!(google.scope, oauth::SCOPE_DRIVE);
    assert!(!format!("{google:?}").contains("\"s\""), "the secret must not appear in Debug output");
}

#[test]
fn config_rejects_junk() {
    for bad in [&b"not json"[..], br#"{}"#, br#"{"client_id":""}"#, br#"{"client_id":"a b"}"#, br#"{"client_id":"a","scope":"all"}"#] {
        assert!(matches!(ClientConfig::from_json(bad), Err(Error::NotConfigured(_))), "{}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn missing_config_file_is_not_configured() {
    let dir = std::env::temp_dir().join(format!("wordcraft-gdrive-none-{}", std::process::id()));
    if std::env::var(oauth::ENV_CLIENT_ID).is_err() {
        assert!(matches!(ClientConfig::load(Some(&dir)), Err(Error::NotConfigured(_))));
    }
}

// ---- consent URL and redirect

#[test]
fn auth_url_carries_pkce_state_and_offline_access() {
    let url = oauth::auth_url(&cfg(), "http://127.0.0.1:5555", "CHAL", "STATE");
    assert!(url.starts_with(oauth::AUTH_ENDPOINT));
    let q = encode::parse_query(url.split_once('?').unwrap().1, 32);
    let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    assert_eq!(get("redirect_uri"), Some("http://127.0.0.1:5555"));
    assert_eq!(get("code_challenge"), Some("CHAL"));
    assert_eq!(get("code_challenge_method"), Some("S256"));
    assert_eq!(get("state"), Some("STATE"));
    assert_eq!(get("access_type"), Some("offline"));
    assert_eq!(get("scope"), Some(oauth::SCOPE_DRIVE));
    assert!(!url.contains("not-really-secret"), "the client secret never goes to the browser");
}

#[test]
fn callback_checks_state_and_reports_denial() {
    assert_eq!(oauth::parse_callback("/?state=s1&code=4%2Fabc", "s1").unwrap(), Some("4/abc".into()));
    assert_eq!(oauth::parse_callback("/favicon.ico", "s1").unwrap(), None);
    assert!(matches!(oauth::parse_callback("/?state=other&code=x", "s1"), Err(Error::SignIn(_))));
    assert!(matches!(oauth::parse_callback("/?code=x", "s1"), Err(Error::SignIn(_))));
    assert!(matches!(oauth::parse_callback("/?error=access_denied&state=s1", "s1"), Err(Error::SignIn(m)) if m.contains("denied")));
    assert!(matches!(oauth::parse_callback("/?state=s1", "s1"), Err(Error::SignIn(_))));
}

#[test]
fn request_line_parsing() {
    assert_eq!(crate::loopback::request_target(b"GET /?code=1 HTTP/1.1\r\nHost: x\r\n\r\n").as_deref(), Some("/?code=1"));
    assert_eq!(crate::loopback::request_target(b"POST / HTTP/1.1\r\n"), None);
    assert_eq!(crate::loopback::request_target(b"\xff\xfe"), None);
}

#[test]
fn loopback_receives_the_redirect() {
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicBool;
    let listener = crate::loopback::Listener::bind().unwrap();
    let uri = listener.redirect_uri();
    let addr = uri.trim_start_matches("http://").to_owned();
    let browser = std::thread::spawn(move || {
        // A favicon request first, as browsers do, then the redirect.
        for target in ["/favicon.ico", "/?state=st&code=the-code"] {
            let mut s = std::net::TcpStream::connect(&addr).unwrap();
            write!(s, "GET {target} HTTP/1.1\r\nHost: {addr}\r\n\r\n").unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
        }
    });
    let cancel = AtomicBool::new(false);
    assert_eq!(listener.wait("st", &cancel, Duration::from_secs(10)).unwrap(), "the-code");
    browser.join().unwrap();
}

#[test]
fn loopback_honours_cancel() {
    use std::sync::atomic::AtomicBool;
    let listener = crate::loopback::Listener::bind().unwrap();
    let cancel = AtomicBool::new(true);
    assert!(matches!(listener.wait("st", &cancel, Duration::from_secs(10)), Err(Error::SignIn(m)) if m == "cancelled"));
}

// ---- tokens

#[test]
fn exchange_sends_verifier_and_stores_refresh_token() {
    let fake = Fake::with(vec![(200, json!({"access_token": "a", "refresh_token": "r", "expires_in": 3599}))]);
    let store = Arc::new(Memory::default());
    struct Shared(Arc<Memory>);
    impl TokenStore for Shared {
        fn load(&self) -> crate::Result<Option<String>> {
            self.0.load()
        }
        fn save(&self, t: &str) -> crate::Result<()> {
            self.0.save(t)
        }
        fn clear(&self) -> crate::Result<()> {
            self.0.clear()
        }
    }
    let mut auth = Auth::new(cfg(), fake.clone(), Box::new(Shared(store.clone())));
    auth.exchange("CODE", "VERIFIER", "http://127.0.0.1:1", Instant::now()).unwrap();
    assert_eq!(store.load().unwrap().as_deref(), Some("r"));
    let seen = fake.seen();
    let body = String::from_utf8(seen[0].body.clone()).unwrap();
    assert_eq!(seen[0].url, oauth::TOKEN_ENDPOINT);
    for part in ["grant_type=authorization_code", "code=CODE", "code_verifier=VERIFIER", "client_secret=not-really-secret"] {
        assert!(body.contains(part), "{part} missing from {body}");
    }
    assert!(!format!("{:?}", seen[0]).contains("VERIFIER"), "request Debug output leaks the body");
}

#[test]
fn access_token_refreshes_only_when_expiring() {
    let fake = Fake::with(vec![token_reply()]);
    let mut auth = signed_in(&fake);
    let now = Instant::now();
    assert_eq!(auth.access_token(now).unwrap(), "access-1");
    assert_eq!(auth.access_token(now + Duration::from_secs(60)).unwrap(), "access-1", "no second refresh");
    assert_eq!(fake.seen().len(), 1);
    let body = String::from_utf8(fake.seen()[0].body.clone()).unwrap();
    assert!(body.contains("grant_type=refresh_token") && body.contains("refresh_token=refresh-1"));
}

#[test]
fn revoked_refresh_token_signs_out() {
    let fake = Fake::with(vec![(400, json!({"error": "invalid_grant", "error_description": "Token has been expired or revoked."}))]);
    let mut auth = signed_in(&fake);
    assert_eq!(auth.access_token(Instant::now()), Err(Error::NotSignedIn));
    assert!(!auth.signed_in());
    assert!(!auth.restore().unwrap(), "the stored token was removed");
}

#[test]
fn network_failure_during_refresh_keeps_the_account() {
    let fake = Fake::with(vec![]);
    let mut auth = signed_in(&fake);
    assert!(matches!(auth.access_token(Instant::now()), Err(Error::Network(_))));
    assert!(auth.signed_in(), "a flaky network must not sign the person out");
}

#[test]
fn sign_out_revokes_and_forgets() {
    let fake = Fake::with(vec![(200, json!({}))]);
    let mut auth = signed_in(&fake);
    auth.sign_out().unwrap();
    assert!(!auth.signed_in());
    assert_eq!(fake.seen()[0].url, oauth::REVOKE_ENDPOINT);
    assert!(!auth.restore().unwrap());
}

#[test]
fn malformed_token_responses_are_errors() {
    let now = Instant::now();
    assert!(oauth::parse_tokens(&json!({}), now).is_err());
    assert!(oauth::parse_tokens(&json!({"access_token": ""}), now).is_err());
    let t = oauth::parse_tokens(&json!({"access_token": "a", "expires_in": u64::MAX}), now).unwrap();
    assert!(t.expires_at <= now + Duration::from_secs(86_400));
}

// ---- Drive calls

#[test]
fn list_filters_documents_and_skips_bad_ids() {
    let fake = Fake::with(vec![
        token_reply(),
        (
            200,
            json!({"nextPageToken": "p2", "files": [
                file_json("abc_1-2", "Essay.docx", drive::MIME_DOCX, "7"),
                file_json("../etc", "Evil.docx", drive::MIME_DOCX, "1"),
                {"id": "gdoc1", "name": "Notes", "mimeType": drive::MIME_GOOGLE_DOC, "version": "3"},
            ]}),
        ),
    ]);
    let mut d = Drive::new(signed_in(&fake));
    let (files, next) = d.list(Some("Ess'ay"), None).unwrap();
    assert_eq!(next.as_deref(), Some("p2"));
    assert_eq!(files.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["abc_1-2", "gdoc1"]);
    assert_eq!(files[0].size, Some(1234));
    assert_eq!(files[1].size, None);
    assert_eq!(files[1].open_name(), "Notes.docx");
    let req = &fake.seen()[1];
    assert_eq!(header(req, "Authorization"), Some("Bearer access-1"));
    let q = encode::parse_query(req.url.split_once('?').unwrap().1, 32);
    let query = &q.iter().find(|(k, _)| k == "q").unwrap().1;
    assert!(query.contains("trashed = false"));
    assert!(query.contains(r"name contains 'Ess\'ay'"), "{query}");
}

#[test]
fn google_docs_are_exported_as_docx() {
    let fake = Fake::with(vec![token_reply()]);
    fake.push_raw(200, b"PK\x03\x04docx");
    let mut d = Drive::new(signed_in(&fake));
    let f = DriveFile {
        id: "g1".into(),
        name: "Notes".into(),
        mime_type: drive::MIME_GOOGLE_DOC.into(),
        modified_time: String::new(),
        version: "1".into(),
        size: None,
    };
    assert_eq!(d.download(&f).unwrap(), b"PK\x03\x04docx");
    let req = &fake.seen()[1];
    assert!(req.url.contains("/files/g1/export?mimeType="), "{}", req.url);
    assert_eq!(req.limit, drive::MAX_DOCUMENT);
}

#[test]
fn save_refuses_when_drive_changed_since_open() {
    let fake = Fake::with(vec![token_reply(), (200, file_json("f1", "Essay.docx", drive::MIME_DOCX, "9"))]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "f1".into(), name: "Essay.docx".into(), version: "7".into() };
    let err = d.save(&target, b"PK").unwrap_err();
    assert_eq!(err, Error::Conflict { name: "Essay.docx".into(), opened: "7".into(), remote: "9".into() });
    assert!(fake.seen().iter().all(|r| r.method != Method::Patch), "nothing was uploaded");
}

#[test]
fn save_updates_when_unchanged() {
    let fake = Fake::with(vec![
        token_reply(),
        (200, file_json("f1", "Essay.docx", drive::MIME_DOCX, "7")),
        (200, file_json("f1", "Essay.docx", drive::MIME_DOCX, "8")),
    ]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "f1".into(), name: "Essay.docx".into(), version: "7".into() };
    assert_eq!(d.save(&target, b"PKdata").unwrap().version, "8");
    let up = &fake.seen()[2];
    assert_eq!(up.method, Method::Patch);
    assert!(up.url.starts_with(&format!("{}/files/f1?uploadType=media", drive::UPLOAD_API)));
    assert_eq!(header(up, "Content-Type"), Some(drive::MIME_DOCX));
    assert_eq!(up.body, b"PKdata");
}

#[test]
fn save_never_overwrites_a_google_doc() {
    let fake = Fake::with(vec![token_reply(), (200, json!({"id": "g1", "name": "Notes", "mimeType": drive::MIME_GOOGLE_DOC, "version": "3"}))]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::Existing { id: "g1".into(), name: "Notes".into(), version: "3".into() };
    assert!(matches!(d.save(&target, b"PK"), Err(Error::Invalid(m)) if m.contains("Google Doc")));
}

#[test]
fn save_new_sends_multipart_with_parent() {
    let fake = Fake::with(vec![token_reply(), (200, file_json("n1", "New.odt", drive::MIME_ODT, "1"))]);
    let mut d = Drive::new(signed_in(&fake));
    let target = SaveTarget::New { name: "New.odt".into(), parent: Some("folder_1".into()) };
    assert_eq!(d.save(&target, b"ODTBYTES").unwrap().id, "n1");
    let up = &fake.seen()[1];
    assert_eq!(up.method, Method::Post);
    assert!(up.url.contains("uploadType=multipart"));
    let ct = header(up, "Content-Type").unwrap();
    let boundary = ct.strip_prefix("multipart/related; boundary=").unwrap();
    let body = String::from_utf8_lossy(&up.body);
    assert!(body.starts_with(&format!("--{boundary}\r\n")));
    assert!(body.ends_with(&format!("--{boundary}--\r\n")));
    assert!(body.contains(r#""parents":["folder_1"]"#) && body.contains(drive::MIME_ODT) && body.contains("ODTBYTES"));
}

#[test]
fn save_rejects_bad_names_ids_and_huge_documents() {
    let fake = Fake::with(vec![]);
    let mut d = Drive::new(signed_in(&fake));
    for t in [
        SaveTarget::New { name: "  ".into(), parent: None },
        SaveTarget::New { name: "a\nb.docx".into(), parent: None },
        SaveTarget::New { name: "ok.docx".into(), parent: Some("x/y".into()) },
        SaveTarget::Existing { id: "a?b".into(), name: "x".into(), version: "1".into() },
    ] {
        assert!(matches!(d.save(&t, b"PK"), Err(Error::Invalid(_))), "{t:?}");
    }
    assert!(fake.seen().is_empty(), "nothing was sent for invalid input");
}

#[test]
fn api_errors_carry_googles_message() {
    let fake = Fake::with(vec![token_reply(), (404, json!({"error": {"code": 404, "message": "File not found: f1."}}))]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.metadata("f1"), Err(Error::Api { status: 404, message: "File not found: f1.".into() }));
}

#[test]
fn a_401_refreshes_the_access_token_and_retries_once() {
    let fake = Fake::with(vec![
        token_reply(),
        (401, json!({"error": {"code": 401, "message": "Invalid Credentials"}})),
        (200, json!({"access_token": "access-2", "expires_in": 3599, "token_type": "Bearer"})),
        (200, json!({"files": []})),
    ]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.list(None, None), Ok((vec![], None)));
    let seen = fake.seen();
    assert_eq!(seen.len(), 4);
    assert_eq!(header(&seen[3], "Authorization"), Some("Bearer access-2"), "retried with the refreshed token");
}

#[test]
fn a_401_then_a_refused_refresh_signs_out() {
    let fake = Fake::with(vec![
        token_reply(),
        (401, json!({"error": {"code": 401, "message": "Invalid Credentials"}})),
        (400, json!({"error": "invalid_grant", "error_description": "Token has been expired or revoked."})),
    ]);
    let mut d = Drive::new(signed_in(&fake));
    assert_eq!(d.list(None, None), Err(Error::NotSignedIn));
    assert_eq!(d.auth.access_token(Instant::now()), Err(Error::NotSignedIn), "signed out, nothing more sent");
    assert_eq!(fake.seen().len(), 3);
}

#[test]
fn not_signed_in_sends_nothing() {
    let fake = Fake::with(vec![]);
    let mut d = Drive::new(Auth::new(cfg(), fake.clone(), Box::new(Memory::default())));
    assert_eq!(d.list(None, None), Err(Error::NotSignedIn));
    assert!(fake.seen().is_empty());
}

#[test]
fn mime_for_names() {
    assert_eq!(drive::mime_for("a.DOCX"), drive::MIME_DOCX);
    assert_eq!(drive::mime_for("a.odt"), drive::MIME_ODT);
    assert_eq!(drive::mime_for("a"), drive::MIME_DOCX);
    let _ = pkce::random_token(0).unwrap();
}

#[test]
fn write_client_checks_the_id_keeps_the_scope_and_reads_back() {
    let dir = std::env::temp_dir().join(format!("wordcraft-gdrive-client-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(matches!(oauth::write_client(&dir, "  ", None), Err(Error::NotConfigured(_))), "an empty ID is refused");
    assert!(!dir.join(oauth::CONFIG_FILE).exists(), "nothing written for a bad ID");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(oauth::CONFIG_FILE), br#"{"client_id": "old", "scope": "file"}"#).unwrap();
    oauth::write_client(&dir, " 123-abc.apps.googleusercontent.com ", Some("s3cret")).unwrap();
    let cfg = ClientConfig::load(Some(&dir)).unwrap();
    assert_eq!(cfg.client_id, "123-abc.apps.googleusercontent.com");
    assert_eq!(cfg.client_secret.as_deref(), Some("s3cret"));
    assert_eq!(cfg.scope, oauth::SCOPE_DRIVE_FILE, "the file's scope is kept");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(oauth::CONFIG_FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "owner-only");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

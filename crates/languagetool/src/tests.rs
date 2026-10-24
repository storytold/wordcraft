//! No network here: requests go to fake transports.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use wordcraft_proof::IssueKind;

use crate::offsets::{utf16_len, utf16_range, utf16_to_byte};
use crate::worker::Pacing;
use crate::*;

fn on() -> Settings {
    Settings { enabled: true, ..Settings::default() }
}

fn form(body: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(body.as_bytes()).into_owned().collect()
}

/// A LanguageTool answer, shaped like the public server's.
fn answer(matches: serde_json::Value) -> String {
    json!({
        "software": {"name": "LanguageTool", "version": "6.6", "apiVersion": 1},
        "language": {"name": "English (US)", "code": "en-US", "detectedLanguage": {"name": "English (US)", "code": "en-US", "confidence": 0.9}},
        "matches": matches,
    })
    .to_string()
}

fn m(offset: u64, length: u64, issue_type: &str, category: &str, replacements: &[&str]) -> serde_json::Value {
    json!({
        "message": format!("{issue_type} here"),
        "shortMessage": "",
        "replacements": replacements.iter().map(|r| json!({"value": r})).collect::<Vec<_>>(),
        "offset": offset,
        "length": length,
        "context": {"text": "…", "offset": 0, "length": 0},
        "sentence": "…",
        "rule": {"id": "SOME_RULE", "description": "d", "issueType": issue_type, "category": {"id": category, "name": "c"}},
    })
}

// ---- offsets ----

#[test]
fn utf16_offsets_map_to_bytes_through_emoji_and_cjk() {
    // "a😀b": 😀 is 2 UTF-16 units and 4 bytes.
    let t = "a😀b";
    assert_eq!(utf16_len(t), 4);
    assert_eq!(utf16_to_byte(t, 0, false), 0);
    assert_eq!(utf16_to_byte(t, 1, false), 1);
    assert_eq!(utf16_to_byte(t, 3, false), 5);
    assert_eq!(utf16_to_byte(t, 4, false), 6);
    // Between the surrogate halves: start of the emoji, or its end when rounding up.
    assert_eq!(utf16_to_byte(t, 2, false), 1);
    assert_eq!(utf16_to_byte(t, 2, true), 5);
    // CJK: one unit, three bytes each.
    let c = "日本語のテキスト";
    assert_eq!(utf16_to_byte(c, 3, false), 9);
    assert_eq!(utf16_range(c, 4, 4), Some((12, 24)));
    assert_eq!(c.get(12..24), Some("テキスト"));
    // Past the end clamps; huge numbers don't overflow.
    assert_eq!(utf16_to_byte(t, 99, false), t.len());
    assert_eq!(utf16_range(t, u64::MAX, u64::MAX), None);
    assert_eq!(utf16_range(t, 3, u64::MAX), Some((5, 6)));
    assert_eq!(utf16_range("", 0, 5), None);
    assert_eq!(utf16_range(t, 1, 0), None);
}

proptest::proptest! {
    /// Any offsets on any text: in range, on char boundaries, never a panic.
    #[test]
    fn utf16_mapping_is_always_a_char_boundary(t in "\\PC{0,40}", off in 0u64..100, len in 0u64..100) {
        for up in [false, true] {
            let b = utf16_to_byte(&t, off, up);
            proptest::prop_assert!(b <= t.len() && t.is_char_boundary(b));
        }
        if let Some((a, b)) = utf16_range(&t, off, len) {
            proptest::prop_assert!(a < b && t.get(a..b).is_some());
        }
    }

    /// Junk answers never panic and never produce a range that isn't in the text.
    #[test]
    fn junk_answers_never_panic(t in "\\PC{0,30}", body in "\\PC{0,200}", off in proptest::num::i64::ANY, len in proptest::num::i64::ANY) {
        let _ = response::parse(&body, &t);
        let crafted = json!({"matches": [{"offset": off, "length": len, "message": body, "replacements": [{"value": 1}, {"value": "x"}], "rule": 7}]}).to_string();
        if let Ok(issues) = response::parse(&crafted, &t) {
            for i in issues {
                proptest::prop_assert!(i.start < i.end && t.get(i.start..i.end).is_some());
            }
        }
    }
}

// ---- response ----

#[test]
fn answers_map_to_spelling_and_grammar_issues() {
    // "I has a 😀 speling error." — offsets count the emoji as two units.
    let text = "I has a 😀 speling error.";
    let body = answer(json!([
        m(11, 7, "misspelling", "TYPOS", &["spelling", "spieling"]),
        m(2, 3, "grammar", "GRAMMAR", &["have"]),
        m(0, 1, "style", "STYLE", &[]),
    ]));
    let issues = response::parse(&body, text).unwrap();
    let got: Vec<(&str, IssueKind, Vec<String>)> =
        issues.iter().map(|i| (text.get(i.start..i.end).unwrap(), i.kind, i.suggestions.clone())).collect();
    assert_eq!(
        got,
        [
            ("I", IssueKind::Grammar, vec![]),
            ("has", IssueKind::Grammar, vec!["have".to_string()]),
            ("speling", IssueKind::Spelling, vec!["spelling".to_string(), "spieling".to_string()]),
        ]
    );
    assert_eq!(issues.iter().find(|i| i.kind == IssueKind::Spelling).unwrap().message, "misspelling here");
}

#[test]
fn category_typos_is_spelling_even_without_an_issue_type() {
    let v = json!({"rule": {"category": {"id": "TYPOS"}}});
    assert_eq!(response::kind_of(&v), IssueKind::Spelling);
    assert_eq!(response::kind_of(&json!({"rule": {"issueType": "typographical", "category": {"id": "PUNCTUATION"}}})), IssueKind::Grammar);
    assert_eq!(response::kind_of(&json!({})), IssueKind::Grammar);
}

#[test]
fn bad_answers_are_errors_and_bad_matches_are_skipped() {
    assert!(matches!(response::parse("<html>Bad gateway</html>", "x"), Err(LtError::Protocol(_))));
    assert!(matches!(response::parse(r#"{"software": {}}"#, "x"), Err(LtError::Protocol(_))));
    let text = "Hello world";
    let body = json!({"matches": [
        {"offset": -1, "length": 2},
        {"offset": "3", "length": 2},
        {"offset": 6, "length": 0},
        {"offset": 50, "length": 2},
        {"offset": 6, "length": 500, "message": "\u{0007}bell\nnewline", "replacements": [{"value": ""}, {"value": "a\u{0000}b"}, {"value": "World"}, {}]},
    ]})
    .to_string();
    let issues = response::parse(&body, text).unwrap();
    assert_eq!(issues.len(), 1);
    let i = &issues[0];
    assert_eq!((i.start, i.end), (6, 11), "clamped to the text");
    assert_eq!(i.suggestions, ["World"]);
    assert_eq!(i.message, "bell newline");
    // Many matches and suggestions are capped.
    let many: Vec<_> = (0..2000).map(|_| m(0, 1, "grammar", "G", &["a"; 50])).collect();
    let issues = response::parse(&answer(json!(many)), "x").unwrap();
    assert_eq!(issues.len(), response::MAX_MATCHES);
    assert_eq!(issues[0].suggestions.len(), response::MAX_SUGGESTIONS);
}

// ---- request ----

#[test]
fn requests_are_form_encoded_with_text_and_language() {
    let r = request::build(&on(), "Ça va? 1+1=2 & 😀", "fr").unwrap();
    assert_eq!(r.url, "https://api.languagetool.org/v2/check");
    assert_eq!(form(&r.body), [("text".to_string(), "Ça va? 1+1=2 & 😀".to_string()), ("language".to_string(), "fr".to_string())]);
    // Junk language codes go as auto.
    let r = request::build(&on(), "x", "en US; drop").unwrap();
    assert_eq!(form(&r.body)[1].1, "auto");
    // Too long: refused before anything is sent.
    let long = "a".repeat(request::MAX_TEXT_BYTES + 1);
    assert!(matches!(request::build(&on(), &long, "auto"), Err(LtError::TooLong { .. })));
}

#[test]
fn credentials_go_in_the_body_only_when_both_are_set_and_never_in_debug() {
    let s = Settings { username: "me@example.org".into(), api_key: "SECRET-KEY-123".into(), ..on() };
    let r = request::build(&s, "text", "auto").unwrap();
    let f = form(&r.body);
    assert!(f.contains(&("username".into(), "me@example.org".into())) && f.contains(&("apiKey".into(), "SECRET-KEY-123".into())));
    assert!(!r.url.contains("SECRET"), "the key never goes in the URL");
    assert!(!format!("{r:?}").contains("SECRET") && !format!("{r:?}").contains("text"), "{r:?}");
    assert!(!format!("{s:?}").contains("SECRET"), "{s:?}");
    // A key without a username isn't sent.
    let s = Settings { username: String::new(), ..s };
    assert!(!request::build(&s, "text", "auto").unwrap().body.contains("apiKey"));
}

// ---- settings ----

#[test]
fn server_addresses_normalise_to_the_check_endpoint() {
    let ep = |s: &str| Settings { server: s.into(), ..on() }.endpoint();
    assert_eq!(ep("").unwrap(), "https://api.languagetool.org/v2/check");
    assert_eq!(ep("http://localhost:8081").unwrap(), "http://localhost:8081/v2/check");
    assert_eq!(ep("http://localhost:8081/").unwrap(), "http://localhost:8081/v2/check");
    assert_eq!(ep(" https://lt.example.org/v2/ ").unwrap(), "https://lt.example.org/v2/check");
    assert_eq!(ep("https://lt.example.org/proxy/v2/check").unwrap(), "https://lt.example.org/proxy/v2/check");
    for bad in ["ftp://x", "localhost:8081", "https://", "https:///x", "https://a b", "https://x?y=1", "javascript:alert(1)", "https://x\u{0}"] {
        assert!(matches!(ep(bad), Err(LtError::BadServer(_))), "{bad}");
    }
    assert!(Settings::default().is_hosted());
    assert!(!Settings { server: "http://localhost:8081".into(), ..on() }.is_hosted());
    assert!(!Settings { server: "https://api.languagetool.org.evil.example".into(), ..on() }.is_hosted());
}

#[test]
fn language_choice_auto_document_or_code() {
    let with = |l: &str| Settings { language: l.into(), ..on() };
    assert_eq!(with("auto").language_for(Some("de-DE")), "auto");
    assert_eq!(with("document").language_for(Some("de-DE")), "de-DE");
    assert_eq!(with("document").language_for(None), "auto");
    assert_eq!(with("document").language_for(Some("<junk>")), "auto");
    assert_eq!(with("en-GB").language_for(Some("de-DE")), "en-GB");
    assert_eq!(with("").language_for(Some("de-DE")), "auto");
}

#[test]
fn settings_default_off_and_junk_saved_values_read_as_defaults() {
    let d = Settings::default();
    assert!(!d.enabled, "off until the user opts in: text leaves the machine");
    assert_eq!(d.server, DEFAULT_SERVER);
    let s: Settings =
        serde_json::from_str(r#"{"enabled": true, "server": "http://localhost:8081", "apiKey": "k", "username": "u", "language": "document"}"#)
            .unwrap();
    assert!(s.enabled && s.has_account());
    assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
    let junk: Settings = serde_json::from_str(r#"{"enabled": "yes", "server": 5, "language": null, "extra": 1}"#).unwrap();
    assert_eq!(junk, Settings::default());
    assert_eq!(serde_json::from_str::<Settings>("7").unwrap(), Settings::default());
}

// ---- check_with and the worker ----

#[test]
fn a_rejected_language_is_retried_with_detection() {
    let mut seen = Vec::new();
    let mut t = |r: &Request| {
        let lang = form(&r.body).into_iter().find(|(k, _)| k == "language").map(|(_, v)| v).unwrap_or_default();
        seen.push(lang.clone());
        if lang == "auto" {
            Ok(answer(json!([m(0, 2, "misspelling", "TYPOS", &[])])))
        } else {
            Err(LtError::Http { status: 400, message: "unknown language".into() })
        }
    };
    let issues = check_with(&on(), &mut t, "Xy z", "xx-YY").unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(seen, ["xx-YY", "auto"]);
    // Other failures are passed on.
    let mut t = |_: &Request| Err(LtError::Timeout);
    assert_eq!(check_with(&on(), &mut t, "x", "auto"), Err(LtError::Timeout));
    assert!(LtError::Http { status: 401, message: String::new() }.is_auth());
}

#[test]
fn the_worker_checks_in_the_background_and_backs_off_when_limited() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = calls.clone();
    let transport: worker::Transport = Box::new(move |r: &Request| {
        let text = form(&r.body).into_iter().find(|(k, _)| k == "text").map(|(_, v)| v).unwrap_or_default();
        log.lock().unwrap().push(text.clone());
        if text == "slow down" { Err(LtError::RateLimited) } else { Ok(answer(json!([m(0, 1, "grammar", "G", &["A"])]))) }
    });
    let woken = Arc::new(Mutex::new(0));
    let w2 = woken.clone();
    let pacing = Pacing { gap: Duration::from_millis(1), backoff: Duration::from_millis(1), timeout: Duration::from_secs(1) };
    let w = Worker::spawn(on(), pacing, transport, Box::new(move || *w2.lock().unwrap() += 1)).unwrap();
    assert!(w.send(Job { text: "a text".into(), language: "auto".into() }));
    assert!(w.send(Job { text: "slow down".into(), language: "auto".into() }));
    let d1 = w.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(d1.text, "a text");
    assert_eq!(d1.result.unwrap().len(), 1);
    let d2 = w.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(d2.result, Err(LtError::RateLimited));
    assert_eq!(*calls.lock().unwrap(), ["a text", "slow down"]);
    // The worker wakes the UI right after sending each result, so the second wake can land just
    // after the result was received (seen on the FreeBSD runner): wait for it, bounded.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while *woken.lock().unwrap() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*woken.lock().unwrap(), 2);
    assert_eq!(w.try_recv(), Ok(None));
}

#[test]
fn hosted_free_use_is_paced_under_its_rate_limit() {
    assert!(Pacing::for_settings(&on()).gap >= Duration::from_secs(3), "≤ 20 requests a minute");
    assert!(Pacing::for_settings(&Settings { server: "http://localhost:8081".into(), ..on() }).gap < Duration::from_secs(1));
}

/// #1283: the retry with `auto` after a rejected language waits out the pacing gap like any
/// request, so the hosted server's 20 requests a minute hold.
#[test]
fn the_retry_with_detection_waits_for_the_pacing_gap() {
    let times = Arc::new(Mutex::new(Vec::<std::time::Instant>::new()));
    let log = times.clone();
    let transport: worker::Transport = Box::new(move |r: &Request| {
        log.lock().unwrap().push(std::time::Instant::now());
        let lang = form(&r.body).into_iter().find(|(k, _)| k == "language").map(|(_, v)| v).unwrap_or_default();
        if lang == "auto" { Ok(answer(json!([]))) } else { Err(LtError::Http { status: 400, message: "unknown language".into() }) }
    });
    let gap = Duration::from_millis(150);
    let pacing = Pacing { gap, backoff: gap, timeout: Duration::from_secs(1) };
    let w = Worker::spawn(on(), pacing, transport, Box::new(|| {})).unwrap();
    assert!(w.send(Job { text: "Hallo".into(), language: "xx-YY".into() }));
    let d = w.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(d.result, Ok(Vec::new()));
    let t = times.lock().unwrap();
    assert_eq!(t.len(), 2);
    assert!(t[1].duration_since(t[0]) >= gap, "{:?}", t[1].duration_since(t[0]));
}

/// #1283: a username and API key never go over plain http:// to another computer; to this
/// computer, or over https://, they do.
#[test]
fn credentials_are_refused_over_plain_http_to_other_computers() {
    let with = |server: &str| Settings { server: server.into(), username: "me@example.org".into(), api_key: "SECRET".into(), ..on() };
    for bad in [
        "http://lt.example.org",
        "http://192.168.1.5:8081",
        "HTTP://lt.example.org/v2",
        "http://localhost.example.org",
        "http://127.0.0.1@evil.example",
    ] {
        assert_eq!(request::build(&with(bad), "text", "auto"), Err(LtError::InsecureAccount), "{bad}");
        assert_eq!(with(bad).account_check(), Err(LtError::InsecureAccount), "{bad}");
    }
    for good in [
        "https://lt.example.org",
        "http://localhost:8081",
        "http://127.0.0.1:8081/",
        "http://[::1]:8081",
        "http://lt.localhost",
        "http://evil@localhost:8081",
    ] {
        let r = request::build(&with(good), "text", "auto").unwrap();
        assert!(form(&r.body).contains(&("apiKey".into(), "SECRET".into())), "{good}");
    }
    // A backslash can make a URL parser read a different host than our check did.
    for bad in [r"http://127.0.0.1\@evil.example", r"http://localhost\.evil.example"] {
        assert!(matches!(with(bad).endpoint(), Err(LtError::BadServer(_))), "{bad}");
        assert!(request::build(&with(bad), "text", "auto").is_err(), "{bad}");
    }
    // Without an account, http:// to anywhere is the user's choice.
    let s = Settings { server: "http://lt.example.org".into(), ..on() };
    assert!(request::build(&s, "text", "auto").is_ok());
    assert!(LtError::InsecureAccount.stops_checking());
}

/// #1283: answers the server will never accept for this paragraph skip it instead of retrying
/// every 30 seconds; credentials and addresses stop checking; the network pauses it.
#[test]
fn failures_are_classified_as_skip_stop_or_pause() {
    let http = |status| LtError::Http { status, message: String::new() };
    for e in [http(400), http(413), http(422), LtError::Protocol("x".into()), LtError::TooLong { bytes: 1, max: 0 }] {
        assert!(e.skips_paragraph() && !e.stops_checking(), "{e:?}");
    }
    for e in [http(401), http(403), http(404), LtError::BadServer("x".into()), LtError::InsecureAccount] {
        assert!(e.stops_checking() && !e.skips_paragraph(), "{e:?}");
    }
    for e in [http(429), http(500), http(503), LtError::RateLimited, LtError::Timeout, LtError::Network("x".into())] {
        assert!(!e.stops_checking() && !e.skips_paragraph(), "{e:?}");
    }
    assert!(matches!(http::utf8_body(vec![b'{', 0xff, b'}']), Err(LtError::Protocol(_))));
    assert_eq!(http::utf8_body(b"{}".to_vec()), Ok("{}".into()));
}

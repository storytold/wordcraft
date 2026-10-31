//! A command request's `author` signs that command's tracked changes and comments, and the session
//! signs with its own name again afterwards.

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn change_authors(s: &mut Session) -> Vec<String> {
    let list = run(s, "review.changes", json!({}));
    list.as_array().into_iter().flatten().filter_map(|c| c["author"].as_str().map(str::to_string)).collect()
}

#[test]
fn request_author_is_trimmed_and_cut_and_blank_means_none() {
    assert_eq!(Session::request_author(&json!({})), Ok(None));
    assert_eq!(Session::request_author(&json!({"author": null})), Ok(None));
    assert_eq!(Session::request_author(&json!({"author": "   "})), Ok(None));
    assert_eq!(Session::request_author(&json!({"author": "  Drafting assistant "})), Ok(Some("Drafting assistant".to_string())));
    let long = "é".repeat(Session::MAX_REQUEST_AUTHOR_CHARS + 50);
    let cut = Session::request_author(&json!({"author": long})).ok().flatten().unwrap_or_default();
    assert_eq!(cut.chars().count(), Session::MAX_REQUEST_AUTHOR_CHARS);
    assert!(Session::request_author(&json!({"author": 42})).is_err());
}

#[test]
fn as_author_signs_the_tracked_change_then_restores_the_session_signature() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.author = "Alice Smith".into();
    s.initials = "AS".into();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": "mine "}));
    let r = s.as_author(Some("Drafting assistant".into()), |s| s.run("text.insert", &json!({"text": "agent"})));
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(change_authors(&mut s), ["Alice Smith", "Drafting assistant"]);
    assert_eq!(s.author, "Alice Smith");
    assert_eq!(s.initials, "AS");
}

#[test]
fn as_author_restores_the_signature_when_the_command_fails() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.author = "Alice Smith".into();
    let r = s.as_author(Some("Drafting assistant".into()), |s| s.run("no.such.command", &json!({})));
    assert!(r.is_err());
    assert_eq!(s.author, "Alice Smith");
}

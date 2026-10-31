//! Harper's English grammar checks (#799) through the session: its marks arrive from the
//! background thread, the context menu's suggestion applies, and File › Options › Proofing and
//! the front end decide whether it runs.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use wordcraft_doc::StoryRef;

use crate::Session;

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The issues `review.issues` lists on exactly `text`.
fn issues_on(s: &mut Session, text: &str) -> Vec<Value> {
    run(s, "review.issues", json!({})).as_array().unwrap().iter().filter(|i| i["text"] == text).cloned().collect()
}

fn wait() {
    assert!(wordcraft_proof::harper::wait(Duration::from_secs(300)), "Harper didn't finish");
}

#[test]
fn harper_marks_arrive_and_their_suggestion_applies() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Yesterday we could of gone to the lake."));
    s.harper = true;
    let before = s.layout();
    assert!(issues_on(&mut s, "could of").is_empty(), "not checked yet");
    wait();
    // The result arrived without an edit: the next layout is made again and shows it.
    assert!(!Arc::ptr_eq(&before, &s.layout()));
    let found = issues_on(&mut s, "could of");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["kind"], "grammar");
    assert!(found[0]["suggestions"].as_array().unwrap().iter().any(|x| x == "could have"), "{found:?}");
    run(&mut s, "select.text", json!({"text": "could of"}));
    run(&mut s, "review.applySuggestion", json!({"text": "could have"}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Yesterday we could have gone to the lake.");
}

#[test]
fn harper_runs_only_where_the_front_end_starts_it_and_the_option_allows() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Last week they could of known better."));
    // On by default, but a headless session never runs Harper.
    assert!(s.prefs.harper_grammar);
    assert!(!s.harper_grammar());
    s.layout();
    assert!(issues_on(&mut s, "could of").is_empty());
    s.harper = true;
    assert!(s.harper_grammar());
    let off = run(&mut s, "file.options", json!({"harperGrammar": false}));
    assert_eq!(off["harperGrammar"], false);
    assert!(!s.harper_grammar());
    assert_eq!(s.harper_waiting(), 0);
    s.layout();
    wait();
    assert!(issues_on(&mut s, "could of").is_empty());
    assert!(s.run("file.options", &json!({"harperGrammar": "no"})).is_err(), "not true or false");
    assert!(!s.prefs.harper_grammar);
    run(&mut s, "file.options", json!({"harperGrammar": true}));
    s.layout();
    wait();
    assert_eq!(issues_on(&mut s, "could of").len(), 1);
}

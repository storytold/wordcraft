#![allow(clippy::unwrap_used)]
use super::*;

#[test]
fn opening_a_bubble_closes_any_previous_request_in_the_hub() {
    let (mut app, hub) = app();
    hub.listen(7, None, Duration::ZERO, || true).unwrap();
    command(&mut app, &json!({"instruction": "First"})).unwrap();
    let first = hub.listen(7, None, Duration::ZERO, || true).unwrap().unwrap().id;
    assert!(hub.snapshot(first).is_some());
    command(&mut app, &json!({})).unwrap();
    assert!(hub.snapshot(first).is_none(), "opening a new bubble closed the earlier request");
}

#[test]
fn failed_command_does_not_record_edited_from_for_undo_join() {
    let (mut app, hub) = app();
    hub.listen(7, None, Duration::ZERO, || true).unwrap();
    command(&mut app, &json!({"instruction": "Edit"})).unwrap();
    let id = hub.listen(7, None, Duration::ZERO, || true).unwrap().unwrap().id;
    let before = app.session.undo_depth();
    // A failing command does not set edited_from
    let _ = as_agent(&mut app, Some(7), "text.insert", &json!({"request": id}), |a| a.run("invalid.command", json!({})));
    assert_eq!(app.ask_agent.open.as_ref().unwrap().edited_from, None);
    // Successful command afterwards sets edited_from
    let p = json!({"text": "done", "request": id});
    as_agent(&mut app, Some(7), "text.insert", &p, |a| a.run("text.insert", p.clone())).unwrap();
    assert_eq!(app.ask_agent.open.as_ref().unwrap().edited_from, Some(before));
}
use std::time::Duration;

fn app() -> (WordApp, AgentHub) {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let hub = AgentHub::default();
    app.ask_agent.hub = Some(hub.clone());
    app.run("text.insert", json!({"text": "Kilns fire clay."})).unwrap();
    app.run("select.all", json!({})).unwrap();
    (app, hub)
}

#[test]
fn the_agents_edits_are_one_undo_step_authored_by_it_and_hold_the_text() {
    let (mut app, hub) = app();
    hub.listen(7, Some("Claude"), Duration::ZERO, || true).unwrap();
    command(&mut app, &json!({"instruction": "Shorter"})).unwrap();
    let id = hub.listen(7, Some("Claude"), Duration::ZERO, || true).unwrap().unwrap().id;
    let before = app.session.undo_depth();
    for text in ["Kilns ", "fire."] {
        let p = json!({"text": text, "request": id});
        as_agent(&mut app, Some(7), "text.insert", &p, |a| a.run("text.insert", p.clone())).unwrap();
    }
    assert_eq!(app.session.undo_depth(), before + 1, "one undo step");
    assert!(held(&app, "text.insert").unwrap().ends_with("(Claude)"), "the person's edit is refused");
    let other = as_agent(&mut app, Some(8), "text.insert", &json!({"text": "x"}), |a| a.run("text.insert", json!({"text": "x"})));
    assert!(other.unwrap_err().contains(hub::HELD), "another connection without the request");
    hub.reply(7, id, Reply { text: "Shortened.", ..Reply::default() }, Some(&current(&app.session))).unwrap();
    comment_once(&mut app, &hub.snapshot(id).unwrap());
    assert_eq!(app.session.undo_depth(), before + 1, "the closing comment joins the step");
    let comment = app.session.doc.comments.values().next().unwrap();
    assert_eq!(comment.author, "Agent (Claude)");
    let persisted = app.session.run("review.comments", &json!({})).unwrap().to_string();
    assert!(persisted.contains("Shorter") && persisted.contains("Shortened."), "comment retains instruction and answer");
    assert!(held(&app, "text.insert").is_none(), "answered: the text is free");
}

#[test]
fn a_follow_up_carries_the_thread_and_stays_in_the_same_undo_step() {
    let (mut app, hub) = app();
    hub.listen(1, None, Duration::ZERO, || true).unwrap();
    command(&mut app, &json!({"instruction": "Formal"})).unwrap();
    let first = hub.listen(1, None, Duration::ZERO, || true).unwrap().unwrap().id;
    hub.reply(1, first, Reply { text: "Made formal.", ..Reply::default() }, Some(&current(&app.session))).unwrap();
    send(&mut app, "Shorter".into()).unwrap();
    let second = hub.listen(1, None, Duration::ZERO, || true).unwrap().unwrap();
    assert_eq!(second.history, vec![Turn { instruction: "Formal".into(), answer: "Made formal.".into() }]);
    assert!(hub.snapshot(first).is_none(), "the answered request left the queue");
}

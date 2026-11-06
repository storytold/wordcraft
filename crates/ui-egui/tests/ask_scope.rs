//! Public adapter regression retained by verif-repro while production files are stashed.
use serde_json::json;
use std::time::Duration;
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::{Selection, Session};
use wordcraft_ui_egui::{
    WordApp,
    ask_agent::{self, AgentHub},
};

#[test]
fn scoped_typing_does_not_autocorrect_or_overtype_neighboring_text() {
    let mut app = WordApp::new(Session::new(Document::from_text("teh suffix")), Default::default());
    let hub = AgentHub::default();
    app.ask_agent.hub = Some(hub.clone());
    app.session.sel = Selection::caret(Pos::body(0, 3));
    app.session.prefs.overtype = true;
    app.session.prefs.typing_replaces_selection = false;
    let _ = hub.listen(7, Some("Agent"), Duration::ZERO, || true).expect("register");
    ask_agent::command(&mut app, &json!({"instruction":"Insert only one space here"})).expect("ask");
    let request = hub.listen(7, None, Duration::ZERO, || true).expect("listen").expect("request");
    let params = json!({"request":request.id,"text":" "});
    ask_agent::as_agent(&mut app, Some(7), "text.insert", &params, |app| app.run("text.insert", params.clone())).expect("owned insertion");
    assert_eq!(app.session.doc.plain_text(StoryRef::Body), "teh  suffix", "typing may change only the requested insertion point");
    assert!(app.session.prefs.overtype);
    assert!(!app.session.prefs.typing_replaces_selection);
}

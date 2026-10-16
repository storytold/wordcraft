use egui::vec2;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default())
}

/// The app as the desktop runs it: Nib introduced and turned on, speaking on its own.
fn chatty() -> WordApp {
    let mut a = app();
    a.assistant.automatic = true;
    a.ui.assistant.introduced = true;
    a.ui.assistant.visible = true;
    a.assistant.welcomed = true;
    a
}

fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    )
}

fn showing(a: &WordApp) -> Option<&Content> {
    a.assistant.balloon.as_ref().map(|b| &b.content)
}

fn offer_id(a: &WordApp) -> Option<&'static str> {
    match showing(a) {
        Some(Content::Offer(o)) => Some(o.id),
        _ => None,
    }
}

#[test]
fn commands_turn_nib_on_and_set_it_up() {
    let mut a = app();
    assert_eq!(a.run("ui.assistant", json!({})).unwrap(), json!({"visible": true}), "no value toggles");
    assert!(a.ui.assistant.introduced, "turning Nib on counts as having met it");
    assert_eq!(a.run("ui.assistant", json!({"show": false})).unwrap(), json!({"visible": false}));
    assert!(a.run("ui.assistant", json!({"show": "yes"})).is_err());
    assert_eq!(a.run("ui.assistantSayings", json!({})).unwrap(), json!({"level": "sometimes"}), "Sometimes by default");
    assert_eq!(a.run("ui.assistantSayings", json!({"level": "annoying"})).unwrap(), json!({"level": "annoying"}));
    assert!(a.run("ui.assistantSayings", json!({"level": "always"})).is_err());
    assert_eq!(a.run("ui.assistantMute", json!({"value": true})).unwrap(), json!({"muted": true}));
    a.ui.assistant.dismissed_tips = vec!["letter".into()];
    a.run("ui.assistantResetTips", json!({})).unwrap();
    assert!(a.ui.assistant.dismissed_tips.is_empty());
}

#[test]
fn settings_survive_a_restart() {
    let mut a = app();
    a.run("ui.assistant", json!({"show": true})).unwrap();
    a.run("ui.assistantSayings", json!({"level": "off"})).unwrap();
    a.ui.assistant.dismissed_tips.push("letter".into());
    a.ui.assistant.position = Some([10.0, 20.0]);
    let saved = serde_json::to_vec(&a.ui).unwrap();
    let back = crate::UiState::from_json(&saved).unwrap();
    assert_eq!(back.assistant, a.ui.assistant);
    // A hostile or older file falls back instead of failing.
    let junk = crate::UiState::from_json(br#"{"assistant": {"visible": "maybe", "sayings": "loud", "position": ["far", 5]}}"#).unwrap();
    assert!(!junk.assistant.visible);
}

#[test]
fn agent_messages_are_checked_and_limited() {
    let mut a = app();
    assert!(a.run("ui.assistantSay", json!({})).is_err(), "text is required");
    assert!(a.run("ui.assistantSay", json!({"text": "x".repeat(281)})).is_err(), "too long");
    assert!(a.run("ui.assistantSay", json!({"text": "Hi", "choices": ["a", "b", "c", "d"]})).is_err(), "too many choices");
    assert!(a.run("ui.assistantSay", json!({"text": "Hi", "choices": [""]})).is_err());
    assert!(a.run("ui.assistantSay", json!({"text": "Hi", "choices": "a"})).is_err());
    let id = a.run("ui.assistantSay", json!({"text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    assert!(a.run("ui.assistantSay", json!({"text": "Again"})).unwrap_err().contains("too soon"));
    assert_eq!(a.run("ui.assistantReply", json!({"id": id})).unwrap(), json!({"choice": null, "open": true}));
    assert!(a.run("ui.assistantReply", json!({"id": 999})).is_err());
    assert!(a.run("ui.assistantReply", json!({})).is_err());
    // The queue holds three.
    for n in 0..2 {
        a.assistant.now += 5.0;
        a.run("ui.assistantSay", json!({"text": format!("m{n}")})).unwrap();
    }
    a.assistant.now += 5.0;
    assert!(a.run("ui.assistantSay", json!({"text": "one more"})).unwrap_err().contains("queue full"));
    // Muted: the agent is told.
    a.run("ui.assistantMute", json!({"value": true})).unwrap();
    a.assistant.now += 5.0;
    assert!(a.run("ui.assistantSay", json!({"text": "hello?"})).unwrap_err().contains("muted"));
}

#[test]
fn hostile_params_never_panic() {
    let mut a = app();
    let junk = [
        json!(null),
        json!(1),
        json!("x"),
        json!([1]),
        json!({"show": 1, "level": [], "value": {}, "text": 5, "choices": [1, null], "id": -1, "from": 3}),
        json!({"text": "\u{0}\u{7}", "from": "\u{202e}".repeat(500)}),
    ];
    for id in ["ui.assistant", "ui.assistantSayings", "ui.assistantResetTips", "ui.assistantMute", "ui.assistantSay", "ui.assistantReply"] {
        for p in &junk {
            a.assistant.now += 10.0;
            let _ = a.run(id, p.clone());
        }
    }
}

#[test]
fn an_agent_asks_and_hears_back() {
    let mut a = app();
    let id = a.run("ui.assistantSay", json!({"text": "Shall I tidy the headings?", "choices": ["Yes", "Not now"], "from": "Helper"})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert!(matches!(showing(h.state()), Some(Content::Agent { .. })), "Nib pops in for it even when hidden");
    h.get_by_label("From Helper");
    h.get_by_label("Yes").click();
    h.run_steps(4);
    assert_eq!(h.state_mut().run("ui.assistantReply", json!({"id": id})).unwrap(), json!({"choice": "Yes", "open": false}));
    assert!(showing(h.state()).is_none());
}

#[test]
fn typing_with_the_balloon_open_reaches_the_document() {
    let mut a = app();
    a.run("ui.assistant", json!({"show": true})).unwrap();
    a.run("ui.assistantSay", json!({"text": "Still here!", "choices": ["Thanks"]})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert!(showing(h.state()).is_some());
    h.event(egui::Event::Text("Hello".into()));
    h.run_steps(4);
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Hello"), "the keystrokes went to the document");
    // And after a click in the Balloon too.
    h.get_by_label("Thanks").click();
    h.run_steps(4);
    h.event(egui::Event::Text(" again".into()));
    h.run_steps(4);
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Hello again"));
}

#[test]
fn the_introduction_runs_once() {
    let mut a = app();
    a.assistant.automatic = true;
    let mut h = harness(a);
    h.run_steps(4);
    assert!(matches!(showing(h.state()), Some(Content::Intro)));
    h.get_by_label("Not now").click();
    h.run_steps(4);
    assert!(h.state().ui.assistant.introduced && !h.state().ui.assistant.visible);
    for _ in 0..5 {
        h.step();
    }
    assert!(showing(h.state()).is_none(), "never again");
    // Without `automatic` (tests, headless, ui_shot) it never introduces itself.
    let mut h = harness(app());
    h.run_steps(4);
    assert!(showing(h.state()).is_none());
}

#[test]
fn hidden_nib_stays_silent() {
    let mut a = chatty();
    a.ui.assistant.visible = false;
    a.run("text.insert", json!({"text": "Dear Kim,"})).unwrap();
    next_balloon(&mut a);
    assert!(showing(&a).is_none());
}

#[test]
fn a_letter_brings_the_letter_tip_once() {
    let mut a = chatty();
    a.run("text.insert", json!({"text": "Dear Kim,"})).unwrap();
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("letter"));
    a.assistant.close(&mut a.ui.assistant);
    a.assistant.now += 600.0;
    a.run("text.insert", json!({"text": " "})).unwrap();
    next_balloon(&mut a);
    assert_ne!(offer_id(&a), Some("letter"), "each Tip at most once a session");
}

#[test]
fn dont_show_again_is_remembered() {
    let mut a = chatty();
    a.run("text.insert", json!({"text": "Dear Kim,"})).unwrap();
    next_balloon(&mut a);
    if let Some(b) = &mut a.assistant.balloon {
        b.dont_show = true;
    }
    a.assistant.close(&mut a.ui.assistant);
    assert_eq!(a.ui.assistant.dismissed_tips, ["letter"]);
}

#[test]
fn the_ribbon_three_times_brings_the_shortcut_tip() {
    let mut a = chatty();
    a.run("text.insert", json!({"text": "Bold words"})).unwrap();
    for _ in 0..3 {
        crate::widgets::run_from_ribbon(&mut a, "format.bold", json!({}));
    }
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("shortcut"));
}

#[test]
fn sayings_keep_to_their_level() {
    let mut a = chatty();
    a.run("text.insert", json!({"text": "lorem ipsum"})).unwrap();
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("lorem"));
    a.assistant.close(&mut a.ui.assistant);
    // Sometimes: not again within fifteen minutes.
    a.assistant.now += 60.0;
    a.run("text.insert", json!({"text": " lorem ipsum"})).unwrap();
    next_balloon(&mut a);
    assert!(showing(&a).is_none());
    // Annoying: every three minutes.
    a.run("ui.assistantSayings", json!({"level": "annoying"})).unwrap();
    a.assistant.now += 200.0;
    a.run("text.insert", json!({"text": " lorem ipsum"})).unwrap();
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("lorem"));
    a.assistant.close(&mut a.ui.assistant);
    // Off: never.
    a.run("ui.assistantSayings", json!({"level": "off"})).unwrap();
    a.assistant.now += 2000.0;
    a.run("text.insert", json!({"text": " lorem ipsum"})).unwrap();
    next_balloon(&mut a);
    assert!(showing(&a).is_none());
}

#[test]
fn a_saying_fades_and_a_tip_waits_for_typing() {
    let mut a = chatty();
    a.run("text.insert", json!({"text": "lorem ipsum"})).unwrap();
    next_balloon(&mut a);
    a.assistant.now += SAYING_S + 0.1;
    expire(&mut a);
    assert!(showing(&a).is_none(), "the Saying faded");
    a.assistant.now += 1000.0;
    a.run("text.newParagraph", json!({})).unwrap();
    a.run("text.insert", json!({"text": "Dear Kim,"})).unwrap();
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("letter"));
    a.assistant.now += 120.0;
    expire(&mut a);
    assert!(showing(&a).is_some(), "a Tip waits while the user reads it");
    a.run("text.insert", json!({"text": " "})).unwrap();
    a.assistant.now += TIP_TYPING_S + 0.1;
    expire(&mut a);
    assert!(showing(&a).is_none(), "and goes once they type on");
}

#[test]
fn turning_nib_on_answers_the_introduction() {
    let mut a = app();
    a.assistant.automatic = true;
    next_balloon(&mut a);
    assert!(matches!(showing(&a), Some(Content::Intro)));
    a.run("ui.assistant", json!({"show": true})).unwrap();
    assert!(showing(&a).is_none());
    assert!(a.ui.assistant.introduced && a.ui.assistant.visible);
}

/// Field report: with the letter Tip still open (or just closed), a list started or the ribbon
/// used three times brought no Tip. A new Tip replaces an open one, and there's no gap between
/// Tips (each shows once a session anyway).
#[test]
fn a_new_tip_replaces_an_open_one_and_follows_a_closed_one() {
    let mut a = chatty();
    a.run("text.insert", json!({"text": "Dear Kim,"})).unwrap();
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("letter"));
    a.run("text.newParagraph", json!({})).unwrap();
    a.run("text.insert", json!({"text": "1."})).unwrap();
    a.run("text.insert", json!({"text": " "})).unwrap();
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("listStarted"), "the list Tip replaced the open letter Tip");
    a.assistant.close(&mut a.ui.assistant);
    a.assistant.now += 5.0;
    for _ in 0..3 {
        crate::widgets::run_from_ribbon(&mut a, "format.bold", json!({}));
    }
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("shortcut"), "seconds after another Tip");
}

/// Field report: the list Tip's "Restart at 1" did nothing once the list was deleted with the
/// Balloon still open. The command is disabled outside a numbered list, so the Balloon (which
/// only shows enabled buttons) drops it.
#[test]
fn restart_at_1_goes_when_the_list_does() {
    let mut a = chatty();
    a.run("text.insert", json!({"text": "1."})).unwrap();
    a.run("text.insert", json!({"text": " "})).unwrap();
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("listStarted"));
    assert!(crate::widgets::enabled(&a, "para.restartNumbering"), "in the list: offered");
    // Backspace at the start of the item turns it back into text.
    a.run("text.backspace", json!({})).unwrap();
    assert!(!crate::widgets::enabled(&a, "para.restartNumbering"), "the list is gone: not offered");
}

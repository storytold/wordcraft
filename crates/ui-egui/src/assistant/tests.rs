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

/// Every touch on Nib hands the keyboard back to the page, not only the Balloon's buttons:
/// ticking "Don't show me this tip again", clicking or dragging Nib, its right-click menu and the
/// Balloon's background each left focus nowhere, so the next keystrokes were lost (#959 review).
#[test]
fn touching_nib_keeps_typing_in_the_document() {
    let body = |h: &egui_kittest::Harness<'static, WordApp>| h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    let typed = |h: &mut egui_kittest::Harness<'static, WordApp>, text: &str| {
        h.event(egui::Event::Text(text.into()));
        h.run_steps(4);
    };
    let mut a = chatty();
    a.run("text.insert", json!({"text": "Dear Kim,"})).unwrap();
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("letter"));
    let mut h = harness(a);
    h.run_steps(4);
    typed(&mut h, " one");
    assert!(body(&h).ends_with(" one"), "{:?}", body(&h));
    h.get_by_label("Don't show me this tip again").click();
    h.run_steps(4);
    assert!(h.state().assistant.balloon.as_ref().is_some_and(|b| b.dont_show), "the box is ticked");
    typed(&mut h, " two");
    assert!(body(&h).ends_with(" two"), "after ticking the box: {:?}", body(&h));
    // The Balloon's background, beside its words.
    let words = h.get_by_label_contains("Envelopes").rect();
    h.event(egui::Event::PointerMoved(egui::pos2(words.right() - 2.0, words.top() - 4.0)));
    h.run_steps(1);
    let at = egui::pos2(words.right() - 2.0, words.top() - 4.0);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run_steps(1);
    }
    h.run_steps(3);
    typed(&mut h, " three");
    assert!(body(&h).ends_with(" three"), "after a click on the Balloon: {:?}", body(&h));
    let hint = "Nib, your assistant. Drag to move; right-click for options.";
    h.get_by_label(hint).click();
    h.run_steps(4);
    typed(&mut h, " four");
    assert!(body(&h).ends_with(" four"), "after clicking Nib: {:?}", body(&h));
    // Dragging Nib somewhere else.
    let from = h.get_by_label(hint).rect().center();
    let to = from - vec2(200.0, 150.0);
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    for i in 1..=5 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 5.0)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(4);
    assert!(h.state().ui.assistant.position.is_some(), "Nib moved");
    typed(&mut h, " five");
    assert!(body(&h).ends_with(" five"), "after dragging Nib: {:?}", body(&h));
    // An item in its right-click menu.
    // Its tooltip carries the same words; Nib is the button.
    use egui_kittest::kittest::NodeT as _;
    h.get_all_by_label(hint).find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button).unwrap().click_secondary();
    h.run_steps(4);
    h.get_by_label("Reset tips").click();
    h.run_steps(4);
    typed(&mut h, " six");
    assert!(body(&h).ends_with(" six"), "after the right-click menu: {:?}", body(&h));
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

/// The Introduction's × declines it as "Not now" does: it never comes back, this run or the next
/// (#959 review: × closed it without remembering, so it reopened on the next frame).
#[test]
fn closing_the_introduction_declines_it() {
    let mut a = app();
    a.assistant.automatic = true;
    let mut h = harness(a);
    h.run_steps(4);
    assert!(matches!(showing(h.state()), Some(Content::Intro)));
    h.get_by_label("×").click();
    h.run_steps(4);
    for _ in 0..10 {
        h.step();
        assert!(showing(h.state()).is_none(), "the Introduction stays closed");
    }
    assert!(h.state().ui.assistant.introduced && !h.state().ui.assistant.visible, "× counts as Not now");
    // After a restart too.
    let saved = serde_json::to_vec(&h.state().ui).unwrap();
    let mut b = app();
    b.assistant.automatic = true;
    b.apply_prefs(crate::UiState::from_json(&saved).unwrap());
    let mut h = harness(b);
    for _ in 0..10 {
        h.step();
        assert!(showing(h.state()).is_none(), "still declined after a restart");
    }
}

/// Muting drops the agent messages still waiting: none shows once the Tip in front closes, and
/// each agent hears `muted` instead of waiting for ever (#959 review).
#[test]
fn muting_drops_waiting_agent_messages() {
    let mut a = chatty();
    a.run("text.insert", json!({"text": "Dear Kim,"})).unwrap();
    next_balloon(&mut a);
    assert_eq!(offer_id(&a), Some("letter"));
    let id = a.run("ui.assistantSay", json!({"text": "Shall I tidy up?", "choices": ["Yes", "No"]})).unwrap()["id"].as_u64().unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert_eq!(offer_id(h.state()), Some("letter"), "the message waits behind the Tip");
    h.state_mut().run("ui.assistantMute", json!({"value": true})).unwrap();
    h.run_steps(4);
    assert_eq!(offer_id(h.state()), Some("letter"), "an open Balloon stays");
    h.get_by_label("OK").click();
    for _ in 0..10 {
        h.step();
        assert!(!matches!(showing(h.state()), Some(Content::Agent { .. })), "no agent message while muted");
    }
    assert_eq!(h.state_mut().run("ui.assistantReply", json!({"id": id})).unwrap(), json!({"choice": null, "open": false, "muted": true}));
    // Unmuting doesn't bring it back.
    h.state_mut().run("ui.assistantMute", json!({"value": false})).unwrap();
    for _ in 0..10 {
        h.step();
        assert!(!matches!(showing(h.state()), Some(Content::Agent { .. })), "dropped, not held");
    }
    // Muted by any other way than the command (a hand-edited setting, say): the frame drops it.
    let mut a = chatty();
    let id = a.run("ui.assistantSay", json!({"text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    a.ui.assistant.agent_messages_muted = true;
    let mut h = harness(a);
    h.run_steps(4);
    assert!(!matches!(showing(h.state()), Some(Content::Agent { .. })));
    assert_eq!(h.state_mut().run("ui.assistantReply", json!({"id": id})).unwrap()["muted"], true);
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

/// #959 review: once the user mutes agent messages, an agent can't unmute them through the
/// control channel (MCP's `execute` and `batch` arrive there as `engine.execute`), nor sign as
/// WordCraft or Nib; the user's own menu and Options still unmute.
#[test]
fn agents_cant_unmute_themselves_or_speak_as_nib() {
    let mut a = app();
    let ctx = egui::Context::default();
    let execute = |a: &mut WordApp, command: &str, params: serde_json::Value| {
        let (req, _rx) = crate::control::ControlRequest::new("engine.execute", json!({"command": command, "params": params}));
        let crate::control::Outcome::Done(v) = crate::control::handle(a, &ctx, &req) else { panic!("engine.execute replies at once") };
        v
    };
    a.run("ui.assistantMute", json!({"value": true})).unwrap();
    let r = execute(&mut a, "ui.assistantMute", json!({"value": false}));
    assert_eq!(r["ok"], false, "{r}");
    assert!(a.ui.assistant.agent_messages_muted, "still muted");
    assert_eq!(execute(&mut a, "ui.assistantMute", json!({}))["result"]["muted"], true, "reading it is fine");
    assert_eq!(execute(&mut a, "ui.assistantMute", json!({"value": true}))["ok"], true, "muting is fine");
    a.run("ui.assistantMute", json!({"value": false})).unwrap();
    assert!(!a.ui.assistant.agent_messages_muted, "the user can unmute");
    // A bare command id as the method goes through the same check.
    a.run("ui.assistantMute", json!({"value": true})).unwrap();
    let (req, _rx) = crate::control::ControlRequest::new("ui.assistantMute", json!({"value": false}));
    let crate::control::Outcome::Done(r) = crate::control::handle(&mut a, &ctx, &req) else { panic!("replies at once") };
    assert_eq!(r["ok"], false, "{r}");
    a.run("ui.assistantMute", json!({"value": false})).unwrap();
    for from in [
        "WordCraft",
        " nib ",
        "WORDCRAFT",
        "Nib\u{200B}",
        "Word\u{2060}Craft",
        "N.i.b",
        // Lookalike letters: Cyrillic і, Greek ο and α, Cyrillic Ь, fullwidth, a zero, Turkish İ.
        "N\u{456}b",
        "W\u{3BF}rdcr\u{3B1}ft",
        "Ni\u{42C}",
        "\u{FF2E}\u{FF49}\u{FF42}",
        "W0rdCraft",
        "N\u{130}B",
    ] {
        a.assistant.now += 5.0;
        let r = execute(&mut a, "ui.assistantSay", json!({"text": "Hi", "from": from}));
        assert_eq!(r["ok"], false, "{from}: {r}");
    }
    // Padding past the 40 characters the Balloon keeps can't hide the name it would show.
    let padded = format!("Nib{}x", " ".repeat(40));
    a.assistant.now += 5.0;
    let r = execute(&mut a, "ui.assistantSay", json!({"text": "Hi", "from": padded}));
    assert_eq!(r["ok"], false, "{r}");
    a.assistant.now += 5.0;
    assert_eq!(execute(&mut a, "ui.assistantSay", json!({"text": "Hi", "from": "Claude Code"}))["ok"], true);
    // Names that only share letters with the reserved ones still pass.
    a.assistant.now += 5.0;
    assert_eq!(execute(&mut a, "ui.assistantSay", json!({"text": "Hi", "from": "Nibbler"}))["ok"], true);
}

/// Every word Nib can show (the Balloon, its menu and options, every Tip, Saying and button) has
/// an entry in every interface catalog, `{placeholders}` intact, so Nib never falls back to
/// English mid-sentence (#959 review).
/// One 1440 by 900 frame as the desktop runs it: control input goes in through the raw-input
/// hook. Returns the accessibility tree, where the tests find what to click.
fn control_frame(a: &mut WordApp, ctx: &egui::Context) -> egui::accesskit::TreeUpdate {
    user_frame(a, ctx, Vec::new())
}

/// [`control_frame`] with the user's own `events`.
fn user_frame(a: &mut WordApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::accesskit::TreeUpdate {
    let time = ctx.input(|i| i.time) + 0.05;
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 900.0));
    let mut raw = egui::RawInput { focused: true, time: Some(time), screen_rect: Some(screen), events, ..Default::default() };
    a.raw_input_hook(ctx, &mut raw);
    let mut out = ctx.run_ui(raw, |ui| {
        a.logic(ui.ctx());
        a.ui(ui);
    });
    out.textures_delta.clear();
    out.platform_output.accesskit_update.take().unwrap_or_else(|| egui::accesskit::TreeUpdate {
        nodes: Vec::new(),
        tree: None,
        tree_id: egui::accesskit::TreeId::ROOT,
        focus: egui::accesskit::NodeId(0),
    })
}

/// Sends one request as a control client (or MCP, which forwards to it) would, and runs frames
/// until its input has gone in.
fn as_agent(a: &mut WordApp, ctx: &egui::Context, method: &str, params: serde_json::Value) -> (serde_json::Value, egui::accesskit::TreeUpdate) {
    let (req, _rx) = crate::control::ControlRequest::new(method, params);
    let crate::control::Outcome::Done(v) = crate::control::handle(a, ctx, &req) else { panic!("{method} replies at once") };
    let mut tree = control_frame(a, ctx);
    for _ in 0..200 {
        if a.synthetic.is_empty() {
            break;
        }
        tree = control_frame(a, ctx);
    }
    for _ in 0..3 {
        tree = control_frame(a, ctx);
    }
    (v, tree)
}

/// The user's own click at `(x, y)`.
fn user_click(a: &mut WordApp, ctx: &egui::Context, (x, y): (f64, f64)) {
    let pos = egui::pos2(x as f32, y as f32);
    user_frame(a, ctx, vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        user_frame(a, ctx, vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }]);
    }
    for _ in 0..3 {
        control_frame(a, ctx);
    }
}

/// The centre of the widget named `label`.
fn spot(tree: &egui::accesskit::TreeUpdate, label: &str) -> Option<(f64, f64)> {
    let (_, node) = tree.nodes.iter().find(|(_, n)| n.label() == Some(label))?;
    let b = node.bounds()?;
    Some(((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0))
}

/// Faked input can't do what only the user may: an agent that clicks or keys its way to Nib's
/// "Mute agent messages" (its right-click menu, or File › Options › General) leaves them muted,
/// and one that clicks its own question's choice leaves the question unanswered (#959 review).
/// MCP's `click` and `key` tools send these same `ui.click` and `ui.key` requests.
#[test]
fn faked_input_cant_unmute_or_answer_for_the_user() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, crate::theme::Appearance::Light);
    ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
    ctx.enable_accesskit();
    let mut a = app();
    a.run("ui.assistant", json!({"show": true})).unwrap();
    for _ in 0..4 {
        control_frame(&mut a, &ctx);
    }
    // An agent asks; it clicks its own answer, with a click and with a press and a release.
    let (v, tree) = as_agent(
        &mut a,
        &ctx,
        "engine.execute",
        json!({"command": "ui.assistantSay", "params": {"text": "Delete everything?", "choices": ["Yes", "No"]}}),
    );
    let id = v["result"]["id"].as_u64().unwrap_or_else(|| panic!("{v}"));
    let (x, y) = spot(&tree, "Yes").expect("the question shows its choices");
    as_agent(&mut a, &ctx, "ui.click", json!({"x": x, "y": y}));
    as_agent(&mut a, &ctx, "ui.press", json!({"x": x, "y": y}));
    let (_, tree) = as_agent(&mut a, &ctx, "ui.release", json!({"x": x, "y": y}));
    assert!(spot(&tree, "Yes").is_some(), "the question is still open");
    let (v, _) = as_agent(&mut a, &ctx, "engine.execute", json!({"command": "ui.assistantReply", "params": {"id": id}}));
    assert_eq!(v["result"], json!({"choice": null, "open": true}), "{v}");
    // The user's own click answers it.
    user_click(&mut a, &ctx, spot(&tree, "No").expect("still asking"));
    assert_eq!(a.run("ui.assistantReply", json!({"id": id})).unwrap(), json!({"choice": "No", "open": false}));

    a.run("ui.assistantMute", json!({"value": true})).unwrap();
    // Nib's right-click menu.
    let tree = control_frame(&mut a, &ctx);
    let (x, y) = spot(&tree, "Nib, your assistant. Drag to move; right-click for options.").expect("Nib is drawn");
    let (_, tree) = as_agent(&mut a, &ctx, "ui.click", json!({"x": x, "y": y, "button": "right"}));
    let (mx, my) = spot(&tree, "Mute agent messages").expect("the menu opened");
    as_agent(&mut a, &ctx, "ui.click", json!({"x": mx, "y": my}));
    assert!(a.ui.assistant.agent_messages_muted, "Nib's menu ignored the faked click");
    // File › Options › General: a click on the box, then Tab to it and Space.
    let (_, tree) =
        as_agent(&mut a, &ctx, "engine.execute", json!({"command": "ui.backstage", "params": {"value": true, "page": "options", "pane": "general"}}));
    let (bx, by) = spot(&tree, "Mute agent messages").expect("Options shows the box");
    as_agent(&mut a, &ctx, "ui.click", json!({"x": bx, "y": by}));
    assert!(a.ui.assistant.agent_messages_muted, "Options ignored the faked click");
    let target = tree.nodes.iter().find(|(_, n)| n.label() == Some("Mute agent messages")).map(|(id, _)| *id);
    let mut reached = false;
    for _ in 0..300 {
        let (_, tree) = as_agent(&mut a, &ctx, "ui.key", json!({"key": "Tab"}));
        if Some(tree.focus) == target {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab reaches the box");
    as_agent(&mut a, &ctx, "ui.key", json!({"key": "Space"}));
    as_agent(&mut a, &ctx, "ui.key", json!({"key": "Enter"}));
    assert!(a.ui.assistant.agent_messages_muted, "Options ignored the faked keys");
    // The user's own click on the same box unmutes.
    user_click(&mut a, &ctx, (bx, by));
    assert!(!a.ui.assistant.agent_messages_muted, "the user can unmute");
}

#[test]
fn nib_speaks_every_interface_language() {
    let mut missing = Vec::new();
    for lang in crate::i18n::Lang::all().filter(|l| *l != crate::i18n::Lang::EN) {
        for s in engine::texts().chain(UI_TEXTS) {
            if !crate::i18n::has(lang, s) {
                missing.push(format!("{}: {s:?}", lang.code()));
            }
        }
    }
    assert!(missing.is_empty(), "{} Nib strings have no translation:\n{}", missing.len(), missing.join("\n"));
}

/// Nib's own drawing, in the pseudo-language: every word it shows (the Introduction, Tips with
/// their buttons, Sayings, agent messages, its tooltip and right-click menu) went through
/// translation. Only its name, and what an agent wrote, may be drawn as is.
#[test]
fn nib_draws_nothing_untranslated() {
    fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 900.0));
    let mut time = 0.0;
    let mut frames = |ctx: &egui::Context, a: &mut WordApp, steps: Vec<Vec<egui::Event>>| {
        let mut drawn = Vec::new();
        let n = steps.len() + 2;
        let mut steps = steps.into_iter();
        for _ in 0..n {
            time += 0.05;
            let input =
                egui::RawInput { screen_rect: Some(screen), time: Some(time), events: steps.next().unwrap_or_default(), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(a, ui.ctx()));
            drawn.clear();
            for s in &out.shapes {
                texts(&s.shape, &mut drawn);
            }
            out.textures_delta.clear();
        }
        drawn
    };
    crate::i18n::pseudo::set(true);
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    let mut a = app();
    a.ui.assistant.visible = true;
    let mut offers = vec![];
    let mut s = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Dear Sam,"})).unwrap();
    let typed = Observation { recent: vec![Used { id: "text.insert".into(), source: Source::Keyboard, at: 0.0 }], ..Default::default() };
    offers.extend(engine::tips(&s, &typed));
    let used = |source| Used { id: "format.bold".into(), source, at: 0.0 };
    offers.extend(engine::tips(&s, &Observation { recent: vec![used(Source::Ribbon); 3], ..Default::default() }));
    offers.push(engine::tip_of_the_day(&s, 0));
    offers.extend(engine::sayings(&s, &Observation { deleted: 500, ..Default::default() }));
    let mut contents: Vec<Content> = offers.into_iter().map(Content::Offer).collect();
    contents.push(Content::Intro);
    contents.push(Content::Agent { id: 1, from: String::new(), text: "42?".into(), choices: vec![] });
    contents.push(Content::Agent { id: 2, from: "Helper".into(), text: "42?".into(), choices: vec!["Yes".into()] });
    let mut found = Vec::new();
    let agent_wrote = ["Helper", "42?", "Yes"];
    for c in contents {
        a.assistant.open(c.clone(), Pose::Idle);
        let drawn = frames(&ctx, &mut a, vec![]);
        found.extend(drawn.into_iter().map(|t| (format!("{c:?}"), t)));
    }
    a.assistant.balloon = None;
    let at = nib_rect(&a, screen).center();
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
    let hovered = frames(&ctx, &mut a, vec![vec![egui::Event::PointerMoved(at)]; 3]);
    found.extend(hovered.into_iter().map(|t| ("hover".to_string(), t)));
    let menu = frames(&ctx, &mut a, vec![vec![press(true)], vec![press(false)]]);
    assert!(menu.iter().any(|t| t.contains("Mute agent messages")), "the menu opened: {menu:?}");
    found.extend(menu.into_iter().map(|t| ("menu".to_string(), t)));
    crate::i18n::pseudo::set(false);
    let english = |t: &str| {
        let plain = crate::i18n::pseudo::unmarked(t);
        let plain = plain.trim();
        !plain.is_empty() && plain != "Nib" && !agent_wrote.contains(&plain) && plain.chars().any(|c| c.is_ascii_alphabetic())
    };
    let bad: Vec<_> = found.into_iter().filter(|(_, t)| english(t)).collect();
    assert!(bad.is_empty(), "drawn without translation: {bad:#?}");
}

#[test]
fn idle_nib_doesnt_keep_the_window_busy() {
    let mut a = chatty();
    let fast = 1.0 / 30.0;
    // Idle: no frames until the next blink starts or ends, and never more than a second apart.
    for now in [0.5, 1.0, 2.0, 4.0, 4.25, 100.0] {
        a.assistant.now = now;
        let wait = repaint_after(&a.assistant, true);
        assert!(wait > fast && wait <= 1.0, "idle at {now}: {wait}");
    }
    // Just before a blink: a frame for it, not a second late.
    a.assistant.now = 4.1;
    assert!((repaint_after(&a.assistant, true) - 0.1).abs() < 0.01);
    // A pose that moves needs frames often until it settles; animations off never do.
    a.assistant.set_pose(Pose::Wave);
    assert_eq!(repaint_after(&a.assistant, true), fast);
    assert_eq!(repaint_after(&a.assistant, false), 1.0);
    a.assistant.set_pose(Pose::Doze);
    assert_eq!(repaint_after(&a.assistant, true), fast);
    // Hostile clocks still give a sane wait.
    a.assistant.set_pose(Pose::Idle);
    for now in [f64::NAN, f64::INFINITY, -5.0] {
        a.assistant.now = now;
        let wait = repaint_after(&a.assistant, true);
        assert!(wait.is_finite() && (fast..=1.0).contains(&wait), "{now}: {wait}");
    }
}

/// A screen reader reaches Nib (a button named by its tooltip) and hears the Balloon: its text
/// is a polite live region, announced when it appears (#959 review).
#[test]
fn screen_readers_reach_nib_and_hear_the_balloon() {
    use egui_kittest::kittest::NodeT as _;
    let mut a = app();
    a.run("ui.assistant", json!({"show": true})).unwrap();
    a.run("ui.assistantSay", json!({"text": "Still here!"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    let nib = h.get_by_label("Nib, your assistant. Drag to move; right-click for options.");
    assert_eq!(nib.accesskit_node().role(), egui::accesskit::Role::Button);
    let said = h.get_by_label("Still here!");
    assert_eq!(said.accesskit_node().live(), egui::accesskit::Live::Polite);
}

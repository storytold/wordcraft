//! The pane end to end with a fake host and a scripted model: onboarding, a turn that writes
//! through the UI thread, one undo step per turn, confirmations, the margin icon.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use wordcraft_assistant::{Scripted, text_reply, tool_reply};
use wordcraft_engine::Session;

use super::*;
use crate::Services;

struct FakeHost {
    key: AtomicBool,
    /// Replies stream in pieces.
    stream: AtomicBool,
    replies: Mutex<Vec<Value>>,
    events: Mutex<Vec<AssistantEvent>>,
    /// The agreement the host asks for, until it is given.
    consent: Mutex<Option<Consent>>,
    /// The model endpoint was asked for: something was about to be sent.
    asked_provider: AtomicBool,
}

impl FakeHost {
    fn new(replies: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            key: AtomicBool::new(false),
            stream: AtomicBool::new(false),
            replies: Mutex::new(replies),
            events: Mutex::default(),
            consent: Mutex::default(),
            asked_provider: AtomicBool::new(false),
        })
    }
    fn with_key(replies: Vec<Value>) -> Arc<Self> {
        let h = Self::new(replies);
        h.key.store(true, Ordering::SeqCst);
        h
    }
}

impl AssistantHost for FakeHost {
    fn service(&self) -> Service {
        Service { name: "Fake Accounts".into(), url: "https://accounts.example/top-up/".into(), key_prefix: "ck-fant-".into() }
    }
    fn has_key(&self) -> bool {
        self.key.load(Ordering::SeqCst)
    }
    fn connect(&self, key: String) -> Result<Account, String> {
        if !key.starts_with("ck-fant-") {
            return Err("not an account key".into());
        }
        if self.key.swap(true, Ordering::SeqCst) {
            return Err("a key is already set".into());
        }
        Ok(Account { conto_id: "conto-1".into(), model: "m".into(), remaining_usd: Some(0.5) })
    }
    fn change_key(&self, key: String) -> Result<Account, String> {
        if !key.starts_with("ck-fant-") {
            return Err("not an account key".into());
        }
        Ok(Account { conto_id: "conto-2".into(), model: "m2".into(), remaining_usd: Some(9.0) })
    }
    fn account(&self) -> Option<Account> {
        self.has_key().then(|| Account { conto_id: "conto-1".into(), model: "m".into(), remaining_usd: Some(0.5) })
    }
    fn models(&self) -> Result<Vec<ModelInfo>, String> {
        Ok(["a/one", "b/two", "b/two:batch"]
            .iter()
            .map(|id| ModelInfo { id: id.to_string(), name: None, output_usd_per_million: Some(1.0) })
            .collect())
    }
    fn provider(&self) -> Result<(Box<dyn Provider + Send>, String), String> {
        self.asked_provider.store(true, Ordering::SeqCst);
        let replies = std::mem::take(&mut *self.replies.lock().unwrap());
        let scripted = if self.stream.load(Ordering::SeqCst) { Scripted::streaming(replies) } else { Scripted::new(replies) };
        Ok((Box::new(scripted), "m".into()))
    }
    fn balance(&self) -> Option<f64> {
        Some(0.03)
    }
    fn event(&self, event: &AssistantEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
    fn consent(&self) -> Option<Consent> {
        self.consent.lock().unwrap().clone()
    }
    fn agree(&self) -> Result<(), String> {
        *self.consent.lock().unwrap() = None;
        Ok(())
    }
}

#[test]
fn nothing_is_sent_before_the_user_agrees() {
    let host = FakeHost::with_key(vec![text_reply("Ciao.")]);
    *host.consent.lock().unwrap() = Some(Consent { service: "Fake Accounts".into(), destination: "accounts.example".into() });
    let mut app = app_with(host.clone());
    let ctx = egui::Context::default();
    app.run("ui.assistant", json!({})).unwrap();

    // Declined (the pump answers no): no endpoint, the conversation untouched, the reason said.
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    pump(&mut app, &ctx);
    assert!(!host.asked_provider.load(Ordering::SeqCst));
    assert!(app.assistant.messages.iter().any(|m| m.role == Role::Error && m.text.contains("agreement")));
    assert!(host.consent().is_some());

    // Agreed: the question names the service and where the data goes, then the turn runs.
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    wait(&mut app, &ctx, |a| a.assistant.confirm.is_some());
    let (question, reply) = app.assistant.confirm.take().unwrap();
    assert!(question.contains("Fake Accounts") && question.contains("accounts.example"), "{question}");
    reply.send(true).unwrap();
    pump(&mut app, &ctx);
    assert!(host.asked_provider.load(Ordering::SeqCst));
    assert!(host.consent().is_none());
    assert!(app.assistant.messages.iter().any(|m| m.role == Role::Assistant && m.text == "Ciao."));
}

fn app_with(host: Arc<FakeHost>) -> WordApp {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.assistant.set_host(host);
    app
}

/// Run frames until the worker is done.
fn pump(app: &mut WordApp, ctx: &egui::Context) {
    for _ in 0..2000 {
        tick(app, ctx);
        if !app.assistant.busy {
            return;
        }
        if let Some((_, reply)) = app.assistant.confirm.take() {
            let _ = reply.send(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the assistant did not finish");
}

fn wait(app: &mut WordApp, ctx: &egui::Context, done: impl Fn(&WordApp) -> bool) {
    for _ in 0..400 {
        tick(app, ctx);
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the worker did not answer");
}

fn text(app: &WordApp) -> String {
    app.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
}

#[test]
fn onboarding_then_a_turn_writes_as_one_undo_step() {
    let host = FakeHost::new(vec![
        tool_reply(&[("a", "type_text", json!({"text": "Title"}))]),
        tool_reply(&[
            ("b", "execute", json!({"command": "para.style", "params": {"style": "Heading 1"}})),
            ("c", "execute", json!({"command": "text.newParagraph"})),
        ]),
        tool_reply(&[("d", "get_text", json!({}))]),
        tool_reply(&[("e", "type_text", json!({"text": "First line.\nSecond line.", "paragraphs": true}))]),
        text_reply("Fatto."),
    ]);
    let mut app = app_with(host.clone());
    let ctx = egui::Context::default();
    app.run("ui.assistant", json!({})).unwrap();
    assert!(app.assistant.open);

    // No key: the onboarding stores one and tells the host which account it belongs to.
    app.assistant.key_input = "  ck-fant-secret  ".into();
    connect(&mut app);
    pump(&mut app, &ctx);
    assert!(host.has_key());
    assert_eq!(*host.events.lock().unwrap(), [AssistantEvent::KeyRegistered { conto_id: "conto-1".into() }]);
    assert!(app.assistant.key_input.is_empty());
    assert!(app.assistant.messages.iter().all(|m| !m.text.contains("ck-fant-secret")), "the key is never shown");

    // A turn writes through the UI thread.
    let before = app.session.undo_depth();
    app.assistant.input = "scrivi un titolo e due righe".into();
    send(&mut app, &ctx);
    pump(&mut app, &ctx);
    assert_eq!(text(&app), "Title\nFirst line.\nSecond line.");
    assert_eq!(app.assistant.balance, Some(0.03));
    assert!(app.assistant.messages.iter().any(|m| m.role == Role::Assistant && m.text == "Fatto."));
    // The chat keeps what was said, not what was run: no command, no JSON, no status line left.
    assert!(app.assistant.messages.iter().all(|m| matches!(m.role, Role::User | Role::Assistant | Role::Link)), "{:?}", app.assistant.messages);
    assert!(app.assistant.messages.iter().all(|m| !m.text.contains("type_text") && !m.text.contains('{')));
    assert_eq!(app.assistant.activity, None);
    assert_eq!(app.assistant.turn_undo_depth, None);

    // One undo takes the whole turn back; typing afterwards is a new step.
    assert_eq!(app.session.undo_depth(), before + 1, "{:?}", app.session.undo_labels());
    app.run("text.insert", json!({"text": " more"})).unwrap();
    assert_eq!(app.session.undo_depth(), before + 2);
    app.run("edit.undo", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(text(&app), "");
}

#[test]
fn the_prompt_carries_where_the_user_is_in_the_document() {
    let host = FakeHost::with_key(vec![tool_reply(&[("a", "type_text", json!({"text": "nimble"}))]), text_reply("ok")]);
    let mut app = app_with(host);
    let ctx = egui::Context::default();
    assert_eq!(context(&app).as_deref(), Some("Context: the caret is on an empty paragraph; type_text inserts there."));
    app.run("text.insert", json!({"text": "The quick fox."})).unwrap();
    let c = context(&app).unwrap();
    assert!(c.starts_with("Context: nothing is selected") && c.contains("«The quick fox.»"), "{c}");
    app.run("select.text", json!({"text": "quick"})).unwrap();
    let c = context(&app).unwrap();
    assert!(c.contains("type_text replaces it") && c.ends_with("«quick»"), "{c}");
    // The selection is replaced, and the model saw the context after the prompt.
    app.run("ui.assistant.ask", json!({"prompt": "make it vivid"})).unwrap();
    assert!(app.assistant.open && app.assistant.busy);
    pump(&mut app, &ctx);
    assert_eq!(text(&app), "The nimble fox.");
    let conv = app.assistant.conversation.as_ref().unwrap();
    let user = conv.agent.messages()[1]["content"].as_str().unwrap();
    assert!(user.starts_with("make it vivid\n\nContext:") && user.ends_with("«quick»"), "{user}");
    // The chat shows the prompt alone.
    assert!(app.assistant.messages.iter().any(|m| m.role == Role::User && m.text == "make it vivid"));
}

#[test]
fn the_draft_box_asks_for_text_in_the_document() {
    let host = FakeHost::with_key(vec![tool_reply(&[("a", "type_text", json!({"text": "Dear all,"}))]), text_reply("Ecco la bozza.")]);
    let mut app = app_with(host);
    let ctx = egui::Context::default();
    assert!(ask(&mut app, &ctx, "a greeting", true).is_ok());
    pump(&mut app, &ctx);
    assert_eq!(text(&app), "Dear all,");
    let conv = app.assistant.conversation.as_ref().unwrap();
    let user = conv.agent.messages()[1]["content"].as_str().unwrap();
    assert!(user.contains("Draft box"), "{user}");
    // A second request while one runs is refused, not queued.
    app.assistant.busy = true;
    assert!(ask(&mut app, &ctx, "again", false).is_err());
    app.assistant.busy = false;
    // An empty one too.
    assert!(ask(&mut app, &ctx, "   ", false).is_err());
    assert!(app.run("ui.assistant.ask", json!({})).is_err());
}

#[test]
fn a_streaming_reply_fills_a_bubble_and_becomes_the_message() {
    let mut a = Assistant::default();
    let reg = wordcraft_engine::cmd::registry();
    a.busy = true;
    update(&mut a, Update::StreamStart, &reg);
    update(&mut a, Update::StreamText("Hel".into()), &reg);
    update(&mut a, Update::StreamText("lo".into()), &reg);
    assert_eq!(a.streaming.as_deref(), Some("Hello"));
    assert!(a.messages.is_empty());
    // A retry starts the bubble over.
    update(&mut a, Update::StreamStart, &reg);
    assert_eq!(a.streaming.as_deref(), Some(""));
    update(&mut a, Update::StreamText("Hello again".into()), &reg);
    // The whole text replaces the bubble.
    update(&mut a, Update::Text("Hello again".into()), &reg);
    assert!(a.streaming.is_none());
    assert_eq!(a.messages.len(), 1);
    assert_eq!(a.messages[0].text, "Hello again");
    // End to end with a streaming endpoint: the pieces reach the pane and the message is whole.
    let host = FakeHost::with_key(vec![text_reply("Streamed reply.")]);
    host.stream.store(true, Ordering::SeqCst);
    let mut app = app_with(host);
    let ctx = egui::Context::default();
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    pump(&mut app, &ctx);
    assert!(app.assistant.streaming.is_none());
    assert!(app.assistant.messages.iter().any(|m| m.role == Role::Assistant && m.text == "Streamed reply."));
}

#[test]
fn the_status_line_reads_as_plain_words_and_goes_away_when_the_turn_ends() {
    let mut a = Assistant::default();
    let reg = wordcraft_engine::cmd::registry();
    let text = |a: &Assistant| a.activity.as_ref().map(|s| s.text.clone());
    let step = |a: &mut Assistant, name: &str, args: Value| update(a, Update::Step { name: name.into(), args }, &reg);
    a.busy = true;
    step(&mut a, "execute", json!({"command": "format.bold", "params": {}}));
    assert_eq!(text(&a).as_deref(), Some("Running Bold…"));
    step(&mut a, "execute", json!({"command": "no.such.command"}));
    assert_eq!(text(&a).as_deref(), Some("Working…"));
    step(&mut a, "type_text", json!({"text": "secret draft"}));
    assert_eq!(text(&a).as_deref(), Some("Writing…"));
    step(&mut a, "get_text", json!({}));
    assert_eq!(text(&a).as_deref(), Some("Reading the document…"));
    // A refused command marks the line, and the error stays out of the chat.
    update(&mut a, Update::StepFailed, &reg);
    assert_eq!(a.activity, Some(Activity { text: "Reading the document…".into(), failed: true }));
    assert!(a.messages.is_empty());
    // Confirmations ask in words, not JSON.
    update(&mut a, Update::Confirm { name: "save_document".into(), args: json!({"path": " C:/docs/letter.docx "}), reply: channel().0 }, &reg);
    assert_eq!(a.confirm.as_ref().map(|c| c.0.as_str()), Some("The assistant wants to save the document to C:/docs/letter.docx."));
    update(&mut a, Update::Confirm { name: "new_document".into(), args: json!({}), reply: channel().0 }, &reg);
    assert_eq!(a.confirm.as_ref().map(|c| c.0.as_str()), Some("The assistant wants to start a new document: the current one will be closed."));
    assert!(update(&mut a, Update::Done { conversation: None, error: None, balance: None, stopped: false }, &reg));
    assert!(!a.busy);
    assert_eq!(a.activity, None);
    assert!(a.confirm.is_none());
    assert!(a.messages.is_empty(), "nothing that was run is kept in the chat");
    // Out of steps: one short notice, in words.
    assert!(update(&mut a, Update::Done { conversation: None, error: None, balance: None, stopped: true }, &reg));
    assert_eq!(a.messages.len(), 1);
    assert!(a.messages[0].role == Role::Notice && a.messages[0].text.contains("ran out of steps"));
}

#[test]
fn without_a_key_the_greeting_links_to_the_account_site_and_the_key_stays_secret() {
    let mut app = app_with(FakeHost::new(vec![]));
    open(&mut app);
    // The service's name, key prefix and site come from the host, never from the pane.
    let greeting = app.assistant.messages.iter().find(|m| m.text.contains("To start")).expect("the onboarding text");
    assert!(greeting.text.contains("Fake Accounts") && greeting.text.contains("ck-fant-"), "{}", greeting.text);
    let link = app.assistant.messages.iter().find(|m| m.role == Role::Link).expect("a link to the account site");
    assert!(link.text.ends_with("accounts.example/top-up"), "{}", link.text);
    // Asking before connecting says so, and nothing is sent.
    let ctx = egui::Context::default();
    let e = ask(&mut app, &ctx, "ciao", false).unwrap_err();
    assert!(e.contains("Connect your account"), "{e}");
    assert!(!app.assistant.busy);
    // The pane renders with the link and the card.
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
    out.textures_delta.clear();
}

#[test]
fn the_credit_is_read_again_on_request_and_a_fresh_start_keeps_only_the_greeting() {
    let host = FakeHost::with_key(vec![text_reply("ok")]);
    let mut app = app_with(host);
    let ctx = egui::Context::default();
    app.assistant.balance = Some(6.2);
    refresh_balance(&mut app.assistant, &ctx);
    assert!(app.assistant.balance_busy);
    wait(&mut app, &ctx, |a| !a.assistant.balance_busy);
    assert_eq!(app.assistant.balance, Some(0.03));
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    pump(&mut app, &ctx);
    assert!(app.assistant.conversation.is_some());
    assert!(app.assistant.messages.len() > 1);
    // The broom only asks; clearing is the answer.
    app.assistant.clear_asked = true;
    new_conversation(&mut app.assistant);
    assert!(!app.assistant.clear_asked);
    assert!(app.assistant.conversation.is_none());
    assert_eq!(app.assistant.messages.len(), 1, "{:?}", app.assistant.messages);
    assert_eq!(app.assistant.messages[0].role, Role::Assistant);
}

#[test]
fn a_declined_save_writes_nothing_and_the_conversation_continues() {
    let host = FakeHost::with_key(vec![
        tool_reply(&[("a", "save_document", json!({"path": "never-written.docx"}))]),
        text_reply("Va bene, non salvo."),
        text_reply("Ciao di nuovo."),
    ]);
    let mut app = app_with(host);
    let ctx = egui::Context::default();
    app.assistant.input = "salva".into();
    send(&mut app, &ctx);
    pump(&mut app, &ctx);
    assert!(!std::path::Path::new("never-written.docx").exists());
    assert!(app.assistant.messages.iter().any(|m| m.text.contains("Declined")));
    // The same conversation (and model endpoint) carries on.
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    pump(&mut app, &ctx);
    assert!(app.assistant.messages.iter().any(|m| m.text == "Ciao di nuovo."));
}

#[test]
fn errors_are_shown_not_raised() {
    // The scripted model runs out of replies: the provider fails mid-conversation.
    let host = FakeHost::with_key(vec![]);
    let mut app = app_with(host);
    let ctx = egui::Context::default();
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    pump(&mut app, &ctx);
    assert!(app.assistant.messages.iter().any(|m| m.role == Role::Error));
    // A bad key is refused and nothing is stored.
    let host = FakeHost::new(vec![]);
    let mut app = app_with(host.clone());
    app.assistant.key_input = "sk-or-wrong".into();
    connect(&mut app);
    pump(&mut app, &ctx);
    assert!(!host.has_key());
    assert!(app.assistant.messages.iter().any(|m| m.role == Role::Error));
}

#[test]
fn without_a_host_the_pane_says_so_and_the_icon_stays_away() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    assert!(!app.assistant.available());
    assert!(!icon_visible(&app));
    let ctx = egui::Context::default();
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    assert!(!app.assistant.busy);
    let r = app.run("ui.assistant", json!({"value": true})).unwrap();
    assert_eq!(r["available"], false);
    assert!(app.assistant.open);
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
    out.textures_delta.clear();
    app.run("ui.assistant", json!({"value": false})).unwrap();
    assert!(!app.assistant.open);
}

#[test]
fn language_and_model_choices_reach_the_agent_and_persist() {
    let host = FakeHost::with_key(vec![text_reply("Ciao."), text_reply("Hello.")]);
    let mut app = app_with(host);
    let ctx = egui::Context::default();
    app.assistant.set_language(Language::Interface);
    app.assistant.set_model(Some("b/two".into()));
    // A batch variant can't answer a chat: refused, the choice falls back to the suggestion.
    app.assistant.set_model(Some("b/two:batch".into()));
    assert_eq!(app.assistant.model, None);
    app.assistant.set_model(Some("b/two".into()));
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    pump(&mut app, &ctx);
    let agent = &app.assistant.conversation.as_ref().unwrap().agent;
    assert_eq!(agent.config().model, "b/two");
    assert!(agent.messages()[0]["content"].as_str().unwrap().contains("Always reply in English"));
    // Changed mid-conversation: the next turn follows, history kept.
    app.assistant.set_language(Language::Auto);
    app.assistant.set_model(Some("a/one".into()));
    let agent = &app.assistant.conversation.as_ref().unwrap().agent;
    assert!(!agent.messages()[0]["content"].as_str().unwrap().contains("Always reply"));
    assert_eq!(agent.config().model, "a/one");
    assert!(agent.messages().len() > 1);
    // Kept across restarts; defaults are not written.
    app.assistant.set_language(Language::English);
    let prefs = serde_json::to_string(&app.prefs()).unwrap();
    assert!(prefs.contains("\"assistantLanguage\":\"en\"") && prefs.contains("\"assistantModel\":\"a/one\""), "{prefs}");
    let mut again = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    again.apply_prefs(crate::UiState::from_json(prefs.as_bytes()).unwrap());
    assert_eq!((again.assistant.language, again.assistant.model.as_deref()), (Language::English, Some("a/one")));
    let junk = crate::UiState::from_json(br#"{"assistantLanguage": "klingon", "assistantModel": "x/y:batch"}"#).unwrap();
    again.apply_prefs(junk);
    assert_eq!((again.assistant.language, again.assistant.model), (Language::Auto, None));
}

#[test]
fn the_model_list_leaves_out_batch_variants() {
    let host = FakeHost::with_key(vec![]);
    let mut app = app_with(host);
    let ctx = egui::Context::default();
    request_models(&mut app.assistant, &ctx);
    wait(&mut app, &ctx, |a| a.assistant.settings.models.is_some());
    let ids: Vec<String> = match &app.assistant.settings.models {
        Some(Ok(list)) => list.iter().map(|m| m.id.clone()).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(ids, ["a/one", "b/two"]);
}

#[test]
fn changing_the_key_switches_account_and_starts_a_new_conversation() {
    let host = FakeHost::with_key(vec![text_reply("ok")]);
    let mut app = app_with(host.clone());
    let ctx = egui::Context::default();
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    pump(&mut app, &ctx);
    assert!(app.assistant.conversation.is_some());
    app.assistant.settings.new_key = "ck-fant-new".into();
    change_key(&mut app);
    wait(&mut app, &ctx, |a| !a.assistant.settings.busy);
    assert!(app.assistant.settings.new_key.is_empty());
    assert!(matches!(app.assistant.settings.result, Some(Ok(_))));
    assert!(app.assistant.conversation.is_none());
    assert_eq!(app.assistant.account.as_ref().map(|a| a.conto_id.as_str()), Some("conto-2"));
    assert_eq!(host.events.lock().unwrap().last(), Some(&AssistantEvent::KeyChanged { conto_id: "conto-2".into() }));
    assert!(app.assistant.messages.iter().all(|m| !m.text.contains("ck-fant-new")), "the key is never shown");
    // A bad key is refused and the old one stays.
    app.assistant.settings.new_key = "sk-or-x".into();
    change_key(&mut app);
    wait(&mut app, &ctx, |a| !a.assistant.settings.busy);
    assert!(matches!(app.assistant.settings.result, Some(Err(_))));
}

#[test]
fn the_margin_icon_follows_the_caret_and_stays_inside_the_canvas() {
    let vp = Rect::from_min_size(egui::pos2(100.0, 50.0), vec2(1200.0, 700.0));
    let line = Rect::from_min_size(egui::pos2(400.0, 300.0), vec2(1.0, 16.0));
    // In the margin, beside the line, left of the body.
    let r = icon_rect(line, 350.0, vp).unwrap();
    assert!((r.max.x - (350.0 - ICON_GAP)).abs() < 1e-3 && (r.center().y - 308.0).abs() < 1e-3);
    // Zoomed in with the margin off screen: pushed onto the canvas's left edge, never hidden.
    let r = icon_rect(line, 90.0, vp).unwrap();
    assert!((r.min.x - 102.0).abs() < 1e-3);
    // A line scrolled out of view keeps the icon at the viewport's edge, so it is always there.
    let r = icon_rect(Rect::from_min_size(egui::pos2(400.0, 10.0), vec2(1.0, 16.0)), 350.0, vp).unwrap();
    assert!((r.min.y - (vp.top() + ICON_GAP)).abs() < 1e-3, "{r:?}");
    let r = icon_rect(Rect::from_min_size(egui::pos2(400.0, 900.0), vec2(1.0, 16.0)), 350.0, vp).unwrap();
    assert!((r.max.y - (vp.bottom() - ICON_GAP)).abs() < 1e-3, "{r:?}");
    // Degenerate inputs are rejected, not crashed.
    assert!(icon_rect(Rect::from_min_size(egui::pos2(f32::NAN, 1.0), vec2(1.0, 1.0)), 350.0, vp).is_none());
    assert!(icon_rect(line, f32::INFINITY, vp).is_none());
    assert!(icon_rect(line, 350.0, Rect::EVERYTHING).is_none());

    // The icon shows with a host and a document, for body text only.
    let mut app = app_with(FakeHost::with_key(vec![]));
    assert!(icon_visible(&app));
    app.run("view.readMode", json!({})).unwrap();
    assert!(!icon_visible(&app));
    app.run("view.printLayout", json!({})).unwrap();
    assert!(icon_visible(&app));
    app.ui.backstage = true;
    assert!(!icon_visible(&app));
    app.ui.backstage = false;
    app.run("insert.editHeader", json!({})).unwrap();
    assert!(!icon_visible(&app), "headers keep their margins");
    app.run("insert.closeHeader", json!({})).unwrap();
    assert!(icon_visible(&app));
    // Drawn over a real document, the anchor sits left of the body on the caret's line.
    let ctx = egui::Context::default();
    app.run("text.insert", json!({"text": "Hello margin"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        h.step();
    }
    let (line, body_left) = crate::canvas::assistant_anchor(h.state_mut()).expect("the caret is on screen");
    let canvas = h.state().canvas.canvas_rect.unwrap();
    let r = icon_rect(line, body_left, canvas).expect("the icon fits");
    assert!(r.max.x < body_left && r.min.x >= canvas.min.x, "{r:?} body {body_left} canvas {canvas:?}");
    assert!(line.min.y >= canvas.min.y && r.min.y < line.max.y && r.max.y > line.min.y);
    drop(ctx);
}

/// A file command through `execute` or `batch` asks in words: the command and the file, never JSON.
#[test]
fn the_card_for_a_file_command_names_the_command_and_the_file() {
    let app = app_with(FakeHost::with_key(vec![]));
    let r = &app.session.registry;
    let save = confirm_text(r, "execute", &json!({"command": "file.saveAs", "params": {"path": "/tmp/out.docx"}}));
    assert!(save.contains("/tmp/out.docx") && !save.contains('{'), "{save}");
    let open = confirm_text(r, "execute", &json!({"command": "file.open", "params": {"path": "/tmp/in.docx"}}));
    assert!(open.contains("/tmp/in.docx"), "{open}");
    let batch = confirm_text(r, "batch", &json!({"commands": [{"command": "format.bold"}, {"command": "file.saveAs"}]}));
    assert!(batch.contains("Save As") && !batch.contains("Bold") && !batch.contains("file."), "{batch}");
    let unknown = confirm_text(r, "execute", &json!({"command": "file.nothing"}));
    assert!(unknown.contains("file.nothing"), "{unknown}");
}

/// A model chosen while a turn runs is the one the next turn uses; going back to the suggestion
/// during a turn starts the next one afresh.
#[test]
fn a_model_chosen_during_a_turn_applies_when_it_returns() {
    let host = FakeHost::with_key(vec![text_reply("uno"), text_reply("due")]);
    let mut app = app_with(host);
    let ctx = egui::Context::default();
    app.run("ui.assistant", json!({})).unwrap();
    app.assistant.input = "ciao".into();
    send(&mut app, &ctx);
    assert!(app.assistant.busy);
    app.assistant.set_model(Some("b/two".into()));
    pump(&mut app, &ctx);
    let model = app.assistant.conversation.as_ref().map(|c| c.agent.config().model.clone());
    assert_eq!(model.as_deref(), Some("b/two"));

    app.assistant.input = "ancora".into();
    send(&mut app, &ctx);
    app.assistant.set_model(None);
    pump(&mut app, &ctx);
    assert!(app.assistant.conversation.is_none(), "the next turn asks the account again");
}

/// A tool request the agent gave up on (no answer in time) never runs later; one the UI thread
/// already took can no longer be given up on, so its answer is waited for.
#[test]
fn a_tool_request_given_up_on_never_runs_later() {
    let mut app = app_with(FakeHost::with_key(vec![]));
    let ctx = egui::Context::default();
    let insert = json!({"command": "text.insert", "params": {"text": "late"}});
    let (req, answer) = ControlRequest::new("engine.execute", insert.clone());
    assert!(req.give_up_handle().give_up());
    app.assistant.tools_tx.send(req).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(text(&app), "", "the document is untouched");
    assert!(answer.try_recv().is_err());

    let (req, answer) = ControlRequest::new("engine.execute", insert);
    let handle = req.give_up_handle();
    app.assistant.tools_tx.send(req).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(text(&app), "late");
    assert!(!handle.give_up(), "taken, so too late to give up");
    assert_eq!(answer.try_recv().unwrap()["ok"], true);
}

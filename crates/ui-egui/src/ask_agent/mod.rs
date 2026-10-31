//! Ask Agent: the person selects text and asks their own agent (connected over MCP or the control
//! channel) to act on it. `hub.rs` and `ui.rs` are shared with the other apps; this file is
//! WordCraft's part: the selection as JSON, one undo step per thread, authorship and the hold.

pub mod hub;
pub mod ui;

use egui::{Rect, vec2};
pub use hub::{AgentHub, HubError, Request, Snapshot, Status, Turn};
use serde_json::{Value, json};
use wordcraft_doc::Pos;

use crate::WordApp;

/// Opens Ask Agent at the selection (`Ctrl+Shift+O` / `⌘⇧O`).
pub const SHORTCUT: &str = "Mod+Shift+O";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    NoControlChannel,
    NoAgent,
}

impl Unavailable {
    pub fn shown(&self) -> String {
        match self {
            Self::NoControlChannel => tl!("Start WordCraft with --control to let an agent connect").to_string(),
            Self::NoAgent => tl!("No agent is listening: ask your agent to wait for requests").to_string(),
        }
    }
}

/// The open bubble: its current request and the thread of earlier turns.
pub struct Open {
    pub id: Option<u64>,
    pub anchor: Rect,
    pub turns: Vec<Turn>,
    asked: String,
    /// Where the text the agent was given starts, for its closing comment.
    target: Option<Pos>,
    /// Undo depth before the agent's first edit, once it has edited.
    edited_from: Option<usize>,
    commented: bool,
}

#[derive(Default)]
pub struct AskAgent {
    pub hub: Option<AgentHub>,
    pub open: Option<Open>,
    pub draft: String,
    pub picked: Option<String>,
    /// The visible selection on screen, set by the canvas each frame.
    pub selection: Option<Rect>,
    /// Set while a command from an agent runs, so the hold does not refuse it.
    agent_turn: bool,
}

impl AskAgent {
    pub fn unavailable(&self) -> Option<Unavailable> {
        let Some(hub) = &self.hub else { return Some(Unavailable::NoControlChannel) };
        hub.agents().is_empty().then_some(Unavailable::NoAgent)
    }
}

/// Run `command` from control connection `conn`: while a request is open only the agent working on
/// it edits; its edits join one undo step and are authored "Agent (<name>)".
pub fn as_agent(
    app: &mut WordApp,
    conn: Option<u64>,
    command: &str,
    p: &Value,
    run: impl FnOnce(&mut WordApp) -> Result<Value, String>,
) -> Result<Value, String> {
    let mutates = app.session.registry.get(command).is_some_and(|c| c.mutates);
    let (Some(conn), Some(hub), true) = (conn, app.ask_agent.hub.clone(), mutates) else { return run(app) };
    let request = p.get("request").and_then(Value::as_u64);
    hub.may_edit(conn, request).map_err(|e| e.to_string())?;
    let Some(id) = request else { return run(app) };
    let name = hub.snapshot(id).and_then(|s| s.agent).unwrap_or_default();
    let depth = app.session.undo_depth();
    let mine = app.ask_agent.open.as_mut().filter(|o| o.id == Some(id));
    let from = mine.as_ref().and_then(|o| o.edited_from);
    if from.is_some_and(|f| depth == f + 1) {
        app.session.join_next_undo();
    }
    let author = std::mem::replace(&mut app.session.author, format!("Agent ({name})"));
    app.ask_agent.agent_turn = true;
    let result = run(app);
    app.ask_agent.agent_turn = false;
    app.session.author = author;
    if from.is_none()
        && result.is_ok()
        && app.session.undo_depth() > depth
        && let Some(o) = app.ask_agent.open.as_mut().filter(|o| o.id == Some(id))
    {
        o.edited_from = Some(depth);
    }
    result
}

/// The text each agent is working on, in order, with the agent's colour.
pub fn held_ranges(app: &WordApp) -> Vec<(Pos, Pos, [u8; 3])> {
    held_by(app).into_iter().map(|(a, b, name)| (a, b, hub::colour(&name))).collect()
}

fn held_by(app: &WordApp) -> Vec<(Pos, Pos, String)> {
    let Some(hub) = &app.ask_agent.hub else { return vec![] };
    let range = |sel: &Value| {
        let pos = |k: &str| serde_json::from_value::<Pos>(sel[k].clone()).ok();
        let (a, b) = (pos("anchor")?, pos("focus")?);
        Some(if a <= b { (a, b) } else { (b, a) })
    };
    hub.held().into_iter().filter_map(|(sel, name)| range(&sel).map(|(a, b)| (a, b, name))).collect()
}

/// Why the person may not run `command` now: it would change text an agent is working on.
pub fn held(app: &WordApp, command: &str) -> Option<String> {
    if app.ask_agent.agent_turn || !app.session.registry.get(command).is_some_and(|c| c.mutates) {
        return None;
    }
    let (c, d) = app.session.sel.ordered();
    let (_, _, name) = held_by(app).into_iter().find(|(a, b, _)| c <= *b && *a <= d)?;
    Some(format!("{} ({name})", tl!("Held by Agent")))
}

fn ask(app: &mut WordApp, instruction: String) {
    let Some(hub) = app.ask_agent.hub.clone() else { return };
    let sel = &app.session.sel;
    let selection = json!({"text": app.session.selected_text(), "anchor": sel.anchor, "focus": sel.focus});
    let start = sel.ordered().0;
    let document = json!({"title": app.title_stem(), "saved": !app.session.dirty, "language": app.ui.language});
    let Some(open) = &mut app.ask_agent.open else { return };
    let history = open.turns.clone();
    let agent = app.ask_agent.picked.clone();
    open.asked = instruction.clone();
    open.target.get_or_insert(start);
    open.id = Some(hub.ask(Request { id: 0, instruction, history, selection, document, agent }));
    open.commented = false;
}

/// Send the instruction; after an answer it is a follow-up that carries the thread so far.
fn send(app: &mut WordApp, text: String) {
    if let (Some(hub), Some(open)) = (&app.ask_agent.hub, &mut app.ask_agent.open)
        && let Some(answer) = open.id.and_then(|id| hub.snapshot(id)).and_then(|s| s.answer)
    {
        open.turns.push(Turn { instruction: std::mem::take(&mut open.asked), answer });
        open.id.take().into_iter().for_each(|id| hub.close(id));
    }
    ask(app, text);
}

fn open_at_caret(app: &mut WordApp) {
    close(app);
    let caret = crate::canvas::caret_screen(app).map(|(p, h)| Rect::from_min_size(p, vec2(1.0, h)));
    let anchor = app.ask_agent.selection.or(caret).unwrap_or(Rect::from_min_size(egui::pos2(96.0, 160.0), vec2(1.0, 16.0)));
    app.ask_agent.open = Some(Open { id: None, anchor, turns: vec![], asked: String::new(), target: None, edited_from: None, commented: false });
}

fn close(app: &mut WordApp) {
    if let (Some(hub), Some(id)) = (&app.ask_agent.hub, app.ask_agent.open.take().and_then(|o| o.id)) {
        hub.close(id);
    }
}

/// `ui.askAgent {instruction?, close?}`: open the bubble at the selection, and send right away
/// when an instruction is given.
pub fn command(app: &mut WordApp, p: &Value) -> Result<Value, String> {
    if p.get("close").and_then(Value::as_bool) == Some(true) {
        close(app);
        return Ok(json!({"open": false}));
    }
    if let Some(why) = app.ask_agent.unavailable() {
        return Err(why.shown());
    }
    open_at_caret(app);
    if let Some(text) = p.get("instruction").and_then(Value::as_str) {
        ask(app, text.to_string());
    }
    Ok(json!({"open": true, "request": app.ask_agent.open.as_ref().and_then(|o| o.id)}))
}

/// Esc closes the bubble.
pub fn escape(app: &mut WordApp, ctx: &egui::Context) {
    if app.ask_agent.open.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close(app);
    }
}

/// Typing elsewhere in the document puts away a bubble that is not waiting on anything.
pub fn typing_resumed(app: &mut WordApp) {
    if app.ask_agent.open.as_ref().is_some_and(|o| o.id.is_none()) {
        app.ask_agent.open = None;
    }
}

/// The chip beside the selection, or the open bubble with its request.
pub fn show(app: &mut WordApp, ctx: &egui::Context, _viewport: Rect) {
    let Some(hub) = app.ask_agent.hub.clone() else { return };
    let Some(open) = &app.ask_agent.open else {
        if let Some(anchor) = app.ask_agent.selection
            && app.ask_agent.unavailable().is_none()
            && ui::chip(ctx, anchor)
        {
            open_at_caret(app);
        }
        return;
    };
    let (anchor, snapshot) = (open.anchor, open.id.and_then(|id| hub.snapshot(id)));
    if let Some(s) = snapshot.as_ref().filter(|s| s.status == Status::Answered) {
        comment_once(app, s);
    }
    let agents = hub.agents();
    let Some(open) = app.ask_agent.open.as_ref() else { return };
    let view = ui::View { snapshot: snapshot.as_ref(), agents: &agents, actions: QUICK_ACTIONS, history: &open.turns, instruction: &open.asked };
    match ui::bubble(ctx, anchor, &view, &mut app.ask_agent.draft, &mut app.ask_agent.picked) {
        Some(ui::Action::Send(text)) => send(app, text),
        Some(ui::Action::Undo) => {
            if let Some(from) = app.ask_agent.open.as_ref().and_then(|o| o.edited_from) {
                if app.session.undo_depth() == from + 1 {
                    app.session.undo();
                } else {
                    app.status(tl!("The document changed since the agent's edits; use Undo in the Edit menu"));
                }
            }
            close(app);
        }
        Some(ui::Action::Keep | ui::Action::Close) => close(app),
        None => {}
    }
}

/// The agent's answer, once, as a comment on what it was asked about, authored like its edits.
fn comment_once(app: &mut WordApp, s: &Snapshot) {
    let Some(open) = app.ask_agent.open.as_mut().filter(|o| !o.commented) else { return };
    open.commented = true;
    let target = open.target.clone();
    let (Some(answer), Some(name)) = (&s.answer, &s.agent) else { return };
    let from = open.edited_from;
    let depth = app.session.undo_depth();
    if from.is_some_and(|f| depth == f + 1) {
        app.session.join_next_undo();
    }
    let author = std::mem::replace(&mut app.session.author, format!("Agent ({name})"));
    let sel = app.session.sel.clone();
    if let Some(at) = target {
        (app.session.sel.anchor, app.session.sel.focus) = (at.clone(), at);
    }
    let text = format!("You: {}\n\nAgent: {answer}", open.asked);
    let result = app.session.run("review.newComment", &json!({"text": text}));
    if from.is_none() && result.is_ok() && app.session.undo_depth() > depth {
        open.edited_from = Some(depth);
    }
    app.session.sel = sel;
    app.session.author = author;
}

const QUICK_ACTIONS: &[ui::QuickAction] = &[
    ui::QuickAction { label: "Shorten", group: "", instruction: "Shorten the selected text, preserving its facts and meaning." },
    ui::QuickAction { label: "Fix grammar", group: "", instruction: "Correct grammar and spelling in the selected text, preserving its meaning." },
    ui::QuickAction {
        label: "Professional",
        group: "Rewrite",
        instruction: "Rewrite the selected text in a professional tone, preserving the facts.",
    },
    ui::QuickAction { label: "Friendly", group: "Rewrite", instruction: "Rewrite the selected text in a friendly tone, preserving the facts." },
    ui::QuickAction { label: "Concise", group: "Rewrite", instruction: "Rewrite the selected text concisely, preserving the facts." },
    ui::QuickAction { label: "English", group: "Translate", instruction: "Translate the selected text into English, preserving its formatting." },
    ui::QuickAction { label: "French", group: "Translate", instruction: "Translate the selected text into French, preserving its formatting." },
    ui::QuickAction {
        label: "Summarize",
        group: "",
        instruction: "Summarize the selected text and insert the summary below it, keeping the original text.",
    },
];

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_bubble_closes_any_previous_request_in_the_hub() {
        let (mut app, hub) = app();
        hub.listen(7, None, Duration::ZERO, || true);
        command(&mut app, &json!({"instruction": "First"})).unwrap();
        let first = hub.listen(7, None, Duration::ZERO, || true).unwrap().id;
        assert!(hub.snapshot(first).is_some());
        command(&mut app, &json!({})).unwrap();
        assert!(hub.snapshot(first).is_none(), "opening a new bubble closed the earlier request");
    }

    #[test]
    fn failed_command_does_not_record_edited_from_for_undo_join() {
        let (mut app, hub) = app();
        hub.listen(7, None, Duration::ZERO, || true);
        command(&mut app, &json!({"instruction": "Edit"})).unwrap();
        let id = hub.listen(7, None, Duration::ZERO, || true).unwrap().id;
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
        hub.listen(7, Some("Claude"), Duration::ZERO, || true);
        command(&mut app, &json!({"instruction": "Shorter"})).unwrap();
        let id = hub.listen(7, Some("Claude"), Duration::ZERO, || true).unwrap().id;
        let before = app.session.undo_depth();
        for text in ["Kilns ", "fire."] {
            let p = json!({"text": text, "request": id});
            as_agent(&mut app, Some(7), "text.insert", &p, |a| a.run("text.insert", p.clone())).unwrap();
        }
        assert_eq!(app.session.undo_depth(), before + 1, "one undo step");
        assert!(held(&app, "text.insert").unwrap().ends_with("(Claude)"), "the person's edit is refused");
        let other = as_agent(&mut app, Some(8), "text.insert", &json!({"text": "x"}), |a| a.run("text.insert", json!({"text": "x"})));
        assert!(other.unwrap_err().contains(hub::HELD), "another connection without the request");
        hub.reply(7, id, "Shortened.", true, None).unwrap();
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
        hub.listen(1, None, Duration::ZERO, || true);
        command(&mut app, &json!({"instruction": "Formal"})).unwrap();
        let first = hub.listen(1, None, Duration::ZERO, || true).unwrap().id;
        hub.reply(1, first, "Made formal.", true, None).unwrap();
        send(&mut app, "Shorter".into());
        let second = hub.listen(1, None, Duration::ZERO, || true).unwrap();
        assert_eq!(second.history, vec![Turn { instruction: "Formal".into(), answer: "Made formal.".into() }]);
        assert!(hub.snapshot(first).is_none(), "the answered request left the queue");
    }
}

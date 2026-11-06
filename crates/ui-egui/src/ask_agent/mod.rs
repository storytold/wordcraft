//! Ask Agent's Word adapter. The queue and bubble mirror storytold/craft-libs#142 until it
//! merges; app-owned document identities, revisions, geometry and undo provenance stay here.
//! Hub behavior starts from common revision `37443257`, plus the transport-only `owns_work`
//! query from #142; that query keeps live sockets open and never authorizes a document edit.

pub mod hub;
#[cfg(not(target_arch = "wasm32"))]
mod launcher;
pub mod ui;

use egui::{Rect, vec2};
pub use hub::{AgentHub, HubError, Reply, Request, Snapshot, Status, Target, Turn};
use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_engine::{Session, UndoToken};

use crate::WordApp;

pub const SHORTCUT: &str = "Mod+Shift+O";
#[cfg(not(target_arch = "wasm32"))]
const LAUNCH_VARIABLE: &str = "WORDCRAFT_AGENT_LAUNCH";
const MAX_TURNS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    NoControlChannel,
    NoAgent,
    InvalidLauncher,
}

impl Unavailable {
    /// Present the actionable cause of unavailability in the app's current language.
    pub fn shown(&self) -> String {
        match self {
            Self::NoControlChannel => tl!("Start WordCraft with --control to let an agent connect").to_string(),
            Self::NoAgent => tl!("No agent is listening: ask your agent to wait for requests").to_string(),
            Self::InvalidLauncher => tl!("The configured agent launcher must be an absolute executable path").to_string(),
        }
    }
}

/// One conversation belongs to one opened document, irrespective of which window is active.
pub struct Open {
    pub id: Option<u64>,
    pub anchor: Rect,
    pub turns: Vec<Turn>,
    document: u64,
    revision: u64,
    asked: String,
    selection: Value,
    edited_from: Option<usize>,
    undo_token: Option<UndoToken>,
    last_agent: Option<String>,
    commented: bool,
}

#[derive(Default)]
pub struct AskAgent {
    pub hub: Option<AgentHub>,
    pub open: Option<Open>,
    pub draft: String,
    pub picked: Option<String>,
    pub selection: Option<Rect>,
    agent_turn: bool,
    #[cfg(not(target_arch = "wasm32"))]
    launcher: launcher::Launcher,
}

impl AskAgent {
    /// Check channel, listener and opt-in launcher availability without spawning a process.
    /// The actual launch remains tied to the person's explicit Send action.
    pub fn unavailable(&self) -> Option<Unavailable> {
        let Some(hub) = &self.hub else { return Some(Unavailable::NoControlChannel) };
        if !hub.agents().is_empty() {
            return None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        match launcher::available(LAUNCH_VARIABLE) {
            Ok(true) => return None,
            Err(_) => return Some(Unavailable::InvalidLauncher),
            Ok(false) => {}
        }
        Some(Unavailable::NoAgent)
    }
}

/// App-owned state, never an agent-supplied document id or revision.
fn current(session: &Session) -> Target {
    Target { document: session.document_id().to_string(), revision: session.rev() }
}

/// Capture native endpoints as the scope authority alongside plain text for the agent.
/// Plain-text extraction alone is not sufficient to validate embedded objects or cell ranges.
fn captured_selection(session: &Session) -> Value {
    json!({"text": session.selected_text(), "anchor": session.sel.anchor, "focus": session.sel.focus})
}

/// Decode both native endpoints, refusing malformed positions or a scope spanning two stories.
fn range(selection: &Value) -> Option<(Pos, Pos)> {
    let a: Pos = serde_json::from_value(selection.get("anchor")?.clone()).ok()?;
    let b: Pos = serde_json::from_value(selection.get("focus")?.clone()).ok()?;
    if a.story != b.story {
        return None;
    }
    Some(if a <= b { (a, b) } else { (b, a) })
}

/// Plain-text extraction hides inline objects; inspect the actual selected structure instead.
fn ordinary_text(session: &Session) -> bool {
    if session.math.is_some() || session.column.is_some() || session.cell_range().is_some() || !session.also_selected.is_empty() {
        return false;
    }
    let (start, end) = session.sel.ordered();
    if start.path.cell() != end.path.cell() {
        return false;
    }
    session.doc.copy_range(&start, &end).blocks.iter().all(|block| {
        block.as_para().is_some_and(|para| {
            para.objects
                .iter()
                .all(|object| matches!(object, wordcraft_doc::InlineObject::CommentStart { .. } | wordcraft_doc::InlineObject::CommentEnd { .. }))
        })
    })
}

/// Only native commands whose complete effect is confined to the selected text are accepted.
/// Whole-document commands, undo/redo, macros and document replacement are not selection edits.
fn within_selection(session: &Session, command: &str, selection: &Value) -> bool {
    if !ordinary_text(session) {
        return false;
    }
    let Some((a, b)) = range(selection) else { return false };
    let (c, d) = session.sel.ordered();
    if a > c || d > b || a.story != c.story || b.story != d.story {
        return false;
    }
    match command {
        // Replacing the complete target, or appending at its end, has a well-defined rebased
        // interval. Arbitrary partial structural replacements require a separate app adapter.
        "text.insert" => {
            // Content-control placeholders and field boundaries may redirect ordinary typing.
            // This text adapter deliberately requires the native insertion point to stay put.
            if c == d && matches!(session.doc.settings.protection.as_deref(), Some("readOnly" | "comments")) {
                return false;
            }
            (a == c || c == d)
                && b == d
                && session.doc.control_at(&c).is_none()
                && session.doc.control_at(&d).is_none()
                && session.doc.typing_pos(&c) == c
                && session.doc.past_field_ends(&c) == c
        }
        "format.bold"
        | "format.italic"
        | "format.underline"
        | "format.strikethrough"
        | "format.font"
        | "format.size"
        | "format.color"
        | "format.highlight" => c < d,
        _ => false,
    }
}

/// Scoped typing cannot AutoCorrect a neighboring word or widen the target into paragraphs.
pub(crate) fn command_params(app: &WordApp, command: &str, mut params: Value) -> Result<Value, String> {
    if app.ask_agent.agent_turn && command == "text.insert" {
        if params.get("paragraphs").and_then(Value::as_bool) == Some(true)
            || params.get("text").and_then(Value::as_str).is_some_and(|text| text.contains(['\r', '\u{2029}']))
        {
            return Err("OUTSIDE_SELECTION: structural paragraph creation needs an explicit structural scope".into());
        }
        if let Some(object) = params.as_object_mut() {
            object.insert("raw".into(), json!(true));
        }
    }
    Ok(params)
}

/// Every mutating control command goes through the same owner, document and geometry checks.
pub fn as_agent(
    app: &mut WordApp,
    conn: Option<u64>,
    command: &str,
    p: &Value,
    run: impl FnOnce(&mut WordApp) -> Result<Value, String>,
) -> Result<Value, String> {
    let request = request_id(p)?;
    if conn.is_some() && command == "ui.askAgent" && app.ask_agent.open.as_ref().is_some_and(|open| open.id.is_some()) {
        return Err("HUMAN_REVIEW_REQUIRED: keep, undo or close this conversation in the application before opening another".into());
    }
    let mutates = app.session.registry.get(command).is_none_or(|command| command.mutates)
        || command.starts_with("file.")
        || matches!(command, "edit.undo" | "edit.undoTo" | "edit.redo");
    let (Some(conn), Some(hub), true) = (conn, app.ask_agent.hub.clone(), mutates) else { return run(app) };
    let before = current(&app.session);
    hub.may_edit(conn, request, &before, |selection| within_selection(&app.session, command, selection)).map_err(|error| error.to_string())?;
    let Some(id) = request else { return run(app) };
    let open = app
        .ask_agent
        .open
        .as_ref()
        .filter(|open| open.id == Some(id) && open.document == app.session.document_id())
        .ok_or("STALE_REQUEST: no conversation owns this document")?;
    let token = app.session.undo_token();
    let join = open.revision == before.revision && open.undo_token.is_some() && open.undo_token == token;
    if open.undo_token.is_some() && !join {
        return Err("STALE_REQUEST: the undo checkpoint changed".into());
    }
    let selection = open.selection.clone();
    let depth = app.session.undo_depth();
    if join {
        app.session.join_next_undo();
    } else {
        app.session.close_typing();
    }
    let name = hub.snapshot(id).and_then(|snapshot| snapshot.agent).unwrap_or_default();
    let author = std::mem::replace(&mut app.session.author, format!("Agent ({name})"));
    let typing = (app.session.prefs.overtype, app.session.prefs.typing_replaces_selection);
    app.session.prefs.overtype = false;
    app.session.prefs.typing_replaces_selection = true;
    app.ask_agent.agent_turn = true;
    let result = run(app);
    app.ask_agent.agent_turn = false;
    app.session.author = author;
    (app.session.prefs.overtype, app.session.prefs.typing_replaces_selection) = typing;
    app.session.cancel_undo_join();
    let after = current(&app.session);
    if after != before {
        let rebased = if command == "text.insert" {
            let Some((start, _)) = range(&selection) else { return Err("STALE_REQUEST: selection unavailable".into()) };
            json!({"anchor": start, "focus": app.session.sel.focus, "text": ""})
        } else {
            selection
        };
        if let Some(open) = app.ask_agent.open.as_mut().filter(|open| open.id == Some(id)) {
            open.revision = after.revision;
            open.selection = rebased.clone();
            if token != app.session.undo_token() || join {
                open.edited_from.get_or_insert(depth);
                open.undo_token = app.session.undo_token();
                open.last_agent = Some(name);
            }
        }
        // A disconnect can race with the native command. Its checkpoint belongs to this
        // conversation even when queue ownership is lost before advance accepts the revision.
        if result.is_ok() {
            hub.advance(conn, id, &before, after.clone(), rebased).map_err(|error| {
                hub.invalidate(&after);
                error.to_string()
            })?;
        } else {
            hub.invalidate(&after);
        }
    }
    result
}

/// Keep an absent tag distinct from a present malformed one, including JSON null.
/// This runs before dispatch so invalid tags cannot silently become unrestricted commands.
fn request_id(params: &Value) -> Result<Option<u64>, String> {
    match params.get("request") {
        None => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| "INVALID_PARAMS: request must be an unsigned integer".into()),
    }
}

/// No request-tagged edit may bypass the adapter by addressing a different, inactive document.
pub fn guard_addressed(app: &mut WordApp, conn: Option<u64>, document: u64, command: &str, params: &Value) -> Result<(), String> {
    let request = request_id(params)?;
    let Some(hub) = app.ask_agent.hub.clone() else { return Ok(()) };
    if !app.session.registry.get(command).is_some_and(|command| command.mutates) && !command.starts_with("file.") {
        return Ok(());
    }
    if request.is_some() {
        return Err("STALE_REQUEST: activate the request's document before editing it".into());
    }
    let target = app.session.with_document(document, |session| current(session)).map_err(|error| error.to_string())?;
    hub.may_edit(conn.unwrap_or(0), None, &target, |_| false).map_err(|error| error.to_string())
}

/// Synthetic UI input could click Keep/Undo or move the edit target after authorization. Agents
/// must use inspectable native commands while an Ask Agent conversation is open.
pub fn guard_input(app: &WordApp) -> Result<(), String> {
    if app.ask_agent.open.as_ref().is_some_and(|open| open.id.is_some()) {
        return Err("HELD: synthetic UI input is disabled while Ask Agent is open; use native document commands".into());
    }
    Ok(())
}

/// Called on the document owner's UI thread, serialized with document edits.
pub fn reply(app: &mut WordApp, conn: Option<u64>, params: &Value) -> Result<Value, String> {
    let Some(conn) = conn else { return Err("NOT_OWNER: the reply needs an authenticated connection".into()) };
    let Some(hub) = app.ask_agent.hub.clone() else { return Err("NO_REQUEST: Ask Agent is unavailable".into()) };
    hub.handle(
        conn,
        "agent.reply",
        params,
        || true,
        |captured| {
            let id = captured.document.parse::<u64>().ok()?;
            app.session.with_document(id, |session| current(session)).ok()
        },
    )
    .ok_or_else(|| "INVALID_PARAMS: unknown Ask Agent method".into())
}

/// Detect outside edits without mistaking another active document for a revision of this one.
pub fn sync(app: &mut WordApp) {
    let Some(hub) = app.ask_agent.hub.clone() else { return };
    let Some(open) = &app.ask_agent.open else { return };
    if !app.session.is_open(open.document) {
        close(app);
        return;
    }
    if open.document == app.session.document_id() && open.revision != app.session.rev() {
        hub.invalidate(&current(&app.session));
    }
}

/// Expose only active-document holds as native ranges and stable agent colours for painting.
pub fn held_ranges(app: &WordApp) -> Vec<(Pos, Pos, [u8; 3])> {
    held_by(app).into_iter().map(|(a, b, name)| (a, b, hub::colour(&name))).collect()
}

/// Map hub holds back to native positions only while the original conversation document is active.
fn held_by(app: &WordApp) -> Vec<(Pos, Pos, String)> {
    let Some(hub) = &app.ask_agent.hub else { return vec![] };
    if app.ask_agent.open.as_ref().is_none_or(|open| open.document != app.session.document_id()) {
        return vec![];
    }
    hub.held().into_iter().filter_map(|(selection, name)| range(&selection).map(|(a, b)| (a, b, name))).collect()
}

/// Explain the document hold for command enablement. Actual edits still undergo native ownership
/// and selection validation; this label is never used as authorization.
pub fn held(app: &WordApp, command: &str) -> Option<String> {
    if app.ask_agent.agent_turn || !app.session.registry.get(command).is_some_and(|command| command.mutates) {
        return None;
    }
    let (c, d) = app.session.sel.ordered();
    let (_, _, name) = held_by(app).into_iter().find(|(a, b, _)| c <= *b && *a <= d)?;
    Some(format!("{} ({name})", tl!("Held by Agent")))
}

/// Validate the real selection and enqueue a bounded snapshot of the current document revision.
/// A changed revision starts new edit provenance rather than inheriting an unrelated checkpoint.
fn ask(app: &mut WordApp, instruction: String) -> Result<(), String> {
    let Some(hub) = app.ask_agent.hub.clone() else { return Err(Unavailable::NoControlChannel.shown()) };
    if !ordinary_text(&app.session) {
        return Err("OUTSIDE_SELECTION: Ask Agent requires an ordinary text selection, not whole cells or objects".into());
    }
    let target = current(&app.session);
    let selection = captured_selection(&app.session);
    let document = json!({"title": app.title_stem(), "saved": !app.session.dirty, "language": app.ui.language});
    let Some(open) = &mut app.ask_agent.open else { return Err("NO_REQUEST: open Ask Agent first".into()) };
    if open.document != app.session.document_id() {
        return Err("STALE_REQUEST: return to the original document".into());
    }
    if open.revision != target.revision {
        open.edited_from = None;
        open.undo_token = None;
        open.last_agent = None;
    }
    let request = Request {
        id: 0,
        instruction: instruction.clone(),
        history: open.turns.clone(),
        selection: selection.clone(),
        document,
        target: target.clone(),
        agent: app.ask_agent.picked.clone(),
    };
    let id = hub.ask(request).map_err(|error| error.to_string())?;
    open.id = Some(id);
    open.asked = instruction;
    open.selection = selection;
    open.revision = target.revision;
    open.commented = false;
    #[cfg(not(target_arch = "wasm32"))]
    if let Err(error) = app.ask_agent.launcher.start(LAUNCH_VARIABLE, &hub) {
        hub.close(id);
        if let Some(open) = &mut app.ask_agent.open {
            open.id = None;
        }
        return Err(error);
    }
    Ok(())
}

/// Close every prior request, including failed ones, before replacing its id.
fn send(app: &mut WordApp, text: String) -> Result<(), String> {
    let snapshot = app.ask_agent.hub.as_ref().and_then(|hub| app.ask_agent.open.as_ref().and_then(|open| open.id).and_then(|id| hub.snapshot(id)));
    let completed =
        snapshot.as_ref().is_some_and(|snapshot| snapshot.status == Status::Answered || (snapshot.status == Status::Failed && can_undo(app)));
    // A replacement leaves a caret at the target's end. A follow-up still refers to the text
    // just produced unless the person deliberately selected or moved somewhere else.
    let carry = app
        .ask_agent
        .open
        .as_ref()
        .and_then(|open| range(&open.selection))
        .is_some_and(|(_, end)| completed && app.session.sel.is_collapsed() && app.session.sel.focus == end);
    if let Some(snapshot) = snapshot.as_ref().filter(|_| completed) {
        comment_once(app, snapshot);
        if app.ask_agent.open.as_ref().is_some_and(|open| !open.commented) {
            return Err("NOTE_NOT_SAVED: the agent's note could not be retained; use Undo or retry".into());
        }
    }
    if let (Some(hub), Some(open)) = (&app.ask_agent.hub, &mut app.ask_agent.open) {
        if open.turns.len() >= MAX_TURNS {
            return Err("LIMIT_EXCEEDED: start a new Ask Agent conversation".into());
        }
        if let Some(snapshot) = snapshot.filter(|_| completed) {
            open.turns.push(Turn { instruction: std::mem::take(&mut open.asked), answer: snapshot.review_text() });
        }
        if let Some(id) = open.id.take() {
            hub.close(id);
        }
    }
    let selection = app.session.sel.clone();
    if let Some((anchor, focus)) = carry.then(|| app.ask_agent.open.as_ref().and_then(|open| range(&open.selection))).flatten() {
        app.session.sel.anchor = anchor;
        app.session.sel.focus = focus;
    }
    let result = ask(app, text);
    app.session.sel = selection;
    result
}

/// Create an unsent conversation anchored to the current selection and native document identity.
fn open_at_caret(app: &mut WordApp) {
    close(app);
    let caret = crate::canvas::caret_screen(app).map(|(p, h)| Rect::from_min_size(p, vec2(1.0, h)));
    let anchor = app.ask_agent.selection.or(caret).unwrap_or(Rect::from_min_size(egui::pos2(96.0, 160.0), vec2(1.0, 16.0)));
    app.ask_agent.open = Some(Open {
        id: None,
        anchor,
        turns: vec![],
        document: app.session.document_id(),
        revision: app.session.rev(),
        asked: String::new(),
        selection: Value::Null,
        edited_from: None,
        undo_token: None,
        last_agent: None,
        commented: false,
    });
}

/// Remove the conversation's queue entry and local review state without reversing native edits.
fn close(app: &mut WordApp) {
    if let (Some(hub), Some(id)) = (&app.ask_agent.hub, app.ask_agent.open.take().and_then(|open| open.id)) {
        hub.close(id);
    }
}

/// Handle the native Ask Agent command: open, explicitly close, or send an instruction.
/// The caller's connection boundary separately prevents remote dismissal of human review.
pub fn command(app: &mut WordApp, params: &Value) -> Result<Value, String> {
    if params.get("close").and_then(Value::as_bool) == Some(true) {
        close(app);
        return Ok(json!({"open":false}));
    }
    if let Some(why) = app.ask_agent.unavailable() {
        return Err(why.shown());
    }
    open_at_caret(app);
    if let Some(instruction) = params.get("instruction").and_then(Value::as_str) {
        ask(app, instruction.to_string())?;
    }
    Ok(json!({"open":true,"request":app.ask_agent.open.as_ref().and_then(|open|open.id)}))
}

/// Escape closes this conversation only when no higher-priority dialog, menu or keytip owns it.
pub fn escape(app: &mut WordApp, ctx: &egui::Context) {
    if app.ask_agent.open.is_some()
        && !app.modal_open()
        && app.dialog.is_none()
        && !app.ui.backstage
        && app.ui.keytips == crate::keytips::Phase::Off
        && !egui::Popup::is_any_open(ctx)
        && ctx.input(|input| input.key_pressed(egui::Key::Escape))
    {
        close(app);
    }
}
/// Dismiss only an unsent draft when native typing resumes, leaving owned/reviewable work intact.
pub fn typing_resumed(app: &mut WordApp) {
    if app.ask_agent.open.as_ref().is_some_and(|open| open.id.is_none()) {
        close(app);
    }
}

/// Keep means retain the native edits and their review note; a refused note stays recoverable.
fn keep(app: &mut WordApp) -> Result<(), String> {
    let edited = app.ask_agent.open.as_ref().is_some_and(|open| open.edited_from.is_some());
    let snapshot = app.ask_agent.hub.as_ref().and_then(|hub| app.ask_agent.open.as_ref().and_then(|open| open.id).and_then(|id| hub.snapshot(id)));
    if let Some(snapshot) = snapshot.filter(|snapshot| snapshot.status == Status::Answered || (snapshot.status == Status::Failed && edited)) {
        comment_once(app, &snapshot);
        if app.ask_agent.open.as_ref().is_some_and(|open| !open.commented) {
            return Err("NOTE_NOT_SAVED: the agent's note could not be retained; use Undo or retry".into());
        }
    }
    close(app);
    Ok(())
}

/// Require the exact retained native checkpoint at the conversation's revision before Undo.
fn can_undo(app: &WordApp) -> bool {
    app.ask_agent.open.as_ref().is_some_and(|open| {
        open.document == app.session.document_id()
            && open.revision == app.session.rev()
            && open.undo_token.is_some()
            && open.undo_token == app.session.undo_token()
    })
}

/// Render the shared literal-text conversation and apply only the person's explicit actions
/// on the UI thread; a proposal never becomes an executable document command.
pub fn show(app: &mut WordApp, ctx: &egui::Context, _viewport: Rect) {
    sync(app);
    let Some(hub) = app.ask_agent.hub.clone() else { return };
    let tr = |text| crate::i18n::t(text).to_string();
    let Some(open) = &app.ask_agent.open else {
        if let Some(anchor) = app.ask_agent.selection
            && app.ask_agent.unavailable().is_none()
            && ui::chip(ctx, anchor, &tr)
        {
            open_at_caret(app);
        }
        return;
    };
    if open.document != app.session.document_id() {
        return;
    }
    let (anchor, snapshot) = (open.anchor, open.id.and_then(|id| hub.snapshot(id)));
    if let Some(snapshot) = snapshot.as_ref().filter(|snapshot| snapshot.status == Status::Answered) {
        comment_once(app, snapshot);
    }
    let agents = hub.agents();
    let Some(open) = app.ask_agent.open.as_ref() else { return };
    let view = ui::View { snapshot: snapshot.as_ref(), agents: &agents, actions: QUICK_ACTIONS, history: &open.turns, instruction: &open.asked };
    match ui::bubble(ctx, anchor, &view, &mut app.ask_agent.draft, &mut app.ask_agent.picked, &tr) {
        Some(ui::Action::Send(text)) => {
            if let Err(error) = send(app, text.clone()) {
                app.ask_agent.draft = text;
                app.status(error);
            }
        }
        Some(ui::Action::Undo) => {
            if can_undo(app) {
                app.session.undo();
            } else {
                app.status(tl!("The document changed since the agent's edits; use Undo in the Edit menu"));
            }
            close(app);
        }
        Some(ui::Action::Keep) => {
            if let Err(error) = keep(app) {
                app.status(error);
            }
        }
        Some(ui::Action::Close) => close(app),
        None => {}
    }
}

/// Attach one native comment for the response and rebase its anchors while preserving the
/// conversation's undo provenance. Failure remains recoverable instead of marking the note saved.
fn comment_once(app: &mut WordApp, snapshot: &Snapshot) {
    let Some(open) = app.ask_agent.open.as_ref().filter(|open| {
        !open.commented && open.id == Some(snapshot.id) && open.document == app.session.document_id() && open.revision == app.session.rev()
    }) else {
        return;
    };
    let Some(name) = snapshot.agent.as_ref().or(open.last_agent.as_ref()) else { return };
    let (target, from, join, asked) =
        (range(&open.selection), open.edited_from, open.undo_token.is_some() && open.undo_token == app.session.undo_token(), open.asked.clone());
    if open.undo_token.is_some() && !join {
        return;
    }
    if join {
        app.session.join_next_undo();
    }
    let depth = app.session.undo_depth();
    let token = app.session.undo_token();
    let revision = app.session.rev();
    let author = std::mem::replace(&mut app.session.author, format!("Agent ({name})"));
    let selection = app.session.sel.clone();
    if let Some((anchor, focus)) = target {
        (app.session.sel.anchor, app.session.sel.focus) = (anchor, focus);
    }
    let text = format!("You: {asked}\n\nAgent: {}", snapshot.review_text());
    let result = app.session.run("review.newComment", &json!({"text":text}));
    app.session.sel = selection;
    app.session.author = author;
    // Native comments insert two inline markers. Read their real positions and move every
    // captured position past the start marker but before the end marker. Restoring raw offsets
    // would truncate the next request and leave the person's caret inside the rewritten text.
    if let Ok(value) = &result
        && let Some(id) = value.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok())
    {
        rebase_comment(app, id);
    }
    app.session.cancel_undo_join();
    if let Some(open) = &mut app.ask_agent.open {
        open.commented = result.is_ok();
        if result.is_ok() {
            if from.is_none() && token != app.session.undo_token() {
                open.edited_from = Some(depth);
            }
            if join || (revision != app.session.rev() && token != app.session.undo_token()) {
                open.undo_token = app.session.undo_token();
            }
            open.revision = app.session.rev();
        }
    }
}

/// Rebase from the markers the native command actually inserted, including caret word anchors.
fn rebase_comment(app: &mut WordApp, id: u32) {
    let Some(open) = app.ask_agent.open.as_ref() else { return };
    let Some((range_start, _)) = range(&open.selection) else { return };
    let story = range_start.story;
    let (mut start, mut end) = (None, None);
    for path in app.session.doc.para_paths(story) {
        let Some(para) = app.session.doc.para(story, &path) else { continue };
        for off in para.object_offsets() {
            let position = Pos { story, path: path.clone(), off };
            match para.object_at(off) {
                Some(wordcraft_doc::InlineObject::CommentStart { id: found }) if *found == id => start = Some(position),
                Some(wordcraft_doc::InlineObject::CommentEnd { id: found }) if *found == id => end = Some(position),
                _ => {}
            }
        }
    }
    let (Some(start), Some(mut end)) = (start, end) else { return };
    let width = wordcraft_doc::para::OBJ.len_utf8();
    if start.path == end.path {
        end.off = end.off.saturating_sub(width);
    }
    let map = |position: &mut Pos| {
        if position.story != story {
            return;
        }
        let shift =
            usize::from(position.path == start.path && position.off >= start.off) + usize::from(position.path == end.path && position.off > end.off);
        position.off = position.off.saturating_add(shift * width);
    };
    map(&mut app.session.sel.anchor);
    map(&mut app.session.sel.focus);
    if let Some(open) = &mut app.ask_agent.open
        && let Some((mut anchor, mut focus)) = range(&open.selection)
    {
        map(&mut anchor);
        map(&mut focus);
        open.selection = json!({"anchor":anchor,"focus":focus,"text":""});
    }
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
#[path = "tests/adapter.rs"]
mod tests;

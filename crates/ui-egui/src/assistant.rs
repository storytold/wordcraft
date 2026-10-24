//! GeenioAI, the AI assistant pane (Home › GeenioAI, `ui.assistant`) and its margin icon: a chat whose
//! agent writes and edits the open document with WordCraft's own commands, the way Word's
//! Copilot pane and its margin icon do.
//!
//! The pane is thin, like every other one. The account key, the model and the network belong to
//! the host ([`AssistantHost`], injected by the desktop app); without a host (the web build) the
//! pane says the assistant isn't available yet. The agent loop (`wordcraft-assistant`) runs on a
//! worker thread so the interface never waits for the model; its tools reach the document through
//! the control methods, run here on the UI thread ([`tick`]). One turn is one undo step.
//!
//! The key is write-only for the interface: the onboarding stores one when none is set, the
//! settings window can replace it with a new one, and nothing in this crate can read or show it.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use egui::{Rect, RichText, Sense, vec2};
use serde_json::{Value, json};
use wordcraft_assistant::{Agent, Config, Delta, Event, Observer, Provider};
use wordcraft_mcp::Backend;

use crate::WordApp;
use crate::control::{ControlRequest, Outcome};
use crate::theme::{Tokens, regular, semibold};

/// How long a tool may wait for the UI thread to run it.
const TOOL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a confirmation card waits for an answer before counting as declined.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(600);
/// Messages kept in the pane; older ones scroll away for good.
const MAX_MESSAGES: usize = 300;
/// Most characters of the selection or the caret's paragraph sent as context with a prompt.
const MAX_CONTEXT_CHARS: usize = 12_000;
/// The margin icon's size, in points.
pub const ICON: f32 = 22.0;
/// Gap between the margin icon and the page body's left edge.
const ICON_GAP: f32 = 10.0;

/// The account service behind the assistant, as the pane names it. The pane itself doesn't know
/// which service that is: the host says.
#[derive(Clone, Debug, PartialEq)]
pub struct Service {
    /// The service's name, as shown to the user.
    pub name: String,
    /// Where an account key comes from and where the credit is topped up.
    pub url: String,
    /// What an account key starts with, for the onboarding text.
    pub key_prefix: String,
}

impl Service {
    /// The site as it reads in a sentence: the URL without its scheme and trailing slash.
    fn site(&self) -> &str {
        self.url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/')
    }
}

/// A connected account — never the key.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    /// The account's identifier: it can be shown and recorded, unlike the key.
    pub conto_id: String,
    /// The model the account service suggests.
    pub model: String,
    pub remaining_usd: Option<f64>,
}

/// A model the account can use.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelInfo {
    /// The id sent with each request, e.g. `anthropic/claude-sonnet-5`.
    pub id: String,
    /// A readable name, when the account service gives one.
    pub name: Option<String>,
    /// The price of a million output tokens, in USD.
    pub output_usd_per_million: Option<f64>,
}

impl ModelInfo {
    /// Batch variants (`…:batch`) can't answer a chat: they are left out of the list.
    pub fn is_chat(id: &str) -> bool {
        !id.to_ascii_lowercase().ends_with(":batch")
    }

    fn label(&self) -> String {
        let name = self.name.as_deref().unwrap_or(&self.id);
        match self.output_usd_per_million {
            Some(p) => format!("{name}  ·  {p:.2} USD/M"),
            None => name.to_string(),
        }
    }
}

/// What the host is told.
#[derive(Clone, Debug, PartialEq)]
pub enum AssistantEvent {
    /// Onboarding finished: the key of account `conto_id` is now stored on this computer. The
    /// host records it wherever keys are managed (to be changed or disabled there).
    KeyRegistered { conto_id: String },
    /// The user replaced the key: the stored key now belongs to account `conto_id`.
    KeyChanged { conto_id: String },
}

/// The language the assistant replies in (Assistant settings › Assistant language).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    /// The language the user writes in.
    #[default]
    Auto,
    /// The interface language (File › Options › Language).
    Interface,
    English,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::Auto, Language::Interface, Language::English];

    /// The code kept in the preferences.
    pub fn code(self) -> &'static str {
        match self {
            Language::Auto => "auto",
            Language::Interface => "interface",
            Language::English => "en",
        }
    }

    pub fn parse(code: &str) -> Option<Language> {
        Language::ALL.into_iter().find(|l| l.code() == code.trim())
    }

    /// The name shown in the settings.
    fn name(self) -> String {
        match self {
            Language::Auto => tl!("Automatic (the language you write in)").to_string(),
            Language::Interface => crate::i18n::fmt(tl!("The interface language ({language})"), &[("language", crate::i18n::current().name())]),
            Language::English => "English".to_string(),
        }
    }

    /// The instruction added to the system prompt.
    fn instruction(self) -> Option<String> {
        match self {
            Language::Auto => None,
            Language::Interface => Some(format!("Always reply in {}, whatever language the user writes in.", crate::i18n::current().name())),
            Language::English => Some("Always reply in English, whatever language the user writes in.".into()),
        }
    }
}

/// The host side of the assistant. Nothing here returns the key: the interface can store one or
/// replace it with a new one, never read it.
pub trait AssistantHost: Send + Sync {
    /// The account service: its name, its site and what its keys look like.
    fn service(&self) -> Service;
    fn has_key(&self) -> bool;
    /// Check `key` with the account service and store it, if no key is stored yet. Blocking: runs
    /// on a worker thread.
    fn connect(&self, key: String) -> Result<Account, String>;
    /// Check `key` with the account service and put it in place of the stored one. Blocking.
    fn change_key(&self, key: String) -> Result<Account, String>;
    /// The account of the stored key, when it can be read. Blocking.
    fn account(&self) -> Option<Account>;
    /// The models the account can use for a chat. Blocking.
    fn models(&self) -> Result<Vec<ModelInfo>, String>;
    /// A model endpoint for a conversation and the model to ask for. Blocking.
    fn provider(&self) -> Result<(Box<dyn Provider + Send>, String), String>;
    /// The credit left, when it can be read. Blocking.
    fn balance(&self) -> Option<f64>;
    fn event(&self, event: &AssistantEvent);
    /// What the user has to agree to before anything is sent, while they haven't: the pane asks
    /// before the turn and records a yes with [`agree`](Self::agree). Blocking.
    fn consent(&self) -> Option<Consent> {
        None
    }
    /// Record the agreement [`consent`](Self::consent) asked for. Blocking.
    fn agree(&self) -> Result<(), String> {
        Ok(())
    }
}

/// What a turn sends, and where, for the user to agree to once.
#[derive(Clone, Debug, PartialEq)]
pub struct Consent {
    /// The service, as the user knows it.
    pub service: String,
    /// The service's address. Which provider serves the chosen model is the service's business,
    /// so the question names none.
    pub destination: String,
}

/// The question that asks for [`Consent`].
fn consent_text(c: &Consent) -> String {
    crate::i18n::fmt(
        tl!(
            "To answer, GeenioAI sends your requests, the document text it reads and what its commands return to {service} ({destination}) and to the provider of the chosen model. Allow this for WordCraft?"
        ),
        &[("service", &c.service), ("destination", &c.destination)],
    )
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Role {
    User,
    Assistant,
    Notice,
    Error,
    /// A link to the account site; the text is its label.
    Link,
}

#[derive(Clone, Debug)]
pub(crate) struct Message {
    pub(crate) role: Role,
    pub(crate) text: String,
}

/// What the assistant is doing right now, in plain words: shown next to the spinner while a turn
/// runs, gone when it ends. The chat keeps what was said, never what was run.
#[derive(Clone, Debug, PartialEq)]
struct Activity {
    text: String,
    /// The last command was refused; the agent reads the error and usually tries another way.
    failed: bool,
}

/// A conversation: the agent (with its history) and the model endpoint (with its key in memory).
struct Conversation {
    agent: Agent,
    provider: Box<dyn Provider + Send>,
}

/// From the worker threads to the pane.
enum Update {
    Connected(Result<Account, String>),
    KeyChanged(Result<Account, String>),
    Account(Option<Account>),
    /// The credit, read again on request.
    Balance(Option<f64>),
    Models(Result<Vec<ModelInfo>, String>),
    Text(String),
    /// A reply starts streaming (text shown for an earlier attempt is stale).
    StreamStart,
    /// The next piece of the streaming reply.
    StreamText(String),
    /// The agent asked for a tool.
    Step {
        name: String,
        args: Value,
    },
    /// The last tool was refused.
    StepFailed,
    Declined {
        name: String,
    },
    Confirm {
        name: String,
        args: Value,
        reply: Sender<bool>,
    },
    /// Nothing has been sent: the user is asked to agree first.
    Consent {
        consent: Consent,
        reply: Sender<bool>,
    },
    Done {
        conversation: Option<Conversation>,
        error: Option<String>,
        balance: Option<f64>,
        /// The step budget ran out; the reply says what was done.
        stopped: bool,
    },
}

/// The settings window.
#[derive(Default)]
struct Settings {
    open: bool,
    new_key: String,
    busy: bool,
    /// The outcome of the last key change: `Ok` message or error.
    result: Option<Result<String, String>>,
    /// The model list: `None` until asked for, then the list or why it couldn't be read.
    models: Option<Result<Vec<ModelInfo>, String>>,
    models_requested: bool,
    model_filter: String,
}

pub struct Assistant {
    /// The pane is shown (Home › GeenioAI, `ui.assistant`).
    pub open: bool,
    host: Option<Arc<dyn AssistantHost>>,
    pub(crate) messages: Vec<Message>,
    pub(crate) input: String,
    /// The margin icon's own prompt box ("Draft with GeenioAI").
    pub(crate) quick_prompt: String,
    pub(crate) key_input: String,
    pub(crate) busy: bool,
    /// The current action while `busy`, for the status line under the messages.
    activity: Option<Activity>,
    /// The reply streaming in, shown as a bubble that fills up; replaced by the whole text when
    /// the step ends.
    pub(crate) streaming: Option<String>,
    conversation: Option<Conversation>,
    /// A question to the user (what the assistant wants to do) and where its answer goes.
    pub(crate) confirm: Option<(String, Sender<bool>)>,
    account: Option<Account>,
    /// The account of the stored key was asked for (once per run).
    account_requested: bool,
    pub(crate) balance: Option<f64>,
    /// The credit is being read again (a click on it).
    balance_busy: bool,
    /// The fresh-start question is up.
    clear_asked: bool,
    /// The input field takes the keyboard on the next frame (the pane just opened for a question).
    focus_input: bool,
    /// The undo depth when the running turn's first tool ran: later changes of the turn join
    /// that undo step, so one Undo takes the whole turn back.
    turn_undo_depth: Option<usize>,
    /// The reply language, kept in the preferences.
    pub language: Language,
    /// The model chosen in the settings, kept in the preferences; `None` follows the account's
    /// suggestion.
    pub model: Option<String>,
    /// The model went back to the suggestion while a turn had the conversation: drop it when the
    /// turn returns, so the next one asks the account again.
    drop_conversation_after_turn: bool,
    settings: Settings,
    updates_tx: Sender<Update>,
    updates_rx: Option<Receiver<Update>>,
    tools_tx: Sender<ControlRequest>,
    tools_rx: Option<Receiver<ControlRequest>>,
}

impl Default for Assistant {
    fn default() -> Self {
        let (updates_tx, updates_rx) = channel();
        let (tools_tx, tools_rx) = channel();
        Self {
            open: false,
            host: None,
            messages: Vec::new(),
            input: String::new(),
            quick_prompt: String::new(),
            key_input: String::new(),
            busy: false,
            activity: None,
            streaming: None,
            conversation: None,
            confirm: None,
            account: None,
            account_requested: false,
            balance: None,
            balance_busy: false,
            clear_asked: false,
            focus_input: false,
            turn_undo_depth: None,
            language: Language::default(),
            model: None,
            drop_conversation_after_turn: false,
            settings: Settings::default(),
            updates_tx,
            updates_rx: Some(updates_rx),
            tools_tx,
            tools_rx: Some(tools_rx),
        }
    }
}

impl Assistant {
    /// Give the assistant its host (the desktop app does, at startup).
    pub fn set_host(&mut self, host: Arc<dyn AssistantHost>) {
        self.host = Some(host);
    }

    pub fn available(&self) -> bool {
        self.host.is_some()
    }

    /// Whether an account key is stored: the assistant can be asked.
    pub fn ready(&self) -> bool {
        self.host.as_ref().is_some_and(|h| h.has_key())
    }

    /// Change the reply language; a conversation in hand follows it from the next turn (one on a
    /// worker thread follows it when it comes back).
    pub fn set_language(&mut self, language: Language) {
        self.language = language;
        if let Some(c) = &mut self.conversation {
            c.agent.set_instructions(language.instruction());
        }
    }

    /// Choose the model (`None`: the account's suggestion). A conversation in hand switches at
    /// its next turn; a new one starts with it.
    pub fn set_model(&mut self, model: Option<String>) {
        let model = model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty() && ModelInfo::is_chat(m));
        if let (Some(m), Some(c)) = (&model, &mut self.conversation) {
            c.agent.set_model(m.clone());
        }
        // Back to the suggestion: the next conversation asks the account again. During a turn the
        // worker has the conversation; it is dropped when the turn hands it back.
        if model.is_none() && self.model.is_some() {
            self.conversation = None;
            self.drop_conversation_after_turn = self.busy;
        }
        self.model = model;
    }

    fn push(&mut self, role: Role, text: impl Into<String>) {
        self.messages.push(Message { role, text: text.into() });
        if self.messages.len() > MAX_MESSAGES {
            let extra = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(..extra);
        }
    }

    fn greet(&mut self) {
        if !self.messages.is_empty() {
            return;
        }
        self.push(
            Role::Assistant,
            tl!("Hi! I'm GeenioAI. Ask me to draft, rewrite, summarize or format, and I'll do it in the document with WordCraft's commands."),
        );
        if let Some(host) = self.host.clone()
            && !host.has_key()
        {
            let s = host.service();
            self.push(
                Role::Assistant,
                crate::i18n::fmt(
                    tl!("To start, connect your {service} account: paste its account key (it starts with {prefix}). The key stays on this computer and is never shown again."),
                    &[("service", &s.name), ("prefix", &s.key_prefix)],
                ),
            );
            self.push(Role::Link, crate::i18n::fmt(tl!("Get an account key or top up your credit at {site}"), &[("site", s.site())]));
        }
    }
}

/// Home › GeenioAI, `ui.assistant`: show the pane.
pub fn open(app: &mut WordApp) {
    app.assistant.open = true;
    app.assistant.greet();
}

/// Run on a worker thread, or report that this build can't.
fn spawn(work: impl FnOnce() + Send + 'static) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new().name("wordcraft-assistant".into()).spawn(work).map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        drop(work);
        Err(tl!("GeenioAI is not available in this version yet.").to_string())
    }
}

/// The agent's way into the document: control methods, queued for the UI thread.
struct UiBackend {
    tx: Sender<ControlRequest>,
    ctx: egui::Context,
}

impl Backend for UiBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let (req, rx) = ControlRequest::new(method, params);
        let handle = req.give_up_handle();
        self.tx.send(req).map_err(|_| "WordCraft is closing".to_string())?;
        self.ctx.request_repaint();
        let v = match rx.recv_timeout(TOOL_TIMEOUT) {
            Ok(v) => v,
            // Given up before it ran: it never will, so the failure the model hears is true.
            Err(_) if handle.give_up() => return Err("WordCraft did not answer in time".to_string()),
            // It is running on the UI thread: its answer, and what it changed, are on the way.
            Err(_) => rx.recv().map_err(|_| "WordCraft is closing".to_string())?,
        };
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(v.get("error").and_then(Value::as_str).unwrap_or("error").to_string())
        }
    }
    /// Command-level tools only: the assistant doesn't drive the pointer.
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "WordCraft".into()
    }
}

/// The agent's progress and confirmations, sent to the pane.
struct UiObserver {
    tx: Sender<Update>,
    ctx: egui::Context,
}

impl UiObserver {
    fn send(&self, u: Update) -> bool {
        let sent = self.tx.send(u).is_ok();
        self.ctx.request_repaint();
        sent
    }
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", s.get(..at).unwrap_or(s)),
        None => s.to_string(),
    }
}

/// The translated label of the command `id` names, without a menu ellipsis.
fn command_label(app_registry: &wordcraft_engine::Registry, id: &str) -> Option<String> {
    let spec = app_registry.get(id.trim())?;
    Some(tl!(spec.label).trim_end_matches(['.', '…']).to_string())
}

/// What a tool call does, in the user's words: the command it runs, never its parameters.
fn describe(registry: &wordcraft_engine::Registry, name: &str, args: &Value) -> String {
    match name {
        "execute" => match args.get("command").and_then(Value::as_str).and_then(|c| command_label(registry, c)) {
            Some(c) => crate::i18n::fmt(tl!("Running {command}…"), &[("command", &c)]),
            None => tl!("Working…").to_string(),
        },
        "batch" => tl!("Running commands…").to_string(),
        "type_text" => tl!("Writing…").to_string(),
        "select_text" => tl!("Selecting text…").to_string(),
        "get_text" | "inspect_document" => tl!("Reading the document…").to_string(),
        "list_commands" => tl!("Looking up commands…").to_string(),
        "new_document" => tl!("Starting a new document…").to_string(),
        "open_document" => tl!("Opening a document…").to_string(),
        "save_document" => tl!("Saving the document…").to_string(),
        _ => tl!("Working…").to_string(),
    }
}

/// The question a confirmation card asks, in words: what the tool does and the file it touches.
fn confirm_text(registry: &wordcraft_engine::Registry, name: &str, args: &Value) -> String {
    let path_in = |v: &Value| v.get("path").and_then(Value::as_str).map(|p| clip(p.trim(), 200)).filter(|p| !p.is_empty());
    let label = |id: &str| command_label(registry, id).unwrap_or_else(|| clip(id.trim(), 60));
    match name {
        // A file command through `execute`: the command, and the file it touches.
        "execute" => {
            let id = args.get("command").and_then(Value::as_str).unwrap_or("").trim();
            let path = args.get("params").and_then(path_in);
            return match path {
                Some(p) if id == "file.open" => crate::i18n::fmt(tl!("The assistant wants to open {path}."), &[("path", &p)]),
                Some(p) if id.starts_with("file.save") || id.starts_with("file.export") => {
                    crate::i18n::fmt(tl!("The assistant wants to save the document to {path}."), &[("path", &p)])
                }
                _ => crate::i18n::fmt(tl!("The assistant wants to run {tool}."), &[("tool", &label(id))]),
            };
        }
        // The file commands of a `batch`, by name.
        "batch" => {
            let ids: Vec<String> = args
                .get("commands")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|c| c.get("command").and_then(Value::as_str))
                .filter(|id| id.trim().starts_with("file."))
                .map(label)
                .collect();
            return crate::i18n::fmt(tl!("The assistant wants to run {tool}."), &[("tool", &ids.join(", "))]);
        }
        _ => {}
    }
    let path = path_in(args);
    match (name, path) {
        ("new_document", _) => tl!("The assistant wants to start a new document: the current one will be closed.").to_string(),
        ("open_document", Some(p)) => crate::i18n::fmt(tl!("The assistant wants to open {path}."), &[("path", &p)]),
        ("open_document", None) => tl!("The assistant wants to open a document.").to_string(),
        ("save_document", Some(p)) => crate::i18n::fmt(tl!("The assistant wants to save the document to {path}."), &[("path", &p)]),
        ("save_document", None) => tl!("The assistant wants to save the document.").to_string(),
        _ => crate::i18n::fmt(tl!("The assistant wants to run {tool}."), &[("tool", name)]),
    }
}

impl Observer for UiObserver {
    fn confirm(&mut self, name: &str, arguments: &Value) -> bool {
        let (reply, answer) = channel();
        if !self.send(Update::Confirm { name: name.to_string(), args: arguments.clone(), reply }) {
            return false;
        }
        answer.recv_timeout(CONFIRM_TIMEOUT).unwrap_or(false)
    }
    fn event(&mut self, event: Event<'_>) {
        let u = match event {
            Event::Text(t) => Update::Text(t.to_string()),
            Event::ToolCall { name, arguments } => Update::Step { name: name.to_string(), args: arguments.clone() },
            Event::ToolResult { ok: false, name, text } => {
                // The error goes back to the model, not into the chat.
                log::debug!("GeenioAI: {name} was refused: {}", clip(text, 300));
                Update::StepFailed
            }
            Event::ToolResult { .. } => return,
            Event::Declined { name } => Update::Declined { name: name.to_string() },
            Event::Stream(Delta::Start) => Update::StreamStart,
            Event::Stream(Delta::Text(t)) => Update::StreamText(t.to_string()),
        };
        self.send(u);
    }
}

fn connect(app: &mut WordApp) {
    let a = &mut app.assistant;
    let key = std::mem::take(&mut a.key_input).trim().to_string();
    let Some(host) = a.host.clone() else { return };
    if key.is_empty() || a.busy {
        return;
    }
    a.busy = true;
    a.activity = Some(Activity { text: tl!("Checking the key…").to_string(), failed: false });
    let tx = a.updates_tx.clone();
    if let Err(e) = spawn(move || {
        let _ = tx.send(Update::Connected(host.connect(key)));
    }) {
        a.busy = false;
        a.activity = None;
        a.push(Role::Error, e);
    }
}

/// Where the user is in the document, for the model: the selection (which `type_text`
/// replaces), an empty paragraph the caret is on, or the paragraph around the caret.
pub(crate) fn context(app: &WordApp) -> Option<String> {
    let s = &app.session;
    let cut = |t: &str| {
        let t = t.trim();
        match t.char_indices().nth(MAX_CONTEXT_CHARS) {
            Some((at, _)) => format!("{}…", t.get(..at).unwrap_or("")),
            None => t.to_string(),
        }
    };
    if !s.sel.is_collapsed() {
        let text = s.selected_text();
        if !text.trim().is_empty() {
            return Some(format!("Context: the user has selected this text in the document (type_text replaces it):\n«{}»", cut(&text)));
        }
    }
    let para = s.doc.para_at(&s.sel.focus)?.plain_text().to_string();
    if para.trim().is_empty() {
        Some("Context: the caret is on an empty paragraph; type_text inserts there.".to_string())
    } else {
        Some(format!(
            "Context: nothing is selected; the caret is inside this paragraph (select_text with an excerpt of it, then type_text, to change it):\n«{}»",
            cut(&para)
        ))
    }
}

/// Send `prompt` as a turn. `draft` says it came from the margin icon's Draft box: the answer
/// goes into the document, not the chat. Errors say why nothing was sent.
pub(crate) fn ask(app: &mut WordApp, ctx: &egui::Context, prompt: &str, draft: bool) -> Result<(), String> {
    let prompt = prompt.trim().to_string();
    let Some(host) = app.assistant.host.clone() else { return Err(tl!("GeenioAI is not available in this version yet.").to_string()) };
    if !host.has_key() {
        return Err(tl!("Connect your account first: paste its account key in the GeenioAI pane.").to_string());
    }
    if prompt.is_empty() {
        return Err("the prompt is empty".into());
    }
    if app.assistant.busy {
        return Err(tl!("The assistant is still working on the last request.").to_string());
    }
    let mut context = context(app).unwrap_or_default();
    if draft {
        context.push_str("\nThe user typed this in the Draft box on the page: write the requested text into the document at the caret with type_text (paragraphs: true for several paragraphs); don't answer in the chat only.");
    }
    let a = &mut app.assistant;
    a.push(Role::User, prompt.clone());
    a.busy = true;
    a.activity = None;
    a.streaming = None;
    let conversation = a.conversation.take();
    let instructions = a.language.instruction();
    let chosen = a.model.clone();
    let (updates, tools, ctx) = (a.updates_tx.clone(), a.tools_tx.clone(), ctx.clone());
    let started = spawn(move || {
        let done = |conversation, error, balance, stopped| {
            let _ = updates.send(Update::Done { conversation, error, balance, stopped });
            ctx.request_repaint();
        };
        // Agreement comes before anything leaves the computer; a no keeps the conversation as it was.
        if let Some(consent) = host.consent() {
            let (reply, answer) = channel();
            let _ = updates.send(Update::Consent { consent, reply });
            ctx.request_repaint();
            if !answer.recv_timeout(CONFIRM_TIMEOUT).unwrap_or(false) {
                return done(conversation, Some(tl!("Nothing was sent: GeenioAI needs your agreement first.").to_string()), None, false);
            }
            if let Err(e) = host.agree() {
                return done(conversation, Some(e), None, false);
            }
        }
        let mut conv = match conversation {
            Some(c) => c,
            None => match host.provider() {
                Ok((provider, suggested)) => {
                    let model = chosen.unwrap_or(suggested);
                    Conversation { agent: Agent::new(Config { instructions, ..Config::new(model) }), provider }
                }
                Err(e) => return done(None, Some(e), None, false),
            },
        };
        let mut backend = UiBackend { tx: tools, ctx: ctx.clone() };
        let mut observer = UiObserver { tx: updates.clone(), ctx: ctx.clone() };
        let r = conv.agent.send_with_context(conv.provider.as_mut(), &mut backend, &prompt, Some(&context), &mut observer);
        // Messages the pane can translate: the endpoint's own text, or a fixed sentence.
        let (error, stopped) = match r {
            Ok(t) => (None, t.stopped),
            Err(wordcraft_assistant::Error::StepLimit { .. }) => (None, true),
            Err(wordcraft_assistant::Error::Provider(m)) => (Some(m), false),
            Err(other) => (Some(other.to_string()), false),
        };
        done(Some(conv), error, host.balance(), stopped);
    });
    if let Err(e) = started {
        let a = &mut app.assistant;
        a.busy = false;
        a.push(Role::Error, e);
        return Err(a.messages.last().map(|m| m.text.clone()).unwrap_or_default());
    }
    Ok(())
}

/// The pane's Send button: the typed prompt, with the document context.
fn send(app: &mut WordApp, ctx: &egui::Context) {
    let prompt = std::mem::take(&mut app.assistant.input);
    if prompt.trim().is_empty() {
        app.assistant.input = prompt;
        return;
    }
    if let Err(e) = ask(app, ctx, &prompt, false) {
        app.assistant.input = prompt;
        app.status(e);
    }
}

/// Settings › Change key: check the new key and put it in place of the stored one.
fn change_key(app: &mut WordApp) {
    let a = &mut app.assistant;
    let key = std::mem::take(&mut a.settings.new_key).trim().to_string();
    let Some(host) = a.host.clone() else { return };
    if key.is_empty() || a.settings.busy || a.busy {
        return;
    }
    a.settings.busy = true;
    a.settings.result = None;
    let tx = a.updates_tx.clone();
    if let Err(e) = spawn(move || {
        let _ = tx.send(Update::KeyChanged(host.change_key(key)));
    }) {
        a.settings.busy = false;
        a.settings.result = Some(Err(e));
    }
}

/// Read the credit again (a click on it in the header).
fn refresh_balance(a: &mut Assistant, ctx: &egui::Context) {
    let Some(host) = a.host.clone() else { return };
    // During a turn the conversation is on the worker thread, and the turn's end brings a
    // fresh reading anyway.
    if a.balance_busy || a.busy || !host.has_key() {
        return;
    }
    a.balance_busy = true;
    let (tx, ctx) = (a.updates_tx.clone(), ctx.clone());
    let started = spawn(move || {
        let _ = tx.send(Update::Balance(host.balance()));
        ctx.request_repaint();
    });
    if started.is_err() {
        a.balance_busy = false;
    }
}

/// Forget the conversation and start over with the greeting.
fn new_conversation(a: &mut Assistant) {
    a.conversation = None;
    a.messages.clear();
    a.clear_asked = false;
    a.greet();
}

/// Read the account of the stored key once, for the credit and the settings window.
fn request_account(a: &mut Assistant, ctx: &egui::Context) {
    let Some(host) = a.host.clone() else { return };
    if a.account_requested || a.account.is_some() || !host.has_key() {
        return;
    }
    a.account_requested = true;
    let (tx, ctx) = (a.updates_tx.clone(), ctx.clone());
    let _ = spawn(move || {
        let _ = tx.send(Update::Account(host.account()));
        ctx.request_repaint();
    });
}

/// Read the model list once, when the settings window first shows it.
fn request_models(a: &mut Assistant, ctx: &egui::Context) {
    let Some(host) = a.host.clone() else { return };
    if a.settings.models_requested || !host.has_key() {
        return;
    }
    a.settings.models_requested = true;
    let (tx, ctx) = (a.updates_tx.clone(), ctx.clone());
    if let Err(e) = spawn(move || {
        let _ = tx.send(Update::Models(host.models()));
        ctx.request_repaint();
    }) {
        a.settings.models = Some(Err(e));
    }
}

/// Per frame: run the tools the agent asked for, and take in its progress.
pub fn tick(app: &mut WordApp, ctx: &egui::Context) {
    if let Some(rx) = app.assistant.tools_rx.take() {
        while let Ok(req) = rx.try_recv() {
            // A request its sender gave up on was reported as failed: running it now would change
            // the document behind the model's back.
            if !req.take() {
                continue;
            }
            // One turn is one undo step: from the second change on, the command joins the step
            // the turn's first change made.
            match app.assistant.turn_undo_depth {
                Some(start) if app.session.undo_depth() > start => app.session.join_next_undo(),
                Some(_) => {}
                None => app.assistant.turn_undo_depth = Some(app.session.undo_depth()),
            }
            let v = match crate::control::handle(app, ctx, &req) {
                Outcome::Done(v) => v,
                Outcome::Screenshot { .. } => json!({"ok": false, "error": "screenshots are not available to the assistant"}),
            };
            let _ = req.reply.send(v);
            ctx.request_repaint();
        }
        app.assistant.tools_rx = Some(rx);
    }
    let Some(rx) = app.assistant.updates_rx.take() else { return };
    let registry = app.session.registry.clone();
    while let Ok(u) = rx.try_recv() {
        if update(&mut app.assistant, u, &registry) {
            // The turn's typing is over: what the user types next is a new undo step.
            app.session.close_typing();
            app.assistant.turn_undo_depth = None;
        }
    }
    app.assistant.updates_rx = Some(rx);
}

/// Take in one update. True when a turn just ended. `registry` words the status line.
fn update(a: &mut Assistant, u: Update, registry: &wordcraft_engine::Registry) -> bool {
    match u {
        Update::Connected(Ok(account)) => {
            a.busy = false;
            a.activity = None;
            a.push(Role::Assistant, crate::i18n::fmt(tl!("Connected to account {conto}."), &[("conto", &account.conto_id)]));
            if let Some(host) = &a.host {
                host.event(&AssistantEvent::KeyRegistered { conto_id: account.conto_id.clone() });
            }
            a.balance = account.remaining_usd;
            a.account = Some(account);
            a.push(Role::Assistant, tl!("What shall we write?"));
            a.focus_input = true;
        }
        Update::Connected(Err(e)) => {
            a.busy = false;
            a.activity = None;
            a.push(Role::Error, tl!(&e).to_string());
        }
        Update::KeyChanged(Ok(account)) => {
            a.settings.busy = false;
            if let Some(host) = &a.host {
                host.event(&AssistantEvent::KeyChanged { conto_id: account.conto_id.clone() });
            }
            let text = crate::i18n::fmt(tl!("The account key was changed: now connected to account {conto}."), &[("conto", &account.conto_id)]);
            a.settings.result = Some(Ok(text.clone()));
            a.push(Role::Notice, text);
            // The conversation's endpoint holds the old key: the next turn starts with the new one.
            a.conversation = None;
            // Another account may offer other models.
            a.settings.models = None;
            a.settings.models_requested = false;
            a.balance = account.remaining_usd;
            a.account = Some(account);
        }
        Update::KeyChanged(Err(e)) => {
            a.settings.busy = false;
            a.settings.result = Some(Err(tl!(&e).to_string()));
        }
        Update::Models(models) => {
            a.settings.models = Some(models.map(|list| list.into_iter().filter(|m| ModelInfo::is_chat(&m.id)).collect()));
        }
        Update::Account(account) => {
            if let Some(acc) = account {
                a.balance = acc.remaining_usd.or(a.balance);
                a.account = Some(acc);
            }
        }
        Update::Balance(balance) => {
            a.balance_busy = false;
            if let Some(b) = balance {
                a.balance = Some(b);
            }
        }
        Update::Text(t) => {
            a.streaming = None;
            a.push(Role::Assistant, t)
        }
        Update::StreamStart => a.streaming = Some(String::new()),
        Update::StreamText(t) => a.streaming.get_or_insert_with(String::new).push_str(&t),
        Update::Step { name, args } => {
            a.activity = Some(Activity { text: describe(registry, &name, &args), failed: false });
        }
        Update::StepFailed => {
            if let Some(s) = &mut a.activity {
                s.failed = true;
            }
        }
        Update::Declined { name } => a.push(Role::Notice, crate::i18n::fmt(tl!("Declined: {tool}"), &[("tool", &name)])),
        Update::Confirm { name, args, reply } => {
            // A new question replaces an unanswered one, which counts as declined.
            if let Some((_, old)) = a.confirm.replace((confirm_text(registry, &name, &args), reply)) {
                let _ = old.send(false);
            }
        }
        Update::Consent { consent, reply } => {
            if let Some((_, old)) = a.confirm.replace((consent_text(&consent), reply)) {
                let _ = old.send(false);
            }
        }
        Update::Done { conversation, error, balance, stopped } => {
            a.busy = false;
            a.activity = None;
            a.streaming = None;
            a.conversation = if std::mem::take(&mut a.drop_conversation_after_turn) { None } else { conversation };
            // The language and the model may have changed while the turn ran.
            let language = a.language;
            a.set_language(language);
            if let (Some(m), Some(c)) = (a.model.clone(), &mut a.conversation) {
                c.agent.set_model(m);
            }
            if let Some(b) = balance {
                a.balance = Some(b);
            }
            if stopped {
                a.push(Role::Notice, tl!("The assistant ran out of steps for this turn and stopped; the document keeps what it did."));
            }
            if let Some(e) = error {
                a.push(Role::Error, tl!(&e).to_string());
            }
            if let Some((_, reply)) = a.confirm.take() {
                let _ = reply.send(false);
            }
            return true;
        }
    }
    false
}

fn bubble(ui: &mut egui::Ui, m: &Message, t: &Tokens, account_url: &str) {
    let width = ui.available_width();
    match m.role {
        Role::User | Role::Assistant | Role::Error => {
            let user = m.role == Role::User;
            let (fill, color) = match m.role {
                Role::User => (t.checked, t.text),
                Role::Error => (t.input, t.red),
                _ => (t.input, t.text),
            };
            let layout = if user { egui::Layout::right_to_left(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Min) };
            ui.with_layout(layout, |ui| {
                egui::Frame::NONE
                    .fill(fill)
                    .stroke(egui::Stroke::new(1.0, t.border))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(9, 6))
                    .show(ui, |ui| {
                        ui.set_max_width(width * 0.88);
                        ui.add(egui::Label::new(RichText::new(&m.text).font(regular(12.5)).color(color)).wrap());
                    });
            });
        }
        Role::Notice => {
            ui.add(egui::Label::new(RichText::new(&m.text).font(regular(11.0)).color(t.text_dim)).wrap());
        }
        Role::Link => {
            ui.hyperlink_to(RichText::new(&m.text).font(regular(12.0)), account_url).on_hover_text(account_url);
        }
    }
}

/// The pane's body (inside the right panel [`crate::panes::show`] opens).
pub fn show(app: &mut WordApp, ui: &mut egui::Ui) {
    if crate::panes::header(ui, "GeenioAI") {
        app.assistant.open = false;
        return;
    }
    let t = Tokens::get(ui.ctx());
    app.assistant.greet();
    let Some(host) = app.assistant.host.clone() else {
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(tl!("GeenioAI is not available in this version yet.")).color(t.text_dim)).wrap());
        return;
    };
    let has_key = host.has_key();
    let service = host.service();
    request_account(&mut app.assistant, ui.ctx());
    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
    // Header: the credit (a click reads it again), a fresh start and the settings — icons, so
    // the credit keeps its room in a narrow pane.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::icon_button(ui, "gear", tl!("GeenioAI settings")).clicked() {
                app.assistant.settings.open = !app.assistant.settings.open;
                app.assistant.settings.result = None;
            }
            let can_reset = has_key && !app.assistant.busy && !app.assistant.messages.is_empty();
            if ui.add_enabled_ui(can_reset, |ui| crate::widgets::icon_button(ui, "broom", tl!("New conversation"))).inner.clicked() {
                app.assistant.clear_asked = true;
            }
            if let Some(b) = app.assistant.balance {
                let mut refresh = false;
                if app.assistant.balance_busy {
                    ui.spinner();
                } else {
                    refresh = crate::widgets::icon_button(ui, "refresh", tl!("Refresh credit")).clicked();
                }
                // Top up: the account site, in the browser.
                if crate::widgets::icon_button(ui, "creditCard", tl!("Top up credit")).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&service.url));
                }
                // The credit takes what is left and is cut short, never run over.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let text = crate::i18n::fmt(tl!("Credit available: {usd} USD"), &[("usd", &format!("{b:.2}"))]);
                    let label = egui::Label::new(RichText::new(text).font(regular(11.0)).color(t.text_dim)).truncate().sense(Sense::click());
                    refresh |= ui.add(label).on_hover_text(tl!("Refresh credit")).clicked();
                });
                if refresh {
                    refresh_balance(&mut app.assistant, ui.ctx());
                }
            }
        });
    });
    ui.separator();
    // A long question (the agreement) wraps to more lines than a tool's.
    let card = app.assistant.confirm.as_ref().map_or(0.0, |(q, _)| if q.chars().count() > 120 { 110.0 } else { 76.0 });
    let footer = if has_key { 78.0 } else { 66.0 } + card;
    let list_height = (ui.available_height() - footer).max(80.0);
    egui::ScrollArea::vertical().id_salt("assistant-messages").max_height(list_height).auto_shrink([false, false]).stick_to_bottom(true).show(
        ui,
        |ui| {
            ui.add_space(4.0);
            // A margin frame, not a horizontal row: a right-aligned bubble inside a row takes the
            // row's unbounded width and runs past the pane's edge.
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(2, 0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for m in &app.assistant.messages {
                    bubble(ui, m, &t, &service.url);
                }
                // The reply streaming in, as a bubble that fills up.
                if let Some(text) = app.assistant.streaming.as_ref().filter(|s| !s.is_empty()) {
                    bubble(ui, &Message { role: Role::Assistant, text: text.clone() }, &t, &service.url);
                }
            });
            // One status line: the spinner and the current action, replaced by the next one and
            // gone when the turn ends.
            if app.assistant.busy {
                let (text, failed) = match &app.assistant.activity {
                    Some(s) => (s.text.clone(), s.failed),
                    None => (tl!("Working…").to_string(), false),
                };
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add(egui::Label::new(RichText::new(text).font(regular(12.0)).color(if failed { t.orange } else { t.text_dim })).wrap());
                });
            }
        },
    );
    ui.separator();
    // A tool that replaces the document or writes a file waits for an answer.
    let mut answer = None;
    if let Some((question, _)) = &app.assistant.confirm {
        egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.accent)).corner_radius(6.0).inner_margin(8).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(question).color(t.text)).wrap());
            ui.horizontal(|ui| {
                if ui.add(crate::widgets::primary_button(ui, tl!("Allow"))).clicked() {
                    answer = Some(true);
                }
                if ui.button(tl!("Decline")).clicked() {
                    answer = Some(false);
                }
            });
        });
    }
    if let Some(yes) = answer
        && let Some((_, reply)) = app.assistant.confirm.take()
    {
        let _ = reply.send(yes);
    }
    let ctx = ui.ctx().clone();
    let busy = app.assistant.busy;
    if has_key {
        // Suggestions, as Word's pane offers them, until the conversation starts.
        if app.assistant.conversation.is_none() && !busy {
            ui.horizontal_wrapped(|ui| {
                for (label, prompt) in [
                    ("Summarize this document", "Summarize this document in a few sentences."),
                    ("Suggest a title", "Suggest three possible titles for this document."),
                    ("Check the tone", "Read this document and tell me whether its tone is consistent, and what to improve."),
                ] {
                    if ui.add(egui::Button::new(RichText::new(tl!(label)).font(regular(11.5))).corner_radius(10.0)).clicked() {
                        app.assistant.input = tl!(prompt).to_string();
                        send(app, &ctx);
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 64.0).max(60.0);
            let field = egui::TextEdit::multiline(&mut app.assistant.input)
                .desired_rows(2)
                .hint_text(tl!("Ask GeenioAI…"))
                .desired_width(w)
                .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)));
            let r = ui.add_enabled(!busy, field);
            if std::mem::take(&mut app.assistant.focus_input) {
                r.request_focus();
            }
            let enter = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
            let b = ui.add_enabled(!busy && !app.assistant.input.trim().is_empty(), crate::widgets::primary_button(ui, tl!("Send")));
            if enter || b.clicked() {
                send(app, &ctx);
                r.request_focus();
            }
        });
    } else {
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 84.0).max(60.0);
            let field = egui::TextEdit::singleline(&mut app.assistant.key_input).password(true).hint_text(tl!("Account key")).desired_width(w);
            let r = ui.add_enabled(!busy, field);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let b = ui.add_enabled(!busy && !app.assistant.key_input.trim().is_empty(), crate::widgets::primary_button(ui, tl!("Connect")));
            if enter || b.clicked() {
                connect(app);
            }
        });
    }
    clear_question(app, ui.ctx());
    settings_window(app, ui.ctx(), has_key);
}

/// A fresh start asks first: the messages go away for good.
fn clear_question(app: &mut WordApp, ctx: &egui::Context) {
    if !app.assistant.clear_asked {
        return;
    }
    let t = Tokens::get(ctx);
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("assistant-clear")).show(ctx, |ui| {
        ui.set_width(320.0);
        ui.label(RichText::new(tl!("New conversation")).font(semibold(13.0)));
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(tl!("Start a new conversation? The messages so far will be cleared.")).color(t.text)).wrap());
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(crate::widgets::primary_button(ui, tl!("Clear"))).clicked() {
                answer = Some(true);
            } else if ui.button(tl!("Keep")).clicked() {
                answer = Some(false);
            }
        });
    });
    // Escape or a click outside keeps the conversation.
    if answer.is_none() && modal.should_close() {
        answer = Some(false);
    }
    match answer {
        Some(true) => new_conversation(&mut app.assistant),
        Some(false) => app.assistant.clear_asked = false,
        None => {}
    }
}

/// Assistant settings: the reply language, the model and the account key.
fn settings_window(app: &mut WordApp, ctx: &egui::Context, has_key: bool) {
    if !app.assistant.settings.open {
        return;
    }
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut change = false;
    egui::Window::new(tl!("GeenioAI settings"))
        .id(egui::Id::new("assistant-settings"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            let a = &mut app.assistant;
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.label(RichText::new(tl!("Assistant language")).font(semibold(12.5)));
            let mut language = a.language;
            for l in Language::ALL {
                ui.radio_value(&mut language, l, l.name());
            }
            if language != a.language {
                a.set_language(language);
            }
            if has_key {
                ui.separator();
                ui.label(RichText::new(tl!("Model")).font(semibold(12.5)));
                let suggestion = match a.account.as_ref().map(|acc| acc.model.as_str()).filter(|m| !m.is_empty()) {
                    Some(m) => crate::i18n::fmt(tl!("Suggested by the account ({model})"), &[("model", m)]),
                    None => tl!("Suggested by the account").to_string(),
                };
                request_models(a, ctx);
                let current = match (&a.model, &a.settings.models) {
                    (None, _) => suggestion.clone(),
                    (Some(id), Some(Ok(list))) => list.iter().find(|m| &m.id == id).map_or_else(|| id.clone(), ModelInfo::label),
                    (Some(id), _) => id.clone(),
                };
                ui.label(RichText::new(current).color(t.accent_text));
                // A list, not a drop-down: there can be hundreds of models, and a search field
                // inside a drop-down closes it at the first click.
                ui.add(egui::TextEdit::singleline(&mut a.settings.model_filter).hint_text(tl!("Search models…")).desired_width(330.0));
                let mut choice = None;
                egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(4).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("assistant-models").max_height(200.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.set_min_width(322.0);
                        if ui.selectable_label(a.model.is_none(), &suggestion).clicked() {
                            choice = Some(None);
                        }
                        match &a.settings.models {
                            None => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new(tl!("Loading models…")).color(t.text_dim));
                                });
                            }
                            Some(Err(e)) => {
                                ui.label(RichText::new(tl!("The models could not be loaded")).color(t.red)).on_hover_text(e);
                            }
                            Some(Ok(list)) => {
                                let needle = a.settings.model_filter.trim().to_lowercase();
                                for m in list.iter().filter(|m| {
                                    needle.is_empty()
                                        || m.id.to_lowercase().contains(&needle)
                                        || m.name.as_deref().is_some_and(|n| n.to_lowercase().contains(&needle))
                                }) {
                                    let r = ui.selectable_label(a.model.as_deref() == Some(m.id.as_str()), m.label()).on_hover_text(&m.id);
                                    if r.clicked() {
                                        choice = Some(Some(m.id.clone()));
                                    }
                                }
                            }
                        }
                    });
                });
                if let Some(c) = choice {
                    a.set_model(c);
                }
            }
            ui.separator();
            ui.label(RichText::new(tl!("Account")).font(semibold(12.5)));
            match &a.account {
                Some(acc) => {
                    ui.label(crate::i18n::fmt(tl!("Connected account: {conto}"), &[("conto", &acc.conto_id)]));
                    if let Some(b) = a.balance {
                        ui.label(
                            RichText::new(crate::i18n::fmt(tl!("Credit available: {usd} USD"), &[("usd", &format!("{b:.2}"))])).color(t.text_dim),
                        );
                    }
                }
                None if has_key => {
                    ui.label(RichText::new(tl!("Reading the account…")).color(t.text_dim));
                }
                None => {
                    ui.label(RichText::new(tl!("No account key is set")).color(t.text_dim));
                }
            }
            if has_key {
                ui.add_space(4.0);
                ui.label(tl!("Change account key"));
                ui.horizontal(|ui| {
                    let busy = a.settings.busy || a.busy;
                    let field =
                        egui::TextEdit::singleline(&mut a.settings.new_key).password(true).hint_text(tl!("New account key")).desired_width(220.0);
                    let r = ui.add_enabled(!busy, field);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let b = ui.add_enabled(!busy && !a.settings.new_key.trim().is_empty(), egui::Button::new(tl!("Change key")));
                    change = enter || b.clicked();
                    if a.settings.busy {
                        ui.spinner();
                    }
                });
                match &a.settings.result {
                    Some(Ok(m)) => {
                        ui.label(RichText::new(m).color(t.text_dim));
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(t.red));
                    }
                    None => {}
                }
            }
        });
    if change {
        change_key(app);
    }
    if !open {
        app.assistant.settings.open = false;
        app.assistant.settings.new_key.clear();
    }
}

// ---------------------------------------------------------------------------------------------
// The margin icon: Word's Copilot icon beside the paragraph the caret is in.

/// Whether the margin icon may be shown for the current app state.
pub fn icon_visible(app: &WordApp) -> bool {
    if !app.assistant.available() || !app.has_document() {
        return false;
    }
    // Reading Mode has no editing; neither does a modal dialog, the Backstage or the context
    // menu, which owns the pointer.
    if app.session.view.read_mode || app.dialog.is_some() || app.colors.is_some() || app.ui.backstage || app.canvas.context_menu_open {
        return false;
    }
    // Never while a drag-select or a table drag is in progress, nor while keytips show.
    if app.canvas.drag_selecting() || app.canvas.table_drag.is_some() || app.ui.keytips != crate::keytips::Phase::Off {
        return false;
    }
    // Body text only: headers, footers, notes, text boxes and equations keep their margins.
    app.session.sel.focus.story == wordcraft_doc::StoryRef::Body && app.session.math.is_none() && app.session.cell_range().is_none()
}

/// Where the icon sits: in the left margin beside `line` (the caret's line, or the selection's
/// first), `body_left` points from the page body's left edge, inside `viewport`. With the line
/// scrolled out of view the icon stays at the viewport's top or bottom edge, so it is always
/// there to click. `None` when the inputs are degenerate (NaN rects must never reach egui).
pub fn icon_rect(line: Rect, body_left: f32, viewport: Rect) -> Option<Rect> {
    if !line.is_finite() || !body_left.is_finite() || !viewport.is_finite() || !viewport.is_positive() {
        return None;
    }
    let y = (line.center().y - ICON / 2.0).clamp(viewport.top() + ICON_GAP, (viewport.bottom() - ICON - ICON_GAP).max(viewport.top() + ICON_GAP));
    let x = (body_left - ICON_GAP - ICON).max(viewport.left() + 2.0);
    if x + ICON > viewport.right() {
        return None;
    }
    Some(Rect::from_min_size(egui::pos2(x, y), vec2(ICON, ICON)))
}

/// Draw the margin icon and its menu. `viewport` is the canvas area it must stay inside.
pub fn floating_icon(app: &mut WordApp, ctx: &egui::Context, viewport: Rect) {
    if !icon_visible(app) {
        return;
    }
    let Some((line, body_left)) = crate::canvas::assistant_anchor(app) else { return };
    let Some(rect) = icon_rect(line, body_left, viewport) else { return };
    let t = Tokens::get(ctx);
    let selected = !app.session.sel.is_collapsed() && !app.session.selected_text().trim().is_empty();
    let empty_para = !selected && app.session.doc.para_at(&app.session.sel.focus).is_some_and(|p| p.plain_text().trim().is_empty());
    egui::Area::new(egui::Id::new("wc_assistant_icon")).order(egui::Order::Foreground).interactable(true).fixed_pos(rect.min).show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(rect.size(), Sense::click());
        let hovered = resp.hovered();
        let p = ui.painter();
        p.circle_filled(r.center() + vec2(0.0, 1.0), ICON / 2.0, t.page_shadow);
        p.circle_filled(r.center(), ICON / 2.0, if hovered { t.hover } else { t.menu });
        p.circle_stroke(r.center(), ICON / 2.0, egui::Stroke::new(1.0, if hovered { t.accent } else { t.border_strong }));
        crate::icons::paint(p, Rect::from_center_size(r.center(), vec2(15.0, 15.0)), "assistant", t.icon, t.accent);
        let resp = resp.on_hover_text(if empty_para { tl!("Draft with GeenioAI") } else { tl!("GeenioAI") });
        // A click toggles the menu (the popup's own click handling); Escape or a click outside closes it.
        egui::Popup::menu(&resp).show(|ui| icon_menu(app, ui, selected, empty_para));
    });
}

/// The margin icon's menu: a prompt box and the quick actions that fit where the caret is.
fn icon_menu(app: &mut WordApp, ui: &mut egui::Ui, selected: bool, empty_para: bool) {
    ui.set_min_width(260.0);
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let ready = app.assistant.ready();
    if !ready {
        ui.add(egui::Label::new(RichText::new(tl!("Connect your account in the GeenioAI pane to start.")).color(t.text_dim)).wrap());
        if ui.button(tl!("Open GeenioAI")).clicked() {
            open(app);
            ui.close();
        }
        return;
    }
    let hint = if empty_para { tl!("Describe what you'd like to write") } else { tl!("Ask GeenioAI…") };
    let field = egui::TextEdit::singleline(&mut app.assistant.quick_prompt).hint_text(hint).desired_width(250.0);
    let r = ui.add(field);
    if ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let go = ui.add_enabled(!app.assistant.quick_prompt.trim().is_empty(), egui::Button::new(if empty_para { tl!("Generate") } else { tl!("Send") }));
    if enter || go.clicked() {
        let prompt = std::mem::take(&mut app.assistant.quick_prompt);
        quick(app, &ctx, &prompt, empty_para);
        ui.close();
        return;
    }
    ui.separator();
    let mut action: Option<(&str, bool)> = None;
    if selected {
        for (label, prompt) in [
            ("Rewrite the selection", "Rewrite the selected text so it reads better, keeping its meaning, language and formatting."),
            ("Make it shorter", "Make the selected text shorter, keeping its meaning, language and formatting."),
            ("Fix spelling and grammar", "Fix the spelling and grammar of the selected text; change nothing else."),
            ("Summarize the selection", "Summarize the selected text in a few sentences, in the chat."),
        ] {
            if ui.button(tl!(label)).clicked() {
                action = Some((prompt, false));
            }
        }
    } else if !empty_para && ui.button(tl!("Rewrite this paragraph")).clicked() {
        action = Some(("Rewrite the paragraph the caret is in so it reads better, keeping its meaning, language and formatting.", false));
    }
    ui.separator();
    if ui.button(tl!("Open GeenioAI")).clicked() {
        open(app);
        app.assistant.focus_input = true;
        ui.close();
    }
    if let Some((prompt, draft)) = action {
        quick(app, &ctx, tl!(prompt), draft);
        ui.close();
    }
}

/// A quick action from the margin icon: the pane opens and the prompt is sent.
fn quick(app: &mut WordApp, ctx: &egui::Context, prompt: &str, draft: bool) {
    open(app);
    if let Err(e) = ask(app, ctx, prompt, draft) {
        app.status(e);
    }
}

#[cfg(test)]
mod tests;

//! The agent loop: the model answers or asks for tools, the tools run against a [`Backend`], and
//! their results go back to the model until it answers in text.

use std::fmt;

use serde_json::{Map, Value, json};
use wordcraft_mcp::Backend;
use wordcraft_mcp::tools::{call_tool, tool_definitions};

use crate::provider::{Delta, Provider};

/// The most tool-calling model calls one turn may make (the default and the cap of
/// [`Config::max_steps`]). Past it the model gets one last call, without tools, to say what it
/// did.
pub const MAX_STEPS: usize = 20;
/// Tool calls taken from one model reply; the rest are dropped.
const MAX_CALLS_PER_STEP: usize = 16;
/// A tool result longer than this (in characters) is cut before it goes back to the model.
const MAX_RESULT_CHARS: usize = 24_000;
/// A longer prompt is cut.
const MAX_PROMPT_CHARS: usize = 8_000;
/// A longer context (the selection sent with a prompt) is cut.
const MAX_CONTEXT_CHARS: usize = 12_000;
/// Past this many messages the oldest turns are forgotten.
const MAX_MESSAGES: usize = 400;

/// The MCP tools offered to the model: the ones that work at command level. `render_page`,
/// `screenshot`, `parity` and the `ui_*` tools are left out: images and pointer events don't fit
/// a text conversation.
const TOOLS: &[&str] = &[
    "get_text",
    "inspect_document",
    "select_text",
    "type_text",
    "execute",
    "batch",
    "list_commands",
    "new_document",
    "open_document",
    "save_document",
];

/// Tools that replace the document or write files: they run only when the user confirms.
const CONFIRM: &[&str] = &["new_document", "open_document", "save_document"];

/// The command ids an `execute` or `batch` call runs.
fn command_ids<'a>(name: &str, args: &'a Value) -> Vec<&'a str> {
    match name {
        "execute" => args.get("command").and_then(Value::as_str).into_iter().collect(),
        "batch" => {
            args.get("commands").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("command").and_then(Value::as_str)).collect()
        }
        _ => Vec::new(),
    }
}

/// Whether the call needs the user's yes: a confirmed tool, or a `file.*` command (open, save,
/// export…) through `execute` or `batch`, which would otherwise get past the confirmation.
pub(crate) fn needs_confirmation(name: &str, args: &Value) -> bool {
    CONFIRM.contains(&name) || command_ids(name, args).iter().any(|c| c.trim().starts_with("file."))
}

/// Why the call is refused outright: the interface's own commands (`ui.*`) are not the assistant's
/// to run (they could open the pane again, or change the user's settings).
fn refused(name: &str, args: &Value) -> Option<String> {
    let id = command_ids(name, args).into_iter().find(|c| c.trim().starts_with("ui."))?;
    Some(format!("error: `{}` is an interface command; the assistant can't run it", id.trim()))
}

const SYSTEM: &str = "You are GeenioAI, the writing assistant inside WordCraft, a word processor that works like Microsoft Word. \
You help the user write and edit the open document with your tools: draft text, rewrite, shorten, expand, summarize, \
fix spelling and grammar, format, and answer questions about the document.\n\
- Read before you write: get_text gives the document's plain text, inspect_document its structure (paragraphs with \
styles, lists, tables, the selection, pages). Don't read again what you already read in this turn.\n\
- type_text types at the caret and replaces the selection, so to rewrite a passage select it first with select_text \
(an exact excerpt of the document's text) and then type_text the new version. Pass `paragraphs: true` to type several \
paragraphs (one per line). A draft goes where the caret is; the user's message says where the caret and the selection are.\n\
- Format with execute and command ids: format.bold, format.italic, para.style {\"style\": \"Heading 1\"}, \
para.alignCenter, list.bullets, insert.table {\"rows\", \"cols\"}, insert.pageBreak, edit.findReplace… \
list_commands (with a query) shows command ids and their parameters. Commands act on the selection or at the caret.\n\
- Keep the user's language, tone and formatting unless asked otherwise. When rewriting, replace only the selected \
text and keep its meaning. When summarizing or answering a question, answer in the chat without changing the document \
unless the user asked to insert the result.\n\
- You have a limited number of steps per turn: plan the work, then act. Do what was asked and stop. After changing \
the document, check the result at most once.\n\
- new_document, open_document and save_document ask the user first; if the user declines, don't retry: ask what they \
want instead.\n\
- If a request is ambiguous in a way that changes the result, ask one short question before changing the document.\n\
- If a command is refused, read the error and try another way at most twice, then explain what went wrong.\n\
- Reply briefly, in the user's language: what you changed, or the answer to the question. Never show tool names, \
command ids or JSON to the user.";

/// The last message of a turn that ran out of steps.
const WRAP_UP: &str = "You have used all the steps allowed for this turn and your tools are now off. \
In one or two sentences, in the user's language, tell the user what you did and what is left undone.";

/// How the agent talks to its model.
#[derive(Clone, Debug)]
pub struct Config {
    /// The model id sent with each request, as the provider names it.
    pub model: String,
    /// Model calls allowed per turn (1 to [`MAX_STEPS`]).
    pub max_steps: usize,
    /// Extra instructions appended to the system prompt.
    pub instructions: Option<String>,
}

impl Config {
    pub fn new(model: impl Into<String>) -> Self {
        Self { model: model.into(), max_steps: MAX_STEPS, instructions: None }
    }
}

/// Something that happened during a turn, for showing progress.
#[derive(Debug)]
pub enum Event<'a> {
    /// Text from the model (its reply, or what it says while working).
    Text(&'a str),
    /// The model asked for a tool.
    ToolCall { name: &'a str, arguments: &'a Value },
    /// A tool ran; `text` is what the model sees.
    ToolResult { name: &'a str, ok: bool, text: &'a str },
    /// The user declined a tool that needed confirmation.
    Declined { name: &'a str },
    /// A piece of the model's text as it streams in; the whole text follows as [`Event::Text`].
    Stream(Delta<'a>),
}

/// The user's side of a turn: confirmations and progress. `Send`, so a streaming endpoint can
/// hand it the pieces of a reply from the thread that reads them.
pub trait Observer: Send {
    /// Whether the tool `name` (one that replaces the document or writes a file) may run.
    /// Declined unless the host says otherwise.
    fn confirm(&mut self, name: &str, arguments: &Value) -> bool {
        let _ = (name, arguments);
        false
    }
    fn event(&mut self, event: Event<'_>) {
        let _ = event;
    }
}

/// The observer that declines everything and shows nothing.
impl Observer for () {}

/// How a turn ended.
#[derive(Debug, PartialEq)]
pub struct Turn {
    /// The model's final text.
    pub reply: String,
    /// Model calls made.
    pub steps: usize,
    /// Tools run (or declined).
    pub tool_calls: usize,
    /// The step budget ran out: `reply` is the model's account of what it did and what is left,
    /// made without tools.
    pub stopped: bool,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    EmptyPrompt,
    /// The endpoint could not be reached or refused the request.
    Provider(String),
    /// The endpoint answered with something that isn't a chat-completions response.
    BadResponse(String),
    /// The model was still calling tools after `steps` calls.
    StepLimit {
        steps: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EmptyPrompt => write!(f, "the prompt is empty"),
            Error::Provider(e) => write!(f, "model endpoint: {e}"),
            Error::BadResponse(e) => write!(f, "model endpoint: unexpected reply: {e}"),
            Error::StepLimit { steps } => {
                write!(f, "the assistant was still working after {steps} steps and was stopped; the document keeps what it did so far")
            }
        }
    }
}

impl std::error::Error for Error {}

/// One tool call taken from a model reply.
struct Call {
    id: String,
    name: String,
    /// The parsed arguments, or why they couldn't be parsed.
    arguments: Result<Value, String>,
}

/// A conversation with the model about the open document. Keep it across turns so the model
/// remembers what was said; [`Agent::reset`] starts over.
pub struct Agent {
    config: Config,
    messages: Vec<Value>,
}

impl Agent {
    pub fn new(config: Config) -> Self {
        let mut a = Self { config, messages: Vec::new() };
        a.reset();
        a
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The conversation so far, in chat-completions form (system prompt first).
    pub fn messages(&self) -> &[Value] {
        &self.messages
    }

    /// Forget the conversation.
    pub fn reset(&mut self) {
        self.messages = vec![self.system_message()];
    }

    /// Change the extra instructions (e.g. the reply language) and keep the conversation: the
    /// next turn sends the new system prompt.
    pub fn set_instructions(&mut self, instructions: Option<String>) {
        self.config.instructions = instructions;
        let system = self.system_message();
        match self.messages.first_mut() {
            Some(first) => *first = system,
            None => self.messages.push(system),
        }
    }

    /// Change the model and keep the conversation: the next turn asks the new model.
    pub fn set_model(&mut self, model: impl Into<String>) {
        self.config.model = model.into();
    }

    fn system_message(&self) -> Value {
        let system = match &self.config.instructions {
            Some(extra) if !extra.trim().is_empty() => format!("{SYSTEM}\n\n{}", extra.trim()),
            _ => SYSTEM.to_string(),
        };
        json!({"role": "system", "content": system})
    }

    /// The tool definitions sent to the model, in chat-completions form.
    pub fn tools() -> Vec<Value> {
        tool_definitions()
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["name"].as_str().is_some_and(|n| TOOLS.contains(&n)))
            .map(|t| json!({"type": "function", "function": {"name": t["name"], "description": t["description"], "parameters": t["inputSchema"]}}))
            .collect()
    }

    /// Run one turn: send `prompt`, let the model use its tools until it answers.
    pub fn send(&mut self, provider: &mut dyn Provider, backend: &mut dyn Backend, prompt: &str, observer: &mut dyn Observer) -> Result<Turn, Error> {
        self.send_with_context(provider, backend, prompt, None, observer)
    }

    /// [`Agent::send`] with `context` added after the prompt: where the caret is, what is
    /// selected. The context is part of the user message the model sees, not of what the
    /// user typed.
    pub fn send_with_context(
        &mut self,
        provider: &mut dyn Provider,
        backend: &mut dyn Backend,
        prompt: &str,
        context: Option<&str>,
        observer: &mut dyn Observer,
    ) -> Result<Turn, Error> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(Error::EmptyPrompt);
        }
        self.forget_old_turns();
        let mut content = truncate(prompt, MAX_PROMPT_CHARS);
        if let Some(c) = context.map(str::trim).filter(|c| !c.is_empty()) {
            content.push_str("\n\n");
            content.push_str(&truncate(c, MAX_CONTEXT_CHARS));
        }
        self.messages.push(json!({"role": "user", "content": content}));
        let tools = Self::tools();
        let max_steps = self.config.max_steps.clamp(1, MAX_STEPS);
        let mut tool_calls = 0;
        for step in 1..=max_steps {
            let request = json!({"model": self.config.model, "messages": self.messages, "tools": tools});
            let response = provider.complete_streaming(&request, &mut |d| observer.event(Event::Stream(d))).map_err(Error::Provider)?;
            let (text, calls) = parse_reply(&response)?;
            if let Some(t) = text.as_deref().filter(|t| !t.trim().is_empty()) {
                observer.event(Event::Text(t));
            }
            self.messages.push(assistant_message(text.as_deref(), &calls));
            if calls.is_empty() {
                return Ok(Turn { reply: text.unwrap_or_default(), steps: step, tool_calls, stopped: false });
            }
            for call in &calls {
                tool_calls += 1;
                let result = run_tool(backend, call, observer);
                self.messages.push(json!({"role": "tool", "tool_call_id": call.id, "content": result}));
            }
        }
        // Out of steps: the tools go away and the model gets one last call to tell the user what
        // it did and what is left. The nudge stays out of the history; only the answer goes in.
        let mut messages = self.messages.clone();
        messages.push(json!({"role": "user", "content": WRAP_UP}));
        let request = json!({"model": self.config.model, "messages": messages});
        let response = provider.complete_streaming(&request, &mut |d| observer.event(Event::Stream(d))).map_err(Error::Provider)?;
        let (text, calls) = parse_reply(&response)?;
        match text.filter(|t| !t.trim().is_empty()) {
            Some(reply) if calls.is_empty() => {
                observer.event(Event::Text(&reply));
                self.messages.push(assistant_message(Some(&reply), &[]));
                Ok(Turn { reply, steps: max_steps + 1, tool_calls, stopped: true })
            }
            _ => Err(Error::StepLimit { steps: max_steps }),
        }
    }

    /// Past [`MAX_MESSAGES`], drop the oldest turns (each from its user message up to the next),
    /// keeping the system prompt and tool calls paired with their results.
    fn forget_old_turns(&mut self) {
        while self.messages.len() > MAX_MESSAGES {
            let next_user = self.messages.iter().enumerate().skip(2).find(|(_, m)| m["role"] == "user").map(|(i, _)| i);
            match next_user {
                Some(i) => drop(self.messages.drain(1..i)),
                // One huge turn: start over rather than send a broken history.
                None => self.messages.truncate(1),
            }
        }
    }
}

/// The text and tool calls of a chat-completions response.
fn parse_reply(response: &Value) -> Result<(Option<String>, Vec<Call>), Error> {
    if let Some(e) = response.get("error") {
        let msg = e.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| e.to_string());
        return Err(Error::Provider(msg));
    }
    let message =
        response.pointer("/choices/0/message").filter(|m| m.is_object()).ok_or_else(|| Error::BadResponse(truncate(&response.to_string(), 300)))?;
    let text = message.get("content").and_then(Value::as_str).map(str::to_string);
    let calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_CALLS_PER_STEP)
        .enumerate()
        .map(|(i, c)| {
            let id = c.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).map_or_else(|| format!("call_{i}"), str::to_string);
            let name = c.pointer("/function/name").and_then(Value::as_str).unwrap_or_default().to_string();
            // The arguments arrive as a JSON string; some servers send an object.
            let arguments = match c.pointer("/function/arguments") {
                None | Some(Value::Null) => Ok(json!({})),
                Some(Value::String(s)) if s.trim().is_empty() => Ok(json!({})),
                Some(Value::String(s)) => serde_json::from_str(s).map_err(|e| format!("the arguments are not valid JSON ({e})")),
                Some(v) => Ok(v.clone()),
            };
            Call { id, name, arguments }
        })
        .collect();
    Ok((text, calls))
}

/// The model's reply as it goes back into the history: only what was understood, so a malformed
/// reply can't poison later requests.
fn assistant_message(text: Option<&str>, calls: &[Call]) -> Value {
    let mut m = Map::new();
    m.insert("role".into(), json!("assistant"));
    m.insert("content".into(), text.map_or(Value::Null, |t| json!(t)));
    if !calls.is_empty() {
        let calls: Vec<Value> = calls
            .iter()
            .map(|c| {
                let args = c.arguments.as_ref().map_or_else(|_| "{}".to_string(), Value::to_string);
                json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": args}})
            })
            .collect();
        m.insert("tool_calls".into(), Value::Array(calls));
    }
    Value::Object(m)
}

/// Run one tool call; the text is what the model sees.
fn run_tool(backend: &mut dyn Backend, call: &Call, observer: &mut dyn Observer) -> String {
    let name = call.name.as_str();
    let (ok, text) = match &call.arguments {
        _ if !TOOLS.contains(&name) => (false, format!("error: there is no tool `{name}`")),
        Err(e) => (false, format!("error: {e}")),
        Ok(args) if !args.is_object() => (false, "error: the arguments must be a JSON object".to_string()),
        Ok(args) => {
            observer.event(Event::ToolCall { name, arguments: args });
            if let Some(e) = refused(name, args) {
                let text = truncate(&e, MAX_RESULT_CHARS);
                observer.event(Event::ToolResult { name, ok: false, text: &text });
                return text;
            }
            if needs_confirmation(name, args) && !observer.confirm(name, args) {
                observer.event(Event::Declined { name });
                return format!("The user declined `{name}`. Don't retry it; ask the user what they want instead.");
            }
            let r = call_tool(backend, name, args);
            (!r.is_error, result_text(&r.content).unwrap_or_else(|| r.to_value().to_string()))
        }
    };
    let text = truncate(&text, MAX_RESULT_CHARS);
    observer.event(Event::ToolResult { name, ok, text: &text });
    text
}

/// The text blocks of an MCP tool result, or `None` when there is none.
fn result_text(content: &[Value]) -> Option<String> {
    let parts: Vec<&str> = content.iter().filter_map(|c| c["text"].as_str()).collect();
    if parts.is_empty() { None } else { Some(parts.join("\n")) }
}

/// At most `max` characters of `s`, saying how much was cut.
fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((at, _)) => format!("{}\n[… cut: {} more characters]", s.get(..at).unwrap_or(""), s.get(at..).map_or(0, |r| r.chars().count())),
    }
}

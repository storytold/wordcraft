//! GeenioAI's model endpoint on a FANTASTICO account, through the Craft AI layer's adapter.
//!
//! `craft-ai-fantastico` mints the runtime keys, fits `max_tokens`, retries, classifies failures
//! and streams, each model call a job of the layer. This crate turns GeenioAI's chat request
//! (OpenAI-style, the whole conversation) into such a job ([`request`]) and follows the job back
//! to the reply the agent reads ([`follow`]): for the desktop host, whose jobs go through the
//! layer's client (agreement, job log), and for [`Direct`], `wordcraft-cli ask`'s endpoint on a key
//! of its own.
//!
//! Native only: the adapter blocks on a socket. The web build has no assistant host.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use craft_ai_core::job::{Event, JobState};
use craft_ai_core::{AiError, CapabilityId, InputRole, JobSnapshot, ModelId, Payload, Provider as _, Request, SecretString};
use craft_ai_fantastico::{AccountInfo, FantasticoConfig, FantasticoProvider, Wire};
use serde_json::Value;
use wordcraft_assistant::{Delta, Provider};

pub use craft_ai_fantastico::{Balance, PROVIDER_NAME};

/// How often a running reply is polled for its next pieces.
const POLL: Duration = Duration::from_millis(25);
/// The longest one model call may take before it is cancelled.
const CALL_TIMEOUT: Duration = Duration::from_secs(600);
/// The application name in the provider's usage reports.
pub const TITLE: &str = "WordCraft";

/// The capability every GeenioAI request asks for.
pub fn capability() -> Result<CapabilityId, String> {
    CapabilityId::parse("text.complete").map_err(|e| e.to_string())
}

/// The input role every message of the conversation is sent as.
pub fn turn() -> Result<InputRole, String> {
    InputRole::parse(craft_ai_fantastico::TURN).map_err(|e| e.to_string())
}

/// GeenioAI's chat request as a request of the layer: each message a turn, in order, and the tools
/// as a parameter. The model is the caller's to bind.
pub fn request(capability: CapabilityId, chat: &Value, stream: bool) -> Result<Request, String> {
    let messages = chat.get("messages").and_then(Value::as_array).ok_or("The request has no messages")?;
    let turn = turn()?;
    let mut req = Request::new(capability);
    for m in messages {
        req = req.with_input(turn.clone(), Payload::text(m.to_string()));
    }
    if let Some(tools) = chat.get("tools") {
        req = req.with_param("tools", tools.clone());
    }
    Ok(if stream { req.streaming() } else { req })
}

/// The model `chat` names, trimmed.
pub fn model_of(chat: &Value) -> Result<&str, String> {
    chat.get("model").and_then(Value::as_str).map(str::trim).filter(|m| !m.is_empty()).ok_or_else(|| "The request names no model".to_string())
}

/// Follows a job (`poll`) until it ends: every piece of text to `on_delta` as it arrives, the whole
/// reply, as FANTASTICO would send it unstreamed, out. A job that outlasts the time allowed is
/// cancelled (`cancel`).
pub fn follow(
    mut poll: impl FnMut() -> Result<JobSnapshot, AiError>,
    cancel: impl FnOnce(),
    on_delta: &mut (dyn FnMut(Delta<'_>) + Send),
) -> Result<Value, String> {
    let deadline = Instant::now() + CALL_TIMEOUT;
    let (mut started, mut reply) = (false, None);
    loop {
        let snap = poll().map_err(|e| e.message)?;
        for event in snap.events {
            match event {
                Event::TextDelta { text } => {
                    if !started {
                        on_delta(Delta::Start);
                        started = true;
                    }
                    on_delta(Delta::Text(&text));
                }
                Event::Output(o) if o.label.as_deref() == Some(craft_ai_fantastico::COMPLETION) => {
                    if let Payload::Inline { data, .. } = o.payload {
                        reply = serde_json::from_slice::<Value>(&data).ok();
                    }
                }
                _ => {}
            }
        }
        match snap.state {
            JobState::Succeeded => break,
            JobState::Failed { error } => return Err(error.message),
            JobState::Cancelled => return Err("The request was cancelled".into()),
            JobState::Queued | JobState::Running { .. } => {}
        }
        if Instant::now() > deadline {
            cancel();
            return Err("FANTASTICO did not answer in time".into());
        }
        std::thread::sleep(POLL);
    }
    reply.ok_or_else(|| "FANTASTICO sent an incomplete reply".to_string())
}

/// A FANTASTICO key as the assistant's model endpoint, without the layer's store: the key comes
/// from the caller (`wordcraft-cli ask` reads `$FANTASTICO_KEY`), and each call is a job of the
/// adapter.
pub struct Direct {
    provider: FantasticoProvider,
    capability: CapabilityId,
}

impl Direct {
    /// An endpoint for `key`: an account key (`ck-fant-…`), or a project key (`sk-fant-…`) on a
    /// trusted backend. Only the key's shape is checked; nothing is sent yet.
    pub fn new(key: &str) -> Result<Self, String> {
        Self::build(key, FantasticoProvider::new)
    }

    /// As [`new`](Self::new), sending every request through `wire`.
    pub fn with_wire(key: &str, wire: Arc<dyn Wire>) -> Result<Self, String> {
        Self::build(key, |config| FantasticoProvider::with_wire(config, wire))
    }

    fn build(key: &str, provider: impl FnOnce(FantasticoConfig) -> Result<FantasticoProvider, AiError>) -> Result<Self, String> {
        let config = FantasticoConfig::new(SecretString::new(key.trim())).with_title(TITLE);
        Ok(Self { provider: provider(config).map_err(|e| e.message)?, capability: capability()? })
    }

    /// The account behind an account key, with the model it suggests. Nothing is minted or spent.
    pub fn account(&self) -> Result<AccountInfo, String> {
        self.provider.account().map_err(|e| e.message)
    }

    fn call(&self, chat: &Value, stream: bool, on_delta: &mut (dyn FnMut(Delta<'_>) + Send)) -> Result<Value, String> {
        let model = model_of(chat)?;
        let model = ModelId::new(model).map_err(|_| format!("{model} is not a model name"))?;
        let job = self.provider.submit(&model, &request(self.capability.clone(), chat, stream)?).map_err(|e| e.message)?;
        follow(
            || self.provider.poll(&job),
            || {
                let _ = self.provider.cancel(&job);
            },
            on_delta,
        )
    }
}

impl Provider for Direct {
    fn complete(&mut self, request: &Value) -> Result<Value, String> {
        self.call(request, false, &mut |_| {})
    }

    fn complete_streaming(&mut self, request: &Value, on_delta: &mut (dyn FnMut(Delta<'_>) + Send)) -> Result<Value, String> {
        self.call(request, true, on_delta)
    }
}

#[cfg(test)]
mod tests;

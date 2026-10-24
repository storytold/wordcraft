//! Where the model's answers come from.

use std::collections::VecDeque;

use serde_json::{Value, json};

/// A piece of a reply as it streams in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delta<'a> {
    /// A reply starts streaming; text shown for an earlier attempt of the same call (a retry)
    /// is stale.
    Start,
    /// The next piece of the message's text.
    Text(&'a str),
}

/// A chat-completions endpoint. `request` is an OpenAI-style body (`model`, `messages`,
/// `tools`); the reply is the endpoint's JSON response (`choices[0].message`…).
pub trait Provider {
    fn complete(&mut self, request: &Value) -> Result<Value, String>;

    /// [`Provider::complete`] that hands the message's text to `on_delta` as it arrives, before
    /// the whole reply comes back. An endpoint that can't stream answers whole.
    fn complete_streaming(&mut self, request: &Value, on_delta: &mut (dyn FnMut(Delta<'_>) + Send)) -> Result<Value, String> {
        let _ = on_delta;
        self.complete(request)
    }
}

/// A provider that replays canned responses, for tests and demos without a network. It records
/// every request it was sent.
#[derive(Default)]
pub struct Scripted {
    replies: VecDeque<Value>,
    pub requests: Vec<Value>,
    /// Stream each reply's text in pieces ([`Scripted::streaming`]).
    stream: bool,
}

impl Scripted {
    pub fn new(replies: impl IntoIterator<Item = Value>) -> Self {
        Self { replies: replies.into_iter().collect(), requests: Vec::new(), stream: false }
    }

    /// [`Scripted::new`] whose replies stream: the text arrives a few characters at a time.
    pub fn streaming(replies: impl IntoIterator<Item = Value>) -> Self {
        Self { stream: true, ..Self::new(replies) }
    }
}

impl Provider for Scripted {
    fn complete(&mut self, request: &Value) -> Result<Value, String> {
        self.requests.push(request.clone());
        self.replies.pop_front().ok_or_else(|| "the scripted provider has no more replies".to_string())
    }

    fn complete_streaming(&mut self, request: &Value, on_delta: &mut (dyn FnMut(Delta<'_>) + Send)) -> Result<Value, String> {
        let reply = self.complete(request)?;
        if self.stream
            && let Some(text) = reply.pointer("/choices/0/message/content").and_then(Value::as_str)
        {
            on_delta(Delta::Start);
            let chars: Vec<char> = text.chars().collect();
            for piece in chars.chunks(4) {
                on_delta(Delta::Text(&piece.iter().collect::<String>()));
            }
        }
        Ok(reply)
    }
}

/// A chat-completions response whose message is plain text.
pub fn text_reply(text: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]})
}

/// A chat-completions response asking for tool calls: `(id, tool name, arguments)`.
pub fn tool_reply(calls: &[(&str, &str, Value)]) -> Value {
    let calls: Vec<Value> = calls
        .iter()
        .map(|(id, name, args)| json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}))
        .collect();
    json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": calls}, "finish_reason": "tool_calls"}]})
}

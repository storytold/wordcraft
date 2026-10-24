//! GeenioAI's model endpoint on a FANTASTICO account, with streaming replies.
//!
//! `fantastico-sdk` mints the runtime keys, fills in `max_tokens`, retries and classifies
//! failures; it reads replies whole. [`Streaming`] keeps all of that and adds streaming
//! underneath it: a [`Transport`] of its own that, for a request with `"stream": true`, reads the
//! provider's server-sent events as they arrive, hands every piece of text to the caller, and
//! gives the SDK back the same JSON a non-streaming reply would carry. The SDK never knows.
//!
//! Native only: the transport blocks on a socket. The web build has no assistant host.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use fantastico_sdk::{Error, Fantastico, HttpRequest, HttpResponse, Method, Options, Transport};
use serde_json::{Map, Value, json};
use wordcraft_assistant::{Delta, Provider};

/// A larger response body is refused rather than read into memory (the SDK's own cap).
const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;
/// A longer server-sent event line is refused: a chunk is a few hundred bytes.
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// A piece of a streamed reply, owned, on its way from the transport to the caller.
enum Owned {
    Start,
    Text(String),
}

type Sink = Arc<Mutex<Option<Sender<Owned>>>>;

/// A FANTASTICO account as the assistant's model endpoint.
pub struct Streaming {
    client: Fantastico,
    sink: Sink,
}

impl Streaming {
    /// A client for `key` whose chat completions can stream.
    pub fn new(key: impl Into<String>, opts: Options) -> Result<Self, Error> {
        let sink: Sink = Arc::new(Mutex::new(None));
        let transport = StreamingTransport::new(sink.clone());
        Ok(Self { client: Fantastico::with_transport(key, opts, Box::new(transport))?, sink })
    }

    pub fn client(&self) -> &Fantastico {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut Fantastico {
        &mut self.client
    }
}

/// An SDK error as the user should read it.
pub fn explain(e: &Error) -> String {
    match e.code.as_str() {
        "ALLOWANCE_EXHAUSTED" | "INSUFFICIENT_RESERVE" | "INSUFFICIENT_FUNDS" => {
            "The credit on your FANTASTICO account has run out: top it up to continue".to_string()
        }
        "NETWORK_ERROR" => "FANTASTICO cannot be reached: check the internet connection".to_string(),
        _ => format!("{} ({})", e.message, e.code),
    }
}

impl Provider for Streaming {
    fn complete(&mut self, request: &Value) -> Result<Value, String> {
        self.client.chat_completion(request).map_err(|e| explain(&e))
    }

    /// The request goes out with `stream: true`; the transport feeds the pieces into a channel
    /// that a scoped thread drains into `on_delta` while the SDK's call blocks.
    fn complete_streaming(&mut self, request: &Value, on_delta: &mut (dyn FnMut(Delta<'_>) + Send)) -> Result<Value, String> {
        let mut streamed = request.clone();
        if let Some(o) = streamed.as_object_mut() {
            o.insert("stream".into(), json!(true));
        }
        let (tx, rx) = channel();
        let sink = self.sink.clone();
        set_sink(&sink, Some(tx));
        let client = &mut self.client;
        let result = std::thread::scope(|s| {
            let consumer = s.spawn(move || {
                for d in rx {
                    match &d {
                        Owned::Start => on_delta(Delta::Start),
                        Owned::Text(t) => on_delta(Delta::Text(t)),
                    }
                }
            });
            let r = client.chat_completion(&streamed);
            // Dropping the sender ends the consumer once it has delivered everything.
            set_sink(&sink, None);
            let _ = consumer.join();
            r
        });
        result.map_err(|e| explain(&e))
    }
}

fn set_sink(sink: &Sink, tx: Option<Sender<Owned>>) {
    let mut guard = sink.lock().unwrap_or_else(|e| e.into_inner());
    *guard = tx;
}

/// The SDK's HTTP transport with streaming underneath.
struct StreamingTransport {
    agent: ureq::Agent,
    sink: Sink,
}

impl StreamingTransport {
    fn new(sink: Sink) -> Self {
        let config = ureq::Agent::config_builder().http_status_as_error(false).build();
        Self { agent: config.into(), sink }
    }

    fn emit(&self, piece: Owned) {
        let guard = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(piece);
        }
    }
}

/// Whether `request` asks for a streamed reply.
fn wants_stream(request: &HttpRequest) -> bool {
    request.method == Method::Post
        && request
            .body
            .as_deref()
            .and_then(|b| serde_json::from_str::<Value>(b).ok())
            .is_some_and(|v| v.get("stream").and_then(Value::as_bool) == Some(true))
}

fn with_headers<B>(mut builder: ureq::RequestBuilder<B>, request: &HttpRequest) -> ureq::RequestBuilder<B> {
    for (k, v) in &request.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    builder.config().timeout_global(Some(request.timeout)).build()
}

impl Transport for StreamingTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        let url = request.url.as_str();
        let result = match request.method {
            Method::Get => with_headers(self.agent.get(url), request).call(),
            Method::Delete => with_headers(self.agent.delete(url), request).call(),
            Method::Post => with_headers(self.agent.post(url), request).send(request.body.as_deref().unwrap_or("")),
        };
        let mut response = result.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> =
            response.headers().iter().filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().to_string(), v.to_string()))).collect();
        let event_stream =
            headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.to_ascii_lowercase().contains("text/event-stream"));
        let mut reader = response.body_mut().with_config().limit(MAX_BODY_BYTES).reader();
        // Streamed only when asked and answered as a stream: an error status or a plain JSON
        // reply is read whole, as the SDK expects.
        if !(wants_stream(request) && (200..300).contains(&status) && event_stream) {
            let mut body = String::new();
            reader.read_to_string(&mut body).map_err(|e| e.to_string())?;
            return Ok(HttpResponse { status, headers, body });
        }
        self.emit(Owned::Start);
        let body = assemble(BufReader::new(reader), &mut |t| self.emit(Owned::Text(t.to_string())))?;
        Ok(HttpResponse { status, headers, body: body.to_string() })
    }
}

/// Reads a chat-completions event stream and builds the reply a non-streaming request would
/// have got; `on_text` gets every piece of the message's text as it arrives. An `error` event
/// becomes the body, so the SDK classifies it as it would a `200` carrying an error.
fn assemble(reader: impl BufRead, on_text: &mut dyn FnMut(&str)) -> Result<Value, String> {
    let mut a = Assembler::default();
    let mut line = String::new();
    let mut reader = reader;
    loop {
        line.clear();
        let n = reader.by_ref().take(MAX_LINE_BYTES as u64 + 1).read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if line.len() > MAX_LINE_BYTES {
            return Err("the LLM provider sent an event too long to read".into());
        }
        let l = line.trim_end_matches(['\r', '\n']);
        let Some(data) = l.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else { continue };
        if chunk.get("error").is_some() {
            return Ok(chunk);
        }
        a.feed(&chunk, on_text);
    }
    Ok(a.finish())
}

/// One tool call under construction: the pieces arrive by `index`.
#[derive(Default)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct Assembler {
    id: Option<Value>,
    model: Option<Value>,
    created: Option<Value>,
    role: Option<String>,
    content: String,
    has_content: bool,
    tool_calls: Vec<ToolCall>,
    finish_reason: Option<Value>,
    usage: Option<Value>,
}

impl Assembler {
    fn feed(&mut self, chunk: &Value, on_text: &mut dyn FnMut(&str)) {
        for (slot, key) in [(&mut self.id, "id"), (&mut self.model, "model"), (&mut self.created, "created")] {
            if slot.is_none()
                && let Some(v) = chunk.get(key).filter(|v| !v.is_null())
            {
                *slot = Some(v.clone());
            }
        }
        if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage = Some(u.clone());
        }
        let Some(choice) = chunk.pointer("/choices/0") else { return };
        if let Some(f) = choice.get("finish_reason").filter(|f| !f.is_null()) {
            self.finish_reason = Some(f.clone());
        }
        let Some(delta) = choice.get("delta") else { return };
        if let Some(r) = delta.get("role").and_then(Value::as_str) {
            self.role = Some(r.to_string());
        }
        if let Some(t) = delta.get("content").and_then(Value::as_str) {
            self.has_content = true;
            if !t.is_empty() {
                self.content.push_str(t);
                on_text(t);
            }
        }
        for (n, call) in delta.get("tool_calls").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let index = call.get("index").and_then(Value::as_u64).map_or(n, |i| usize::try_from(i).unwrap_or(n)).min(64);
            while self.tool_calls.len() <= index {
                self.tool_calls.push(ToolCall::default());
            }
            let Some(slot) = self.tool_calls.get_mut(index) else { continue };
            if let Some(id) = call.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                slot.id = id.to_string();
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                slot.name = name.to_string();
            }
            if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                slot.arguments.push_str(args);
            }
        }
    }

    fn finish(self) -> Value {
        let mut message = Map::new();
        message.insert("role".into(), json!(self.role.unwrap_or_else(|| "assistant".into())));
        message.insert("content".into(), if self.has_content || self.tool_calls.is_empty() { json!(self.content) } else { Value::Null });
        if !self.tool_calls.is_empty() {
            let calls: Vec<Value> = self
                .tool_calls
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    json!({"id": if c.id.is_empty() { format!("call_{i}") } else { c.id }, "type": "function", "function": {"name": c.name, "arguments": c.arguments}})
                })
                .collect();
            message.insert("tool_calls".into(), Value::Array(calls));
        }
        let mut out = Map::new();
        out.insert("object".into(), json!("chat.completion"));
        for (key, v) in [("id", self.id), ("model", self.model), ("created", self.created), ("usage", self.usage)] {
            if let Some(v) = v {
                out.insert(key.into(), v);
            }
        }
        out.insert("choices".into(), json!([{"index": 0, "message": message, "finish_reason": self.finish_reason.unwrap_or(Value::Null)}]));
        Value::Object(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(lines: &[&str]) -> (Value, Vec<String>) {
        let text = lines.join("\n") + "\n";
        let mut pieces = Vec::new();
        let v = assemble(text.as_bytes(), &mut |t| pieces.push(t.to_string())).unwrap();
        (v, pieces)
    }

    #[test]
    fn text_pieces_arrive_in_order_and_make_the_usual_reply() {
        let (v, pieces) = stream(&[
            ": OPENROUTER PROCESSING",
            r#"data: {"id":"gen-1","model":"m","created":7,"choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}"#,
            r#"data: {"id":"gen-1","choices":[{"index":0,"delta":{"content":"Hel"}}]}"#,
            "",
            r#"data: {"id":"gen-1","choices":[{"index":0,"delta":{"content":"lo."},"finish_reason":null}]}"#,
            r#"data: {"id":"gen-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"total_tokens":3,"cost":0.01}}"#,
            "data: [DONE]",
            r#"data: {"choices":[{"delta":{"content":"ignored after done"}}]}"#,
        ]);
        assert_eq!(pieces, ["Hel", "lo."]);
        assert_eq!(v["choices"][0]["message"]["content"], "Hello.");
        assert_eq!(v["choices"][0]["message"]["role"], "assistant");
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
        assert_eq!(v["usage"]["cost"], 0.01);
        assert_eq!((v["id"].as_str(), v["model"].as_str(), v["created"].as_u64()), (Some("gen-1"), Some("m"), Some(7)));
        assert!(v["choices"][0]["message"].get("tool_calls").is_none());
    }

    #[test]
    fn tool_calls_are_put_together_from_their_pieces() {
        let (v, pieces) = stream(&[
            r#"data: {"choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"type_text","arguments":""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"text\": \"Hi"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"get_text","arguments":"{}"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "data: [DONE]",
        ]);
        assert!(pieces.is_empty());
        let m = &v["choices"][0]["message"];
        assert_eq!(m["content"], Value::Null);
        assert_eq!(m["tool_calls"][0]["id"], "call_a");
        assert_eq!(m["tool_calls"][0]["function"]["name"], "type_text");
        assert_eq!(m["tool_calls"][0]["function"]["arguments"], r#"{"text": "Hi"}"#);
        assert_eq!(m["tool_calls"][1]["function"]["name"], "get_text");
        assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn an_error_event_becomes_the_body_and_junk_is_skipped() {
        let (v, pieces) = stream(&[
            r#"data: {"choices":[{"delta":{"content":"par"}}]}"#,
            "data: not json at all",
            r#"data: {"error":{"message":"overloaded","code":503}}"#,
            r#"data: {"choices":[{"delta":{"content":"never"}}]}"#,
        ]);
        assert_eq!(pieces, ["par"]);
        assert_eq!(v["error"]["message"], "overloaded");
        // An empty stream still makes a well-formed, empty reply.
        let (v, pieces) = stream(&[]);
        assert!(pieces.is_empty());
        assert_eq!(v["choices"][0]["message"]["content"], "");
        // Hostile pieces never panic: a huge index, a missing choice, strings where objects go.
        let (v, _) = stream(&[
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1e12,"function":{"name":"x"}}]}}]}"#,
            r#"data: {"choices":"nope"}"#,
            r#"data: {"choices":[{"delta":"nope"}]}"#,
            r#"data: 7"#,
        ]);
        // A float index isn't an index: the piece takes its position in the list instead.
        assert_eq!(v["choices"][0]["message"]["tool_calls"].as_array().map(Vec::len), Some(1));
        let (v, _) = stream(&[r#"data: {"choices":[{"delta":{"tool_calls":[{"index":123456789,"function":{"name":"x"}}]}}]}"#]);
        assert_eq!(v["choices"][0]["message"]["tool_calls"].as_array().map(Vec::len), Some(65));
        let long = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}", "x".repeat(MAX_LINE_BYTES));
        assert!(assemble(long.as_bytes(), &mut |_| {}).is_err());
    }

    #[test]
    fn only_a_streamed_post_streams() {
        let req = |method, body: Option<&str>| HttpRequest {
            method,
            url: "https://example.org".into(),
            headers: vec![],
            body: body.map(str::to_string),
            timeout: std::time::Duration::from_secs(1),
        };
        assert!(wants_stream(&req(Method::Post, Some(r#"{"stream": true, "messages": []}"#))));
        assert!(!wants_stream(&req(Method::Post, Some(r#"{"stream": false}"#))));
        assert!(!wants_stream(&req(Method::Post, Some("{not json"))));
        assert!(!wants_stream(&req(Method::Get, None)));
        // The user-facing wording of the SDK's codes.
        assert!(explain(&fantastico_sdk::Error { message: "x".into(), code: "ALLOWANCE_EXHAUSTED".into(), status: 402 }).contains("top it up"));
        assert!(explain(&fantastico_sdk::Error { message: "x".into(), code: "NETWORK_ERROR".into(), status: 503 }).contains("internet"));
        assert_eq!(explain(&fantastico_sdk::Error { message: "boom".into(), code: "OTHER".into(), status: 500 }), "boom (OTHER)");
    }
}

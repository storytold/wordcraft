//! [`Direct`] against a scripted FANTASTICO: the account service and the provider behind it.

use std::io::Cursor;
use std::sync::Mutex;

use craft_ai_fantastico::{HttpRequest, Method, WireResponse};
use serde_json::json;

use super::*;

const BASE: &str = "https://fantastico.cc/api";
const MODELS: &str = "https://models.example/v1";
const MODEL: &str = "anthropic/claude-sonnet-5";

/// Every request is kept; a chat body that says `"fail"` is refused by the provider.
#[derive(Default)]
struct Fake {
    urls: Mutex<Vec<String>>,
    bodies: Mutex<Vec<Value>>,
}

fn reply(status: u16, content_type: &str, body: String) -> WireResponse {
    WireResponse { status, headers: vec![("Content-Type".into(), content_type.into())], body: Box::new(Cursor::new(body)) }
}

impl Wire for Fake {
    fn send(&self, req: &HttpRequest) -> Result<WireResponse, String> {
        self.urls.lock().unwrap().push(req.url.clone());
        if req.url == format!("{BASE}/edge/key") && req.method == Method::Post {
            let key = json!({"runtimeKey": "rk-runtime", "expiresAt": "2099-01-01T00:00:00.000Z", "baseUrl": MODELS});
            return Ok(reply(200, "application/json", key.to_string()));
        }
        if req.url == format!("{BASE}/edge/config") {
            let config =
                json!({"contoId": "conto-7", "status": "active", "model": MODEL, "balance": {"remainingUsd": 4.0, "totalRemainingUsd": 5.5}});
            return Ok(reply(200, "application/json", config.to_string()));
        }
        if req.url != format!("{MODELS}/chat/completions") {
            return Err(format!("unexpected request to {}", req.url));
        }
        assert_eq!(req.header("Authorization"), Some("Bearer rk-runtime"), "only the runtime key goes to the provider");
        let body: Value = serde_json::from_str(req.body.as_deref().unwrap_or("{}")).map_err(|e| e.to_string())?;
        let fail = body.to_string().contains("fail");
        let streamed = body["stream"] == true;
        self.bodies.lock().unwrap().push(body);
        if fail {
            let e = json!({"error": {"message": "this request is not allowed", "type": "invalid_request_error"}});
            return Ok(reply(400, "application/json", e.to_string()));
        }
        let chunks = [
            json!({"choices": [{"delta": {"role": "assistant", "content": "Sto "}}]}),
            json!({"choices": [{"delta": {"content": "scrivendo.", "tool_calls": [
                {"index": 0, "id": "call_7", "type": "function", "function": {"name": "type_text", "arguments": "{\"text\":\"Ciao\"}"}}]}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}], "usage": {"cost": 0.001}}),
        ];
        if streamed {
            let sse: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect::<String>() + "data: [DONE]\n\n";
            return Ok(reply(200, "text/event-stream", sse));
        }
        let whole = json!({"choices": [{"message": {"role": "assistant", "content": "Sto scrivendo.", "tool_calls": [
            {"id": "call_7", "type": "function", "function": {"name": "type_text", "arguments": "{\"text\":\"Ciao\"}"}}]},
            "finish_reason": "tool_calls"}]});
        Ok(reply(200, "application/json", whole.to_string()))
    }
}

fn direct(key: &str) -> (Direct, Arc<Fake>) {
    let fake = Arc::new(Fake::default());
    (Direct::with_wire(key, fake.clone() as Arc<dyn Wire>).unwrap(), fake)
}

fn conversation(model: &str, user: &str) -> Value {
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You are GeenioAI."},
            {"role": "user", "content": user},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "get_text", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "call_1", "content": "(empty document)"}
        ],
        "tools": [{"type": "function", "function": {"name": "type_text", "parameters": {"type": "object"}}}]
    })
}

#[test]
fn a_conversation_streams_and_comes_back_whole() {
    let (mut d, fake) = direct("ck-fant-test-key");
    let pieces = Mutex::new(Vec::new());
    let reply = d
        .complete_streaming(&conversation(MODEL, "scrivi ciao"), &mut |delta| {
            pieces.lock().unwrap().push(match delta {
                Delta::Start => "<start>".to_string(),
                Delta::Text(t) => t.to_string(),
            })
        })
        .unwrap();
    assert_eq!(*pieces.lock().unwrap(), ["<start>", "Sto ", "scrivendo."]);
    let message = &reply["choices"][0]["message"];
    assert_eq!(message["content"], "Sto scrivendo.");
    assert_eq!(message["tool_calls"][0]["id"], "call_7");
    assert_eq!(reply["choices"][0]["finish_reason"], "tool_calls");
    // The provider got the whole conversation, in order, with the model and the tools.
    let bodies = fake.bodies.lock().unwrap();
    let sent = &bodies[0];
    let roles: Vec<_> = sent["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool"]);
    assert_eq!(sent["messages"][3]["tool_call_id"], "call_1");
    assert_eq!((sent["model"].as_str(), sent["stream"].as_bool()), (Some(MODEL), Some(true)));
    assert_eq!(sent["tools"][0]["function"]["name"], "type_text");
}

#[test]
fn a_reply_that_is_not_streamed_reads_the_same() {
    let (mut d, fake) = direct("ck-fant-test-key");
    let reply = d.complete(&conversation(MODEL, "scrivi ciao")).unwrap();
    assert_eq!(reply["choices"][0]["message"]["content"], "Sto scrivendo.");
    assert_eq!(reply["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "type_text");
    assert_ne!(fake.bodies.lock().unwrap()[0]["stream"], true);
    // One key for both calls: a second request doesn't mint again.
    d.complete(&conversation(MODEL, "ancora")).unwrap();
    let mints = fake.urls.lock().unwrap().iter().filter(|u| u.ends_with("/edge/key")).count();
    assert_eq!(mints, 1);
}

#[test]
fn a_refusal_is_the_error_and_a_bad_request_sends_nothing() {
    let (mut d, fake) = direct("ck-fant-test-key");
    let e = d.complete(&conversation(MODEL, "fail")).unwrap_err();
    assert!(e.contains("not allowed"), "{e}");
    let before = fake.urls.lock().unwrap().len();
    assert!(d.complete(&json!({"messages": []})).unwrap_err().contains("names no model"));
    assert!(d.complete(&json!({"model": "  "})).unwrap_err().contains("names no model"));
    assert!(d.complete(&json!({"model": MODEL})).unwrap_err().contains("no messages"));
    assert_eq!(fake.urls.lock().unwrap().len(), before, "nothing was sent");
}

#[test]
fn the_account_is_read_with_an_account_key_only() {
    let (d, fake) = direct("ck-fant-test-key");
    let account = d.account().unwrap();
    assert_eq!((account.account_id.as_str(), account.model.as_deref()), ("conto-7", Some(MODEL)));
    assert_eq!(account.balance.and_then(|b| b.total_remaining_usd), Some(5.5));
    assert!(!fake.urls.lock().unwrap().iter().any(|u| u.ends_with("/key")), "nothing minted");
    let (d, fake) = direct("sk-fant-project-key");
    assert!(d.account().unwrap_err().contains("account key"));
    assert!(fake.urls.lock().unwrap().is_empty());
}

#[test]
fn a_key_of_the_wrong_shape_is_refused_before_any_request() {
    let fake = Arc::new(Fake::default());
    assert!(Direct::with_wire("not-a-key", fake.clone() as Arc<dyn Wire>).is_err());
    assert!(Direct::with_wire("", fake.clone() as Arc<dyn Wire>).is_err());
    assert!(fake.urls.lock().unwrap().is_empty());
}

#[test]
fn the_request_carries_each_message_as_a_turn() {
    let req = request(capability().unwrap(), &conversation(MODEL, "ciao"), true).unwrap();
    assert_eq!(req.inputs.len(), 4);
    assert!(req.inputs.iter().all(|i| i.role == turn().unwrap()));
    assert!(req.params.contains_key("tools"));
    assert!(request(capability().unwrap(), &json!({"model": MODEL}), false).is_err());
    assert_eq!(model_of(&json!({"model": " m "})), Ok("m"));
}

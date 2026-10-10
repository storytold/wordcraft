//! MCP tools for an invited agent: thin wrappers over the functions `wordcraft-cli chat` uses.

use serde_json::{Value, json};

use super::{BackendCaller, Cursor, During, Failure, ReadOpts, briefing, client_command, join, now_ms, owner_selection, poll, read, send};
use crate::backend::Backend;
use crate::tools::{ToolResult, n, obj, s, tool, wrap};

pub const NOT_JOINED: &str = "join the chat first: chat_join with the invite code (or start the bridge with --join CODE)";

/// The line after the briefing for an agent on MCP: the tools that do what the commands do.
pub const OVER_MCP: &str = "Over MCP, use the tools instead of the commands above: chat_wait (listen), chat_send (send), chat_read (read), execute or batch (do), list_commands (commands).";

/// Longest `chat_wait` (the window caps a poll at 25 s).
const MAX_WAIT_S: u64 = 25;

/// The rules for `handle` as an agent on MCP: the briefing, then [`OVER_MCP`].
pub fn mcp_briefing(handle: &str) -> String {
    format!("{}\n{OVER_MCP}", briefing(handle, &client_command()))
}

/// What `chat_wait` remembers between calls.
#[derive(Default)]
pub struct McpChat {
    cursor: Option<Cursor>,
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "chat_join",
            "Join the chat",
            "Join the window's chat with the invite code the owner gave you; returns the rules. From then on every tool runs as you (tracked, checked against the agent allow-list).",
            obj(json!({"code": s("XXXX-XXXX-XXXX"), "as": s("@name from the invite line (optional: the code is bound to a name)")}), &["code"]),
            false,
        ),
        tool(
            "chat_wait",
            "Wait for chat messages",
            "The first call returns the recent history at once; later calls wait up to wait_s seconds (at most 25) for new messages. OWNER lines marked [@you] are for you.",
            obj(json!({"wait_s": n("0 to 25, default 25")}), &[]),
            true,
        ),
        tool("chat_send", "Send a chat message", "Post a one-line message in the chat.", obj(json!({"text": s("The message")}), &["text"]), false),
        tool(
            "chat_read",
            "Read the document",
            "Numbered paragraphs with tracked changes as [+inserted+](@author) [-deleted-](@author). In a big document use find (with context) or from/to; sel gives the owner's selection.",
            obj(
                json!({"from": n("First paragraph"), "to": n("Last paragraph"), "find": s("Text to find"), "context": n("Paragraphs around each hit"), "sel": {"type": "boolean"}}),
                &[],
            ),
            true,
        ),
        tool("chat_members", "Chat members", "The agents in the chat.", obj(json!({}), &[]), true),
    ]
}

fn text_of(lines: Vec<String>, empty: &str) -> ToolResult {
    ToolResult::text(if lines.is_empty() { empty.to_string() } else { lines.join("\n") })
}

/// Seconds `chat_wait` waits: `wait_s` capped at 25; missing, negative or not a number is 25.
fn wait_seconds(a: &Value) -> u64 {
    match a.get("wait_s").and_then(Value::as_f64) {
        Some(w) if w.is_finite() && w >= 0.0 => w.min(MAX_WAIT_S as f64) as u64,
        _ => MAX_WAIT_S,
    }
}

/// `None` when `name` is not a chat tool.
pub fn call(b: &mut dyn Backend, st: &mut McpChat, name: &str, a: &Value) -> Option<ToolResult> {
    if !name.starts_with("chat_") {
        return None;
    }
    let num = |k: &str| a.get(k).and_then(Value::as_u64);
    let failed = |f: Failure| ToolResult::error(f.message);
    Some(match name {
        "chat_join" => {
            let Some(addr) = b.address() else { return Some(ToolResult::error(format!("the chat {}", crate::NEEDS_APP))) };
            let code = a.get("code").and_then(Value::as_str).unwrap_or("");
            match join(&addr, code, a.get("as").and_then(Value::as_str)) {
                Ok(m) => {
                    let h = m.handle.clone();
                    match b.set_member(m.handle, m.key) {
                        Ok(()) => {
                            st.cursor = None;
                            ToolResult::text(mcp_briefing(&h))
                        }
                        Err(e) => ToolResult::error(e),
                    }
                }
                Err(f) => failed(f),
            }
        }
        "chat_members" => wrap(b.call("chat.members", json!({}))),
        _ => {
            let Some(me) = b.member() else { return Some(ToolResult::error(NOT_JOINED)) };
            let mut c = BackendCaller(b);
            match name {
                "chat_send" => match a.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                    Some(t) => send(&mut c, t).map_or_else(failed, |()| ToolResult::text("OK")),
                    None => ToolResult::error("chat_send needs text"),
                },
                "chat_wait" => match st.cursor.as_mut() {
                    None => match poll(&mut c, 0, 0) {
                        Ok(h) => {
                            let (cur, lines) = Cursor::start(&me, &h, now_ms());
                            st.cursor = Some(cur);
                            text_of(lines, "(no messages yet)")
                        }
                        Err(e) => failed(Failure::from_link(e, During::Call)),
                    },
                    Some(cur) => match poll(&mut c, cur.after(), wait_seconds(a)) {
                        Ok(msgs) => text_of(cur.take(&msgs), "(no new messages)"),
                        Err(e) => failed(Failure::from_link(e, During::Listen)),
                    },
                },
                "chat_read" => {
                    let r = if a.get("sel").and_then(Value::as_bool) == Some(true) {
                        owner_selection(&mut c).map(|l| vec![l])
                    } else {
                        let find = a.get("find").and_then(Value::as_str).map(str::to_string);
                        read(&mut c, &ReadOpts { from: num("from"), to: num("to"), find, context: num("context").unwrap_or(0) })
                    };
                    r.map_or_else(failed, |lines| text_of(lines, "(empty document)"))
                }
                other => ToolResult::error(format!("unknown tool `{other}`")),
            }
        }
    })
}

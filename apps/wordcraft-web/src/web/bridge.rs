//! The OMD host bridge: open, save, dirty state and automation commands over postMessage.
//!
//! When WordCraft runs inside an iframe of the OMD file manager (SVAR), file open, saving and
//! engine commands travel through postMessage to the parent page instead of the browser file
//! picker and downloads:
//!
//! host → editor: `{type:"wordcraft:open", name, bytes}`, `{type:"wordcraft:save-ack", ticket, ok, error?}`,
//!                `{type:"wordcraft:cmd", id, params?, requestId?}` (`id` is the engine command, params is a JSON
//!                string, `requestId` an optional string that comes back in the response)
//! editor → host: `{type:"wordcraft:ready"}`, `{type:"wordcraft:failed", error}`, `{type:"wordcraft:dirty", dirty}`,
//!                `{type:"wordcraft:save", ticket, name, bytes}`, `{type:"wordcraft:status", message}`,
//!                `{type:"wordcraft:response", id, requestId?, ok, value?}` / `{type:"wordcraft:response", id, requestId?, ok:false, error}`
//!
//! Only enabled when embedded (`window.parent !== window`) with a concrete origin of its own: the
//! standalone page keeps the browser file picker and downloads, and so does a frame sandboxed
//! without `allow-same-origin` (opaque origin `"null"`: no sender can be vetted and nothing can be
//! addressed to the host, so the bridge is not installed and posts nothing). Incoming messages
//! are accepted only from the parent window, and only when its origin matches the page's own
//! origin (the OMD host serves the editor same-origin); the decision is
//! [`crate::host_filter::decide`].

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, ArrayBuffer, Object, Reflect, Uint8Array};
use serde_json::{Value, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::MessageEvent;
use wordcraft_ui_egui::Inbox;
use wordcraft_ui_egui::async_saves::{AsyncSaves, SaveOutcome};

use crate::host_filter::{self, Incoming, RawParams};

/// Engine commands queued for the next frame: `(command id, params, request id)`.
pub type CommandQueue = Rc<RefCell<Vec<(String, Value, Option<String>)>>>;

fn js(e: JsValue) -> String {
    format!("{e:?}")
}

/// This page runs inside another window's frame (`window.parent !== window`).
pub fn embedded() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    match window.parent() {
        Ok(Some(parent)) => parent != window,
        _ => false,
    }
}

/// The bridge is active: this page runs inside a host iframe and has a concrete origin to vet
/// senders and address the host with. A sandboxed frame with an opaque origin counts as
/// standalone.
pub fn in_host() -> bool {
    embedded() && own_origin().is_some()
}

/// The origin of the page, to answer and vet postMessage. `None` when it is opaque (`"null"`),
/// the wildcard or unknown: the bridge then stays off rather than fall back to `"*"`.
fn own_origin() -> Option<String> {
    web_sys::window().map(|w| w.origin()).filter(|o| host_filter::usable_origin(o))
}

/// Whether a `message` event came from the parent window (not from another same-origin frame).
fn from_parent(e: &MessageEvent) -> bool {
    let parent = web_sys::window().and_then(|w| w.parent().ok().flatten());
    match (e.source(), parent) {
        (Some(source), Some(parent)) => JsValue::from(source) == JsValue::from(parent),
        _ => false,
    }
}

/// Post an object (with an optional structured-clone transfer list) to the host frame.
fn post(obj: &Object, transfer: Option<&Array>) -> Result<(), String> {
    let Some(window) = web_sys::window() else {
        return Err("no window".into());
    };
    let parent = window.parent().ok().flatten().ok_or("no parent")?;
    if parent == window {
        return Err("not embedded".into());
    }
    let origin = own_origin().ok_or("opaque origin")?;
    let result = match transfer {
        Some(list) => parent.post_message_with_transfer(obj.as_ref(), &origin, list.as_ref()),
        None => parent.post_message(obj.as_ref(), &origin),
    };
    result.map_err(js)
}

fn post_json_str(kind: &str, extra_key: &str, extra_value: &str) {
    let obj = Object::new();
    let _ = Reflect::set(&obj, &JsValue::from_str("type"), &JsValue::from_str(kind));
    let _ = Reflect::set(&obj, &JsValue::from_str(extra_key), &JsValue::from_str(extra_value));
    if let Err(e) = post(&obj, None) {
        log::warn!("bridge: couldn't post {kind}: {e}");
    }
}

/// Tell the host the app is up and listening for `{type:"wordcraft:open"}`.
pub fn post_ready() {
    post_json_str("wordcraft:ready", "name", "wordcraft-web");
}

/// Tell the host the app could not start.
pub fn post_failed(error: &str) {
    post_json_str("wordcraft:failed", "error", error);
}

/// The host forwards a save/export to the original document path. Starts a ticket and posts the
/// bytes; the outcome arrives later as `{type:"wordcraft:save-ack"}`.
pub fn send_save(saves: &AsyncSaves, path: &str, bytes: &[u8]) -> Result<(), String> {
    let ticket = saves.start();
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "document".into());
    let uint8 = Uint8Array::from(bytes);
    let buffer = uint8.buffer();
    let obj = Object::new();
    let _ = Reflect::set(&obj, &JsValue::from_str("type"), &JsValue::from_str("wordcraft:save"));
    let _ = Reflect::set(&obj, &JsValue::from_str("ticket"), &JsValue::from_f64(ticket as f64));
    let _ = Reflect::set(&obj, &JsValue::from_str("name"), &JsValue::from_str(&name));
    let _ = Reflect::set(&obj, &JsValue::from_str("bytes"), &uint8);
    let transfer = Array::of1(&buffer);
    match post(&obj, Some(&transfer)) {
        Ok(()) => {
            log::info!("bridge: save {name} (ticket {ticket}) sent to the host");
            Ok(())
        }
        Err(e) => {
            saves.finish(ticket, SaveOutcome::Failed(e.clone()));
            log::error!("bridge: couldn't send save {name}: {e}");
            Err(e)
        }
    }
}

/// Tell the host whether the document is dirty (its save-status indicator and the close dialog).
/// The app calls this every frame; only the changes go through postMessage.
pub fn post_dirty(dirty: bool) {
    thread_local! {
        /// The last value posted (2 = nothing yet).
        static LAST_DIRTY: std::cell::Cell<u8> = const { std::cell::Cell::new(2) };
    }
    let value = dirty as u8;
    if LAST_DIRTY.get() == value {
        return;
    }
    LAST_DIRTY.set(value);
    if !in_host() {
        return;
    }
    let obj = Object::new();
    let _ = Reflect::set(&obj, &JsValue::from_str("type"), &JsValue::from_str("wordcraft:dirty"));
    let _ = Reflect::set(&obj, &JsValue::from_str("dirty"), &JsValue::from_bool(dirty));
    if let Err(e) = post(&obj, None) {
        log::warn!("bridge: couldn't post dirty: {e}");
    }
}

/// The document was closed in the app (File › Close, a QAT button, New/Open replaced it): the
/// host closes its viewer.
pub fn post_doc_closed() {
    // Standalone, `window.parent` is the page itself: nothing to tell.
    if !in_host() {
        return;
    }
    let obj = Object::new();
    let _ = Reflect::set(&obj, &JsValue::from_str("type"), &JsValue::from_str("wordcraft:docClosed"));
    if let Err(e) = post(&obj, None) {
        log::warn!("bridge: couldn't post docClosed: {e}");
    }
}

/// Post an engine-command response: results in the `value` field, failures in `error`, the
/// command's `id` and the host's `requestId` (when it sent one) echoed so replies can be matched.
pub fn post_response(command_id: &str, request_id: Option<&str>, ok: bool, payload: &Value) {
    let obj = Object::new();
    if let Value::Object(fields) = host_filter::response_message(command_id, request_id, ok, payload) {
        for (key, value) in &fields {
            let value = match value {
                Value::Bool(b) => JsValue::from_bool(*b),
                other => JsValue::from_str(other.as_str().unwrap_or_default()),
            };
            let _ = Reflect::set(&obj, &JsValue::from_str(key), &value);
        }
    }
    if let Err(e) = post(&obj, None) {
        log::warn!("bridge: couldn't post response: {e}");
    }
}

/// Post a status line that reached the app's status bar to the host.
pub fn post_status(status: &str) {
    post_json_str("wordcraft:status", "message", status);
}

// ── Incoming messages ─────────────────────────────────────────────────────────────────────────

fn field(data: &JsValue, name: &str) -> Option<JsValue> {
    Reflect::get(data, &JsValue::from_str(name)).ok()
}

fn field_str(data: &JsValue, name: &str) -> Option<String> {
    field(data, name).and_then(|v| v.as_string())
}

fn field_bytes(data: &JsValue, name: &str) -> Option<Vec<u8>> {
    let v = field(data, name)?;
    if let Ok(ab) = v.clone().dyn_into::<ArrayBuffer>() {
        return Some(Uint8Array::new(&ab).to_vec());
    }
    v.dyn_into::<Uint8Array>().ok().map(|u| u.to_vec())
}

fn field_u64(data: &JsValue, name: &str) -> Option<u64> {
    field(data, name).and_then(|v| v.as_f64()).map(|f| f as u64)
}

fn field_bool(data: &JsValue, name: &str) -> Option<bool> {
    field(data, name).and_then(|v| v.as_bool())
}

/// Install the `message` listener for host commands. Only call this when the page is embedded.
pub fn install(inbox: Inbox, ctx: egui::Context, saves: AsyncSaves, commands: CommandQueue) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let own = own_origin().ok_or("the page has an opaque origin (sandboxed without allow-same-origin)")?;
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        let data = e.data();
        let kind = field_str(&data, "type");
        let incoming = match host_filter::decide(&e.origin(), &own, from_parent(&e), kind.as_deref()) {
            Ok(incoming) => incoming,
            Err(why) => {
                // Not from our host frame, or not a host message: ignore.
                log::debug!("bridge: dropped a message ({why:?})");
                return;
            }
        };
        match incoming {
            Incoming::Open => {
                let name = field_str(&data, "name").unwrap_or_else(|| "opened".into());
                let Some(bytes) = field_bytes(&data, "bytes") else {
                    log::warn!("bridge: open without bytes");
                    return;
                };
                // The engine reads the format from the bytes; the name is for display only.
                inbox.lock().unwrap_or_else(|er| er.into_inner()).push((name, bytes));
                ctx.request_repaint();
            }
            Incoming::SaveAck => {
                let Some(ticket) = field_u64(&data, "ticket") else {
                    return;
                };
                let ok = field_bool(&data, "ok").unwrap_or(false);
                let error = field_str(&data, "error");
                let outcome = if ok {
                    SaveOutcome::Saved
                } else if error.as_deref() == Some("cancelled") {
                    SaveOutcome::Cancelled
                } else {
                    SaveOutcome::Failed(error.unwrap_or_else(|| "the host couldn't save the document".into()))
                };
                saves.finish(ticket, outcome);
                ctx.request_repaint();
            }
            Incoming::Cmd => {
                let Some(id) = field_str(&data, "id") else {
                    return;
                };
                let request_id = field_str(&data, "requestId");
                let raw = field(&data, "params");
                let text = raw.as_ref().and_then(|v| v.as_string());
                let raw = match (&raw, &text) {
                    (_, Some(s)) => RawParams::Text(s),
                    (Some(v), None) if !v.is_undefined() && !v.is_null() => RawParams::NotText,
                    _ => RawParams::Absent,
                };
                // Malformed params are refused, not replaced by `{}` (which would run e.g. a save
                // under the default name).
                let Some(params) = host_filter::cmd_params(raw) else {
                    log::warn!("bridge: command {id} with malformed params dropped");
                    post_response(&id, request_id.as_deref(), false, &json!({ "error": "params must be a string of valid JSON", "id": id }));
                    return;
                };
                commands.borrow_mut().push((id, params, request_id));
                ctx.request_repaint();
            }
        }
    });
    window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    // The listener lives as long as the page.
    on_message.forget();
    Ok(())
}

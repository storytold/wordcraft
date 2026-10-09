//! Embedding WordCraft in a host page (`?host`): the page that embeds it in an `<iframe>` opens and
//! saves documents through window messages, so documents can live on the host's server instead of the
//! visitor's disk. Messages are accepted only from `window.parent` at the host's origin: the page's own
//! origin, or the one given as `?host=https://example.com`.
//!
//! From the host:
//! - `{wordcraft: "open", id?, name, data}`: open a document (`data`: ArrayBuffer or Uint8Array).
//! - `{wordcraft: "run", id?, command, params?}`: run any command (`engine.commands` lists them), for
//!   example `file.setAuthor`, `review.trackChanges` or `review.restrict`; the result comes back.
//! - `{wordcraft: "save", id?, name?}`: send the document back as `.docx`.
//!
//! To the host:
//! - `{wordcraft: "ready"}` once the editor is up.
//! - `{wordcraft: "result", id, ok, result | error}` for `open` and `run` (and for a `save` that failed).
//! - `{wordcraft: "saved", id, name, data}` with the `.docx` bytes: after `save`, or when the person
//!   chose Save in WordCraft (then `id` is null). In host mode Save never downloads.
//! - `{wordcraft: "dirty", value}` when the document gets unsaved changes, or loses them.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};

/// A request from the host page.
pub enum Msg {
    Open { id: Value, name: String, bytes: Vec<u8> },
    Run { id: Value, command: String, params: Value },
    Save { id: Value, name: Option<String> },
}

/// Host mode: who may drive the editor, what they asked for, and which save is pending.
#[derive(Clone)]
pub struct Host {
    origin: String,
    pub queue: Rc<RefCell<Vec<Msg>>>,
    /// The `id` of a host-requested save, so the bytes go back tagged with it.
    pub pending_save: Rc<RefCell<Option<Value>>>,
}

impl Host {
    /// Host mode if the page URL has `?host`; the host's origin is `?host=<origin>` or our own.
    pub fn from_url() -> Option<Host> {
        let location = web_sys::window()?.location();
        let search = location.search().ok()?;
        let value = search.trim_start_matches('?').split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k == "host").then(|| js_sys::decode_uri_component(v).ok().and_then(|s| s.as_string()).unwrap_or_default())
        })?;
        let origin = if value.is_empty() { location.origin().ok()? } else { value.trim_end_matches('/').to_string() };
        Some(Host { origin, queue: Rc::default(), pending_save: Rc::default() })
    }

    /// Listen for the host's messages; `wake` asks the app to run a frame so they are handled.
    pub fn listen(&self, wake: impl Fn() + 'static) {
        let Some(window) = web_sys::window() else { return };
        let host = self.clone();
        let parent = window.parent().ok().flatten();
        let cb = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            if e.origin() != host.origin {
                return;
            }
            // Only the embedding page, not some other window that knows our origin.
            let from_parent = match (e.source(), &parent) {
                (Some(src), Some(p)) => JsValue::from(src) == JsValue::from(p.clone()),
                _ => false,
            };
            if !from_parent {
                return;
            }
            if let Some(msg) = parse(&e.data()) {
                host.queue.borrow_mut().push(msg);
                wake();
            }
        });
        if window.add_event_listener_with_callback("message", cb.as_ref().unchecked_ref()).is_ok() {
            cb.forget();
        }
    }

    /// Post `{wordcraft: kind, ...fields}` to the host page (with `data` bytes, if any).
    pub fn post(&self, kind: &str, fields: Value, data: Option<&[u8]>) {
        let Some(parent) = web_sys::window().and_then(|w| w.parent().ok().flatten()) else { return };
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &"wordcraft".into(), &kind.into());
        if let Value::Object(map) = fields {
            for (k, v) in map {
                let js = js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL);
                let _ = js_sys::Reflect::set(&obj, &k.as_str().into(), &js);
            }
        }
        if let Some(bytes) = data {
            let _ = js_sys::Reflect::set(&obj, &"data".into(), &js_sys::Uint8Array::from(bytes).buffer());
        }
        if let Err(e) = parent.post_message(&obj, &self.origin) {
            log::error!("couldn't message the host page: {e:?}");
        }
    }
}

fn get(obj: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(obj, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

/// JSON-compatible JS value (or a JSON string) as serde JSON.
fn to_json(v: &JsValue) -> Value {
    if v.is_undefined() || v.is_null() {
        return Value::Null;
    }
    if let Some(s) = v.as_string()
        && let Ok(parsed) = serde_json::from_str(&s)
    {
        return parsed;
    }
    js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn parse(data: &JsValue) -> Option<Msg> {
    let kind = get(data, "wordcraft").as_string()?;
    let id = to_json(&get(data, "id"));
    match kind.as_str() {
        "open" => {
            let raw = get(data, "data");
            let bytes = if raw.is_instance_of::<js_sys::ArrayBuffer>() || raw.is_instance_of::<js_sys::Uint8Array>() {
                js_sys::Uint8Array::new(&raw).to_vec()
            } else {
                return None;
            };
            let name = get(data, "name").as_string().unwrap_or_else(|| "document.docx".into());
            Some(Msg::Open { id, name, bytes })
        }
        "run" => {
            let command = get(data, "command").as_string()?;
            let params = match to_json(&get(data, "params")) {
                Value::Null => json!({}),
                p => p,
            };
            Some(Msg::Run { id, command, params })
        }
        "save" => Some(Msg::Save { id, name: get(data, "name").as_string() }),
        _ => None,
    }
}

/// `?accent=RRGGBB`: the host's colour for the editor's accents.
pub fn accent_from_url() -> Option<egui::Color32> {
    let search = web_sys::window()?.location().search().ok()?;
    let hex = search.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("accent="))?;
    let hex = hex.trim_start_matches("%23").trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(egui::Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

//! The `postMessage` bridge to a same-origin parent page (`?host=parent`), for pages that embed
//! WordCraft in an `<iframe>` and keep its files (a Nextcloud app, a personal file server). The
//! same messages as GridCraft's and PdfCraft's bridges; each is a plain object with a `type`:
//!
//! - `wordcraft:ready` (to the parent): `{ version }`, sent once the app is running and
//!   listening; send a file now.
//! - `wordcraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. It opens like a document picked with File › Open or dropped on the window.
//! - `wordcraft:save` (to the parent): `{ name, saveAs, bytes }` (a transferred `ArrayBuffer`) for
//!   every file WordCraft would otherwise download. The parent stores it and reports failures
//!   itself. `saveAs` says whether the file becomes the document's file:
//!   - Save: `false`. It writes the opened file under its own name and format, as the desktop
//!     app does. A name the parent never sent (an untitled document, an imported .doc saved as
//!     .docx) is a new file: `true`, as for Save As.
//!   - Save As, and File › Export to another document format (which runs Save As): `true` for a
//!     format that keeps everything (.docx and the other Word packages, .odt, .rtf). The document
//!     is now that file; the next Save sends its name with `false`.
//!   - Save As or Export to a format that doesn't (PDF, Markdown, plain text), Export as PDF and
//!     Export Page as PNG: `false`. They are copies; the document's file stays the same.
//! - `wordcraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
//! - `wordcraft:cmd` (from the parent): `{ id, params?, requestId? }` runs the engine command
//!   `id` (any `wordcraft:cmd` the desktop app accepts) like a ribbon button. `params` is a JSON
//!   string (absent or `null` means `{}`); `requestId` is echoed in the response.
//! - `wordcraft:response` (to the parent): `{ id, requestId?, ok, value? }` or
//!   `{ id, requestId?, ok: false, error }` for every `wordcraft:cmd`, once it has run.
//! - `wordcraft:docClosed` (to the parent): sent when the document the parent opened closes
//!   (a successful `wordcraft:cmd` `file.close`, or a `quit` close); the standalone page never
//!   posts it.
//! - `wordcraft:failed` (to the parent): `{ error }` when the app couldn't start.
//!
//! Only the parent window on the page's own origin is heard or answered. In host mode the parent
//! guards leaving the page, and `?author=` names the user for comments and tracked changes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wordcraft_ui_egui::{Inbox, Services, WordApp};

/// The longest user name taken from `?author=`.
const MAX_AUTHOR_CHARS: usize = 200;

/// Largest `id`/`requestId` accepted from the host.
const MAX_ID_BYTES: usize = 1024;

/// Largest `params` string accepted from the host.
const MAX_PARAMS_BYTES: usize = 1 << 20;

/// Most commands waiting for the next frame.
const MAX_PENDING_COMMANDS: usize = 256;

/// Most `params` text waiting for the next frame, all commands together.
const MAX_PENDING_PARAMS_BYTES: usize = 8 << 20;

#[derive(Clone)]
pub struct Bridge {
    window: web_sys::Window,
    parent: web_sys::Window,
    origin: String,
    /// Commands the parent sent since the last frame: (id, params JSON, request id).
    commands: Rc<RefCell<Vec<(String, String, Option<String>)>>>,
    /// Total length of the `params` text of the commands that haven't run yet.
    queued_bytes: Rc<Cell<usize>>,
    /// Total length of the command identifiers that haven't run yet.
    queued_id_bytes: Rc<Cell<usize>>,
}

impl Bridge {
    /// `None` when the page isn't framed (there is no parent to talk to), or when the parent is
    /// on another origin: messages only go to this origin, so the browser would drop every save
    /// without an error while the document showed as saved.
    pub fn new() -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let origin = window.location().origin().ok()?;
        // An opaque origin (a `file://` page in some browsers) can't be a `postMessage` target.
        if origin == "null" {
            return None;
        }
        // Reading another origin's location throws, which is `None` too.
        if parent.location().origin().ok()? != origin {
            return None;
        }
        Some(Self { window, parent, origin, commands: Rc::default(), queued_bytes: Rc::default(), queued_id_bytes: Rc::default() })
    }

    /// Hand saved and exported files to the parent instead of downloading them, report unsaved
    /// changes to it, and hear the files it sends.
    pub fn connect(&self, services: &mut Services, inbox: Inbox, ctx: egui::Context) {
        self.listen(inbox, ctx);
        let bridge = self.clone();
        services.host_save = Some(Box::new(move |name: &str, bytes: &[u8], save_as: bool| bridge.post_save(name, bytes, save_as)));
        // Anything else that would download goes to the parent too.
        let bridge = self.clone();
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| bridge.post_save(name, bytes, false)));
        // The browser's save picker is never used, so no save finishes later.
        services.async_saves = None;
        let bridge = self.clone();
        let reported: Rc<Cell<Option<bool>>> = Rc::default();
        services.on_dirty = Some(Box::new(move |dirty: bool| {
            if reported.get() != Some(dirty) && bridge.send(&[("type", "wordcraft:dirty".into()), ("dirty", dirty.into())]) {
                reported.set(Some(dirty));
            }
        }));
    }

    /// `?author=`: the host's name for the user signs comments and tracked changes.
    pub fn set_author(app: &mut WordApp, author: &str) {
        let author: String = author.trim().chars().take(MAX_AUTHOR_CHARS).collect();
        if !author.is_empty() {
            app.session.author = author;
        }
    }

    pub fn post_ready(&self) {
        self.send(&[("type", "wordcraft:ready".into()), ("version", env!("CARGO_PKG_VERSION").into())]);
    }

    /// The app couldn't start (no WebGPU or WebGL2, for example): let the parent say so.
    pub fn post_failed(&self, error: &str) {
        self.send(&[("type", "wordcraft:failed".into()), ("error", error.into())]);
    }

    /// Commands the parent sent since the last frame, oldest first: (id, params JSON, request id).
    pub fn take_commands(&self) -> Vec<(String, String, Option<String>)> {
        self.queued_bytes.set(0);
        self.queued_id_bytes.set(0);
        std::mem::take(&mut *self.commands.borrow_mut())
    }

    /// Tell the parent how a command (`wordcraft:cmd`) ended.
    pub fn post_response(&self, id: &str, request_id: Option<&str>, ok: bool, text: Option<&str>) {
        let mut fields: Vec<(&str, JsValue)> = vec![("type", "wordcraft:response".into()), ("id", id.into())];
        if let Some(request_id) = request_id {
            fields.push(("requestId", request_id.into()));
        }
        fields.push(("ok", ok.into()));
        if ok {
            if let Some(value) = text {
                fields.push(("value", value.into()));
            }
        } else {
            fields.push(("error", text.unwrap_or("unknown failure").into()));
        }
        self.send(&fields);
    }

    /// Tell the parent the document it opened has closed.
    pub fn post_doc_closed(&self) {
        self.send(&[("type", "wordcraft:docClosed".into())]);
    }

    fn post_save(&self, path: &str, bytes: &[u8], save_as: bool) -> Result<(), String> {
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let message = message(&[
            ("type", "wordcraft:save".into()),
            ("name", name.as_str().into()),
            ("saveAs", save_as.into()),
            ("bytes", buffer.clone().into()),
        ])
        .ok_or_else(|| "couldn't build the message".to_string())?;
        self.parent
            .post_message_with_transfer(&message, &self.origin, &js_sys::Array::of1(&buffer))
            .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
    }

    fn listen(&self, inbox: Inbox, ctx: egui::Context) {
        let parent = self.parent.clone();
        let origin = self.origin.clone();
        let bridge = self.clone();
        let commands = self.commands.clone();
        let queued_bytes = self.queued_bytes.clone();
        let queued_id_bytes = self.queued_id_bytes.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                return;
            }
            let data = event.data();
            let source_is_parent = event.source().is_some_and(|s| js_sys::Object::is(&s, &parent));
            match crate::host_filter::decide(&event.origin(), &origin, source_is_parent, string(&data, "type").as_deref()) {
                Ok(crate::host_filter::Incoming::Open) => {
                    let name = string(&data, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Document.docx".to_string());
                    let Some(bytes) = bytes(&data) else { return };
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                    ctx.request_repaint();
                }
                Ok(crate::host_filter::Incoming::Cmd) => {
                    let request_id = string(&data, "requestId").filter(|r| !r.is_empty());
                    let id_raw = string(&data, "id");
                    if id_raw.as_ref().map(|s| s.len()).unwrap_or(0) > MAX_ID_BYTES {
                        bridge.post_response("", request_id.as_deref(), false, Some(&format!("id is larger than {MAX_ID_BYTES} bytes")));
                        ctx.request_repaint();
                        return;
                    }
                    let id = id_raw.filter(|i| !i.trim().is_empty());
                    let Some(id) = id else {
                        bridge.post_response("", request_id.as_deref(), false, Some("a command needs an id"));
                        ctx.request_repaint();
                        return;
                    };
                    if request_id.as_ref().map(|s| s.len()).unwrap_or(0) > MAX_ID_BYTES {
                        bridge.post_response(&id, request_id.as_deref(), false, Some(&format!("requestId is larger than {MAX_ID_BYTES} bytes")));
                        ctx.request_repaint();
                        return;
                    }
                    let params_value = js_sys::Reflect::get(&data, &JsValue::from_str("params")).ok().filter(|v| !v.is_undefined() && !v.is_null());
                    let params = match params_value {
                        None => "{}".to_string(),
                        Some(v) => {
                            let js_len = v.dyn_ref::<js_sys::JsString>().map(|s| s.length() as usize);
                            if js_len.unwrap_or(usize::MAX) > MAX_PARAMS_BYTES {
                                bridge.post_response(
                                    &id,
                                    request_id.as_deref(),
                                    false,
                                    Some(&format!("params are larger than {MAX_PARAMS_BYTES} bytes")),
                                );
                                ctx.request_repaint();
                                return;
                            }
                            match v.as_string() {
                                Some(text) if text.len() <= MAX_PARAMS_BYTES => text,
                                Some(_) => {
                                    bridge.post_response(
                                        &id,
                                        request_id.as_deref(),
                                        false,
                                        Some(&format!("params are larger than {MAX_PARAMS_BYTES} bytes")),
                                    );
                                    ctx.request_repaint();
                                    return;
                                }
                                None => {
                                    bridge.post_response(&id, request_id.as_deref(), false, Some("params must be a string of valid JSON"));
                                    ctx.request_repaint();
                                    return;
                                }
                            }
                        }
                    };
                    let id_bytes = id.len() + request_id.as_deref().map(str::len).unwrap_or(0);
                    if commands.borrow().len() >= MAX_PENDING_COMMANDS
                        || queued_bytes.get().saturating_add(params.len()) > MAX_PENDING_PARAMS_BYTES
                        || queued_id_bytes.get().saturating_add(id_bytes) > MAX_PENDING_PARAMS_BYTES
                    {
                        bridge.post_response(&id, request_id.as_deref(), false, Some("too many commands are waiting; try again"));
                        ctx.request_repaint();
                        return;
                    }
                    queued_bytes.set(queued_bytes.get().saturating_add(params.len()));
                    queued_id_bytes.set(queued_id_bytes.get().saturating_add(id_bytes));
                    commands.borrow_mut().push((id, params, request_id));
                    ctx.request_repaint();
                }
                Ok(crate::host_filter::Incoming::SaveAck) | Err(_) => {}
            }
        });
        if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            // The listener lives as long as the page.
            on_message.forget();
        }
    }

    /// Post a message to the parent; false if it couldn't be sent.
    fn send(&self, fields: &[(&str, JsValue)]) -> bool {
        message(fields).is_some_and(|m| self.parent.post_message(&m, &self.origin).is_ok())
    }
}

fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
    let m = js_sys::Object::new();
    for (key, value) in fields {
        js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
    }
    Some(m)
}

fn string(data: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_string()
}

/// `bytes` as an `ArrayBuffer` or `Uint8Array`.
fn bytes(data: &JsValue) -> Option<Vec<u8>> {
    let bytes = js_sys::Reflect::get(data, &JsValue::from_str("bytes")).ok()?;
    if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        Some(array.to_vec())
    } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
        Some(js_sys::Uint8Array::new(&bytes).to_vec())
    } else {
        None
    }
}

//! The `postMessage` bridge to the parent page (`?host=parent`), for pages that embed
//! WordCraft in an `<iframe>` and keep its files (a Nextcloud app, a personal file server). The
//! same messages as GridCraft's and PdfCraft's bridges; each is a plain object with a `type`:
//!
//! - `wordcraft:ready` (to the parent): `{ version }`, sent once the app is running and
//!   listening; send a file now.
//! - `wordcraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. It opens like a document picked with File › Open or dropped on the window.
//! - `wordcraft:save` (to the parent): `{ id, name, saveAs, bytes }` (a transferred `ArrayBuffer`)
//!   for every file WordCraft would otherwise download. The parent stores it and answers
//!   `wordcraft:saved { id }`; until then the document counts as unsaved. With no answer in 30 s
//!   WordCraft posts `wordcraft:failed { id, error }` and the document stays unsaved.
//!   `saveAs` says whether the file becomes the document's file:
//!   - Save: `false`. It writes the opened file under its own name and format, as the desktop
//!     app does. A name the parent never sent (an untitled document, an imported .doc saved as
//!     .docx) is a new file: `true`, as for Save As.
//!   - Save As, and File › Export to another document format (which runs Save As): `true` for a
//!     format that keeps everything (.docx and the other Word packages, .odt, .rtf). The document
//!     is now that file; the next Save sends its name with `false`.
//!   - Save As or Export to a format that doesn't (PDF, Markdown, plain text), Export as PDF and
//!     Export Page as PNG: `false`. They are copies; the document's file stays the same.
//! - `wordcraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
//! - `wordcraft:saved` (from the parent): `{ id }`, the file from that `wordcraft:save` is stored.
//! - `wordcraft:failed` (to the parent): `{ error }` when the app couldn't start, or
//!   `{ id, error }` when a save wasn't confirmed in time.
//!
//! The bridge is installed only for a parent on the page's own origin, or for the exact origin
//! `?host_origin=` names (a bare `scheme://host[:port]`). Messages go to that origin, and only the
//! parent window on it is heard. Any other parent gets no bridge: Save downloads and this page
//! guards leaving it. In host mode the parent
//! guards leaving the page, and `?author=` names the user for comments and tracked changes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wordcraft_ui_egui::async_saves::{AsyncSaves, SaveOutcome};
use wordcraft_ui_egui::{Inbox, Services, WordApp};

use crate::bridge::SaveAcks;

/// The longest user name taken from `?author=`.
const MAX_AUTHOR_CHARS: usize = 200;

#[derive(Clone)]
pub struct Bridge {
    window: web_sys::Window,
    parent: web_sys::Window,
    origin: String,
    /// The unsaved-changes state last reported to the parent.
    reported: Rc<Cell<Option<bool>>>,
    /// The document's own unsaved-changes state (from `on_dirty`).
    dirty: Rc<Cell<bool>>,
    /// The app's saves in flight; each `wordcraft:save` is a ticket.
    saves: AsyncSaves,
    /// Tickets waiting for `wordcraft:saved`, with their deadlines.
    acks: Rc<RefCell<SaveAcks>>,
}

impl Bridge {
    /// `None` when the page isn't framed (there is no parent to talk to), or when the parent is
    /// on another origin that `host_origin` doesn't name exactly ([`crate::bridge::target_origin`]).
    pub fn new(host_origin: Option<&str>, saves: AsyncSaves) -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let own = window.location().origin().ok()?;
        // Reading a cross-origin parent's location throws: that counts as cross-origin.
        let parent_origin = parent.location().origin().ok();
        let origin = crate::bridge::target_origin(&own, parent_origin.as_deref(), host_origin)?;
        Some(Self { window, parent, origin, reported: Rc::default(), dirty: Rc::default(), saves, acks: Rc::default() })
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
        // A save finishes when the parent confirms it (`wordcraft:saved`), like the save picker's.
        services.async_saves = Some(self.saves.clone());
        let bridge = self.clone();
        services.on_dirty = Some(Box::new(move |dirty: bool| {
            bridge.dirty.set(dirty);
            bridge.report_dirty();
        }));
    }

    /// Once a frame: fail saves the parent didn't confirm in time, and report unsaved changes
    /// (a save still waiting for the parent counts as unsaved).
    pub fn tick(&self, ctx: &egui::Context) {
        let expired = self.acks.borrow_mut().expire(js_sys::Date::now());
        for (id, name) in expired {
            let error = format!("{name}: the host page didn't confirm the save");
            self.saves.finish(id, SaveOutcome::Failed(error.clone()));
            self.send(&[("type", "wordcraft:failed".into()), ("id", JsValue::from_f64(id as f64)), ("error", error.into())]);
        }
        if self.acks.borrow().waiting() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        self.report_dirty();
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

    fn report_dirty(&self) {
        let dirty = self.dirty.get() || self.acks.try_borrow().map_or(true, |a| a.waiting());
        if self.reported.get() != Some(dirty) && self.send(&[("type", "wordcraft:dirty".into()), ("dirty", dirty.into())]) {
            self.reported.set(Some(dirty));
        }
    }

    fn post_save(&self, path: &str, bytes: &[u8], save_as: bool) -> Result<(), String> {
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        // The app's save ticket is the id the parent answers with.
        let id = self.saves.start();
        let sent = message(&[
            ("type", "wordcraft:save".into()),
            ("id", JsValue::from_f64(id as f64)),
            ("name", name.as_str().into()),
            ("saveAs", save_as.into()),
            ("bytes", buffer.clone().into()),
        ])
        .ok_or_else(|| "couldn't build the message".to_string())
        .and_then(|message| {
            self.parent
                .post_message_with_transfer(&message, &self.origin, &js_sys::Array::of1(&buffer))
                .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
        });
        match sent {
            Ok(()) => {
                self.acks.borrow_mut().posted_as(id, &name, js_sys::Date::now());
                self.report_dirty();
                Ok(())
            }
            Err(e) => {
                self.saves.finish(id, SaveOutcome::Failed(e.clone()));
                Err(e)
            }
        }
    }

    /// Hear `wordcraft:open` from the parent and queue the file like a dropped one.
    fn listen(&self, inbox: Inbox, ctx: egui::Context) {
        let parent = self.parent.clone();
        let origin = self.origin.clone();
        let bridge = self.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                return;
            }
            let data = event.data();
            match string(&data, "type").as_deref() {
                Some("wordcraft:open") => {}
                Some("wordcraft:saved") => {
                    let id = js_sys::Reflect::get(&data, &JsValue::from_str("id")).ok().and_then(|v| v.as_f64());
                    if let Some(id) = id.filter(|id| id.is_finite() && *id >= 0.0 && id.fract() == 0.0)
                        && bridge.acks.borrow_mut().saved(id as u64)
                    {
                        bridge.saves.finish(id as u64, SaveOutcome::Saved);
                        ctx.request_repaint();
                    }
                    return;
                }
                _ => return,
            }
            let name = string(&data, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Document.docx".to_string());
            let Some(bytes) = bytes(&data) else { return };
            inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
            ctx.request_repaint();
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

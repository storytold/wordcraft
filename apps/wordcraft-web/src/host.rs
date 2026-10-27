//! The `postMessage` bridge to a same-origin parent page (`?host=parent`), for pages that embed
//! WordCraft in an `<iframe>` and keep its files (a Nextcloud app, a personal file server). The
//! same messages as GridCraft's and PdfCraft's bridges; each is a plain object with a `type`:
//!
//! - `wordcraft:ready` (to the parent): `{ version }`, sent once the app is running and
//!   listening; send a file now.
//! - `wordcraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. Without a request ID it opens like a picked or dropped document. An optional
//!   bounded `requestId` requests a correlated import: unsaved/unconfirmed work is refused, and
//!   `wordcraft:opened {requestId, document, revision}` is sent only after the actual import, or
//!   `{requestId, error}` on failure. One correlated import can be pending.
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
//! - `wordcraft:changed` (to the parent): `{ document, revision }` after every native document revision,
//!   including further edits while `dirty` stays true. Revision is an opaque string that also
//!   distinguishes replacing the document. Document is a separate opaque identity string; hosts
//!   must refuse to store snapshots under a different document's identity.
//! - `wordcraft:snapshot` (from the parent): `{ requestId }`, a nonempty string of at most 128
//!   UTF-8 bytes. Requests the complete native JSON. Pending comment/reply text is committed as
//!   for File Save; dirty state is never cleared and storage is not acknowledged.
//! - `wordcraft:snapshot-result` (to the parent): `{ requestId, document, revision, bytes }` (a transferred
//!   `ArrayBuffer` of native JSON), or `{ requestId, error }`. At most one request is pending;
//!   another receives an error. The native Document parser checks the serialized snapshot.
//!   Neither snapshot nor changed grants durable Saved. Reopen native bytes with `wordcraft:open`
//!   and a `.json` name. The host guards unsaved work before replacing a document.
//! With `&save=managed`, Save instead emits `wordcraft:save-request
//! {requestId, document, revision}` without stamping, exporting or marking the document saved.
//! The host requests a native snapshot with that requestId and, only after durable storage,
//! answers `wordcraft:saved {requestId, document, revision}` for that exported snapshot. Only
//! an exact current document/revision (including pending comment drafts) clears dirty. An older
//! known manual save completes its ticket while newer edits remain unsaved. Unknown ACKs do
//! nothing. `wordcraft:save-failed {requestId, error}` ends only its matching manual ticket;
//! error is nonempty and at most 1024 UTF-8 bytes. One manual save may be pending. Save As and
//! Export remain downloadable copies and never acknowledge storage of the native document.
//!
//! - `wordcraft:failed` (to the parent): `{ error }` when the app couldn't start.
//!
//! Only the parent window on the page's own origin is heard or answered. In host mode the parent
//! guards leaving the page, and `?author=` names the user for comments and tracked changes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::managed::{ManagedSaves, SnapshotKey};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wordcraft_ui_egui::async_saves::{AsyncSaves, SaveOutcome};
use wordcraft_ui_egui::{Inbox, Services, WordApp};

#[derive(Clone, PartialEq, Eq)]
enum Acknowledgement {
    Saved(SnapshotKey),
    Failed(String, String),
}

/// The longest user name taken from `?author=`.
const MAX_AUTHOR_CHARS: usize = 200;

struct PendingOpen {
    request_id: String,
    name: String,
    bytes: Vec<u8>,
}

#[derive(Clone)]
pub struct Bridge {
    managed: bool,
    saves: AsyncSaves,
    managed_saves: Rc<RefCell<ManagedSaves>>,
    acknowledgements: Rc<RefCell<Vec<Acknowledgement>>>,
    window: web_sys::Window,
    parent: web_sys::Window,
    origin: String,
    snapshot: Rc<RefCell<Option<String>>>,
    open: Rc<RefCell<Option<PendingOpen>>>,
    observed: Rc<RefCell<crate::snapshot::RevisionObserver>>,
}

impl Bridge {
    /// `None` when the page isn't framed (there is no parent to talk to), or when the parent is
    /// on another origin: messages only go to this origin, so the browser would drop every save
    /// without an error while the document showed as saved.
    pub fn new(managed: bool) -> Option<Self> {
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
        Some(Self {
            managed,
            saves: AsyncSaves::default(),
            managed_saves: Rc::default(),
            acknowledgements: Rc::default(),
            window,
            parent,
            origin,
            snapshot: Rc::default(),
            open: Rc::default(),
            observed: Rc::default(),
        })
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
        if self.managed {
            services.async_saves = Some(self.saves.clone());
            let bridge = self.clone();
            services.host_save_request = Some(Box::new(move |session, draft_revision| {
                if bridge.managed_saves.borrow().pending() {
                    return Err("native save already pending".into());
                }
                let ticket = bridge.saves.start();
                let document = crate::snapshot::document(session);
                let revision = crate::snapshot::revision(session, draft_revision);
                let request_id = match bridge.managed_saves.borrow_mut().start(&document, ticket) {
                    Ok(id) => id,
                    Err(error) => {
                        bridge.saves.finish(ticket, SaveOutcome::Failed(error.clone()));
                        return Err(error);
                    }
                };
                if bridge.send(&[
                    ("type", "wordcraft:save-request".into()),
                    ("requestId", request_id.as_str().into()),
                    ("document", document.as_str().into()),
                    ("revision", revision.as_str().into()),
                ]) {
                    Ok(ticket)
                } else {
                    bridge.managed_saves.borrow_mut().fail(&request_id);
                    let error = "couldn't request native save".to_string();
                    bridge.saves.finish(ticket, SaveOutcome::Failed(error.clone()));
                    Err(error)
                }
            }));
        }
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

    /// Hear `wordcraft:open` from the parent and queue the file like a dropped one.
    fn listen(&self, inbox: Inbox, ctx: egui::Context) {
        let parent = self.parent.clone();
        let origin = self.origin.clone();
        let snapshot = self.snapshot.clone();
        let open = self.open.clone();
        let bridge = self.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                return;
            }
            let data = event.data();
            if bridge.managed {
                let kind = string(&data, "type");
                let acknowledgement = match kind.as_deref() {
                    Some("wordcraft:saved") => {
                        let Some(request_id) = bounded_string(&data, "requestId", 128) else { return };
                        let Some(document) = bounded_string(&data, "document", 128) else { return };
                        let Some(revision) = bounded_string(&data, "revision", 128) else { return };
                        let key = SnapshotKey { request_id, document, revision };
                        if !bridge.managed_saves.borrow().knows(&key) {
                            return;
                        }
                        Some(Acknowledgement::Saved(key))
                    }
                    Some("wordcraft:save-failed") => {
                        let Some(id) = bounded_string(&data, "requestId", 128) else { return };
                        if !bridge.managed_saves.borrow().knows_failure(&id) {
                            return;
                        }
                        let Some(error) = bounded_string(&data, "error", 1024) else { return };
                        Some(Acknowledgement::Failed(id, error))
                    }
                    _ => None,
                };
                if let Some(acknowledgement) = acknowledgement {
                    let mut pending = bridge.acknowledgements.borrow_mut();
                    let id = match &acknowledgement {
                        Acknowledgement::Saved(key) => &key.request_id,
                        Acknowledgement::Failed(id, _) => id,
                    };
                    let already_pending = pending.iter().any(|old| match old {
                        Acknowledgement::Saved(key) => &key.request_id == id,
                        Acknowledgement::Failed(old_id, _) => old_id == id,
                    });
                    if pending.len() < 2 && !already_pending {
                        pending.push(acknowledgement);
                        ctx.request_repaint();
                    }
                    return;
                }
            }
            if string(&data, "type").as_deref() == Some("wordcraft:snapshot") {
                let Some(id) = string(&data, "requestId").filter(|id| !id.is_empty() && id.len() <= 128) else { return };
                if snapshot.borrow().is_some() {
                    bridge.send(&[
                        ("type", "wordcraft:snapshot-result".into()),
                        ("requestId", id.into()),
                        ("error", "snapshot already pending".into()),
                    ]);
                    return;
                }
                *snapshot.borrow_mut() = Some(id);
                ctx.request_repaint();
                return;
            }
            if string(&data, "type").as_deref() != Some("wordcraft:open") {
                return;
            }
            let request_id = if js_sys::Reflect::has(&data, &JsValue::from_str("requestId")).unwrap_or(false) {
                let Some(id) = string(&data, "requestId").filter(|id| !id.is_empty() && id.len() <= 128) else { return };
                Some(id)
            } else {
                None
            };
            if let Some(id) = &request_id
                && open.borrow().is_some()
            {
                bridge.post_open_error(id, "open already pending");
                return;
            }
            let name = string(&data, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Document.docx".to_string());
            let Some(bytes) = bytes(&data) else {
                if let Some(id) = request_id {
                    bridge.post_open_error(&id, "open requires document bytes");
                }
                return;
            };
            if let Some(request_id) = request_id {
                *open.borrow_mut() = Some(PendingOpen { request_id, name, bytes });
            } else {
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
            }
            ctx.request_repaint();
        });
        if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            // The listener lives as long as the page.
            on_message.forget();
        }
    }

    /// Report native revisions independently of dirty state, then answer a pending native
    /// snapshot request after the app has processed its inbox and commands. Pending comment/reply
    /// drafts are committed through the same commands as Save. Neither is a save
    /// acknowledgement; the host must confirm its own durable storage separately.
    pub fn observe(&self, app: &mut WordApp, ctx: &egui::Context) {
        // Never flush drafts on ACK. Compare the current token first: a known old manual save
        // completes its ticket, but newer edits (including unblurred comment text) stay dirty.
        for acknowledgement in std::mem::take(&mut *self.acknowledgements.borrow_mut()) {
            match acknowledgement {
                Acknowledgement::Saved(key) => {
                    let document = crate::snapshot::document(&app.session);
                    let revision = crate::snapshot::revision(&app.session, app.native_draft_revision());
                    if let Some(completion) = self.managed_saves.borrow_mut().acknowledge(&key, &document, &revision) {
                        if completion.mark_saved {
                            app.session.dirty = false;
                        }
                        if let Some(ticket) = completion.ticket {
                            self.saves.finish(ticket, SaveOutcome::Saved);
                        }
                        ctx.request_repaint();
                    }
                }
                Acknowledgement::Failed(id, error) => {
                    if let Some(ticket) = self.managed_saves.borrow_mut().fail(&id) {
                        self.saves.finish(ticket, SaveOutcome::Failed(error));
                        ctx.request_repaint();
                    }
                }
            }
        }
        if let Some(pending) = self.open.borrow_mut().take() {
            match app.open_host_document(&pending.name, &pending.bytes) {
                Ok(()) => {
                    let document = crate::snapshot::document(&app.session);
                    let revision = crate::snapshot::revision(&app.session, app.native_draft_revision());
                    self.send(&[
                        ("type", "wordcraft:opened".into()),
                        ("requestId", pending.request_id.as_str().into()),
                        ("document", document.as_str().into()),
                        ("revision", revision.as_str().into()),
                    ]);
                }
                Err(error) => self.post_open_error(&pending.request_id, &error),
            }
        }
        let pending = self.snapshot.borrow_mut().take();
        let prepared = if pending.is_some() { app.prepare_native_snapshot() } else { Ok(()) };
        let session = &app.session;
        let draft_revision = app.native_draft_revision();
        let document = crate::snapshot::document(session);
        let revision = crate::snapshot::revision(session, draft_revision);
        if self.observed.borrow_mut().changed(session, draft_revision) {
            self.send(&[("type", "wordcraft:changed".into()), ("document", document.as_str().into()), ("revision", revision.as_str().into())]);
        }
        let Some(id) = pending else { return };
        let key = SnapshotKey { request_id: id.clone(), document: document.clone(), revision: revision.clone() };
        let result = prepared
            .and_then(|()| {
                if self.managed {
                    self.managed_saves.borrow().check_export(&key)?;
                }
                crate::snapshot::native_snapshot(session)
            })
            .and_then(|bytes| {
                let buffer = js_sys::Uint8Array::from(bytes.as_slice()).buffer();
                let value = message(&[
                    ("type", "wordcraft:snapshot-result".into()),
                    ("requestId", id.as_str().into()),
                    ("document", document.as_str().into()),
                    ("revision", revision.as_str().into()),
                    ("bytes", buffer.clone().into()),
                ])
                .ok_or_else(|| "couldn't build the snapshot message".to_string())?;
                self.parent
                    .post_message_with_transfer(&value, &self.origin, &js_sys::Array::of1(&buffer))
                    .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
                if self.managed {
                    self.managed_saves.borrow_mut().exported(key)?;
                }
                Ok(())
            });
        if let Err(error) = result {
            self.send(&[("type", "wordcraft:snapshot-result".into()), ("requestId", id.as_str().into()), ("error", error.as_str().into())]);
        }
    }

    fn post_open_error(&self, request_id: &str, error: &str) {
        self.send(&[("type", "wordcraft:opened".into()), ("requestId", request_id.into()), ("error", error.into())]);
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

fn bounded_string(data: &JsValue, key: &str, max: usize) -> Option<String> {
    string(data, key).filter(|value| !value.is_empty() && value.len() <= max)
}

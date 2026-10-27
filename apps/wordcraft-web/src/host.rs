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
//!
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
//! Legacy parent mode only hears and answers the parent window on the page's own origin.
//! `?host=port&parent_origin=<canonical HTTP(S) origin>` instead supports an opaque sandbox.
//! Its immutable path must be `/b/<64 lowercase hex>/index.html`, optionally prefixed by
//! `/_editor` (same-host assets) or `/workbook-editor`. After child load it sends only `{type:"wordcraft:hello",protocol:1,bundle}`
//! over the window channel. One exact-parent, configured-origin connect with exactly those three
//! keys (`type:"wordcraft:connect"`) and one transferred MessagePort is accepted once. All
//! document content and subsequent events use that private port, never a window fallback.
//! Document identities and revisions are local to this editor instance. Hosts must retire old
//! ports and callbacks on navigation, never reusing an old ACK for a new iframe instance.
//! Port mode enables managed saves; native binary frames are nonempty and at most 32 MiB.
//! Correlated Open also checks this bound before copying JS bytes; legacy ID-less window Open
//! retains its behavior. In host mode the parent guards leaving the page, and `?author=` names
//! the user for comments and tracked changes.

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
struct PrivatePort {
    bundle: String,
    endpoint: Rc<RefCell<Option<web_sys::MessagePort>>>,
    hello_sent: Rc<Cell<bool>>,
    ready: Rc<Cell<bool>>,
}

#[derive(Clone)]
pub struct Bridge {
    port: Option<PrivatePort>,
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
            port: None,
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

    /// Opaque iframe profile: trust only a canonical configured parent origin and one private
    /// capability port. The legacy constructor's same-origin checks remain unchanged.
    pub fn new_port(parent_origin: &str) -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let parsed = web_sys::Url::new(parent_origin).ok()?;
        if !crate::capability::canonical_origin(parent_origin, &parsed.protocol(), &parsed.origin()) {
            return None;
        }
        let path = window.location().pathname().ok()?;
        let bundle = crate::capability::bundle_from_path(&path)?.to_owned();
        Some(Self {
            port: Some(PrivatePort { bundle, endpoint: Rc::default(), hello_sent: Rc::default(), ready: Rc::default() }),
            managed: true,
            saves: AsyncSaves::default(),
            managed_saves: Rc::default(),
            acknowledgements: Rc::default(),
            window,
            parent,
            origin: parent_origin.into(),
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
        if let Some(port) = &self.port {
            port.ready.set(true);
        }
        self.send(&[("type", "wordcraft:ready".into()), ("version", env!("CARGO_PKG_VERSION").into())]);
    }

    /// The app couldn't start (no WebGPU or WebGL2, for example): let the parent say so.
    pub fn post_failed(&self, error: &str) {
        self.send(&[("type", "wordcraft:failed".into()), ("error", error.into())]);
    }

    fn post_save(&self, path: &str, bytes: &[u8], save_as: bool) -> Result<(), String> {
        if self.port.is_some() && !crate::capability::native_len_ok(bytes.len()) {
            return Err("native file must be nonempty and at most 32 MiB".into());
        }
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let message = message(&[
            ("type", "wordcraft:save".into()),
            ("name", name.as_str().into()),
            ("saveAs", save_as.into()),
            ("bytes", buffer.clone().into()),
        ])
        .ok_or_else(|| "couldn't build the message".to_string())?;
        self.transfer(&message, &buffer)
    }

    /// Hear `wordcraft:open` from the parent and queue the file like a dropped one.
    fn listen(&self, inbox: Inbox, ctx: egui::Context) {
        let snapshot = self.snapshot.clone();
        let open = self.open.clone();
        let bridge = self.clone();
        let request_ctx = ctx.clone();
        let receive: Rc<dyn Fn(JsValue)> = Rc::new(move |data: JsValue| {
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
                        request_ctx.request_repaint();
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
                request_ctx.request_repaint();
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
            let bytes = match bytes(&data, bridge.port.is_some() || request_id.is_some()) {
                Ok(bytes) => bytes,
                Err(error) => {
                    if let Some(id) = request_id {
                        bridge.post_open_error(&id, &error);
                    }
                    return;
                }
            };
            if let Some(request_id) = request_id {
                *open.borrow_mut() = Some(PendingOpen { request_id, name, bytes });
            } else {
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
            }
            request_ctx.request_repaint();
        });
        if self.port.is_some() {
            self.listen_port(receive, ctx);
            return;
        }
        let parent = self.parent.clone();
        let origin = self.origin.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                return;
            }
            receive(event.data());
        });
        if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            on_message.forget();
        }
    }

    fn listen_port(&self, receive: Rc<dyn Fn(JsValue)>, ctx: egui::Context) {
        let bridge = self.clone();
        let on_connect = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let Some(state) = &bridge.port else {
                return;
            };
            if !state.hello_sent.get()
                || state.endpoint.borrow().is_some()
                || event.origin() != bridge.origin
                || !event.source().is_some_and(|s| js_sys::Object::is(&s, &bridge.parent))
            {
                return;
            }
            let data = event.data();
            let Some(object) = data.dyn_ref::<js_sys::Object>() else {
                return;
            };
            if js_sys::Object::keys(object).length() != 3
                || string(&data, "type").as_deref() != Some("wordcraft:connect")
                || string(&data, "bundle").as_deref() != Some(state.bundle.as_str())
                || js_sys::Reflect::get(&data, &"protocol".into()).ok().and_then(|v| v.as_f64()) != Some(1.)
            {
                return;
            }
            // Only three bounded primitive control fields can reach JSON.stringify; native
            // document content is always binary and never passes through this conversion.
            let valid = js_sys::JSON::stringify(&data)
                .ok()
                .and_then(|v| v.as_string())
                .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
                .is_some_and(|v| crate::capability::connect_matches(&v, &state.bundle, event.ports().length() as usize));
            if !valid {
                return;
            }
            let Ok(port) = event.ports().get(0).dyn_into::<web_sys::MessagePort>() else {
                return;
            };
            let receive = receive.clone();
            let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| receive(event.data()));
            port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            port.start();
            *state.endpoint.borrow_mut() = Some(port);
            on_message.forget();
            if state.ready.get() {
                bridge.post_ready();
            }
            ctx.request_repaint();
        });
        if self.window.add_event_listener_with_callback("message", on_connect.as_ref().unchecked_ref()).is_ok() {
            on_connect.forget();
            if self.window.document().is_some_and(|d| d.ready_state() == "complete") {
                self.defer_hello();
            } else {
                let bridge = self.clone();
                let on_load = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| bridge.defer_hello());
                if self.window.add_event_listener_with_callback("load", on_load.as_ref().unchecked_ref()).is_ok() {
                    on_load.forget();
                }
            }
        }
    }

    fn defer_hello(&self) {
        let bridge = self.clone();
        let callback = Closure::<dyn FnMut()>::once(move || {
            let Some(port) = &bridge.port else {
                return;
            };
            if port.hello_sent.get() {
                return;
            }
            let Some(value) = message(&[("type", "wordcraft:hello".into()), ("protocol", 1.into()), ("bundle", port.bundle.as_str().into())]) else {
                return;
            };
            if bridge.parent.post_message(&value, &bridge.origin).is_ok() {
                port.hello_sent.set(true);
            }
        });
        // A task after child load also lets the parent's iframe load counter advance before
        // its strict handshake verifier receives the hello.
        if self.window.set_timeout_with_callback_and_timeout_and_arguments_0(callback.as_ref().unchecked_ref(), 0).is_ok() {
            callback.forget();
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
                if self.port.is_some() && !crate::capability::native_len_ok(bytes.len()) {
                    return Err("native snapshot must be nonempty and at most 32 MiB".into());
                }
                let buffer = js_sys::Uint8Array::from(bytes.as_slice()).buffer();
                let value = message(&[
                    ("type", "wordcraft:snapshot-result".into()),
                    ("requestId", id.as_str().into()),
                    ("document", document.as_str().into()),
                    ("revision", revision.as_str().into()),
                    ("bytes", buffer.clone().into()),
                ])
                .ok_or_else(|| "couldn't build the snapshot message".to_string())?;
                self.transfer(&value, &buffer)?;
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

    /// Port mode never falls back to the public window channel for content or events.
    fn send(&self, fields: &[(&str, JsValue)]) -> bool {
        message(fields).is_some_and(|m| match &self.port {
            Some(state) => state.endpoint.borrow().as_ref().is_some_and(|port| port.post_message(&m).is_ok()),
            None => self.parent.post_message(&m, &self.origin).is_ok(),
        })
    }

    fn transfer(&self, value: &JsValue, buffer: &js_sys::ArrayBuffer) -> Result<(), String> {
        let transfer = js_sys::Array::of1(buffer);
        match &self.port {
            Some(state) => state
                .endpoint
                .borrow()
                .as_ref()
                .ok_or_else(|| "private editor port is not connected".to_string())?
                .post_message_with_transferable(value, &transfer),
            None => self.parent.post_message_with_transfer(value, &self.origin, &transfer),
        }
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
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

/// Check native bounds before copying any JS buffer into WASM. Legacy ID-less window Open
/// retains its original behavior; correlated imports and all private-port imports are bounded.
fn bytes(data: &JsValue, bounded: bool) -> Result<Vec<u8>, String> {
    let bytes = js_sys::Reflect::get(data, &JsValue::from_str("bytes")).map_err(|_| "open requires document bytes")?;
    let array = if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        array.clone()
    } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
        js_sys::Uint8Array::new(&bytes)
    } else {
        return Err("open requires document bytes".into());
    };
    if bounded
        && (!array.buffer().is_instance_of::<js_sys::ArrayBuffer>()
            || !native_byte_length(array.as_ref())
            || !native_byte_length(array.buffer().as_ref()))
    {
        return Err("native document must be nonempty and at most 32 MiB".into());
    }
    Ok(array.to_vec())
}

fn bounded_string(data: &JsValue, key: &str, max: usize) -> Option<String> {
    string(data, key).filter(|value| !value.is_empty() && value.len() <= max)
}

fn native_byte_length(value: &JsValue) -> bool {
    js_sys::Reflect::get(value, &"byteLength".into()).ok().and_then(|value| value.as_f64()).is_some_and(crate::capability::native_js_len_ok)
}

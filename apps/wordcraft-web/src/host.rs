//! Host modes: a page that runs WordCraft (as an `<iframe>`, or in its own document) opens
//! documents in it and stores what it saves instead of the browser downloading. The messages and
//! the two transports are `craft-web-host`'s (storytold/craft-libs), the same contract as the
//! other Craft apps' bridges, with the `wordcraft:` prefix. This module is WordCraft's part of it.
//!
//! - `?host=parent` (an `<iframe>`, such as a Nextcloud app) talks to the same-origin parent over
//!   `postMessage`; `data-host="page"` on the canvas (the page's own document, no iframe) uses
//!   DOM `CustomEvent`s on the canvas instead. `?host=parent` wins when both are present. With no
//!   parent, or a parent on another origin, there is no bridge: the normal app with downloads.
//! - `wordcraft:ready` once the app is running and listening; send a file now. WordCraft reads
//!   bytes, so it says `file: false` and a host sends `bytes`.
//! - `wordcraft:open` (from the host): `{ name, bytes }`. It opens like a document picked with
//!   File › Open or dropped on the window; a file over [`MAX_OPEN_BYTES`] is refused before it is
//!   copied.
//! - `wordcraft:save` (to the host): `{ name, saveAs, bytes }` for every file WordCraft would
//!   otherwise download. The host stores it and reports failures itself. `saveAs` says whether
//!   the file becomes the document's file:
//!   - Save: `false`. It writes the opened file under its own name and format, as the desktop
//!     app does. A name the parent never sent (an untitled document, an imported .doc saved as
//!     .docx) is a new file: `true`, as for Save As.
//!   - Save As, and File › Export to another document format (which runs Save As): `true` for a
//!     format that keeps everything (.docx and the other Word packages, .odt, .rtf). The document
//!     is now that file; the next Save sends its name with `false`.
//!   - Save As or Export to a format that doesn't (PDF, Markdown, plain text), Export as PDF and
//!     Export Page as PNG: `false`. They are copies; the document's file stays the same.
//! - `wordcraft:dirty` whenever unsaved changes appear or go away, `wordcraft:failed` when the
//!   app couldn't start.
//!
//! The embedded editor (`?embed=1`) also hears `wordcraft:command` from an iframe host and
//! answers `wordcraft:result` — see [`crate::command`].
//!
//! In both host modes the host sends the document to edit, so no sample opens, saves are handed
//! to the host rather than downloaded, the host guards leaving the page, and `?author=` names the
//! user for comments and tracked changes.

use std::rc::Rc;

use craft_web_host::web::{FileData, Host, Message};
use serde_json::Value;
use wasm_bindgen::{JsCast as _, closure::Closure};
use wordcraft_ui_egui::{Inbox, Services, WordApp};

use crate::command::HostCommand;

/// The host's queued commands ([`Bridge::listen_commands`]), drained by the app each frame.
pub(crate) type CommandQueue = Rc<std::cell::RefCell<Vec<HostCommand>>>;

/// The largest queued commands: a hostile host cannot grow the queue past this. The app
/// drains it every frame.
const MAX_PENDING: usize = 32;

/// The app id in the host messages (`wordcraft:ready`).
const APP: &str = "wordcraft";

/// The name a document gets when the host sent none (the crate says `document` for a blank name).
const DEFAULT_NAME: &str = "Document.docx";

/// The largest document taken from the host page, refused before it is copied.
const MAX_OPEN_BYTES: u64 = 512 << 20;

/// The longest user name taken from `?author=`.
const MAX_AUTHOR_CHARS: usize = 200;

#[derive(Clone)]
pub struct Bridge {
    host: Rc<Host>,
}

impl Bridge {
    /// The bridge for this page, or `None` when WordCraft isn't hosted: no `?host=parent` and no
    /// `data-host="page"` on the canvas, an unframed `?host=parent` page (it stays the normal app
    /// with downloads), or a parent on another origin.
    pub fn detect(canvas: &web_sys::HtmlCanvasElement) -> Option<Self> {
        Some(Self { host: Rc::new(Host::connect(APP, canvas)?) })
    }

    /// Hand saved and exported files to the host instead of downloading them, report unsaved
    /// changes to it, and hear the files it sends.
    pub fn connect(&self, services: &mut Services, inbox: Inbox, ctx: egui::Context) {
        self.listen(inbox, ctx);
        let bridge = self.clone();
        services.host_save = Some(Box::new(move |name: &str, bytes: &[u8], save_as: bool| bridge.post_save(name, bytes, save_as)));
        // Anything else that would download goes to the host too.
        let bridge = self.clone();
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| bridge.post_save(name, bytes, false)));
        // The browser's save picker is never used, so no save finishes later.
        services.async_saves = None;
        // The crate does not repeat a state it already reported.
        let host = self.host.clone();
        services.on_dirty = Some(Box::new(move |dirty: bool| host.post_dirty(dirty)));
    }

    /// `?author=`: the host's name for the user signs comments and tracked changes.
    pub fn set_author(app: &mut WordApp, author: &str) {
        let author: String = author.trim().chars().take(MAX_AUTHOR_CHARS).collect();
        if !author.is_empty() {
            app.session.author = author;
        }
    }

    pub fn post_ready(&self) {
        let _ = self.host.post_ready(env!("CARGO_PKG_VERSION"), false);
    }

    /// The app couldn't start (no WebGPU or WebGL2, for example): let the host say so.
    pub fn post_failed(&self, error: &str) {
        let _ = self.host.post_failed(error, None);
    }

    fn post_save(&self, path: &str, bytes: &[u8], save_as: bool) -> Result<(), String> {
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
        self.host.post_save(&name, bytes, None, save_as)
    }

    /// Hear `wordcraft:open` and queue the file like a dropped one.
    fn listen(&self, inbox: Inbox, ctx: egui::Context) {
        self.host.listen(move |message| {
            if let Message::Open { name, data } = message {
                deliver_open(&inbox, &ctx, &name, &data);
            }
        });
    }
}

/// The file an open message carries, pushed onto the inbox for the next frame, like a dropped
/// file. A host's file is untrusted input: an oversized one is refused before it is copied into
/// the app's memory (the data's own length, not the host's claim). The crate has cleaned the name
/// (and cut it to 255 characters); a blank one comes as `document` and becomes `Document.docx`.
fn deliver_open(inbox: &Inbox, ctx: &egui::Context, name: &str, data: &FileData) {
    // A burst of opens between two frames would queue many large copies at once; the inbox
    // holds a bounded number of delivered files, of bounded total bytes (it drains every
    // frame).
    let pending = inbox.lock().unwrap_or_else(|e| e.into_inner());
    let queued_bytes: u64 = pending.iter().map(|(_, bytes)| bytes.len() as u64).sum();
    if crate::command::file_queue_full(pending.len(), queued_bytes) {
        drop(pending);
        log::warn!("host file {name} not taken: the delivered-file queue is full");
        return;
    }
    drop(pending);
    let name = if name == "document" { DEFAULT_NAME } else { name }.to_string();
    let FileData::Bytes(array) = data else {
        log::warn!("host file {name} came as a File; WordCraft reads bytes only; not opened");
        return;
    };
    if data.size() > MAX_OPEN_BYTES as f64 {
        log::warn!("host file is {} bytes, over the {} MiB host open limit; not opened", data.size(), MAX_OPEN_BYTES >> 20);
        return;
    }
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, array.to_vec()));
    ctx.request_repaint();
}

impl Bridge {
    /// Hear `wordcraft:command` from the host page and queue the commands for the next frame
    /// ([`CommandQueue`]). Same-origin iframe hosts only, with the same checks as the file
    /// messages; a page hosting WordCraft in its own document keeps to files and this returns
    /// quietly. A command over the cap is refused, not queued, so a hostile host cannot grow
    /// the queue. Messages that are not well-formed JSON or carry another `type` are ignored,
    /// as craft-web-host ignores its unknown messages; a command with bad fields is refused
    /// with a reply echoing whatever request id it carried.
    pub fn listen_commands(&self, queue: CommandQueue, ctx: egui::Context) -> Result<(), String> {
        if self.host.mode() != craft_web_host::Mode::Parent {
            return Ok(());
        }
        let window = web_sys::window().ok_or("no window")?;
        let parent = window.parent().ok().flatten().ok_or("no parent window")?;
        if js_sys::Object::is(&parent, &window) {
            return Err("no parent window".to_owned());
        }
        let origin = window.location().origin().map_err(|e| format!("{e:?}"))?;
        if parent.location().origin().map_err(|e| format!("{e:?}"))? != origin {
            return Err("the parent is on another origin".to_owned());
        }
        let bridge = self.clone();
        let on_message = Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                return;
            }
            // A plain object (like every craft-web-host message) or its JSON encoding. For
            // objects, only a command is serialized here — file messages carry payloads the
            // size cap would explode into a giant string, and they are not for this listener.
            // Text messages that parse to another type are ignored the same way.
            let text = match event.data().as_string() {
                Some(text) => text,
                None => {
                    let kind = js_sys::Reflect::get(&event.data(), &"type".into()).ok().and_then(|v| v.as_string());
                    if kind.as_deref() != Some("wordcraft:command") {
                        return;
                    }
                    match js_sys::JSON::stringify(&event.data()) {
                        Ok(json) => json.into(),
                        Err(_) => return,
                    }
                }
            };
            // The size cap runs before any parse: a huge message must not build a JSON tree.
            // The request id can't be read before the parse, so the refusal carries none.
            if crate::command::over_cap(&text) {
                let _ = bridge.post_result("", &Err("command message is over the size cap".to_owned()));
                return;
            }
            let Ok(data) = serde_json::from_str::<Value>(&text) else { return };
            if !crate::command::is_command(&data) {
                return;
            }
            match HostCommand::parse(&data) {
                Ok(cmd) => {
                    let mut pending = queue.borrow_mut();
                    if pending.len() >= MAX_PENDING {
                        drop(pending);
                        log::warn!("host command queue is full; refusing {id}", id = cmd.id);
                        let _ = bridge.post_result(&cmd.req_id, &Err("too many queued commands".to_owned()));
                        return;
                    }
                    pending.push(cmd);
                    drop(pending);
                    ctx.request_repaint();
                }
                Err(e) => {
                    log::warn!("{e}");
                    let _ = bridge.post_result(&crate::command::req_id_of(&data), &Err(e));
                }
            }
        });
        window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
        // The listener lives as long as the page.
        on_message.forget();
        Ok(())
    }

    /// Answer a host command: the request id echoed with the run's outcome. Same-origin only.
    pub fn post_result(&self, req_id: &str, outcome: &Result<Value, String>) -> Result<(), String> {
        let window = web_sys::window().ok_or("no window")?;
        let parent = window.parent().ok().flatten().ok_or("no parent window")?;
        if js_sys::Object::is(&parent, &window) {
            return Err("no parent window".to_owned());
        }
        let origin = window.location().origin().map_err(|e| format!("{e:?}"))?;
        if parent.location().origin().map_err(|e| format!("{e:?}"))? != origin {
            return Err("the parent is on another origin".to_owned());
        }
        let reply = serde_json::to_string(&crate::command::reply_message(req_id, outcome)).map_err(|e| format!("{e:?}"))?;
        let parsed = js_sys::JSON::parse(&reply).map_err(|e| format!("{e:?}"))?;
        parent.post_message(&parsed, &origin).map_err(|e| format!("{e:?}"))
    }
}

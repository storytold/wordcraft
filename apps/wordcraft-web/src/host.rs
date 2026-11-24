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
//! In both host modes the host sends the document to edit, so no sample opens, saves are handed
//! to the host rather than downloaded, the host guards leaving the page, and `?author=` names the
//! user for comments and tracked changes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;

use craft::host::{FileError, Files, PickId, PickRequest, Picked, WriteIntent};
use craft::platform::browser_files::BrowserFiles;
use craft::platform::open::{DropTarget, DroppedFile, IntakeSender};
use craft::web_host::web::{FileData, Host, Message};
use wordcraft_ui_egui::{Services, WordApp};

/// The app id in the host messages (`wordcraft:ready`).
const APP: &str = "wordcraft";

/// The name a document gets when the host sent none (the crate says `document` for a blank name).
const DEFAULT_NAME: &str = "Document.docx";

/// The largest document taken from the host page, refused before it is copied.
const MAX_OPEN_BYTES: u64 = 512 << 20;

/// The longest user name taken from `?author=`.
const MAX_AUTHOR_CHARS: usize = 200;

thread_local! {
    /// The page's bridge, for [`HostedFiles`]: `Files` is `Send + Sync` but the page host is not. The browser
    /// runs everything on one thread, so a save, called in the pass of the user's gesture, reaches it here.
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

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

    /// Report unsaved changes to the host page, hear the files it sends, and let [`HostedFiles`]
    /// hand it the saves.
    pub fn connect(&self, services: &mut Services, intake: IntakeSender) {
        self.listen(intake);
        BRIDGE.with(|b| *b.borrow_mut() = Some(self.clone()));
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
        let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
        self.host.post_save(&name, bytes, None, save_as)
    }

    /// Hear `wordcraft:open` and queue the file like a dropped one.
    fn listen(&self, intake: IntakeSender) {
        self.host.listen(move |message| {
            if let Message::Open { name, data } = message {
                deliver_open(&intake, &name, &data);
            }
        });
    }
}

/// The file an open message carries, queued for the next frame like a dropped file. A host's file
/// is untrusted input: an oversized one is refused before it is copied into the app's memory (the
/// data's own length, not the host's claim). The crate has cleaned the name (and cut it to 255
/// characters); a blank one comes as `document` and becomes `Document.docx`.
fn deliver_open(intake: &IntakeSender, name: &str, data: &FileData) {
    let name = if name == "document" { DEFAULT_NAME } else { name }.to_string();
    let FileData::Bytes(array) = data else {
        log::warn!("host file {name} came as a File; WordCraft reads bytes only; not opened");
        return;
    };
    if data.size() > MAX_OPEN_BYTES as f64 {
        log::warn!("host file is {} bytes, over the {} MiB host open limit; not opened", data.size(), MAX_OPEN_BYTES >> 20);
        return;
    }
    let file = DroppedFile { path: None, name, bytes: Some(Arc::from(array.to_vec())) };
    intake.dropped(vec![file], DropTarget::Elsewhere);
}

/// The files of a hosted page: reads go to the browser's files, but a save goes to the host page,
/// which stores it, so it answers at once.
pub struct HostedFiles {
    inner: Arc<BrowserFiles>,
    /// Save answers by the id the inner files issued for them.
    answers: Mutex<HashMap<u64, Picked>>,
}

impl HostedFiles {
    /// Hosted files over the browser's.
    pub fn new(inner: Arc<BrowserFiles>) -> Self {
        Self { inner, answers: Mutex::new(HashMap::new()) }
    }
}

impl Files for HostedFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        self.inner.pick(request)
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        if let Some(answer) = self.answers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id.0) {
            return Some(answer);
        }
        self.inner.take_answer(id)
    }

    fn forget(&self, id: PickId) {
        if self.answers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id.0).is_none() {
            self.inner.forget(id);
        }
    }

    fn pick_pending(&self) -> bool {
        self.inner.pick_pending()
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
        self.inner.read(path)
    }

    fn write(&self, path: &Path, bytes: &[u8], intent: WriteIntent) -> Result<bool, FileError> {
        self.inner.write(path, bytes, intent)
    }

    fn last_folder(&self, purpose: &str) -> Option<PathBuf> {
        self.inner.last_folder(purpose)
    }

    fn save(&self, request: PickRequest, bytes: Vec<u8>) -> PickId {
        let name = request.file_name.clone().unwrap_or_else(|| DEFAULT_NAME.to_string());
        let sent = BRIDGE.with(|b| match b.borrow().as_ref() {
            Some(bridge) => bridge.post_save(&name, &bytes, request.save_as),
            None => Err("no host page".to_string()),
        });
        let answer = match sent {
            // A Save As or first Save makes the page hold the document as this file: the answer
            // names it, and the next Save posts the name with `save_as` false.
            Ok(()) if request.save_as => Picked::Paths(vec![PathBuf::from(&name)]),
            // An overwrite of the opened file, or a copy: the document's file stays as it is.
            Ok(()) => Picked::Downloaded(name),
            Err(error) => Picked::Failed(error),
        };
        // The inner files own the id space: a Folder pick is answered `Unavailable` at once, so it
        // opens no picker and only yields an id, which is released and reused for this answer.
        let id = self.inner.pick(PickRequest::folder("hosted-save"));
        self.inner.forget(id);
        self.answers.lock().unwrap_or_else(|e| e.into_inner()).insert(id.0, answer);
        id
    }

    /// The host page opens documents, so no picker is offered.
    fn can_pick(&self) -> bool {
        false
    }

    fn browser_saves(&self) -> bool {
        true
    }
}

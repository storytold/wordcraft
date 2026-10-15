//! WordCraft's half of Open and Save against cloud storage.
//!
//! The pages themselves are `craft_ui::cloud`. Every Craft app offers the same ones. What is here
//! is what only WordCraft can answer. A document is a document. These extensions and these content
//! types are what that means, and they are the ones the engine already opens and the ones the
//! desktop entry already claims. Bytes become a document through `file.open` and go back through
//! `save_bytes_with`. The words are the ones `i18n` has.
//!
//! WordCraft holds no provider credential and speaks no provider protocol. Every call goes to the
//! CollabCraft service, which is the single place a provider is configured and permissions are
//! decided. Replacing Nextcloud with something else changes nothing here.
//!
//! Nothing in this file opens a socket either. The transport half is a [`CloudBackend`] the host
//! application supplies (see `apps/wordcraft/src/cloud.rs`). Every CollabCraft call blocks. A
//! blocking read of a forty megabyte document inside a frame would freeze the window for seconds.
//! So the pages state a [`Job`], the host runs it on a worker thread, and the answer arrives on a
//! channel that [`poll`] drains on a later frame.

use std::collections::HashMap;

// Re-exported so a host application, and a test, can name these types without depending on the
// crates they come from.
pub use craft_cloud::{Capability, Entry, EntryKind};
pub use craft_ui::cloud::{
    Account, Busy, CloudApp, CloudBackend, CloudFiles, Conflict, CopyFormat, Destination, DocumentKind, Event, Fault, Folder, Job, Link, Origin,
    Post, Source, UNAVAILABLE, available, browse, can_converse, conflict_modal, destination_name, has_target, link, load_shared, open_entry,
    open_page, origin, parent_of, prefers_cloud, progress, pump, reload_active, save, save_as, save_page, share_button, shares, sign_in, sign_out,
    start, who, work_together,
};
use egui::{Color32, Painter, Rect, Ui};
use serde_json::{Value as Json, json};

use crate::theme::{self, Tokens};
use crate::{WordApp, icons};

/// Content types WordCraft opens, matched against the start of what the provider said.
///
/// Read beside the extension because neither is reliable alone. A WebDAV server hands back
/// `application/octet-stream` for a perfectly good `.docx`, and a file exported by a web
/// application often has a correct content type and no extension at all.
///
/// Each is a prefix, so one entry covers a family: `application/vnd.ms-word` is both the
/// macro-enabled document and the macro-enabled template. The set is the one the Linux desktop
/// entry claims (`packaging/linux/ai.storyteller.wordcraft.desktop`), which is this application's
/// own published answer to what it opens.
const MIMES: [&str; 10] = [
    "application/vnd.openxmlformats-officedocument.wordprocessingml",
    "application/vnd.ms-word",
    "application/msword",
    "application/vnd.oasis.opendocument.text",
    "application/rtf",
    "text/rtf",
    "text/markdown",
    "text/x-tex",
    "text/html",
    "text/plain",
];

/// What a WordCraft document is, as a cloud file list needs to know.
///
/// The extensions are the engine's own list of the formats it opens, so a format WordCraft learns
/// is offered here without a second list to remember.
pub const DOCUMENT: DocumentKind =
    DocumentKind { extensions: wordcraft_engine::io::OPEN_EXTS, mimes: &MIMES, noun: "document", default_extension: "docx" };

/// The Save a Copy formats the Share menu offers under the share itself.
///
/// Three of the ten File › Export writes, which are the three a person shares: the one anybody
/// can read, the one Word reads, and the one a browser reads. Export has the rest.
const COPIES: [(&str, &str); 3] =
    [("pdf", "Save a Copy as PDF (.pdf)…"), ("docx", "Save a Copy as Word Document (.docx)…"), ("html", "Export as Web Page (.html)…")];

/// What each save in flight encoded: the document revision, and the file name whose format the
/// bytes are in.
///
/// Only WordCraft knows what a version of its document is. The pages hold none of these. A save
/// that succeeds clears the unsaved mark for exactly the revision that reached the server. An
/// edit made while the bytes were on the wire stays unsaved.
pub type Sent = HashMap<u64, (u64, String)>;

/// A cloud open a prompt stood in the way of.
///
/// Opening a document replaces the one in the window, so unsaved changes are asked about first,
/// and a password-protected file asks for its password. Either way the prompt carries the bytes
/// on and the document opens frames later, after [`CloudApp::open_bytes`] has already answered.
pub struct Opening {
    /// The file name the bytes arrived under.
    name: String,
    /// The document that was open when the prompt went up. A different one means the prompt was
    /// answered and the bytes are now a document.
    was: u64,
    /// The entry, when it was already known: a reload reads the document's own origin, and that
    /// origin is dropped as soon as the read is answered.
    entry: Option<Entry>,
}

/// Whether WordCraft can open this entry.
pub fn opens(entry: &Entry) -> bool {
    craft_ui::cloud::opens(entry, DOCUMENT)
}

/// A typed name as a document file name. Any folder separators are dropped and a missing or
/// unknown extension becomes `.docx`.
pub fn document_name(typed: &str) -> String {
    craft_ui::cloud::document_name(typed, DOCUMENT)
}

/// The whole of the cloud state, for the control channel and for tests.
pub fn status(app: &WordApp) -> Json {
    craft_ui::cloud::report(app)
}

/// The Share menu's export half, as the control it is drawn in wants it.
pub fn copies() -> Vec<CopyFormat> {
    COPIES.iter().map(|(id, label)| CopyFormat::new(*id, *label)).collect()
}

/// The cloud commands, for the control channel's own listing (as `printing::commands` does).
pub fn commands(app: &WordApp) -> Vec<Json> {
    let enabled = available(app);
    [
        ("cloud.open", "Open from the Cloud", "{} opens File › Open and lists your cloud files"),
        ("cloud.browse", "Browse a Cloud Folder", "{path?} (the root when empty) → the folder being listed"),
        ("cloud.shared", "List Shared With Me", "{}"),
        ("cloud.openFile", "Open a Cloud File", "{id} | {name}, from the current listing"),
        ("cloud.save", "Save to the Cloud", "{} writes back under the version the document was opened at"),
        ("cloud.saveAs", "Save to the Cloud As", "{folder, name} saves at once; without them, opens File › Save As on the cloud"),
        ("cloud.reload", "Reload from the Cloud", "{} replaces local edits with the stored copy"),
        ("cloud.link", "Link a Cloud Account", "{}"),
        ("cloud.status", "Cloud Status", "{} → account, capabilities, folder, transfer in flight, conflict, origin"),
    ]
    .into_iter()
    .map(|(id, label, params)| json!({"id": id, "label": label, "location": "File › Cloud", "shortcut": null, "params": params, "enabled": enabled}))
    .collect()
}

/// The host part of the service address, for the account menu's first line.
///
/// Read from the environment and compiled in nowhere, because the service is
/// deployment-specific. Somebody with an account on two deployments needs to know which one a
/// document is about to land in.
fn service_host() -> String {
    ["WORDCRAFT_CLOUD_URL", "CRAFT_CLOUD_URL", "COLLABCRAFT_URL"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .find(|value| !value.is_empty())
        .map(|value| value.trim_start_matches("https://").trim_start_matches("http://").to_owned())
        .unwrap_or_default()
}

/// Answers from the cloud worker, and anything still in flight. Called once a frame.
pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    pump(app, ctx);
    finish_open(app);
}

/// The document in the window came from the cloud.
///
/// It has no file on this computer. `file.open` names the format by the file name, so the bytes
/// arrive under one. Leaving it as the document's path would point Save, AutoSave and the window
/// caption at a relative file in whatever folder the application happens to be running in.
///
/// The File view goes away too, as it does for every other way of opening something: a document
/// that opened wants to be read.
fn from_the_cloud(app: &mut WordApp) {
    app.session.path = None;
    app.ui.backstage = false;
    app.canvas.want_focus = true;
}

/// Give a deferred cloud open the entry it came from.
///
/// Without this a document that opened through a prompt would have no origin, and Save would have
/// nowhere to go. The entry is the one the reload already held, or the row still in the listing
/// the user picked it from. A document that arrives with neither keeps no origin, and Save then
/// asks where to put it, as it does for a document that has never been saved.
fn finish_open(app: &mut WordApp) {
    let Some(opening) = &app.cloud_opening else { return };
    if app.session.document_id() == opening.was {
        // Still waiting. A prompt answered Cancel, or a password nobody typed, ends it: the bytes
        // are gone and nothing is going to open.
        if app.dialog.is_none() && !app.file_dialog_open() {
            app.cloud_opening = None;
        }
        return;
    }
    let Some(opening) = app.cloud_opening.take() else { return };
    let found = opening.entry.or_else(|| {
        let entries = app.cloud.mine.entries.iter().chain(app.cloud.shared.entries.iter());
        entries.filter(|e| e.name == opening.name).find(|e| opens(e)).cloned()
    });
    let Some(entry) = found else { return };
    from_the_cloud(app);
    let uid = app.session.document_id();
    app.cloud.origins.insert(uid, Origin { entry });
}

impl CloudApp for WordApp {
    fn cloud(&self) -> &CloudFiles {
        &self.cloud
    }

    fn cloud_mut(&mut self) -> &mut CloudFiles {
        &mut self.cloud
    }

    fn kind(&self) -> DocumentKind {
        DOCUMENT
    }

    /// Always a document: a WordCraft window holds exactly one, and a blank one counts.
    fn active(&self) -> Option<u64> {
        Some(self.session.document_id())
    }

    fn title(&self, uid: u64) -> String {
        if uid == self.session.document_id() { self.title_stem() } else { String::new() }
    }

    fn open_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<Option<u64>, String> {
        let was = self.session.document_id();
        // The entry the open document already came from, when these bytes are for that entry.
        // Reading it again is a reload. Both doors to one, the conflict modal's Reload and
        // `cloud.reload`, say that local edits are replaced, so asking again would be asking for
        // consent already given. Picking the open document out of the Open page is the same
        // request, and it is answered the same way.
        let reload = self.cloud.origins.get(&was).map(|o| o.entry.clone()).filter(|e| e.name == name);
        let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
        let open = json!({"path": name, "data": data});
        // Anything else replaces one document with another, so it goes through `run`: unsaved
        // changes are asked about rather than thrown away, and a password-protected file asks for
        // its password. Both answer with `pending`, and both carry these same parameters on,
        // bytes included.
        let answer = match reload.is_some() {
            true => self.execute_user("file.open", open)?,
            false => self.run("file.open", open)?,
        };
        if answer.get("pending").is_some() {
            self.cloud_opening = Some(Opening { name: name.to_owned(), was, entry: reload });
            return Ok(None);
        }
        from_the_cloud(self);
        Ok(Some(self.session.document_id()))
    }

    fn encode(&mut self, uid: u64, into: Destination<'_>) -> Result<Vec<u8>, String> {
        if uid != self.session.document_id() {
            return Err("that document is no longer open".to_owned());
        }
        let name = into.name.to_owned();
        // A save is a save, cloud or not: the modified stamp and the revision advance, as they do
        // for a save to a file and for the browser's download.
        self.session.stamp_save();
        let password = self.session.password.as_ref().map(wordcraft_engine::Password::as_str);
        let options = wordcraft_engine::cmd::file::export_options(&self.session, &json!({}));
        let bytes = wordcraft_engine::io::save_bytes_with(&name, &self.session.doc, password, &options)?;
        self.cloud_sent.insert(uid, (self.session.rev(), name));
        Ok(bytes)
    }

    /// Nothing to close. A WordCraft window holds one document, and the replacement a reload read
    /// has already taken its place by the time this is called.
    fn close(&mut self, _uid: u64) {}

    fn mark_saved(&mut self, uid: u64) {
        let Some((revision, name)) = self.cloud_sent.remove(&uid) else { return };
        // An edit made while the bytes were on the wire is not in them, so it stays unsaved. So
        // does everything a format cannot hold: a copy written as plain text or a page image
        // leaves the document exactly as unsaved as it was.
        if uid == self.session.document_id() && revision == self.session.rev() && crate::keeps_everything(&name) {
            self.session.dirty = false;
        }
    }

    fn drop_pending(&mut self, uid: u64) {
        self.cloud_sent.remove(&uid);
    }

    fn sharing(&mut self) -> &mut craft_ui::share::ShareDialog {
        &mut self.share
    }

    fn presence(&mut self) -> &mut craft_ui::presence::PresenceBar {
        &mut self.presence
    }

    fn paint_icon(&self, painter: &Painter, rect: Rect, kind: EntryKind, colour: Color32) {
        let icon = if kind == EntryKind::Folder { "folder" } else { "file" };
        icons::paint(painter, rect, icon, colour, colour);
    }

    fn open_url(&self, url: &str) {
        match &self.services.open_url {
            Some(open) => open(url),
            // What the page canvas does with a hyperlink, and what the web shell wants.
            None => {
                if let Some(ctx) = &self.ctx {
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
            }
        }
    }

    /// The status bar, which is where every other command failure is reported too.
    fn say(&mut self, text: String) {
        self.status(text);
    }

    fn toast(&mut self, text: String) {
        self.status(text);
    }

    fn go_to_save_as(&mut self) {
        self.ui.backstage = true;
        self.ui.backstage_page = "saveAs".into();
        self.ui.save_to_cloud = Some(true);
    }

    fn app_name(&self) -> &str {
        "WordCraft"
    }

    fn tr(&self, text: &str) -> String {
        crate::i18n::t(text).to_owned()
    }

    fn service(&self) -> String {
        service_host()
    }

    fn save_note(&self) -> Option<String> {
        Some(self.tr("A name with no extension is saved as a Word document (.docx) in the folder above."))
    }

    fn style(&self, ui: &Ui) -> craft_ui::cloud::CloudStyle {
        let t = Tokens::get(ui.ctx());
        craft_ui::cloud::CloudStyle {
            surface: t.panel,
            border: t.border,
            text: t.text,
            dim: t.text_dim,
            hover: t.hover,
            soft: t.checked,
            family: theme::ui_family(),
            bold: theme::ui_bold_family(),
        }
    }
}

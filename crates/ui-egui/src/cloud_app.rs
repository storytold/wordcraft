//! WordCraft's side of the real `craft_ui::cloud` contract.
//!
//! No backend is connected in this build: nothing here calls
//! [`craft_ui::cloud::CloudFiles::connect`], so
//! [`craft_ui::cloud::available`] stays false and every cloud page, menu item and account badge
//! this module backs stays hidden. What this file supplies is the application half of the
//! contract (`CloudApp`), so a storytold-hosted backend connects later with no change here: only
//! a `cloud.connect(backend)` call at start-up.

use egui::{Color32, Painter, Rect};

use craft_cloud::EntryKind;
use craft_ui::cloud::{CloudApp, CopyFormat, Destination, DocumentKind};

use crate::{WordApp, file_dialogs, icons, theme};

/// What a WordCraft document is, for the Open and Save As pages.
const KIND: DocumentKind = DocumentKind {
    extensions: file_dialogs::DOCUMENTS,
    mimes: &["application/vnd.openxmlformats-officedocument.wordprocessingml.document", "application/msword"],
    noun: "document",
    default_extension: "docx",
};

/// WordCraft's own Save a Copy formats, for the second half of the Share split button. The same
/// list File › Export offers.
pub(crate) fn copies() -> Vec<CopyFormat> {
    crate::print_page::EXPORT_FORMATS.iter().map(|&(label, ext)| CopyFormat::new(ext, crate::tl!(label))).collect()
}

/// Metadata for the `cloud.*` ids, for `engine.commands` ([`crate::printing::commands`] does the
/// same for `ui.print*`). Without this an agent asking the control channel what it can call would
/// never learn these ids exist to try.
pub(crate) fn commands(app: &WordApp) -> Vec<serde_json::Value> {
    use serde_json::json;
    let available = craft_ui::cloud::available(app);
    let has_target = craft_ui::cloud::has_target(app);
    let can_reload = available && craft_ui::cloud::origin(app).is_some();
    vec![
        json!({"id": "cloud.status", "label": "Cloud status", "location": "File", "shortcut": null,
            "params": "none. Reports the whole of the cloud state: whether a service is configured, the signed-in account, the folder listing and the active document's cloud origin. Never fails.",
            "enabled": true}),
        json!({"id": "cloud.open", "label": "Open from the cloud", "location": "File › Open", "shortcut": null,
            "params": "none. Shows the Open page's Cloud section and starts listing the account's root folder.", "enabled": available}),
        json!({"id": "cloud.browse", "label": "Browse a cloud folder", "location": "File › Open", "shortcut": null,
            "params": "path?: string, empty or omitted for the root.", "enabled": available}),
        json!({"id": "cloud.shared", "label": "List shared with me", "location": "File › Open", "shortcut": null,
            "params": "none.", "enabled": available}),
        json!({"id": "cloud.openFile", "label": "Open a cloud file", "location": "File › Open", "shortcut": null,
            "params": "id?: string; name?: string. One of the two must name an entry already listed by cloud.browse or cloud.shared.", "enabled": available}),
        json!({"id": "cloud.save", "label": "Save to the cloud", "location": "File › Save", "shortcut": null,
            "params": "none. Only for a document that already has a writable cloud origin.", "enabled": has_target}),
        json!({"id": "cloud.saveAs", "label": "Save to the cloud as", "location": "File › Save As", "shortcut": null,
            "params": "folder: string; name: string. Omit both to go to the Save As page's cloud destination instead of saving.", "enabled": available}),
        json!({"id": "cloud.reload", "label": "Reload from the cloud", "location": "File", "shortcut": null,
            "params": "none. Only for the active document, and only when it has a cloud origin.", "enabled": can_reload}),
    ]
}

impl CloudApp for WordApp {
    fn cloud(&self) -> &craft_ui::cloud::CloudFiles {
        &self.cloud
    }

    fn cloud_mut(&mut self) -> &mut craft_ui::cloud::CloudFiles {
        &mut self.cloud
    }

    fn kind(&self) -> DocumentKind {
        KIND
    }

    fn active(&self) -> Option<u64> {
        self.has_document().then(|| self.session.document_id())
    }

    fn title(&self, uid: u64) -> String {
        self.session.documents().into_iter().find(|d| d.id == uid).map(|d| d.title).unwrap_or_default()
    }

    fn open_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<Option<u64>, String> {
        let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
        let result = self.open_from_platform(name, Some(&data))?;
        let uid = result.get("id").and_then(serde_json::Value::as_u64);
        // `file.open` always stamps the name it was given as a local path (it has no other way
        // to know this one is a cloud entry's display name, not a real file on this computer).
        // Left alone, the document would read as having a local file called "notes.docx" sitting
        // in whatever the working directory happens to be, on top of the real cloud origin
        // `craft_ui::cloud` already records for it.
        if let Some(uid) = uid {
            let _ = self.session.with_document(uid, |s| s.path = None);
        }
        Ok(uid)
    }

    fn encode(&mut self, uid: u64, into: Destination<'_>) -> Result<Vec<u8>, String> {
        let name = if into.extension().is_empty() { format!("{}.{}", into.name, KIND.default_extension) } else { into.name.to_owned() };
        match self.session.with_document(uid, |s| wordcraft_engine::io::save_bytes(&name, &s.doc)) {
            Ok(encoded) => encoded,
            Err(e) => Err(e.to_string()),
        }
    }

    fn close(&mut self, uid: u64) {
        let _ = self.session.close_document(uid);
    }

    /// The bytes [`CloudApp::encode`] made reached the service: this version is saved, the same
    /// as a local `file.save` clearing [`Session::dirty`](wordcraft_engine::Session) for its own
    /// document. Without this a document saved to the cloud would read as unsaved forever, and
    /// quitting or closing it would ask to save work that is already there.
    fn mark_saved(&mut self, uid: u64) {
        let _ = self.session.with_document(uid, |s| s.dirty = false);
    }

    fn sharing(&mut self) -> &mut craft_ui::share::ShareDialog {
        &mut self.share
    }

    fn presence(&mut self) -> &mut craft_ui::presence::PresenceBar {
        &mut self.presence
    }

    fn paint_icon(&self, painter: &Painter, rect: Rect, kind: EntryKind, colour: Color32) {
        let name = match kind {
            EntryKind::File => "file",
            EntryKind::Folder => "folder",
        };
        icons::paint(painter, rect, name, colour, colour);
    }

    fn open_url(&self, url: &str) {
        if let Some(ctx) = &self.ctx {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    fn say(&mut self, text: String) {
        self.status_long(text);
    }

    fn toast(&mut self, text: String) {
        self.status(text);
    }

    fn go_to_save_as(&mut self) {
        self.ui.backstage = true;
        self.ui.backstage_page = "saveAs".into();
    }

    fn app_name(&self) -> &str {
        "WordCraft"
    }

    fn tr(&self, text: &str) -> String {
        crate::i18n::t(text).to_owned()
    }

    fn style(&self, ui: &egui::Ui) -> craft_ui::cloud::CloudStyle {
        let t = theme::Tokens::get(ui.ctx());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_a_copy_formats_are_not_empty_and_each_has_an_id_and_a_label() {
        let formats = copies();
        assert!(!formats.is_empty());
        for format in &formats {
            assert!(!format.id.is_empty());
            assert!(!format.label.is_empty());
        }
    }

    /// Every `cloud.*` id `app/commands.rs` dispatches is listed here too, so `engine.commands`
    /// (the control channel's own listing) names it: an id nothing lists is one an agent could
    /// never learn to try.
    #[test]
    fn every_dispatched_cloud_id_is_listed_in_commands() {
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let listing = commands(&app);
        let listed: Vec<&str> = listing.iter().filter_map(|c| c["id"].as_str()).collect();
        for id in ["cloud.status", "cloud.open", "cloud.browse", "cloud.shared", "cloud.openFile", "cloud.save", "cloud.saveAs", "cloud.reload"] {
            assert!(listed.contains(&id), "{id} is dispatched but not listed: {listed:?}");
        }
        // With no backend connected, nothing here is actionable except the status report, which
        // never needs one.
        for command in &listing {
            let enabled = command["enabled"] == serde_json::Value::Bool(true);
            assert_eq!(enabled, command["id"] == "cloud.status", "{command}");
        }
    }
}

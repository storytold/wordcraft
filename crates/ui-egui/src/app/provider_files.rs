//! The optional transport never opens a dialog from engine/control-channel commands.
//! This layer owns document identity, lossless serialization and durable-save admission.

use crate::file_providers::{Capture, Command, Host};
use crate::{WordApp, json};

impl WordApp {
    /// UI Save/Save As only. Exports, automation and ordinary local saves keep their route.
    pub(crate) fn choose_provider_save(&mut self, save_as: bool) -> bool {
        let Some(host) = self.services.file_providers.clone().filter(|host| host.enabled()) else { return false };
        if !self.has_document() {
            return true;
        }
        let doc = self.session.document_id();
        let name = host.filename(doc).unwrap_or_else(|| self.default_save_name());
        if let Err(error) = host.choose(doc, self.session.rev(), name, save_as) {
            self.status(error);
        }
        true
    }

    pub(crate) fn provider_tick(&mut self) {
        let Some(host) = self.services.file_providers.clone() else { return };
        for command in host.take_commands() {
            match command {
                command @ Command::Save { .. } => self.provider_save(&host, command),
                command @ Command::Result { .. } => self.provider_result(&host, command),
                command @ Command::Open { .. } => self.provider_open(&host, command),
            }
        }
        let documents = self.session.documents();
        let ids: Vec<u64> = documents.iter().map(|doc| doc.id).collect();
        let snapshot = documents
            .iter()
            .map(|doc| {
                json!({"id":doc.id.to_string(), "title":doc.title,
            "dirty":doc.dirty, "provider":host.bound(doc.id), "pending":host.pending_for(doc.id)})
            })
            .collect::<Vec<_>>();
        host.publish_documents(json!(snapshot), &ids);
    }
    fn provider_save(&mut self, host: &Host, command: Command) {
        let Command::Save { request, doc, destination, name, save_as } = command else { return };
        let encoded = self
            .session
            .with_document(doc, |session| {
                let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                if !wordcraft_engine::io::is_word_package(&ext) {
                    return Err("Provider Save requires a Word package (.docx, .docm, .dotx, .dotm). Use Export for other formats.".to_string());
                }
                // The same serializer as ordinary Save, including embedded pictures,
                // comments, headers, macros and the document's existing encryption.
                session.stamp_save();
                let options = wordcraft_engine::cmd::file::export_options(session, &json!({}));
                let password = session.password.as_ref().map(wordcraft_engine::Password::as_str);
                let bytes = wordcraft_engine::io::save_bytes_with(&name, &session.doc, password, &options)?;
                Ok((session.rev(), bytes))
            })
            .map_err(|e| e.to_string())
            .and_then(|result| result);
        match encoded {
            Ok((revision, bytes)) if destination == "provider" => {
                let capture = Capture { doc, revision, name, save_as };
                if let Err(error) = host.capture(&request, capture, bytes) {
                    host.failed(&request, &error);
                    self.status(error);
                }
            }
            Ok((revision, bytes)) => {
                let result = self
                    .services
                    .provider_download
                    .as_ref()
                    .ok_or_else(|| "No device download service is available".to_string())
                    .and_then(|download| download(&name, &bytes));
                match result {
                    Ok(()) => {
                        // A device copy must not assert that the provider now has it.
                        if !host.bound(doc) {
                            let _ = self.session.with_document(doc, |session| {
                                if session.rev() == revision {
                                    session.dirty = false;
                                }
                            });
                        }
                        host.finish_device(&request);
                        self.status(format!("Downloaded {name}"));
                    }
                    Err(error) => {
                        host.failed(&request, &error);
                        self.status(error);
                    }
                }
            }
            Err(error) => {
                host.failed(&request, &error);
                self.status(error);
            }
        }
    }
    fn provider_result(&mut self, host: &Host, command: Command) {
        let Command::Result { capture, resource, status } = command else { return };
        if status == "saved" && self.session.documents().iter().any(|d| d.id == capture.doc) {
            let aliases = host.bind(&capture, resource);
            for alias in aliases {
                let _ = self.session.with_document(alias, |session| {
                    session.dirty = true;
                });
            }
            let _ = self.session.with_document(capture.doc, |session| {
                if session.rev() == capture.revision {
                    session.dirty = false;
                    if session.path.is_none() {
                        session.file_name = Some(capture.name.clone());
                    }
                }
            });
            self.status(format!("Saved {} to provider", capture.name));
        } else if status == "failed" {
            self.status("Provider save failed. Your changes are still open.");
        }
    }
    fn provider_open(&mut self, host: &Host, command: Command) {
        let Command::Open { request, resource: _, name, bytes } = command else { return };
        let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
        // `data` forces an import, never a lookup by a homonymous device path.
        let result = self.execute("file.open", json!({"path":name, "data":data}));
        match result {
            Ok(_) => {
                self.session.path = None;
                self.session.file_name = Some(name.clone());
                self.no_document = None;
                self.autosave_path = None;
                self.ui.backstage = false;
                self.canvas.want_focus = true;
                let capture = Capture { doc: self.session.document_id(), revision: self.session.rev(), name, save_as: false };
                host.opened(&request, Some(capture), "");
            }
            Err(error) => {
                // An encrypted file without a password is a failed read, not an
                // admission of the previous active document under this resource ID.
                host.opened(&request, None, &error);
                self.status(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::file_providers::Host;
    use crate::{Services, WordApp, json};

    fn app() -> (WordApp, Host) {
        let host = Host::default();
        host.set_session("0123456789abcdef0123456789abcdef".into()).unwrap();
        host.connect("test".into(), "Test storage".into(), "account".into()).unwrap();
        let services = Services { file_providers: Some(host.clone()), ..Default::default() };
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        app.session.run("text.insert", &json!({"text":"Documento con ñ, formato y contenido"})).unwrap();
        (app, host)
    }

    fn save(app: &mut WordApp, host: &Host) -> (String, Vec<u8>) {
        assert!(app.choose_provider_save(false));
        let dialog = host.take_events().remove(0);
        let request = dialog.metadata["requestId"].as_str().unwrap().to_string();
        host.resolve_dialog(&request, "provider", "Document.docx").unwrap();
        app.provider_tick();
        let event = host.take_events().into_iter().find(|e| e.metadata["type"] == "save").unwrap();
        (request, event.bytes.unwrap())
    }

    #[test]
    fn durable_save_and_reopen_use_native_docx_and_separate_recents() {
        let (mut app, host) = app();
        let (request, bytes) = save(&mut app, &host);
        assert!(app.session.dirty, "producing DOCX bytes is not a durable save");
        host.result(request, "saved".into(), "resource-one".into()).unwrap();
        app.provider_tick();
        assert!(!app.session.dirty);
        assert!(app.session.path.is_none(), "a provider ID is never a device path");
        assert!(app.ui.recent.is_empty());
        assert_eq!(host.snapshot()["resources"][0]["name"], "Document.docx");
        let open = host.request_open("resource-one").unwrap();
        host.open(open, "Document.docx".into(), bytes).unwrap();
        app.provider_tick();
        assert!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Documento con ñ"));
        assert!(app.has_document());
        assert!(app.ui.recent.is_empty());
    }

    #[test]
    fn later_edit_or_failed_provider_result_cannot_clear_dirty_state() {
        let (mut app, host) = app();
        let (request, _) = save(&mut app, &host);
        app.session.run("text.insert", &json!({"text":" edición posterior"})).unwrap();
        host.result(request, "saved".into(), "resource-one".into()).unwrap();
        app.provider_tick();
        assert!(app.session.dirty);
        let (request, _) = save(&mut app, &host);
        host.result(request, "failed".into(), String::new()).unwrap();
        app.provider_tick();
        assert!(app.session.dirty);
    }
}

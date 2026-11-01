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
            "dirty":doc.dirty||self.has_comment_drafts_in(doc.id), "provider":host.bound(doc.id), "pending":host.pending_for(doc.id)})
            })
            .collect::<Vec<_>>();
        host.publish_documents(json!(snapshot), &ids);
    }
    fn provider_save(&mut self, host: &Host, command: Command) {
        let Command::Save { request, doc, destination, name, save_as } = command else { return };
        if self.session.document_id() == doc {
            if let Err(error) = crate::panes::flush_comment_drafts(self) {
                host.failed(&request, &error);
                self.status(error);
                return;
            }
        } else if self.has_comment_drafts_in(doc) {
            let error = "Activate the document and finish its comment drafts before saving it";
            host.failed(&request, error);
            self.status(error);
            return;
        }
        let encoded = self
            .session
            .with_document(doc, |session| {
                let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                if !wordcraft_engine::io::is_word_package(&ext) {
                    return Err("Provider Save requires a Word package (.docx, .docm, .dotx, .dotm). Use Export for other formats.".to_string());
                }
                if destination == "provider"
                    && matches!(ext.as_str(), "docx" | "dotx")
                    && session.doc.passthrough.get("word/vbaProject.bin").is_some_and(|bytes| !bytes.is_empty())
                {
                    return Err("This document contains VBA. Choose .docm or .dotm to preserve it.".into());
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
            Ok((_, bytes)) => {
                let result = self
                    .services
                    .provider_download
                    .as_ref()
                    .ok_or_else(|| "No device download service is available".to_string())
                    .and_then(|download| download(&name, &bytes));
                match result {
                    Ok(()) => {
                        // The browser may still cancel or fail after accepting these bytes.
                        // Neither a provider binding nor a device checkpoint is acknowledged.
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
                    // Invalidate captures created before another tab overwrote this resource.
                    session.touch();
                });
            }
            let drafts = self.has_comment_drafts_in(capture.doc);
            let _ = self.session.with_document(capture.doc, |session| {
                if session.rev() == capture.revision && !drafts {
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
        let Command::Open { request, name, bytes } = command else { return };
        let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
        // `data` forces an import, never a lookup by a homonymous device path.
        // Ordinary imports record their path in device history. This name identifies
        // provider bytes, so retain the complete device list even when decoding fails.
        let recent = self.ui.recent.clone();
        let result = self.execute("file.open", json!({"path":name, "data":data}));
        self.ui.recent = recent;
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
        let dialog = host.take_events().into_iter().find(|event| event.metadata["type"] == "saveDialog").unwrap();
        let request = dialog.metadata["requestId"].as_str().unwrap().to_string();
        host.resolve_dialog(&request, "provider", "Document.docx").unwrap();
        app.provider_tick();
        let event = host.take_events().into_iter().find(|e| e.metadata["type"] == "save").unwrap();
        (request, event.bytes.unwrap())
    }

    #[test]
    fn provider_enabled_export_and_template_dialogs_write_the_picked_local_file() {
        for (extension, template) in [("docx", false), ("dotx", true)] {
            let directory = std::env::temp_dir().join(format!("wordcraft-provider-local-{extension}-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join(format!("Local copy.{extension}"));
            let picked = path.to_string_lossy().into_owned();
            let (mut app, host) = app();
            app.services.pick_save = Some(Box::new(move |_| Some(picked.clone())));
            app.services.templates_dir = Some(directory.clone());
            if template {
                let _ = app.save_as_template_dialog();
            } else {
                app.export_dialog(extension);
            }
            let restored = wordcraft_engine::io::open_path(&path).unwrap();
            assert!(restored.plain_text(wordcraft_doc::StoryRef::Body).contains("Documento con ñ"));
            assert!(!app.session.dirty, "an unbound native local save keeps its original checkpoint behavior");
            assert!(!host.busy(), "explicit local paths must not open a provider chooser");
            assert!(host.take_events().is_empty());
            std::fs::remove_file(&path).unwrap();
            std::fs::remove_dir(&directory).unwrap();
        }
    }

    #[test]
    fn bound_explicit_export_and_template_copies_keep_provider_changes_dirty_until_ack() {
        for (extension, template) in [("docx", false), ("dotx", true)] {
            let directory = std::env::temp_dir().join(format!("wordcraft-provider-bound-{extension}-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join(format!("Local copy.{extension}"));
            let picked = path.to_string_lossy().into_owned();
            let (mut app, host) = app();
            let document = app.session.document_id();
            let (request, _) = save(&mut app, &host);
            host.result(request, "saved".into(), "resource-one".into()).unwrap();
            app.provider_tick();
            assert!(!app.session.dirty);
            assert!(host.bound(document));
            app.session.run("text.insert", &json!({"text":" New content only on this device"})).unwrap();
            assert!(app.session.dirty);
            app.services.pick_save = Some(Box::new(move |_| Some(picked.clone())));
            app.services.templates_dir = Some(directory.clone());
            if template {
                let _ = app.save_as_template_dialog();
            } else {
                app.export_dialog(extension);
            }
            let restored = wordcraft_engine::io::open_path(&path).unwrap();
            assert!(restored.plain_text(wordcraft_doc::StoryRef::Body).contains("New content only on this device"));
            assert!(host.bound(document), "a local export preserves its provider binding");
            assert_eq!(host.filename(document).as_deref(), Some("Document.docx"));
            assert!(app.session.dirty, "the provider has not acknowledged the edited content");
            assert!(!app.saved_here(), "a provider-bound export does not arm local AutoSave");
            app.autosave_tick(10_000.0);
            assert!(app.session.dirty);
            app.provider_tick();
            assert_eq!(host.snapshot()["documents"][0]["dirty"], true);
            assert!(!host.busy());
            let (request, bytes) = save(&mut app, &host);
            let restored = wordcraft_engine::io::open_bytes("Document.docx", &bytes).unwrap();
            assert!(restored.plain_text(wordcraft_doc::StoryRef::Body).contains("New content only on this device"));
            assert!(app.session.dirty);
            host.result(request, "saved".into(), "resource-one".into()).unwrap();
            app.provider_tick();
            assert!(!app.session.dirty, "only the correlated provider acknowledgement clears its checkpoint");
            std::fs::remove_file(&path).unwrap();
            std::fs::remove_dir(&directory).unwrap();
        }
    }

    #[test]
    fn bound_explicit_browser_copy_does_not_acknowledge_provider_persistence() {
        let (mut app, host) = app();
        let document = app.session.document_id();
        let (request, _) = save(&mut app, &host);
        host.result(request, "saved".into(), "resource-one".into()).unwrap();
        app.provider_tick();
        assert!(!app.session.dirty);
        app.session.run("text.insert", &json!({"text":" Browser copy after cloud save"})).unwrap();
        let copies = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = copies.clone();
        app.services.download = Some(Box::new(move |name, bytes| {
            received.borrow_mut().push((name.to_string(), bytes.to_vec()));
            Ok(())
        }));
        app.run("file.saveAs", json!({"path":"Browser copy.docx"})).unwrap();
        let copies = copies.borrow();
        assert_eq!(copies.len(), 1);
        let restored = wordcraft_engine::io::open_bytes(&copies[0].0, &copies[0].1).unwrap();
        assert!(restored.plain_text(wordcraft_doc::StoryRef::Body).contains("Browser copy after cloud save"));
        assert!(host.bound(document));
        assert!(app.session.dirty);
        app.provider_tick();
        assert_eq!(host.snapshot()["documents"][0]["dirty"], true);
        assert!(!host.busy());
        drop(copies);
        let (request, _) = save(&mut app, &host);
        host.result(request, "saved".into(), "resource-one".into()).unwrap();
        app.provider_tick();
        assert!(!app.session.dirty);
    }

    #[test]
    fn disabled_provider_owned_browser_copies_and_async_outcomes_keep_the_provider_checkpoint_dirty() {
        use crate::async_saves::{AsyncSaves, SaveOutcome};

        for initially_bound in [false, true] {
            for command in ["file.save", "file.saveAs"] {
                for outcome in [Some(SaveOutcome::Saved), Some(SaveOutcome::Cancelled), Some(SaveOutcome::Failed("write failed".into())), None] {
                    let (mut app, host) = app();
                    let document = app.session.document_id();
                    let (request, _) = save(&mut app, &host);
                    if initially_bound {
                        host.result(request.clone(), "saved".into(), "resource-one".into()).unwrap();
                        app.provider_tick();
                    }
                    host.set_enabled(false).unwrap();
                    app.session.run("text.insert", &json!({"text":" Edited while provider choices are disabled"})).unwrap();

                    let saves = AsyncSaves::default();
                    let pending = saves.clone();
                    let copies = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
                    let received = copies.clone();
                    let accepted = outcome.is_some();
                    app.services.async_saves = Some(saves.clone());
                    app.services.download = Some(Box::new(move |name, bytes| {
                        if !accepted {
                            return Err("download refused".into());
                        }
                        received.borrow_mut().push((name.to_string(), bytes.to_vec()));
                        pending.start();
                        Ok(())
                    }));
                    let params = if command == "file.saveAs" { json!({"path":"Device copy.docx"}) } else { json!({}) };
                    let result = app.run(command, params);
                    assert!(app.session.dirty, "a disabled provider still owns its unsaved checkpoint");
                    assert_eq!(host.bound(document), initially_bound);
                    assert_eq!(host.pending_for(document), !initially_bound);
                    if let Some(outcome) = outcome {
                        let ticket = result.unwrap()["pendingSave"].as_u64().unwrap();
                        let copies = copies.borrow();
                        let restored = wordcraft_engine::io::open_bytes(&copies[0].0, &copies[0].1).unwrap();
                        assert!(restored.plain_text(wordcraft_doc::StoryRef::Body).contains("Edited while provider choices are disabled"));
                        saves.finish(ticket, outcome);
                        app.settle_async_saves();
                    } else {
                        assert!(result.unwrap_err().contains("download refused"));
                        assert!(copies.borrow().is_empty());
                    }
                    assert!(app.session.dirty, "a device outcome never acknowledges the provider revision");
                    host.set_enabled(true).unwrap();
                    app.provider_tick();
                    assert_eq!(host.snapshot()["documents"][0]["dirty"], true);
                    if !initially_bound {
                        host.result(request, "saved".into(), "resource-one".into()).unwrap();
                        app.provider_tick();
                        assert!(app.session.dirty, "an old pending capture cannot acknowledge the later copy's edits");
                    }
                    assert!(host.bound(document));
                    assert_eq!(host.filename(document).as_deref(), Some("Document.docx"));
                    let (request, _) = save(&mut app, &host);
                    host.result(request, "saved".into(), "resource-one".into()).unwrap();
                    app.provider_tick();
                    assert!(!app.session.dirty, "only the provider ACK acknowledges the edited revision");
                }
            }
        }
    }

    #[test]
    fn device_download_acceptance_and_cancel_do_not_clear_unsaved_document_state() {
        for destination in ["device", "cancel"] {
            let (mut app, host) = app();
            let copies = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let received = copies.clone();
            app.services.provider_download = Some(Box::new(move |name, bytes| {
                received.borrow_mut().push((name.to_string(), bytes.to_vec()));
                Ok(())
            }));
            assert_eq!(app.run("file.save", json!({})).unwrap()["pending"], "fileProvider");
            let events = host.take_events();
            let request = events[0].metadata["requestId"].as_str().unwrap();
            host.resolve_dialog(request, destination, "Device.docx").unwrap();
            app.provider_tick();
            assert!(app.session.dirty, "an accepted browser download has no durable acknowledgement");
            assert!(!host.bound(app.session.document_id()));
            assert!(!host.busy());
            assert_eq!(copies.borrow().len(), usize::from(destination == "device"));
            if let Some((name, bytes)) = copies.borrow().first() {
                let restored = wordcraft_engine::io::open_bytes(name, bytes).unwrap();
                assert!(restored.plain_text(wordcraft_doc::StoryRef::Body).contains("Documento con ñ"));
            }
            app.session.run("text.insert", &json!({"text":" later edit"})).unwrap();
            app.provider_tick();
            assert_eq!(host.snapshot()["documents"][0]["dirty"], true);
            assert_eq!(app.run("file.close", json!({})).unwrap()["pending"], "saveChanges");
            assert!(app.has_document());
        }
    }

    #[test]
    fn durable_save_and_reopen_use_native_docx_and_separate_recents() {
        let (mut app, host) = app();
        let device_recents = vec!["/device/Existing.docx".to_string(), "/device/Older.docm".to_string()];
        app.ui.recent = device_recents.clone();
        let (request, bytes) = save(&mut app, &host);
        assert!(app.session.dirty, "producing DOCX bytes is not a durable save");
        host.result(request, "saved".into(), "resource-one".into()).unwrap();
        app.provider_tick();
        assert!(!app.session.dirty);
        assert!(app.session.path.is_none(), "a provider ID is never a device path");
        assert_eq!(app.ui.recent, device_recents);
        assert_eq!(host.snapshot()["resources"][0]["name"], "Document.docx");
        let open = host.request_open("resource-one").unwrap();
        host.open(open, "Document.docx".into(), bytes).unwrap();
        app.provider_tick();
        assert!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Documento con ñ"));
        assert!(app.has_document());
        assert_eq!(app.ui.recent, device_recents);
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

    #[test]
    fn resource_overwrite_invalidates_the_alias_revision_before_its_old_ack() {
        let (mut app, host) = app();
        let first = app.session.document_id();
        let (request, bytes) = save(&mut app, &host);
        host.result(request, "saved".into(), "shared".into()).unwrap();
        app.provider_tick();
        let read = host.request_open("shared").unwrap();
        host.open(read, "Document.docx".into(), bytes).unwrap();
        app.provider_tick();
        let second = app.session.document_id();
        assert_ne!(first, second);
        app.show_document(first).unwrap();
        app.session.run("text.insert", &json!({"text":"First capture"})).unwrap();
        let (old, _) = save(&mut app, &host);
        app.show_document(second).unwrap();
        app.session.run("text.insert", &json!({"text":"New overwrite"})).unwrap();
        let (new, _) = save(&mut app, &host);
        host.result(new, "saved".into(), "shared".into()).unwrap();
        app.provider_tick();
        host.result(old, "saved".into(), "shared".into()).unwrap();
        app.provider_tick();
        assert!(app.session.with_document(first, |session| session.dirty).unwrap(), "a late ACK cannot undo alias invalidation");
    }
    #[test]
    fn provider_macro_free_name_is_rejected_without_dropping_opaque_vba() {
        let (mut app, host) = app();
        let vba = std::sync::Arc::new(vec![0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3]);
        app.session.doc.passthrough.insert("word/vbaProject.bin".into(), vba.clone());
        assert!(app.choose_provider_save(true));
        let events = host.take_events();
        let request = events[0].metadata["requestId"].as_str().unwrap();
        host.resolve_dialog(request, "provider", "Macro-free.docx").unwrap();
        app.provider_tick();
        let events = host.take_events();
        assert!(events.iter().any(|e| e.metadata["type"] == "saveFailed"));
        assert!(!events.iter().any(|e| e.metadata["type"] == "save"));
        assert_eq!(app.session.doc.passthrough.get("word/vbaProject.bin"), Some(&vba));
        assert!(app.session.dirty);
        assert!(!host.busy());
        assert!(app.choose_provider_save(true));
        let events = host.take_events();
        let request = events[0].metadata["requestId"].as_str().unwrap();
        host.resolve_dialog(request, "provider", "Macro-preserving.docm").unwrap();
        app.provider_tick();
        let events = host.take_events();
        let bytes = events.iter().find(|e| e.metadata["type"] == "save").unwrap().bytes.as_ref().unwrap();
        let restored = wordcraft_engine::io::open_bytes("Macro-preserving.docm", bytes).unwrap();
        assert_eq!(restored.passthrough.get("word/vbaProject.bin"), Some(&vba));
    }

    #[test]
    fn malformed_provider_import_keeps_the_device_history_and_current_document() {
        let (mut app, host) = app();
        let (request, _) = save(&mut app, &host);
        host.result(request, "saved".into(), "resource-one".into()).unwrap();
        app.provider_tick();
        let document = app.session.document_id();
        let device_recents = vec!["/device/Existing.docx".to_string()];
        app.ui.recent = device_recents.clone();
        let read = host.request_open("resource-one").unwrap();
        host.open(read, "Invalid.docx".into(), vec![0, 1, 2]).unwrap();
        app.provider_tick();
        assert_eq!(app.session.document_id(), document);
        assert_eq!(app.ui.recent, device_recents);
        assert!(!host.busy());
    }
}

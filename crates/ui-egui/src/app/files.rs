use super::*;

impl WordApp {
    /// Answer the Save Changes prompt: Save (through Save As for a new document) and carry on,
    /// carry on without saving, or cancel. A failed or cancelled save cancels too.
    pub(super) fn answer_save_changes(&mut self, choice: SaveChoice) -> Result<Value, String> {
        let Some(dialogs::Dialog::SaveChanges { then, params, document, .. }) = self.dialog.take() else {
            return Err("no Save Changes prompt is open".into());
        };
        if document != self.session.document_id() {
            return Err("the document the prompt asked about has been replaced".into());
        }
        // Don't Save is the user's say-so to discard the changes, which Close otherwise refuses.
        let mut params = params;
        if choice == SaveChoice::DontSave && then == "file.close" {
            match params.as_object_mut() {
                Some(p) => {
                    p.insert("force".into(), json!(true));
                }
                None => params = json!({"force": true}),
            }
        }
        let go = match choice {
            SaveChoice::Cancel => false,
            SaveChoice::DontSave => true,
            // A new document or an import-only Word file: ask where through Save As. The command
            // runs once that has saved, on a later frame with the non-blocking dialog (#94).
            SaveChoice::Save if (self.session.path.is_none() || self.legacy_word_path()) && self.services.download.is_none() => {
                return match self.save_as(file_dialogs::AfterSave::Continue { then, params, document }) {
                    file_dialogs::Asked::Done(r) => r,
                    file_dialogs::Asked::Pending => Ok(json!({"pending": "saveAs"})),
                    file_dialogs::Asked::Busy => Ok(json!({"done": false})),
                };
            }
            SaveChoice::Save => match self.save_for_prompt() {
                Ok(PromptSave::Saved) => true,
                Ok(PromptSave::NotSaved) => false,
                // The browser's picker finishes later: the command waits for it (#1315).
                Ok(PromptSave::Later(ticket)) => {
                    let rev = self.session.rev();
                    self.awaiting_save = Some(AwaitingSave { ticket, then, params, document, rev });
                    self.status(tl!("Waiting for the save to finish…"));
                    return Ok(json!({"pending": "save"}));
                }
                Err(error) => return Err(self.save_failed(&error, then, params, document)),
            },
        };
        if !go {
            return Ok(json!({"done": false}));
        }
        self.execute_user(&then, params).map(|r| json!({"done": true, "result": r}))
    }

    /// Save before New/Open/Close (and the mailings that replace the document): true once the
    /// document is safely written. A save in a format that doesn't keep everything (a page
    /// image, plain text) writes a copy and leaves the document unsaved, so it doesn't count:
    /// the command is cancelled and the document stays. A save over the document's own file
    /// (a note.txt that was opened) is a save in any format (#2376). A save that fails (no permission, the
    /// file locked by another app, a missing folder) is the save's error ([`Self::save_failed`]
    /// tells the user).
    /// In the browser the save picker may write the file later ([`PromptSave::Later`], #1315).
    fn save_for_prompt(&mut self) -> Result<PromptSave, String> {
        let v = self.execute("file.save", json!({}))?;
        let saved = v.get("saved").and_then(Value::as_bool) == Some(true) || v.get("downloaded").is_some();
        if !self.safely_saved(saved) {
            return Ok(PromptSave::NotSaved);
        }
        Ok(match v.get("pendingSave").and_then(Value::as_u64) {
            Some(ticket) => PromptSave::Later(ticket),
            None => PromptSave::Saved,
        })
    }

    /// Web: a save of this document is still in the browser's picker (#1315). The document
    /// looks saved, but nothing is written yet, so New/Open ask first.
    pub(crate) fn save_unconfirmed(&self) -> bool {
        let document = self.session.document_id();
        self.saves_in_flight.iter().any(|s| s.marked_saved && s.document == document)
    }

    /// The document's name as the Save Changes prompt shows it: its file's name, else its title.
    pub(super) fn prompt_name(&self) -> String {
        self.session.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.title_stem())
    }

    /// Save (or Save As) from the Save Changes prompt failed (#1445): say so plainly on the status
    /// bar and in the prompt, which comes back for Save again, Don't Save or Cancel, with the
    /// held-back command (`then`) still waiting. Nothing closes and nothing is discarded. Returns
    /// the message, the prompt answer's error.
    pub(crate) fn save_failed(&mut self, error: &str, then: String, params: Value, document: u64) -> String {
        let name = self.prompt_name();
        let message = i18n::fmt(tl!("Couldn't save {name}, so it is still open and nothing was lost: {error}"), &[("name", &name), ("error", error)]);
        log::error!("save from the Save Changes prompt failed: {error}");
        self.prompt_again(message, then, params, document)
    }

    /// The Save Changes prompt comes back with `message` (also on the status bar), the held-back
    /// command still waiting, if the document is still the one it asked about. Returns `message`.
    fn prompt_again(&mut self, message: String, then: String, params: Value, document: u64) -> String {
        self.status(message.clone());
        if document == self.session.document_id() {
            let name = self.prompt_name();
            self.dialog = Some(dialogs::Dialog::SaveChanges { name, then, params, document, message: message.clone() });
        }
        message
    }

    /// Read how the saves that finished later ended (#1315). Called every frame.
    /// A save that didn't write the file leaves the document it was for unsaved again (only that
    /// document: one that has replaced it since is left alone). The command a Save Changes prompt
    /// held back runs once its save is written and closed, if the document is still the same and
    /// unchanged since; a cancelled or failed save brings the prompt back with the reason.
    pub(crate) fn settle_async_saves(&mut self) {
        let Some(saves) = self.services.async_saves.clone() else { return };
        for (ticket, outcome) in saves.take_finished() {
            let current = self.session.document_id();
            if let Some(i) = self.saves_in_flight.iter().position(|s| s.ticket == ticket) {
                let save = self.saves_in_flight.swap_remove(i);
                match &outcome {
                    async_saves::SaveOutcome::Saved => log::info!("saved {} through the save picker", save.name),
                    async_saves::SaveOutcome::Cancelled | async_saves::SaveOutcome::Failed(_) => {
                        if save.marked_saved && save.document == current {
                            self.session.dirty = true;
                        } else if save.marked_saved {
                            // Another document is shown: the one saved is unsaved again where it waits.
                            let _ = self.session.with_document(save.document, |s| s.dirty = true);
                        }
                    }
                }
                if let async_saves::SaveOutcome::Failed(error) = &outcome {
                    let msg = i18n::fmt(tl!("Couldn't download {name}: {error}"), &[("name", &save.name), ("error", error)]);
                    log::error!("the save picker couldn't save {}: {error}", save.name);
                    self.status(msg);
                }
            }
            let Some(wait) = self.awaiting_save.take_if(|w| w.ticket == ticket) else { continue };
            if wait.document != current {
                log::info!("the document a held-back {} was for has been replaced; dropped", wait.then);
                continue;
            }
            match outcome {
                // Only when nothing changed since and no later save of it is still unconfirmed.
                async_saves::SaveOutcome::Saved if !self.session.dirty && self.session.rev() == wait.rev && !self.save_unconfirmed() => {
                    if let Err(e) = self.execute_user(&wait.then, wait.params) {
                        self.status(e);
                    }
                }
                // Edited while the picker was open (those edits may not be written yet): ask again.
                async_saves::SaveOutcome::Saved => {
                    let name = self.prompt_name();
                    self.dialog = Some(dialogs::Dialog::SaveChanges {
                        name,
                        then: wait.then,
                        params: wait.params,
                        document: wait.document,
                        message: String::new(),
                    });
                }
                async_saves::SaveOutcome::Cancelled => {
                    let name = self.prompt_name();
                    let message = i18n::fmt(tl!("Saving {name} was cancelled, so it is still open and nothing was lost."), &[("name", &name)]);
                    self.prompt_again(message, wait.then, wait.params, wait.document);
                }
                async_saves::SaveOutcome::Failed(error) => {
                    self.save_failed(&error, wait.then, wait.params, wait.document);
                }
            }
        }
    }

    /// Whether a save before New/Open/Close counts: it happened, and kept everything.
    pub(crate) fn safely_saved(&mut self, saved: bool) -> bool {
        if saved && self.session.dirty {
            self.status(tl!("That format doesn't keep everything, so the document is still open. Save it as a Word document (.docx) to go on."));
            return false;
        }
        saved
    }

    /// The window's close button (or the system) asked to quit. Returns true to let the window
    /// close; with unsaved changes it asks first and returns false.
    pub fn close_requested(&mut self) -> bool {
        if self.printing.busy() {
            self.status(tl!("Wait for the print job to finish submitting before closing."));
            return false;
        }
        if self.quit_requested {
            return true;
        }
        // Nothing to save, also with no document open in this window (#1129) and none elsewhere.
        if !self.has_unsaved_documents() {
            return true;
        }
        // A comment draft the document refuses is an unsaved change: the prompt can drop it.
        if self.has_document() {
            let _ = panes::flush_comment_drafts(self);
        }
        if !self.has_unsaved_documents() {
            return true;
        }
        // Each document with unsaved work is shown and asked about in turn; Cancel stops.
        self.ask_before_quit();
        self.quit_requested
    }

    /// File › Close (Mod+W, Mod+F4) closes the shown document, or only its window while other
    /// windows show the document (View › New Window), asking nothing then. Its window closes too while other
    /// windows are open, and the one focused before it comes forward (in the web app's one window,
    /// the next document shows). The last window stays, as Word's does (#1712), with no page, as
    /// after Esc on the Start screen (#1129), and File › New or Open goes on from there. `{"quit": true}`,
    /// from the window's close button, goes on quitting: the shown document's prompt has been
    /// answered, so it closes and the next document with unsaved changes is asked about
    /// ([`WordApp::ask_before_quit`]). With no document open there is nothing to close. Unsaved
    /// changes are never discarded unasked: a dirty document closes only with `{"force": true}`
    /// (Don't Save in the prompt passes it).
    pub(super) fn close_document(&mut self, params: &Value) -> Result<Value, String> {
        if self.closes_only_its_window("file.close", params) && self.close_window_only() {
            return Ok(json!({"closed": true, "window": true}));
        }
        if let Some(e) = crate::control::close_refused(self, "file.close", params) {
            return Err(e.into());
        }
        let last = self.session.documents().len() <= 1;
        if params.get("quit").and_then(Value::as_bool) == Some(true) {
            if last {
                // Don't Save: the document stays open until the app exits, but its changes,
                // and so its recovery copy, are discarded.
                if params.get("force").and_then(Value::as_bool) == Some(true) {
                    self.session.discard_recovery_copy();
                }
                self.quit_requested = true;
                return Ok(json!({"quit": true}));
            }
            self.close_shown_document()?;
            return Ok(self.ask_before_quit());
        }
        if !self.has_document() {
            return Ok(json!({"closed": false}));
        }
        self.close_shown_document()?;
        self.recovery_seen = true;
        if last {
            self.no_document = Some(self.session.document_id());
            self.session.release_untitled_name();
        }
        // The window that stays leaves the File page; another window that came forward keeps
        // its own.
        if last || !self.windows.native() {
            self.ui.backstage = false;
        }
        Ok(json!({"closed": true}))
    }

    /// These paths have readers but no writer, so an interactive save needs another format.
    pub(super) fn legacy_word_path(&self) -> bool {
        self.session
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("doc") || e.eq_ignore_ascii_case("dot"))
    }

    /// The name a browser Save or Export without a name uses. An export is named after the
    /// document (`report.pdf`, not `report.docx.pdf`). A host page that keeps the files
    /// ([`Services::host_save`]) gets Save back under the opened file's own name and format, as
    /// the desktop app writes it, except an import-only Word 97–2003 file (#2445).
    pub(super) fn web_save_name(&self, id: &str) -> String {
        match id {
            "file.exportPdf" => return format!("{}.pdf", self.title_stem()),
            "file.exportPng" => return format!("{}.png", self.title_stem()),
            _ => {}
        }
        if id == "file.save"
            && self.services.host_save.is_some()
            && !self.legacy_word_path()
            && let Some(name) = self.session.path.as_ref().and_then(|p| p.file_name())
        {
            return name.to_string_lossy().to_string();
        }
        self.default_save_name()
    }

    /// The name Save suggests: the open file's own Word format (so a .docm keeps its macros),
    /// otherwise .docx.
    pub(crate) fn default_save_name(&self) -> String {
        let ext = self.session.path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_string());
        match ext {
            Some(e) if wordcraft_engine::io::is_word_package(&e) => format!("{}.{e}", self.title_stem()),
            _ => format!("{}.docx", self.title_stem()),
        }
    }

    /// Say how the print job went, once, in the window showing the printed document.
    pub(super) fn print_feedback(&mut self) {
        if self.printing.message == self.print_feedback_seen {
            return;
        }
        self.print_feedback_seen = self.printing.message.clone();
        let Some(message @ (printing::Feedback::Submitted(_) | printing::Feedback::Failed(_) | printing::Feedback::WorkerStopped)) =
            self.printing.message.clone()
        else {
            return;
        };
        let text = printing::feedback(&message, |label| i18n::t(label).to_owned());
        self.status(text);
        // Word returns to the document once a print job is away, the same as after Save; a
        // failure stays on the Print page so the error (and the settings to fix it) are visible
        // (#3210: the Print page used to stay up after a successful submission).
        if matches!(message, printing::Feedback::Submitted(_)) && self.ui.backstage && self.ui.backstage_page == "print" {
            self.ui.backstage = false;
            self.canvas.want_focus = true;
        }
    }

    /// Files dropped on the native window. Raw input is taken only in the first UI pass: a
    /// hidden `run_logic` tick leaves the previous shown frame's events stale, and replaying a
    /// drop would open or insert it again.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn open_dropped_files(&mut self, files: Vec<egui::DroppedFileHandle>) {
        let before = self.session.document_id();
        let mut placed_picture = false;
        for f in &files {
            let path = f.path().to_string_lossy().to_string();
            let lp = path.to_ascii_lowercase();
            let img = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".wmf", ".emf"].iter().any(|e| lp.ends_with(e));
            let sheet = wordcraft_engine::cmd::mailings::is_spreadsheet_name(&lp);
            let _ = if img {
                self.caret_after_placed_picture(placed_picture);
                let r = self.run("insert.picture", json!({"path": path}));
                placed_picture = Self::placed_in_text(&r);
                r
            } else if sheet {
                // A spreadsheet isn't a document: it's a recipient list (#334).
                self.load_recipients(json!({"path": path}))
            } else if wordcraft_fonts::is_font_file_name(&lp) {
                self.run("font.add", json!({"path": path}))
            } else {
                self.open_file(&path, None)
            };
        }
        if self.session.document_id() != before {
            self.autosave_path = None;
            self.dialog = None;
            self.ui.backstage = false;
            self.canvas.want_focus = true;
        }
        if !files.is_empty() {
            self.ensure_caption_fonts();
            self.ensure_comment_fonts();
            self.ensure_recover_fonts();
            self.ensure_recent_fonts();
            self.ensure_find_fonts();
        }
    }

    /// Whether the document's file is one the user saved to in this session.
    pub fn saved_here(&self) -> bool {
        self.session.path.is_some() && self.session.path == self.autosave_path
    }

    /// Why AutoSave can't cover the document, or `None` when it can: it writes only a file the
    /// user saved to in this session, in a format that keeps everything, and never in the browser
    /// (a page can't write to the user's files; saving there is a download, #176).
    pub fn autosave_block(&self) -> Option<AutoSaveBlock> {
        if self.services.download.is_some() {
            return Some(AutoSaveBlock::Browser);
        }
        let Some(path) = &self.session.path else { return Some(AutoSaveBlock::Unsaved) };
        if !keeps_everything(&path.to_string_lossy()) {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            return Some(AutoSaveBlock::Format { name });
        }
        if !self.saved_here() {
            return Some(AutoSaveBlock::NotSavedHere);
        }
        None
    }

    /// Whether AutoSave covers the document: it is on and nothing blocks it ([`Self::autosave_block`]).
    pub fn autosaves(&self) -> bool {
        self.autosave && self.autosave_block().is_none()
    }

    /// `file.autosave` (the AutoSave switch, and scripts): `value` turns AutoSave on or off;
    /// without it, it toggles what the switch shows. Turning it on for a document AutoSave can't
    /// cover is an error that says why, instead of a switch that is on but saves nothing (#196).
    pub(super) fn set_autosave(&mut self, params: &Value) -> Result<Value, String> {
        let on = params.get("value").and_then(Value::as_bool).unwrap_or(!self.autosaves());
        if on && let Some(block) = self.autosave_block() {
            return Err(block.reason());
        }
        self.autosave = on;
        self.session.autosave = on;
        Ok(json!({"value": on}))
    }

    /// AutoSave: write a saved document a couple of seconds after the last change. A failure is
    /// shown in the status bar and turns AutoSave off for the file until the user saves it again.
    pub(crate) fn autosave_tick(&mut self, now: f64) {
        if self.autosaves() && self.session.dirty && now - self.last_autosave > 2500.0 && now - self.canvas.caret_visible_since > 1500.0 {
            self.last_autosave = now;
            if let Err(e) = self.session.run("file.save", &json!({})) {
                log::warn!("AutoSave failed: {e}");
                self.autosave_path = None;
                self.status(i18n::fmt(tl!("AutoSave failed: {error}. Save the document to turn AutoSave back on."), &[("error", &e.to_string())]));
            }
        }
    }

    /// Files that arrived asynchronously (web picker, drops): documents open, pictures insert,
    /// recipient lists (CSV/TSV, spreadsheets, or a text file asked for as one) load as recipients.
    pub(crate) fn drain_inbox(&mut self) {
        let Some(inbox) = self.services.inbox.clone() else { return };
        let files = std::mem::take(&mut *inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let mut placed_picture = false;
        for (name, bytes) in files {
            let lower = name.to_ascii_lowercase();
            let img = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".wmf", ".emf"].iter().any(|e| lower.ends_with(e));
            // Any recipient file ends the pending picker request, so a later .txt drop opens normally.
            let asked = std::mem::take(&mut self.recipient_list_pending);
            let list = lower.ends_with(".csv") || lower.ends_with(".tsv") || (lower.ends_with(".txt") && asked);
            let sheet = wordcraft_engine::cmd::mailings::is_spreadsheet_name(&lower);
            self.recipient_list_pending = asked && !list && !sheet;
            let r = if sheet {
                self.load_recipients(json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}))
            } else if wordcraft_fonts::is_font_file_name(&lower) {
                self.run("font.add", json!({"name": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}))
            } else if list {
                self.load_recipients(json!({"csv": wordcraft_engine::io::decode_text(&bytes)}))
            } else if img {
                self.caret_after_placed_picture(placed_picture);
                let r = self.insert_or_change_picture(json!({"data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}));
                placed_picture = Self::placed_in_text(&r);
                r
            } else {
                self.run("file.open", json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}))
            };
            if r.is_ok() {
                self.ui.backstage = false;
            }
        }
    }
}

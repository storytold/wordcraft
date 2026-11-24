use craft::host::{PickId, Picked};
use craft::platform::open::{DropTarget, DroppedFile, Intake};
use craft::quit::{CloseAnswer, CloseGuard, CloseScope, GuardStep};
use craft::ui::dialog::{DialogId, DialogResult, UnsavedChangesDialog};
use craft::ui::shell::embed::CloseDecision;

use super::{AutoSaveBlock, PromptSave, Value, WordApp, file_dialogs, i18n, json, keeps_everything, panes, printing};

/// The close operation in progress: the unsaved-changes guard, the command it holds back and the
/// prompt it has open.
pub(crate) struct Closing {
    /// Decides, one document at a time, what the operation does next.
    pub guard: CloseGuard<u64>,
    /// The command that runs once every document is answered (a quit sets `quit_requested` instead).
    pub then: String,
    /// The held-back command's parameters; Don't Save adds `force` for a close.
    pub params: Value,
    /// The open Save Changes prompt, when the guard waits for an answer.
    pub prompt: Option<DialogId>,
}

impl WordApp {
    /// Answer the Save Changes prompt of the running close operation: Save (through Save As for a
    /// new document) and carry on, carry on without saving, or cancel. A cancelled Save As cancels.
    pub(crate) fn answer_save_changes(&mut self, answer: CloseAnswer) -> Result<Value, String> {
        self.apply_close_answer(answer, false)
    }

    /// Feed one answer to the guard and do what it says next. `all` is the prompt's "Apply to All".
    fn apply_close_answer(&mut self, answer: CloseAnswer, all: bool) -> Result<Value, String> {
        let document = self.session.document_id();
        match self.closing.as_ref().map(|c| c.guard.current() != Some(&document)) {
            None => return Err("no Save Changes prompt is open".into()),
            Some(true) => {
                self.cancel_closing();
                return Err("the document the prompt asked about has been replaced".into());
            }
            Some(false) => {}
        }
        if let Some(id) = self.closing.as_mut().and_then(|c| c.prompt.take()) {
            self.close_prompt(id);
        }
        let step = self.with_guard(|g| if all { g.answer_all(answer) } else { g.answer(answer) });
        self.drive_guard(step, None)
    }

    /// Start a close operation over `documents` (asked in this order) that ends by running `then`
    /// (or, for a quit, by quitting), and ask about the first one.
    pub(crate) fn begin_closing(&mut self, scope: CloseScope, documents: Vec<u64>, then: String, params: Value) -> Result<Value, String> {
        // PLAN: shell-w6's `run` starts `file.close` and the other held-back commands through this
        let guard = CloseGuard::new(scope, documents);
        let step = guard.current().copied().map_or(GuardStep::Finished, GuardStep::Ask);
        self.closing = Some(Closing { guard, then, params, prompt: None });
        self.drive_guard(step, None)
    }

    /// Run the guard's steps until one has to wait (a prompt, a picker) or the operation ends.
    /// `reason` is why the last save failed; the prompt that comes back shows it.
    fn drive_guard(&mut self, mut step: GuardStep<u64>, mut reason: Option<String>) -> Result<Value, String> {
        loop {
            match step {
                GuardStep::Ask(doc) => return self.ask_prompt(doc, reason.take()),
                GuardStep::Cancelled => {
                    self.cancel_closing();
                    return Ok(json!({"done": false}));
                }
                GuardStep::Finished => return self.finish_closing(),
                GuardStep::Discard(doc) => {
                    self.discard_for_guard(doc);
                    step = self.with_guard(|g| g.discarded());
                }
                GuardStep::Save(doc) => {
                    if let Err(e) = self.show_guard_document(doc) {
                        self.cancel_closing();
                        return Err(e);
                    }
                    // A new document or an import-only Word file: ask where through Save As. The
                    // guard goes on once that has saved, on a later frame (#94).
                    if (self.session.path.is_none() || self.legacy_word_path()) && !self.browser_saves() {
                        return match self.save_as(file_dialogs::AfterSave::Closing) {
                            // PLAN: assumed `AfterSave::Closing` (no fields: the state is `WordApp::closing`)
                            file_dialogs::Asked::Done(r) => r,
                            file_dialogs::Asked::Pending => Ok(json!({"pending": "saveAs"})),
                            file_dialogs::Asked::Busy => {
                                self.cancel_closing();
                                Ok(json!({"done": false}))
                            }
                        };
                    }
                    match self.save_for_prompt() {
                        Ok(PromptSave::Saved) => step = self.with_guard(|g| g.saved()),
                        Ok(PromptSave::NotSaved) => {
                            self.cancel_closing();
                            return Ok(json!({"done": false}));
                        }
                        // The browser's picker finishes later: `settle_saves` carries on (#1315).
                        Ok(PromptSave::Later) => {
                            self.status(tl!("Waiting for the save to finish…"));
                            return Ok(json!({"pending": "save"}));
                        }
                        Err(error) => return Err(self.save_failed(&error)),
                    }
                }
            }
        }
    }

    /// Run the guard on the shown window's behalf: `Cancelled` when no operation is running.
    fn with_guard(&mut self, f: impl FnOnce(&mut CloseGuard<u64>) -> GuardStep<u64>) -> GuardStep<u64> {
        match self.closing.as_mut() {
            Some(closing) => f(&mut closing.guard),
            None => GuardStep::Cancelled,
        }
    }

    fn show_guard_document(&mut self, doc: u64) -> Result<(), String> {
        if doc == self.session.document_id() { Ok(()) } else { self.show_document(doc) }
    }

    /// Show `doc` and open its Save Changes prompt, with `reason` under the message when the last
    /// save of it failed.
    fn ask_prompt(&mut self, doc: u64, reason: Option<String>) -> Result<Value, String> {
        if let Err(e) = self.show_guard_document(doc) {
            self.cancel_closing();
            return Err(e);
        }
        let Some((scope, remaining)) = self.closing.as_ref().map(|c| (c.guard.scope(), c.guard.remaining())) else {
            return Ok(json!({"done": false}));
        };
        let mut dialog = UnsavedChangesDialog::new(scope, self.prompt_name()).remaining(remaining);
        if let Some(reason) = reason {
            dialog = dialog.reason(reason);
        }
        // PLAN: assumed `WordApp::dialog_host: DialogHost`, the shown window's
        let id = self.dialog_host.open(dialog);
        if let Some(closing) = self.closing.as_mut() {
            closing.prompt = Some(id);
        }
        Ok(json!({"pending": "saveChanges"}))
    }

    /// Don't Save for `doc`. A quit closes it now (the last document stays until the app exits, with
    /// its recovery copy dropped); a close passes `force` to the held-back command.
    fn discard_for_guard(&mut self, doc: u64) {
        let Some(scope) = self.closing.as_ref().map(|c| c.guard.scope()) else { return };
        if scope == CloseScope::Quit {
            let _ = self.show_guard_document(doc);
            if self.session.documents().len() <= 1 {
                self.session.discard_recovery_copy();
            } else {
                let _ = self.session.close_document(doc);
                self.sync_document();
            }
        } else if let Some(closing) = self.closing.as_mut() {
            match closing.params.as_object_mut() {
                Some(p) => {
                    p.insert("force".into(), json!(true));
                }
                None => closing.params = json!({"force": true}),
            }
        }
    }

    /// Every document is answered: quit, or run the held-back command.
    fn finish_closing(&mut self) -> Result<Value, String> {
        let Some(closing) = self.closing.take() else { return Ok(json!({"done": false})) };
        if closing.guard.scope() == CloseScope::Quit {
            self.quit_requested = true;
            return Ok(json!({"quit": true}));
        }
        self.execute_held_back(&closing.then, closing.params).map(|r| json!({"done": true, "result": r}))
    }

    /// End the operation without running anything; the prompt closes with it.
    fn cancel_closing(&mut self) {
        if let Some(id) = self.closing.take().and_then(|c| c.prompt) {
            self.close_prompt(id);
        }
    }

    /// Close prompt `id` from outside and drop the answer that leaves behind.
    pub(crate) fn close_prompt(&mut self, id: DialogId) {
        if let Some(ctx) = self.ctx.clone() {
            self.dialog_host.cancel_by_id(&ctx, id);
        }
        let _ = self.dialog_host.take_by_id::<UnsavedChangesDialog>(id);
    }

    /// Take the answer of the open Save Changes prompt, if the user gave one. Called every frame
    /// from `logic`.
    pub(crate) fn pump_close_prompt(&mut self) {
        let Some(id) = self.closing.as_ref().and_then(|c| c.prompt) else { return };
        let Some(result) = self.dialog_host.take_by_id::<UnsavedChangesDialog>(id) else { return };
        if let Some(closing) = self.closing.as_mut() {
            closing.prompt = None;
        }
        let (answer, all) = match result {
            DialogResult::Ok(a) => (a.answer, a.apply_to_all),
            // Cancel, and any way of ending the prompt the framework adds: nothing is decided.
            _ => (CloseAnswer::Cancel, false),
        };
        if let Err(e) = self.apply_close_answer(answer, all) {
            self.status(e);
        }
    }

    /// Whether the window may close now, for the platform's close request: `Hold` while a close
    /// operation runs or only one window of several closes, else as [`Self::close_requested`].
    pub fn close_decision(&mut self) -> CloseDecision {
        if self.closing.is_none() && self.closes_a_window() {
            // The root window's close button closes that window alone (File › Close in it, which asks
            // about unsaved changes first); the framework leaves the root's window open on `Hold`, and
            // the root entry moves to the window that stays.
            let root = self.windows.root_key();
            self.close_window(root);
            CloseDecision::Hold
        } else if self.closing.is_some() {
            CloseDecision::Hold
        } else if self.close_requested() {
            CloseDecision::Close
        } else {
            CloseDecision::Hold
        }
    }

    /// The save a close operation started through Save As ended: carry on with the guard. `result`
    /// is the save's own answer (`None` when the dialog was cancelled) and `saved` whether it wrote
    /// the document.
    pub(crate) fn continue_after_save(&mut self, result: Option<Result<Value, String>>, saved: bool) -> Result<Value, String> {
        if self.closing.is_none() {
            return Ok(json!({"done": false}));
        }
        match result {
            Some(Err(error)) => Err(self.save_failed(&error)),
            None => {
                self.cancel_closing();
                Ok(json!({"done": false}))
            }
            Some(Ok(_)) => {
                let document = self.session.document_id();
                let same = self.closing.as_ref().is_some_and(|c| c.guard.current() == Some(&document));
                if !self.safely_saved(saved) || !same {
                    self.cancel_closing();
                    return Ok(json!({"done": false}));
                }
                let step = self.with_guard(|g| g.saved());
                self.drive_guard(step, None)
            }
        }
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
            Some(_) => PromptSave::Later,
            None => PromptSave::Saved,
        })
    }

    /// Web: a save of this document is still in the browser's picker (#1315). The document
    /// looks saved, but nothing is written yet, so New/Open ask first.
    pub(crate) fn save_unconfirmed(&self) -> bool {
        let document = self.session.document_id();
        self.saves_in_flight.iter().any(|s| s.marked_saved && s.document == document)
    }

    /// Whether the shown document has work that isn't saved yet: changes that aren't written
    /// to a file, or a save that hasn't come back. The Save Changes prompt appears for a
    /// command that would replace the document; a programmatic call (control channel, MCP
    /// bridge), which never prompts, refuses the command instead — so both paths guard the
    /// same work (#1268, #3274).
    pub(crate) fn document_unsaved(&self) -> bool {
        self.document_unsaved_by_id(self.session.document_id())
    }

    /// The document's name as the Save Changes prompt shows it: its file's name, else its title.
    pub(super) fn prompt_name(&self) -> String {
        self.session.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.title_stem())
    }

    /// Save (or Save As) from the Save Changes prompt failed (#1445): say so plainly on the status
    /// bar and in the prompt, which comes back for Save again, Don't Save or Cancel, with the
    /// held-back command still waiting. Nothing closes and nothing is discarded. Returns
    /// the message, the prompt answer's error.
    pub(crate) fn save_failed(&mut self, error: &str) -> String {
        let name = self.prompt_name();
        let message = i18n::fmt(tl!("Couldn't save {name}, so it is still open and nothing was lost: {error}"), &[("name", &name), ("error", error)]);
        log::error!("save from the Save Changes prompt failed: {error}");
        self.prompt_again(message)
    }

    /// The Save Changes prompt comes back with `message` (also on the status bar) for the document
    /// whose save failed. Returns `message`.
    fn prompt_again(&mut self, message: String) -> String {
        self.status(message.clone());
        let step = self.with_guard(|g| g.retry_after_failure());
        if self.closing.is_some()
            && let Err(e) = self.drive_guard(step, Some(message.clone()))
        {
            self.status(e);
        }
        message
    }

    /// Read how the saves that finished later ended (#1315). Called every frame.
    /// A save that didn't write the file leaves the document it was for unsaved again (only that
    /// document: one that has replaced it since is left alone). A close operation waiting on the
    /// save goes on once it is written and the document is unchanged since; a cancelled or failed
    /// save brings the prompt back with the reason.
    pub(crate) fn settle_saves(&mut self) {
        let tickets: Vec<PickId> = self.saves_in_flight.iter().map(|s| s.ticket).collect();
        for ticket in tickets {
            let Some(picked) = self.host.files().take_answer(ticket) else { continue };
            let Some(i) = self.saves_in_flight.iter().position(|s| s.ticket == ticket) else { continue };
            let mut save = self.saves_in_flight.swap_remove(i);
            let current = self.session.document_id();
            // A save that never reached its destination leaves no stamp of it, when the document is as
            // the save left it (an edit since keeps its own marks and stays unsaved).
            let mut undone = false;
            if !matches!(picked, Picked::Paths(_) | Picked::Downloaded(_))
                && save.document == current
                && save.rev == self.session.rev()
                && let Some(undo) = save.undo_stamp.take()
            {
                undo(&mut self.session);
                undone = true;
            }
            // A download the browser accepted is a finished save too: the page holds no file of it.
            if matches!(picked, Picked::Paths(_) | Picked::Downloaded(_)) {
                log::info!("saved {} through the save picker", save.name);
                // The page now holds the document as that file (a hosted Save As or first Save, S-L159):
                // the next Save goes back to it. A download names nothing the app could reopen.
                if save.marked_saved
                    && picked.records_file()
                    && let Some(path) = picked.first_path()
                {
                    if save.document == current {
                        self.session.path = Some(path);
                    } else {
                        let _ = self.session.with_document(save.document, |s| s.path = Some(path));
                    }
                }
            } else if save.marked_saved && save.document == current {
                // The undone stamp already put `dirty` back as it was before the save (a clean document stays clean).
                if !undone {
                    self.session.dirty = true;
                }
            } else if save.marked_saved {
                // Another document is shown: the one saved is unsaved again where it waits.
                let _ = self.session.with_document(save.document, |s| s.dirty = true);
            }
            if let Picked::Failed(error) = &picked {
                let msg = i18n::fmt(tl!("Couldn't download {name}: {error}"), &[("name", &save.name), ("error", error)]);
                log::error!("the save picker couldn't save {}: {error}", save.name);
                self.status(msg);
            }
            // The guard waits on this save when it asks about no one and its front document is
            // the one saved.
            let waits = self.closing.as_ref().is_some_and(|c| c.prompt.is_none() && c.guard.current() == Some(&save.document));
            if !waits {
                continue;
            }
            // A failed open was already reported by name in `execute_user`.
            let then_open = self.closing.as_ref().is_some_and(|c| c.then == "file.open");
            if let Err(e) = self.show_guard_document(save.document) {
                self.cancel_closing();
                self.status(e);
                continue;
            }
            let result = match picked {
                // Only when nothing changed since and no later save of it is still unconfirmed.
                Picked::Paths(_) | Picked::Downloaded(_) if !self.session.dirty && self.session.rev() == save.rev && !self.save_unconfirmed() => {
                    let step = self.with_guard(|g| g.saved());
                    self.drive_guard(step, None)
                }
                // Edited while the picker was open (those edits may not be written yet): ask again.
                Picked::Paths(_) | Picked::Downloaded(_) => {
                    let step = self.with_guard(|g| g.retry_after_failure());
                    self.drive_guard(step, None)
                }
                // No picker could be shown, or the answer is not one a save gets: nothing was written.
                Picked::Cancelled | Picked::Unavailable(_) | Picked::Contents(_) => {
                    let name = self.prompt_name();
                    let message = i18n::fmt(tl!("Saving {name} was cancelled, so it is still open and nothing was lost."), &[("name", &name)]);
                    self.prompt_again(message);
                    Ok(Value::Null)
                }
                Picked::Failed(error) => {
                    self.save_failed(&error);
                    Ok(Value::Null)
                }
            };
            if let Err(e) = result
                && !then_open
            {
                self.status(e);
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
    /// gets Save back under the opened file's own name and format, as
    /// the desktop app writes it, except an import-only Word 97–2003 file (#2445).
    pub(super) fn web_save_name(&self, id: &str) -> String {
        match id {
            "file.exportPdf" => return format!("{}.pdf", self.title_stem()),
            "file.exportPng" => return format!("{}.png", self.title_stem()),
            _ => {}
        }
        if id == "file.save"
            && self.browser_saves()
            && !self.legacy_word_path()
            && let Some(name) = self.session.path.as_ref().and_then(|p| p.file_name())
        {
            return name.to_string_lossy().to_string();
        }
        self.default_save_name()
    }

    /// The name Save suggests: the open file's own Word format (so a .docm keeps its macros) or,
    /// in a dialog, its own Markdown format (a browser's download keeps the whole document as
    /// .docx), otherwise .docx. A document never saved and without a title of its own is named after
    /// its first words, as Word suggests them (#5793), instead of `Document1`.
    pub(crate) fn default_save_name(&self) -> String {
        let ext = self.session.path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_string());
        let unnamed = self.session.path.is_none() && self.session.file_name.is_none() && self.session.doc.core.title.trim().is_empty();
        let first = || self.session.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).find_map(|t| suggested_stem(&t));
        let stem = unnamed.then(first).flatten().unwrap_or_else(|| self.title_stem());
        let markdown = !self.browser_saves() && self.session.path.as_deref().is_some_and(wordcraft_engine::cmd::md_source::is_markdown_file);
        match ext {
            Some(e) if markdown || wordcraft_engine::io::is_word_package(&e) => format!("{stem}.{e}"),
            _ => format!("{stem}.docx"),
        }
    }

    /// Say how the print job went, once, in the window showing the printed document.
    pub(super) fn print_feedback(&mut self) {
        if self.printing.message == self.print_feedback_seen {
            return;
        }
        self.print_feedback_seen = self.printing.message.clone();
        // A new message: what any window said before no longer stands for it.
        self.print_gen = self.print_gen.wrapping_add(1);
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
            if let Some(ctx) = self.ctx.clone() {
                self.canvas.focus_page(&ctx);
            }
        }
        // Said only where the status bar is drawn: not under File's other pages, and a failure on
        // the Print page is the page's to show.
        if !self.ui.backstage {
            // PLAN: chrome-w10 makes `PrintPage::said` `pub(crate)`
            self.print.said = Some(self.print_gen);
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
        if self.browser_saves() {
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
        if self.autosaves() && self.session.dirty && now - self.last_autosave > 2500.0 && self.canvas.glide.since_restart_ms(now) > 1500.0 {
            self.last_autosave = now;
            if let Err(e) = self.session.run("file.save", &json!({})) {
                log::warn!("AutoSave failed: {e}");
                self.autosave_path = None;
                self.status(i18n::fmt(tl!("AutoSave failed: {error}. Save the document to turn AutoSave back on."), &[("error", &e.to_string())]));
            }
        }
    }

    /// What arrived through the platform's file intake: documents open, pictures insert,
    /// recipient lists (CSV/TSV, spreadsheets, or a text file asked for as one) load as recipients,
    /// and a quit request goes through the close guard.
    pub(crate) fn drain_inbox(&mut self) {
        let Some(intake) = self.services.intake.as_ref() else { return };
        for item in intake.take() {
            match item {
                Intake::Dropped { files, target } => self.open_dropped(&files, target),
                Intake::Open(paths) | Intake::OpenFor { paths, .. } => {
                    for p in paths {
                        // A path that is not Unicode cannot be passed on as text: skipped.
                        if let Some(path) = p.to_str()
                            && let Err(e) = self.open_from_platform(path, None)
                        {
                            self.report_open_failure(path, &e);
                        }
                        // Backstage hides the status bar; leave it so a failure shows.
                        self.ui.backstage = false;
                    }
                }
                // `logic_pass` closes the root window once `quit_requested` is set.
                Intake::Quit => {
                    self.quit();
                }
                // An OS open request that arrived as bytes, with no path.
                Intake::Bytes { name, bytes } => {
                    let data = wordcraft_engine::cmd::insert::base64_encode(bytes.as_ref());
                    if let Err(e) = self.run("file.open", json!({"path": name, "data": data})) {
                        self.report_open_failure(&name, &e);
                    }
                    self.ui.backstage = false;
                }
            }
        }
    }

    /// Route files dropped on the window, in drop order: the one place the drop rules live. A drop
    /// behind a modal dialog does nothing, since it would insert a picture or open a document
    /// under it. Every drop target routes by file kind, so `_target` does not change the routing.
    pub(crate) fn open_dropped(&mut self, files: &[DroppedFile], _target: DropTarget) {
        if self.modal_open() {
            return;
        }
        let mut placed_picture = false;
        for file in files {
            if self.open_dropped_file(file, &mut placed_picture).is_ok() {
                self.ui.backstage = false;
            }
        }
    }

    /// Route one dropped file by its kind: picture, recipient list, spreadsheet, font, else a
    /// document. A file with bytes (the web) is passed as data; one with a path is read from it.
    fn open_dropped_file(&mut self, file: &DroppedFile, placed_picture: &mut bool) -> Result<Value, String> {
        let name = file.path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| file.name.clone());
        let lower = name.to_ascii_lowercase();
        let data = file.bytes.as_ref().map(|b| wordcraft_engine::cmd::insert::base64_encode(b.as_ref()));
        let img = file_dialogs::is_picture_name(&name);
        // Any recipient file ends the pending picker request, so a later .txt drop opens normally.
        let asked = std::mem::take(&mut self.recipient_list_pending);
        let list = lower.ends_with(".csv") || lower.ends_with(".tsv") || (lower.ends_with(".txt") && asked);
        let sheet = wordcraft_engine::cmd::mailings::is_spreadsheet_name(&lower);
        self.recipient_list_pending = asked && !list && !sheet;
        let with_data = |mut p: Value| {
            if let Some(d) = &data {
                p["data"] = json!(d);
            }
            p
        };
        let r = if sheet {
            self.load_recipients(with_data(json!({"path": name})))
        } else if wordcraft_fonts::is_font_file_name(&lower) {
            match &data {
                Some(d) => self.run("font.add", json!({"name": name, "data": d})),
                None => self.run("font.add", json!({"path": name})),
            }
        } else if list {
            match &file.bytes {
                Some(b) => self.load_recipients(json!({"csv": wordcraft_engine::io::decode_text(b)})),
                None => self.load_recipients(json!({"path": name})),
            }
        } else if img {
            self.caret_after_placed_picture(*placed_picture);
            let r = match &data {
                Some(d) => self.insert_or_change_picture(json!({"data": d})),
                None => self.run("insert.picture", json!({"path": name})),
            };
            *placed_picture = Self::placed_in_text(&r);
            r
        } else if data.is_some() {
            self.run("file.open", with_data(json!({"path": name})))
        } else {
            self.open_file(&name, None)
        };
        if let Err(e) = &r {
            self.report_open_failure(&name, e);
            // Backstage hides the status bar; leave it so the error shows.
            self.ui.backstage = false;
        }
        r
    }
}

/// The most characters a name taken from the text keeps ([`suggested_stem`]).
const SUGGESTED_MAX: usize = 50;

/// A file name from a paragraph's text (#5793): up to its first sentence or clause end
/// (`.`, `!`, `?`, `;`, `:` and their full-width forms) or line break, without the characters a
/// file name can't hold (`\ / : * ? " < > |` and controls), white space collapsed, at most
/// [`SUGGESTED_MAX`] characters cut at a word's end, and no dot or space at either end (Windows
/// drops them). `None` when nothing is left, so the next paragraph, or `Document1`, is used.
pub(crate) fn suggested_stem(text: &str) -> Option<String> {
    let end = |c: char| matches!(c, '.' | '!' | '?' | ';' | ':' | '\n' | '\r' | '\u{0B}' | '\u{2029}' | '。' | '！' | '？' | '；' | '：');
    let head = text.split(end).next().unwrap_or("");
    let kept: String = head.chars().map(|c| if c.is_control() || matches!(c, '\\' | '/' | '*' | '"' | '<' | '>' | '|') { ' ' } else { c }).collect();
    let mut words = String::new();
    for w in kept.split_whitespace() {
        let next = if words.is_empty() { w.chars().count() } else { words.chars().count() + 1 + w.chars().count() };
        if next > SUGGESTED_MAX {
            if words.is_empty() {
                words = w.chars().take(SUGGESTED_MAX).collect();
            }
            break;
        }
        if !words.is_empty() {
            words.push(' ');
        }
        words.push_str(w);
    }
    let words = words.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    (!words.is_empty()).then_some(words)
}

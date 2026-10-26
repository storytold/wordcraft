//! Document tabs adapted from PR #528 (23c107a5), by EvilDemonFluffball.
use super::*;

impl WordApp {
    pub fn empty_window(&mut self) {
        self.no_document = Some(self.session.document_id());
    }

    /// List all open documents, in display order.
    pub fn document_tabs(&self) -> Vec<DocumentTabInfo> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.active_tab || self.has_document())
            .map(|(index, slot)| {
                let session = slot.as_ref().map_or(&self.session, |t| &t.session);
                let name = session.path.as_ref().and_then(|p| p.file_name()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| {
                    if session.doc.core.title.is_empty() {
                        format!("{} {}", tl!("Document"), self.tab_ids.get(index).copied().unwrap_or(1))
                    } else {
                        session.doc.core.title.clone()
                    }
                });
                DocumentTabInfo {
                    id: self.tab_ids[index],
                    index,
                    name,
                    dirty: self.tab_has_unsaved_work(index),
                    active: index == self.active_tab,
                    pinned: self.pinned_tabs[index],
                }
            })
            .collect()
    }

    pub fn active_document_tab(&self) -> usize {
        self.active_tab
    }

    pub fn document_tab_path(&self, index: usize) -> Option<String> {
        if index >= self.tabs.len() {
            return None;
        }
        let session = self.tabs[index].as_ref().map_or(&self.session, |t| &t.session);
        session.path.as_ref().map(|p| p.to_string_lossy().into_owned())
    }

    /// Stable ID for egui state such as a document's last scroll offset.
    pub(crate) fn active_document_id(&self) -> u64 {
        self.tab_ids.get(self.active_tab).copied().unwrap_or(0)
    }

    pub fn switch_document_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        if index == self.active_tab {
            return true;
        }
        if self.document_modal() {
            return false;
        }
        let Some(mut next) = self.tabs.get_mut(index).and_then(Option::take) else { return false };
        // A platform composition belongs to the visible document, never an inactive tab.
        let composing = !self.canvas.ime_preedit.is_empty();
        self.canvas.ime_preedit.clear();
        self.canvas.ime_preedit_range = None;
        self.canvas.ime_join_text = None;
        self.session.clear_ime_joins();
        next.canvas.ime_interrupt_now = true;
        next.canvas.ime_interrupted_preedit |= composing;
        copy_preferences(&self.session, &mut next.session);
        // Swap the entire session and editor state, never copying a document or its undo stack.
        let previous = DocumentTab {
            session: std::mem::replace(&mut self.session, next.session),
            canvas: std::mem::replace(&mut self.canvas, next.canvas),
            autosave_path: std::mem::replace(&mut self.autosave_path, next.autosave_path),
            autosave: std::mem::replace(&mut self.autosave, next.autosave),
            last_autosave: std::mem::replace(&mut self.last_autosave, next.last_autosave),
            word_count: std::mem::replace(&mut self.word_count, next.word_count),
            read_aloud_at: std::mem::replace(&mut self.read_aloud_at, next.read_aloud_at),
            read_aloud_error: std::mem::replace(&mut self.read_aloud_error, next.read_aloud_error),
            change_picture_target: std::mem::replace(&mut self.change_picture_target, next.change_picture_target),
            equation_prev_tab: std::mem::replace(&mut self.equation_prev_tab, next.equation_prev_tab),
            comment_drafts: std::mem::replace(&mut self.comment_drafts, next.comment_drafts),
            restrict: std::mem::replace(&mut self.restrict, next.restrict),
            controls: std::mem::replace(&mut self.controls, next.controls),
            outline_prev_tab: std::mem::replace(&mut self.outline_prev_tab, next.outline_prev_tab),
            smart_art_item: std::mem::replace(&mut self.smart_art_item, next.smart_art_item),
            smart_art_typing: std::mem::replace(&mut self.smart_art_typing, next.smart_art_typing),
            info_editing: std::mem::replace(&mut self.info_editing, next.info_editing),
        };
        self.tabs[self.active_tab] = Some(previous);
        self.active_tab = index;
        self.smart_art_row_focused = false;
        options::sync(self);
        self.dialog = None;
        self.colors = None;
        self.ui.backstage = false;
        self.canvas.want_focus = true;
        self.status_msg = None;
        self.applied_theme = None; // page-specific dark mode
        self.previews = previews::Previews::default(); // style previews belong to the document
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
        true
    }

    /// Close a tab by its displayed index. Unsaved work asks before the tab is removed.
    pub fn close_document_tab(&mut self, index: usize) -> Result<Value, String> {
        if !self.switch_document_tab(index) {
            return Err("unable to select that document while a save prompt is open".into());
        }
        self.run("file.close", json!({}))
    }

    /// Toggle pinning; pinned documents form an ordered group at the left edge.
    pub fn pin_document_tab(&mut self, index: usize, pinned: bool) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        if self.pinned_tabs[index] == pinned {
            return true;
        }
        let other_pins = self.pinned_tabs.iter().filter(|&&p| p).count();
        let destination = if pinned { other_pins } else { other_pins - 1 };
        self.pinned_tabs[index] = pinned;
        self.move_document_tab_unchecked(index, destination)
    }

    /// Queue a close operation that respects each tab's Save Changes prompt.
    pub fn close_other_document_tabs(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() || self.document_modal() {
            return false;
        }
        self.dialog = None;
        self.colors = None;
        self.pending_keep_tab = Some(self.tab_ids[index]);
        self.pending_closes =
            self.tab_ids.iter().enumerate().filter(|(i, _)| *i != index && !self.pinned_tabs[*i]).map(|(_, id)| *id).rev().collect();
        self.continue_pending_closes();
        true
    }

    /// Close every tab, including pinned tabs, leaving an empty application window.
    pub fn close_all_document_tabs(&mut self) -> bool {
        if self.document_modal() {
            return false;
        }
        self.dialog = None;
        self.colors = None;
        self.pending_keep_tab = None;
        self.pending_closes = self.tab_ids.iter().copied().rev().collect();
        self.continue_pending_closes();
        true
    }

    pub(super) fn continue_pending_closes(&mut self) {
        if self.document_modal() || self.closing_all || self.quit_requested {
            return;
        }
        // Keep the remaining queue off self while closing, so clean tabs don't recurse.
        let mut pending = std::mem::take(&mut self.pending_closes);
        let keep = self.pending_keep_tab.take();
        while let Some(id) = pending.pop() {
            if let Some(index) = self.tab_ids.iter().position(|t| *t == id) {
                if let Err(error) = self.close_document_tab(index) {
                    self.status(error);
                    return;
                }
                if self.dialog.is_some() || self.file_dialog_is_continuation() || self.awaiting_save.is_some() {
                    self.pending_closes = pending;
                    self.pending_keep_tab = keep;
                    return;
                }
            }
        }
        if let Some(id) = keep
            && let Some(index) = self.tab_ids.iter().position(|t| *t == id)
        {
            self.switch_document_tab(index);
        }
    }

    /// Reopen the last closed file from disk, not discarded unsaved edits.
    pub fn reopen_closed_document_tab(&mut self) -> Result<Value, String> {
        let Some(closed) = self.recently_closed.last().cloned() else {
            return Err("there is no previously closed saved document".into());
        };
        let path = closed.path.clone();
        // A failed reopen must remain on the history stack for a future retry.
        let result = self.add_document_tab("file.open", &json!({"path": path}))?;
        self.recently_closed.pop();
        if result.get("alreadyOpen").and_then(Value::as_bool) == Some(true) {
            return Ok(result);
        }
        let from = self.active_tab;
        // Pin first, then reorder within the same group so pinned tabs stay to the left.
        self.pin_document_tab(from, closed.pinned);
        let from = self.active_tab;
        let pin_count = self.pinned_tabs.iter().filter(|&&p| p).count();
        let min = if closed.pinned { 0 } else { pin_count };
        let max = if closed.pinned { pin_count.saturating_sub(1) } else { self.tabs.len() - 1 };
        self.reorder_document_tab(from, closed.index.clamp(min, max));
        Ok(result)
    }

    /// Move an existing tab to another position without affecting any document state.
    pub fn reorder_document_tab(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tabs.len() || to >= self.tabs.len() {
            return false;
        }
        if from == to {
            return true;
        }
        // Only move within the pinned or unpinned group; use pin_document_tab to cross it.
        if self.pinned_tabs[from] != self.pinned_tabs[to] {
            return false;
        }
        self.move_document_tab_unchecked(from, to)
    }

    pub(super) fn move_document_tab_unchecked(&mut self, from: usize, to: usize) -> bool {
        if from == to {
            return true;
        }
        let slot = self.tabs.remove(from);
        self.tabs.insert(to, slot);
        let id = self.tab_ids.remove(from);
        self.tab_ids.insert(to, id);
        let pin = self.pinned_tabs.remove(from);
        self.pinned_tabs.insert(to, pin);
        if self.active_tab == from {
            self.active_tab = to;
        } else if from < self.active_tab && to >= self.active_tab {
            self.active_tab -= 1;
        } else if from > self.active_tab && to <= self.active_tab {
            self.active_tab += 1;
        }
        true
    }

    pub(super) fn add_document_tab(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        if self.document_modal() {
            return Err("finish the document dialog or operation before opening another tab".into());
        }
        let placeholder = !self.has_document() && !self.tab_has_unsaved_work(self.active_tab);
        // Never put two independent edit sessions on the same file. Re-open selects its tab.
        if id == "file.open"
            && let Some(path) = params.get("path").and_then(Value::as_str)
            && params.get("data").is_none()
        {
            let sought = std::fs::canonicalize(path).ok();
            let found = self.document_tabs().into_iter().find(|tab| {
                let session = self.tabs[tab.index].as_ref().map_or(&self.session, |t| &t.session);
                session
                    .path
                    .as_ref()
                    .is_some_and(|p| p.as_path() == std::path::Path::new(path) || (sought.is_some() && std::fs::canonicalize(p).ok() == sought))
            });
            if let Some(tab) = found {
                self.switch_document_tab(tab.index);
                return Ok(json!({"path": path, "tab": tab.index, "alreadyOpen": true}));
            }
        }
        if self.tabs.len() >= 128 && !placeholder {
            return Err("at most 128 document tabs may be open in one window".into());
        }
        if id == "file.recover" {
            let restore = params.get("restore").and_then(Value::as_str).ok_or("restore must be a recovery id")?;
            if params.get("discard").is_some() || !self.session.recovery_documents().iter().any(|d| d.id == restore) {
                return Err("choose an available recovery copy to restore".into());
            }
        }
        let mut session = Session::new(self.session.blank_document());
        copy_preferences(&self.session, &mut session);
        session.view = self.session.view.clone();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((root, wake)) = &self.recovery_config {
            session.enable_recovery(root.clone(), wake.clone())?;
        }
        let result = if id == "file.recover" {
            #[cfg(not(target_arch = "wasm32"))]
            if self.recovery_config.is_none() {
                return Err("local recovery requires the desktop host".into());
            }
            self.pending_restores.push((session.document_id(), session.rev(), params.clone()));
            json!({"pending": "recovery"})
        } else {
            session.run(id, params).map_err(|e| e.to_string())?
        };
        if placeholder {
            self.session.finish_recovery();
            self.session = session;
            self.canvas = Default::default();
            self.no_document = None;
            self.autosave_path = None;
            self.recovery_seen = true;
            self.after_command(id);
            return Ok(result);
        }
        self.tabs.push(Some(DocumentTab::new(session)));
        self.tab_ids.push(self.next_tab_id);
        self.pinned_tabs.push(false);
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        let index = self.tabs.len() - 1;
        self.switch_document_tab(index);
        self.recovery_seen = true;
        self.after_command(id);
        Ok(result)
    }

    pub(super) fn close_active_tab_now(&mut self) -> Result<Value, String> {
        if !self.has_document() && !self.tab_has_unsaved_work(self.active_tab) {
            return Ok(json!({"closed": false}));
        }
        if let Some(path) = self.session.path.as_ref() {
            // Reopening loads the saved file. Explicitly discarded edits are not recoverable.
            self.recently_closed.push(ClosedDocument {
                path: path.to_string_lossy().into_owned(),
                index: self.active_tab,
                pinned: self.pinned_tabs[self.active_tab],
            });
            if self.recently_closed.len() > 20 {
                self.recently_closed.remove(0);
            }
        }
        self.confirmed_saves.remove(&self.session.document_id());
        // An explicit close supersedes Quit review; remaining dirty tabs need a fresh review.
        if self.closing_all {
            self.cancel_close_sequence();
        }
        self.dialog = None;
        self.colors = None;
        self.session.dirty = false;
        self.session.finish_recovery();
        if self.tabs.len() == 1 {
            self.session.set_document(self.session.blank_document());
            self.session.path = None;
            self.no_document = Some(self.session.document_id());
            self.canvas = Default::default();
            self.comment_drafts = Default::default();
            self.restrict = Default::default();
            self.controls = Default::default();
            self.previews = Default::default();
            self.session.read_aloud.stop();
            self.read_aloud_at = None;
            self.read_aloud_error = None;
            self.change_picture_target = None;
            self.equation_prev_tab = None;
            self.outline_prev_tab = None;
            self.smart_art_item = None;
            self.smart_art_typing = None;
            self.smart_art_row_focused = false;
            self.info_editing = None;
            self.word_count = (0, 0);
            self.autosave = true;
            self.last_autosave = 0.0;
            self.autosave_path = None;
            self.ui.backstage = false;
            self.pinned_tabs[0] = false;
            self.tab_ids[0] = self.next_tab_id;
            self.next_tab_id = self.next_tab_id.saturating_add(1);
            return Ok(json!({"closed": true}));
        }
        self.tabs.remove(self.active_tab);
        self.tab_ids.remove(self.active_tab);
        self.pinned_tabs.remove(self.active_tab);
        let index = self.active_tab.min(self.tabs.len() - 1);
        let Some(mut next) = self.tabs[index].take() else {
            return Err("document tabs are in an invalid state".into());
        };
        copy_preferences(&self.session, &mut next.session);
        self.session = next.session;
        self.canvas = next.canvas;
        self.autosave_path = next.autosave_path;
        self.autosave = next.autosave;
        self.last_autosave = next.last_autosave;
        self.word_count = next.word_count;
        self.read_aloud_at = next.read_aloud_at;
        self.read_aloud_error = next.read_aloud_error;
        self.change_picture_target = next.change_picture_target;
        self.equation_prev_tab = next.equation_prev_tab;
        self.comment_drafts = next.comment_drafts;
        self.restrict = next.restrict;
        self.controls = next.controls;
        self.outline_prev_tab = next.outline_prev_tab;
        self.smart_art_item = next.smart_art_item;
        self.smart_art_typing = next.smart_art_typing;
        self.info_editing = next.info_editing;
        self.active_tab = index;
        self.dialog = None;
        self.status_msg = None;
        self.previews = previews::Previews::default();
        self.applied_theme = None;
        self.canvas.want_focus = true;
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
        Ok(json!({"closed": true, "activeTab": index}))
    }

    pub(super) fn any_dirty_documents(&self) -> bool {
        (0..self.tabs.len()).any(|i| self.tab_has_unsaved_work(i))
    }

    /// Review the dirty tabs without disposing of sessions: the full workspace must still
    /// exist when `on_exit` serializes preferences, including files saved during these prompts.
    pub(super) fn continue_close_all(&mut self) {
        if !self.closing_all {
            return;
        }
        if let Some(index) = self.tab_ids.iter().enumerate().find_map(|(i, id)| {
            let dirty = self.tab_has_unsaved_work(i);
            (dirty && !self.quit_reviewed.contains(id)).then_some(i)
        }) {
            if index != self.active_tab && !self.switch_document_tab(index) {
                return;
            }
            let name =
                self.session.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.title_stem());
            self.dialog = Some(dialogs::Dialog::SaveChanges {
                name,
                then: "ui.confirmQuitTab".into(),
                params: json!({}),
                document: self.session.document_id(),
                message: String::new(),
            });
        } else {
            self.session.dirty = false;
            self.comment_drafts = Default::default();
            for tab in self.tabs.iter_mut().flatten() {
                tab.session.dirty = false;
                tab.comment_drafts = Default::default();
            }
            // Prompting may have selected several dirty documents. Restore the user's
            // selected document before persisting the workspace on exit.
            if let Some(id) = self.quit_original_tab
                && let Some(index) = self.tab_ids.iter().position(|&candidate| candidate == id)
            {
                self.switch_document_tab(index);
            }
            self.quit_requested = true;
        }
    }

    /// Capture tab order, paths and pinning. Empty tabs reopen blank (never resurrect unsaved text).
    pub fn workspace_snapshot(&self) -> WorkspaceState {
        WorkspaceState {
            tabs: self
                .tabs
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != self.active_tab || self.has_document())
                .map(|(i, slot)| {
                    let session = slot.as_ref().map_or(&self.session, |t| &t.session);
                    WorkspaceTab {
                        path: session.path.as_ref().map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()).to_string_lossy().into_owned()),
                        pinned: self.pinned_tabs[i],
                    }
                })
                .collect(),
            active: self.active_tab,
        }
    }

    /// Called after preferences load, only for desktop launches without explicit files or --sample.
    /// Missing or unreadable documents are skipped; the initial blank stays if nothing opens.
    pub fn restore_workspace(&mut self) -> usize {
        let saved = self.ui.workspace.clone();
        if saved.tabs.is_empty() || self.services.download.is_some() {
            return 0;
        }
        if !self.session.dirty && self.session.path.is_none() {
            self.no_document = Some(self.session.document_id());
        }
        let mut errors = Vec::new();
        if saved.tabs.len() > 128 {
            errors.push(tl!("Skipped workspace entries beyond the 128-tab limit.").to_string());
        }
        let mut count = 0;
        let mut active_id = None;
        for (i, tab) in saved.tabs.iter().take(128).enumerate() {
            let result = match &tab.path {
                Some(path) => self.add_document_tab("file.open", &json!({"path": path})),
                None => self.add_document_tab("file.new", &json!({})),
            };
            match result {
                Ok(value) if value.get("alreadyOpen").and_then(Value::as_bool) == Some(true) => {
                    if saved.active == i {
                        active_id = Some(self.active_document_id());
                    }
                }
                Ok(_) => {
                    count += 1;
                    let index = self.active_tab;
                    self.pin_document_tab(index, tab.pinned);
                    if saved.active == i {
                        active_id = Some(self.active_document_id());
                    }
                }
                Err(error) => {
                    log::warn!("could not restore document tab: {error}");
                    errors.push(error);
                }
            }
        }
        if count > 0 {
            let target = active_id
                .and_then(|id| self.tab_ids.iter().position(|candidate| *candidate == id))
                .unwrap_or_else(|| saved.active.min(self.tabs.len() - 1));
            self.switch_document_tab(target);
        }
        if !errors.is_empty() {
            self.status(format!("{}: {}", tl!("Could not restore document"), errors.join("; ")));
        }
        count
    }
}

fn copy_preferences(from: &Session, to: &mut Session) {
    to.author = from.author.clone();
    to.initials = from.initials.clone();
    to.prefs = from.prefs.clone();
    to.system_region = from.system_region.clone();
    to.user_dict_path = from.user_dict_path.clone();
    to.set_editing_language(from.editing_lang.as_deref());
    to.keymap = from.keymap.clone();
    to.ribbon = from.ribbon.clone();
    to.read_aloud.set_rate(from.read_aloud.rate());
    to.read_aloud.skip_citations = from.read_aloud.skip_citations;
    to.view.dark_mode = from.view.dark_mode;
    to.view.proofing = from.view.proofing;
    to.view.draw.pens = from.view.draw.pens.clone();
}

pub(crate) fn is_tab_command(id: &str) -> bool {
    matches!(
        id,
        "ui.documentTabs"
            | "ui.switchDocument"
            | "ui.closeDocument"
            | "ui.reopenClosedDocument"
            | "ui.closeOtherDocuments"
            | "ui.closeAllDocuments"
            | "ui.pinDocument"
            | "ui.moveDocument"
            | "ui.nextDocument"
            | "ui.previousDocument"
            | "ui.newWindow"
    )
}

impl WordApp {
    pub(super) fn document_modal(&self) -> bool {
        matches!(
            self.dialog,
            Some(
                dialogs::Dialog::SaveChanges { .. }
                    | dialogs::Dialog::Password { .. }
                    | dialogs::Dialog::EncryptPassword { .. }
                    | dialogs::Dialog::DiscardRecovery { .. }
            )
        ) || self.awaiting_save.is_some()
            || self.zotero.busy.is_some()
            || self.file_dialog_is_continuation()
    }

    pub(super) fn tab_has_unsaved_work(&self, index: usize) -> bool {
        let Some(slot) = self.tabs.get(index) else { return false };
        let (session, drafts) = slot.as_ref().map_or((&self.session, &self.comment_drafts), |t| (&t.session, &t.comment_drafts));
        session.dirty
            || drafts.has_work(session.document_id())
            || self.pending_restores.iter().any(|(id, _, _)| *id == session.document_id())
            || self.saves_in_flight.iter().any(|s| s.document == session.document_id())
    }

    pub(super) fn document_session_mut(&mut self, document: u64) -> Option<&mut Session> {
        if self.session.document_id() == document {
            return Some(&mut self.session);
        }
        self.tabs.iter_mut().flatten().find(|t| t.session.document_id() == document).map(|t| &mut t.session)
    }

    pub(super) fn tab_index(&self, p: &Value, field: &str) -> Result<usize, String> {
        if field == "index"
            && let Some(id) = p.get("id")
        {
            let id = id.as_u64().ok_or("id must be an unsigned integer")?;
            return self.tab_ids.iter().position(|&t| t == id).ok_or_else(|| "unknown document tab id".into());
        }
        let n = p
            .get(field)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| format!("{field} must be a nonnegative integer"))?;
        if n >= self.tabs.len() {
            return Err("document tab index is out of range".into());
        }
        Ok(n)
    }

    pub(super) fn tab_command(&mut self, id: &str, p: &Value, user: bool) -> Result<Value, String> {
        match id {
            "ui.documentTabs" => Ok(json!({"active": self.active_tab, "activeId": self.active_document_id(),
                "tabs": self.document_tabs().iter().map(|t| json!({"id": t.id, "index": t.index, "name": t.name,
                    "dirty": t.dirty, "active": t.active, "pinned": t.pinned})).collect::<Vec<_>>()})),
            "ui.switchDocument" => {
                let index = self.tab_index(p, "index")?;
                if !self.switch_document_tab(index) {
                    return Err("finish the document dialog or operation before switching tabs".into());
                }
                Ok(json!({"active": self.active_tab, "id": self.active_document_id()}))
            }
            "ui.nextDocument" | "ui.previousDocument" => {
                if self.document_tabs().len() < 2 {
                    return Ok(json!({"changed": false}));
                }
                let n = self.tabs.len();
                let index = (self.active_tab + if id == "ui.nextDocument" { 1 } else { n - 1 }) % n;
                self.tab_command("ui.switchDocument", &json!({"index": index}), user)
            }
            "ui.closeDocument" | "ui.closeOtherDocuments" | "ui.closeAllDocuments" => {
                let index = if id == "ui.closeAllDocuments" || (user && p.get("id").is_none() && p.get("index").is_none()) {
                    self.active_tab
                } else {
                    self.tab_index(p, "index")?
                };
                let targets: Vec<_> = (0..self.tabs.len())
                    .filter(|&i| match id {
                        "ui.closeDocument" => i == index,
                        "ui.closeOtherDocuments" => i != index && !self.pinned_tabs[i],
                        _ => true,
                    })
                    .collect();
                if !user && p.get("force").and_then(Value::as_bool) != Some(true) && targets.iter().any(|&i| self.tab_has_unsaved_work(i)) {
                    return Err("documents have unsaved work; save first or pass force: true".into());
                }
                if self.document_modal() {
                    return Err("finish the document dialog or operation before closing tabs".into());
                }
                if !user {
                    let ids: Vec<_> = targets.iter().filter_map(|&i| self.tab_ids.get(i).copied()).collect();
                    for id in ids {
                        if let Some(i) = self.tab_ids.iter().position(|&t| t == id) {
                            if !self.switch_document_tab(i) {
                                return Err("unable to switch document".into());
                            }
                            self.execute("file.close", p.clone())?;
                        }
                    }
                } else {
                    match id {
                        "ui.closeDocument" => return self.close_document_tab(index),
                        "ui.closeOtherDocuments" => {
                            self.close_other_document_tabs(index);
                        }
                        _ => {
                            self.close_all_document_tabs();
                        }
                    }
                }
                Ok(json!({"pending": !self.pending_closes.is_empty() || self.dialog.is_some()}))
            }
            "ui.reopenClosedDocument" => self.reopen_closed_document_tab(),
            "ui.pinDocument" => {
                let index = if user && p.get("id").is_none() && p.get("index").is_none() { self.active_tab } else { self.tab_index(p, "index")? };
                let pinned = if user && p.get("pinned").is_none() {
                    !self.pinned_tabs[index]
                } else {
                    p.get("pinned").and_then(Value::as_bool).ok_or("pinned must be a boolean")?
                };
                if !self.pin_document_tab(index, pinned) {
                    return Err("unable to pin tab".into());
                }
                Ok(json!({"active": self.active_tab}))
            }
            "ui.moveDocument" => {
                let from = self.tab_index(p, "from")?;
                let to = self.tab_index(p, "to")?;
                if !self.reorder_document_tab(from, to) {
                    return Err("unable to reorder tabs across pinned groups".into());
                }
                Ok(json!({"active": self.active_tab}))
            }
            "ui.newWindow" => {
                let launch = self.services.document_window.as_ref().ok_or("separate windows require the desktop host")?;
                launch()?;
                Ok(json!({"opened": true}))
            }
            _ => Err("unknown tab command".into()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn enable_recovery(&mut self, root: std::path::PathBuf, wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        self.session.enable_recovery(root.clone(), wake.clone())?;
        for tab in self.tabs.iter_mut().flatten() {
            tab.session.enable_recovery(root.clone(), wake.clone())?;
        }
        self.recovery_config = Some((root, wake));
        Ok(())
    }

    pub fn finish_recovery(&mut self) {
        self.session.finish_recovery();
        for tab in self.tabs.iter_mut().flatten() {
            tab.session.finish_recovery();
        }
    }
}

const COMMANDS: &[(&str, &str, &str)] = &[
    ("ui.documentTabs", "Document Tabs", "{}"),
    ("ui.switchDocument", "Switch Document", "{id: stable tab id} or {index: display index}"),
    ("ui.closeDocument", "Close Tab", "{id|index, force?: bool}"),
    ("ui.closeOtherDocuments", "Close Other Tabs", "{id|index, force?: bool}; pinned tabs stay open"),
    ("ui.closeAllDocuments", "Close All Tabs", "{force?: bool}"),
    ("ui.reopenClosedDocument", "Reopen Closed Tab", "{}"),
    ("ui.pinDocument", "Pin Tab", "{id|index, pinned: bool}"),
    ("ui.moveDocument", "Move Tab", "{from: display index, to: display index}"),
    ("ui.nextDocument", "Next Document", "{}"),
    ("ui.previousDocument", "Previous Document", "{}"),
    ("ui.newWindow", "New Window", "{}"),
];

pub(crate) fn commands(app: &WordApp) -> Vec<Value> {
    COMMANDS
        .iter()
        .map(|(id, label, params)| {
            json!({"id": id, "label": label, "location": "Window",
        "params": params, "enabled": *id != "ui.newWindow" || app.services.document_window.is_some(),
        "shortcut": shortcut(app, id)})
        })
        .collect()
}

pub(crate) fn shortcut(app: &WordApp, id: &str) -> String {
    let key = match id {
        "ui.nextDocument" => "Ctrl+Tab",
        "ui.previousDocument" => "Ctrl+Shift+Tab",
        "ui.reopenClosedDocument" => "Ctrl+Shift+T",
        _ => return String::new(),
    };
    let canonical = if cfg!(target_os = "macos") { key.to_string() } else { key.replacen("Ctrl+", "Mod+", 1) };
    if app.session.keymap.removed.contains(&canonical) || app.session.keymap.assigned.contains_key(&canonical) {
        return String::new();
    }
    let bound = app.session.keymap.resolve(&app.session.registry, &canonical);
    if bound.is_some_and(|s| !(s.id == "text.insertTab" && id != "ui.reopenClosedDocument")) {
        return String::new();
    }
    key.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }
    fn edit(a: &mut WordApp, text: &str) {
        a.run("text.insert", json!({"text": text})).unwrap();
    }

    #[test]
    fn stable_ids_survive_moves_and_malformed_indexes_never_close_work() {
        let mut a = app();
        edit(&mut a, "keep me");
        let first = a.document_tabs()[0].id;
        a.run("file.new", json!({})).unwrap();
        a.run("ui.moveDocument", json!({"from": 0, "to": 1})).unwrap();
        a.execute("ui.switchDocument", json!({"id": first})).unwrap();
        assert!(a.session.dirty);
        for value in [json!(-1), json!(1.2), json!("0"), json!(u64::MAX), Value::Null] {
            assert!(a.execute("ui.closeDocument", json!({"index": value, "force": true})).is_err());
            assert!(a.execute("ui.moveDocument", json!({"from": value, "to": 0})).is_err());
        }
        assert!(a.execute("ui.closeDocument", json!({"id": first})).is_err());
        assert!(a.execute("ui.closeAllDocuments", json!({})).is_err());
        assert!(a.execute("ui.confirmQuitTab", json!({})).is_err());
        assert_eq!(a.document_tabs().len(), 2);
    }

    #[test]
    fn switching_tabs_interrupts_the_previous_input_method_target() {
        let mut a = app();
        edit(&mut a, "first");
        a.run("file.new", json!({})).unwrap();
        a.canvas.ime_preedit = "unfinished composition".into();
        a.canvas.ime_preedit_range = Some(0..2);
        assert!(a.switch_document_tab(0));
        assert!(a.canvas.ime_interrupt_now && a.canvas.ime_interrupted_preedit);
        assert!(a.canvas.ime_preedit.is_empty());
        assert!(a.switch_document_tab(1));
        assert!(a.canvas.ime_preedit.is_empty());
        assert!(a.canvas.ime_preedit_range.is_none());
        assert!(a.canvas.ime_interrupt_now);
    }

    #[test]
    fn first_open_replaces_start_placeholder_and_last_close_keeps_empty_window() {
        let mut a = app();
        a.start_screen();
        assert!(a.document_tabs().is_empty());
        a.run("file.new", json!({})).unwrap();
        assert_eq!(a.document_tabs().len(), 1);
        a.restrict.open = true;
        a.restrict.asking = true;
        a.run("file.close", json!({})).unwrap();
        assert!(!a.has_document() && !a.quit_requested);
        assert!(!a.restrict.open && !a.restrict.asking);
        assert!(a.workspace_snapshot().tabs.is_empty());
        a.run("file.new", json!({})).unwrap();
        assert_eq!(a.document_tabs().len(), 1);
    }

    #[test]
    fn repeated_window_close_and_cancel_preserve_the_entire_workspace() {
        let mut a = app();
        edit(&mut a, "first");
        a.run("file.new", json!({})).unwrap();
        edit(&mut a, "second");
        assert!(!a.close_requested());
        let prompt = serde_json::to_value(&a.dialog).unwrap();
        assert!(!a.close_requested());
        assert_eq!(serde_json::to_value(&a.dialog).unwrap(), prompt);
        assert!(a.run("file.new", json!({})).is_err());
        assert!(!a.switch_document_tab(1));
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert!(!a.quit_requested);
        assert_eq!(a.document_tabs().len(), 2);
        assert!(a.document_tabs().iter().all(|t| t.dirty));
    }

    #[test]
    fn preferences_follow_existing_and_new_tabs_without_resetting_zoom() {
        let mut a = app();
        a.session.view.zoom = 1.8;
        a.run("file.new", json!({})).unwrap();
        a.session.author = "Tabs author".into();
        a.session.initials = "TA".into();
        a.session.prefs.check_spelling = false;
        a.session.view.zoom = 0.7;
        assert!(a.switch_document_tab(0));
        assert_eq!(a.session.view.zoom, 1.8);
        assert_eq!(a.prefs().author, "Tabs author");
        assert!(!a.prefs().editing.check_spelling);
        a.run("file.new", json!({})).unwrap();
        assert_eq!(a.prefs().initials, "TA");
        assert!(!a.prefs().editing.check_spelling);
    }

    #[test]
    fn inactive_async_failure_restores_its_own_dirty_state() {
        for outcome in [async_saves::SaveOutcome::Cancelled, async_saves::SaveOutcome::Failed("disk full".into())] {
            let mut a = app();
            a.empty_window();
            a.run("file.new", json!({})).unwrap();
            let saves = async_saves::AsyncSaves::default();
            a.services.async_saves = Some(saves.clone());
            let download_saves = saves.clone();
            a.services.download = Some(Box::new(move |_, _| {
                download_saves.start();
                Ok(())
            }));
            edit(&mut a, "original");
            let ticket = a.run("file.save", json!({"path": "first.docx"})).unwrap()["pendingSave"].as_u64().unwrap();
            a.run("file.new", json!({})).unwrap();
            assert!(a.document_tabs()[0].dirty, "pending saves count as unsaved");
            saves.finish(ticket, outcome);
            a.settle_async_saves();
            assert!(!a.session.dirty);
            assert!(a.switch_document_tab(0));
            assert!(a.session.dirty);
        }
    }

    #[test]
    fn pending_save_as_saves_originating_tab_and_picture_pick_cancels_after_switch() {
        let dir = std::env::temp_dir().join(format!("wordcraft-tab-picker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("original.docx");
        let mut a = app();
        edit(&mut a, "source writing");
        let (tx, rx) = std::sync::mpsc::channel();
        let rx = std::cell::RefCell::new(Some(rx));
        a.services.file_dialog = Some(Box::new(move |_| rx.borrow_mut().take().unwrap()));
        a.save_as_dialog();
        a.run("file.new", json!({})).unwrap();
        edit(&mut a, "other writing");
        tx.send(Ok(vec![path.to_string_lossy().into_owned()])).unwrap();
        a.poll_file_dialog().unwrap().unwrap();
        assert_eq!(a.active_document_tab(), 1);
        assert!(a.session.dirty && a.session.path.is_none());
        assert!(a.switch_document_tab(0));
        assert!(!a.session.dirty && a.session.path.as_ref() == Some(&path));
        let (tx, rx) = std::sync::mpsc::channel();
        let rx = std::cell::RefCell::new(Some(rx));
        a.services.file_dialog = Some(Box::new(move |_| rx.borrow_mut().take().unwrap()));
        a.ask_file(file_dialogs::FileDialogRequest::Open { purpose: "picture".into() }, file_dialogs::AfterPick::Picture);
        assert!(a.switch_document_tab(1));
        tx.send(Ok(vec!["/not/a/picture.png".into()])).unwrap();
        assert!(a.poll_file_dialog().unwrap().unwrap_err().contains("no longer active"));
        assert!(a.session.dirty && a.session.path.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn automation_quit_cannot_bypass_inactive_unsaved_work() {
        let mut a = app();
        edit(&mut a, "keep");
        a.run("file.new", json!({})).unwrap();
        let ctx = egui::Context::default();
        let request = control::ControlRequest::new("app.quit", json!({})).0;
        let control::Outcome::Done(result) = control::handle(&mut a, &ctx, &request) else { panic!("expected reply") };
        assert_eq!(result["ok"], false);
        assert!(!a.quit_requested);
        assert!(a.execute("file.close", json!({"quit": true})).is_err());
    }
}

impl WordApp {
    pub(super) fn tick_recovery(&mut self, ctx: &egui::Context) {
        let pending = std::mem::take(&mut self.pending_restores);
        for (document, revision, params) in pending {
            if let Some(session) = self.document_session_mut(document) {
                if session.rev() != revision {
                    self.status(tl!("Recovery cancelled: the document changed while the copy was loading."));
                    continue;
                }
                if session.recovery_ready() && !session.recovery_busy() {
                    if let Err(error) = session.run("file.recover", &params) {
                        self.status(error.to_string());
                    }
                } else {
                    self.pending_restores.push((document, revision, params));
                }
            }
        }
        self.session.recovery_tick();
        for tab in self.tabs.iter_mut().flatten() {
            tab.session.recovery_tick();
        }
        let waits: Vec<_> = self.tabs.iter().flatten().filter_map(|t| t.session.recovery_wait()).collect();
        for wait in waits {
            ctx.request_repaint_after(wait);
        }
        if let Some(error) = self
            .session
            .recovery_error()
            .map(str::to_string)
            .or_else(|| self.tabs.iter().flatten().find_map(|t| t.session.recovery_error().map(str::to_string)))
        {
            self.status(error);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod recovery_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn snapshots(root: &std::path::Path) -> std::io::Result<usize> {
        let mut count = 0;
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            if entry.path().is_dir() {
                for file in std::fs::read_dir(entry.path())? {
                    if file?.file_name().to_string_lossy().starts_with("snapshot-") {
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    #[test]
    fn inactive_recovery_survives_abrupt_process_termination() {
        if let Some(root) = std::env::var_os("WORDCRAFT_TAB_RECOVERY_CHILD") {
            let root = std::path::PathBuf::from(root);
            let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
            a.enable_recovery(root.join("recovery"), std::sync::Arc::new(|| {})).unwrap();
            a.run("text.insert", json!({"text": "First inactive recovery draft"})).unwrap();
            a.run("file.new", json!({})).unwrap();
            a.run("text.insert", json!({"text": "Second inactive recovery draft"})).unwrap();
            a.run("file.new", json!({})).unwrap();
            a.run("text.insert", json!({"text": "Encrypted draft must not be plaintext"})).unwrap();
            a.run("file.encrypt", json!({"password": "synthetic-test-only"})).unwrap();
            let ctx = egui::Context::default();
            loop {
                a.tick_recovery(&ctx);
                if snapshots(&root.join("recovery")).unwrap() == 2 {
                    std::fs::write(root.join("ready"), "ready").unwrap();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let root = std::env::temp_dir().join(format!("wordcraft-tab-crash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "document_tabs::recovery_tests::inactive_recovery_survives_abrupt_process_termination", "--nocapture"])
            .env("WORDCRAFT_TAB_RECOVERY_CHILD", &root)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let end = Instant::now() + Duration::from_secs(25);
        while !root.join("ready").exists() && Instant::now() < end {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let ready = root.join("ready").exists();
        let _ = child.kill();
        child.wait().unwrap();
        assert!(ready, "both inactive sessions must snapshot before termination");
        assert_eq!(snapshots(&root.join("recovery")).unwrap(), 2, "encrypted writing never reaches plaintext recovery");
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.enable_recovery(root.join("recovery"), std::sync::Arc::new(|| {})).unwrap();
        let ctx = egui::Context::default();
        let end = Instant::now() + Duration::from_secs(5);
        while a.session.recovery_documents().len() != 2 {
            a.tick_recovery(&ctx);
            assert!(Instant::now() < end, "terminated sessions must be discoverable");
            std::thread::sleep(Duration::from_millis(5));
        }
        a.run("text.insert", json!({"text": "Current writing must survive restoration"})).unwrap();
        let id = a.session.recovery_documents()[0].id.clone();
        a.run("file.recover", json!({"restore": id})).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while !a.session.dirty {
            a.tick_recovery(&ctx);
            assert!(Instant::now() < end, "restoration must complete");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("inactive recovery draft"));
        assert!(a.session.path.is_none() && !a.saved_here());
        assert!(a.switch_document_tab(0));
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Current writing must survive restoration");
        a.finish_recovery();
        drop(a);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod close_and_save_tests {
    use super::*;

    #[test]
    fn close_all_includes_pins_and_cancel_stops_remaining_documents() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.pin_document_tab(0, true);
        a.run("file.new", json!({})).unwrap();
        a.run("text.insert", json!({"text": "first"})).unwrap();
        a.run("file.new", json!({})).unwrap();
        a.run("text.insert", json!({"text": "second"})).unwrap();
        a.run("ui.closeAllDocuments", json!({})).unwrap();
        assert_eq!(a.document_tabs().len(), 2, "the clean tab closed first");
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert!(a.pending_closes.is_empty() && !a.quit_requested);
        assert!(a.document_tabs().iter().all(|t| !t.pinned), "Close All also closed the clean pinned tab");
        a.run("ui.closeAllDocuments", json!({})).unwrap();
        for _ in 0..2 {
            a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        }
        assert!(a.document_tabs().is_empty() && !a.has_document() && !a.quit_requested);
    }

    #[test]
    fn save_as_cannot_take_another_tabs_destination() {
        let root = std::env::temp_dir().join(format!("wordcraft-tab-collision-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("original.docx");
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("text.insert", json!({"text": "original"})).unwrap();
        a.run("file.save", json!({"path": path})).unwrap();
        a.run("file.new", json!({})).unwrap();
        a.run("text.insert", json!({"text": "second"})).unwrap();
        assert!(a.run("file.saveAs", json!({"path": root.join(".").join("original.docx")})).is_err());
        assert!(a.session.dirty && a.session.path.is_none());
        assert_eq!(wordcraft_engine::io::open_path(&path).unwrap().plain_text(wordcraft_doc::StoryRef::Body), "original");
        a.run("file.open", json!({"path": path})).unwrap();
        assert_eq!(a.document_tabs().len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dropping_an_open_file_keeps_its_existing_autosave_authority() {
        #[derive(Debug)]
        struct Dropped(std::path::PathBuf);
        impl egui::DroppedFile for Dropped {
            fn path(&self) -> &std::path::Path {
                &self.0
            }
            fn bytes(&self) -> Result<Vec<u8>, String> {
                std::fs::read(&self.0).map_err(|e| e.to_string())
            }
        }
        let root = std::env::temp_dir().join(format!("wordcraft-drop-authority-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("original.docx");
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("text.insert", json!({"text": "saved"})).unwrap();
        a.run("file.save", json!({"path": path})).unwrap();
        let document = a.session.document_id();
        a.run("text.insert", json!({"text": "unsaved edit"})).unwrap();
        a.run("file.new", json!({})).unwrap();
        a.open_dropped_files(vec![std::sync::Arc::new(Dropped(path.clone()))]);
        assert_eq!(a.session.document_id(), document);
        assert_eq!(a.autosave_path.as_ref(), Some(&path));
        assert!(a.autosaves());
        assert!(a.session.dirty);
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("unsaved edit"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_successful_export_does_not_confirm_a_failed_document_save() {
        for outcome in [async_saves::SaveOutcome::Cancelled, async_saves::SaveOutcome::Failed("disk full".into())] {
            let saves = async_saves::AsyncSaves::default();
            let hook = saves.clone();
            let services = Services {
                download: Some(Box::new(move |_, _| {
                    hook.start();
                    Ok(())
                })),
                async_saves: Some(saves.clone()),
                ..Default::default()
            };
            let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
            a.run("text.insert", json!({"text": "unsaved writing"})).unwrap();
            let document = a.session.document_id();
            let save = a.run("file.save", json!({})).unwrap()["pendingSave"].as_u64().unwrap();
            let export = a.run("file.exportPdf", json!({})).unwrap()["pendingSave"].as_u64().unwrap();
            a.run("file.new", json!({})).unwrap();
            saves.finish(export, async_saves::SaveOutcome::Saved);
            a.settle_async_saves();
            saves.finish(save, outcome);
            a.settle_async_saves();
            assert!(a.document_session_mut(document).unwrap().dirty);
            assert!(!a.confirmed_saves.contains_key(&document));
            assert!(a.switch_document_tab(0));
            assert!(a.execute("file.close", json!({})).is_err());
            assert_eq!(a.document_tabs().len(), 2);
        }
    }

    #[test]
    fn late_failure_of_an_older_save_does_not_unsave_a_confirmed_newer_revision() {
        let saves = async_saves::AsyncSaves::default();
        let hook = saves.clone();
        let services = Services {
            download: Some(Box::new(move |_, _| {
                hook.start();
                Ok(())
            })),
            async_saves: Some(saves.clone()),
            ..Default::default()
        };
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        a.run("text.insert", json!({"text": "first"})).unwrap();
        let old = a.run("file.save", json!({})).unwrap()["pendingSave"].as_u64().unwrap();
        a.run("text.insert", json!({"text": "newer"})).unwrap();
        let new = a.run("file.save", json!({})).unwrap()["pendingSave"].as_u64().unwrap();
        saves.finish(new, async_saves::SaveOutcome::Saved);
        a.settle_async_saves();
        saves.finish(old, async_saves::SaveOutcome::Failed("old picker failed".into()));
        a.settle_async_saves();
        assert!(!a.session.dirty);
        assert!(!a.save_unconfirmed());
    }
}

impl WordApp {
    pub(super) fn cancel_close_sequence(&mut self) {
        self.closing_all = false;
        self.quit_reviewed.clear();
        self.quit_original_tab = None;
        self.pending_closes.clear();
        self.pending_keep_tab = None;
    }
}

#[cfg(test)]
mod picker_cancel_tests {
    use super::*;

    #[test]
    fn cancelled_save_as_stops_close_all_and_allows_quit_retry() {
        let services = Services { pick_save: Some(Box::new(|_| None)), ..Default::default() };
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        a.run("text.insert", json!({"text": "first"})).unwrap();
        a.run("file.new", json!({})).unwrap();
        a.run("text.insert", json!({"text": "second"})).unwrap();
        a.run("ui.closeAllDocuments", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert!(a.pending_closes.is_empty() && !a.closing_all);
        assert_eq!(a.document_tabs().len(), 2);
        assert!(!a.close_requested());
        assert!(matches!(a.dialog, Some(dialogs::Dialog::SaveChanges { .. })));
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert!(!a.quit_requested);
    }
}

#[cfg(test)]
mod close_during_picker_tests {
    use super::*;

    #[test]
    fn os_close_does_not_replace_a_pending_save_as_continuation() {
        let (tx, rx) = std::sync::mpsc::channel();
        let rx = std::cell::RefCell::new(Some(rx));
        let services = Services { file_dialog: Some(Box::new(move |_| rx.borrow_mut().take().unwrap())), ..Default::default() };
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        a.run("text.insert", json!({"text": "draft"})).unwrap();
        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert!(a.file_dialog_is_continuation() && a.dialog.is_none());
        assert!(!a.close_requested());
        assert!(!a.close_requested());
        assert!(a.file_dialog_is_continuation() && a.dialog.is_none());
        assert!(!a.closing_all);
        tx.send(Ok(Vec::new())).unwrap();
        a.poll_file_dialog().unwrap().unwrap();
        assert!(a.session.dirty && a.has_document() && !a.quit_requested);
    }
}

#[cfg(test)]
mod overflow_tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn overflow_strip_reveals_the_selected_document() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        for index in 0..12 {
            a.run("file.new", json!({})).unwrap();
            a.run("file.properties", json!({"title": format!("Synthetic long document title number {index}")})).unwrap();
        }
        let label = format!("{} — Unsaved changes", a.document_tabs().last().unwrap().name);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(780.0, 600.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            a,
        );
        h.run_steps(10);
        assert!(h.ctx.content_rect().contains(h.get_by_label(&label).rect().center()));
        h.state_mut().run("ui.switchDocument", json!({"index": 1})).unwrap();
        h.run_steps(10);
        let first = "Synthetic long document title number 0 — Unsaved changes";
        assert!(h.ctx.content_rect().contains(h.get_by_label(first).rect().center()));
    }
}

#[cfg(test)]
mod workspace_limit_tests {
    use super::*;

    #[test]
    fn workspace_restoration_caps_processing_and_reports_skipped_entries() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.ui.workspace.tabs = vec![WorkspaceTab::default(); 140];
        assert_eq!(a.restore_workspace(), 128);
        assert_eq!(a.document_tabs().len(), 128);
        assert!(a.status_msg.as_ref().is_some_and(|(message, _)| message.contains("128-tab limit")));
        assert!(a.run("file.new", json!({})).is_err());
        assert_eq!(a.document_tabs().len(), 128);
    }
}

#[cfg(test)]
mod interrupted_quit_tests {
    use super::*;

    #[test]
    fn forced_close_during_quit_allows_window_and_close_all_retry() {
        for window_close in [true, false] {
            let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
            for text in ["first", "second", "third"] {
                if text != "first" {
                    a.run("file.new", json!({})).unwrap();
                }
                a.run("text.insert", json!({"text": text})).unwrap();
            }
            assert!(!a.close_requested());
            a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
            assert_eq!(a.active_document_tab(), 1);
            assert!(a.execute("file.close", json!({})).is_err());
            assert!(a.closing_all && a.dialog.is_some());
            a.execute("file.close", json!({"force": true})).unwrap();
            assert_eq!(a.document_tabs().len(), 2);
            assert!(a.document_tabs().iter().all(|t| t.dirty));
            assert!(!a.quit_requested);
            if window_close {
                assert!(!a.close_requested());
            } else {
                a.run("ui.closeAllDocuments", json!({})).unwrap();
            }
            assert!(matches!(a.dialog, Some(dialogs::Dialog::SaveChanges { document, .. }) if document == a.session.document_id()));
            assert_eq!(a.active_document_tab(), 0, "previously reviewed dirty tabs must be reviewed again");
            a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
            assert_eq!(a.document_tabs().len(), 2);
            assert!(a.document_tabs().iter().all(|t| t.dirty));
            assert!(!a.closing_all && a.pending_closes.is_empty());
        }
    }
}

#[cfg(test)]
mod review_preferences_tests {
    use super::*;

    #[test]
    fn codex_review_new_tabs_keep_effective_editing_language() {
        for language in ["pl-PL", "en-AU"] {
            let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
            a.session.set_editing_language(Some(language));
            a.run("text.insert", json!({"text": "original"})).unwrap();
            a.session.doc.styles.default_chr.lang = Some("de-DE".into());
            a.run("file.new", json!({})).unwrap();
            assert_eq!(a.session.editing_lang.as_deref(), Some(language));
            assert_eq!(a.session.doc.styles.default_chr.lang.as_deref(), Some(language));
            for extension in ["txt", "md"] {
                a.run("file.open", json!({"path": format!("synthetic.{extension}"), "data": "SGVsbG8="})).unwrap();
                assert_eq!(a.session.editing_lang.as_deref(), Some(language));
                assert_eq!(a.session.doc.styles.default_chr.lang.as_deref(), Some(language));
            }
            assert!(a.switch_document_tab(0));
            assert_eq!(a.session.doc.styles.default_chr.lang.as_deref(), Some("de-DE"), "switching must preserve the document language");
        }
    }

    #[test]
    fn codex_review_new_tabs_persist_custom_dictionary_changes() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let root = std::env::temp_dir().join(format!("wordcraft-tab-dictionary-{}-{}", std::process::id(), a.session.document_id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(Session::USER_DICT_FILE);
        a.session.user_dict_path = Some(path.clone());
        a.run("file.new", json!({})).unwrap();
        a.run("review.addToDictionary", json!({"word": "codextabpersistword"})).unwrap();
        let persisted = std::fs::read_to_string(&path);
        a.run("file.options", json!({"removeWords": ["codextabpersistword"]})).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(persisted.unwrap().lines().any(|w| w == "codextabpersistword"));
        assert!(a.switch_document_tab(0));
        assert_eq!(a.session.user_dict_path, Some(path));
    }
}

#[cfg(test)]
mod web_picker_review_tests {
    use super::*;
    use std::{
        cell::RefCell,
        rc::Rc,
        sync::mpsc::{Sender, channel},
    };
    type Replies = Rc<RefCell<Vec<Sender<PickedFiles>>>>;

    fn browser_app(replies: Replies) -> WordApp {
        let services = Services {
            open_async: Some(Box::new(move |_| {
                let (tx, rx) = channel();
                replies.borrow_mut().push(tx);
                rx
            })),
            inbox: Some(Inbox::default()),
            ..Default::default()
        };
        WordApp::new(Session::new(wordcraft_doc::Document::new()), services)
    }

    #[test]
    fn late_picker_result_cannot_adopt_a_new_tabs_origin() {
        let replies = Replies::default();
        let mut a = browser_app(replies.clone());
        a.pick_recipient_list();
        a.run("file.new", json!({})).unwrap();
        a.pick_recipient_list();
        assert_eq!(replies.borrow().len(), 1, "a second picker cannot replace a pending operation");
        let old = replies.borrow_mut().remove(0);
        old.send(vec![("old.txt".into(), b"OldOwner\nPrivate\n".to_vec())]).unwrap();
        a.drain_inbox();
        assert!(a.session.merge.headers.is_empty());
        assert!(a.status_msg.as_ref().is_some_and(|(message, _)| message.contains("no longer active")));
        assert!(!a.file_dialog_open() && !a.recipient_list_pending);
        a.pick_recipient_list();
        assert!(old.send(vec![("late.csv".into(), b"Stale\nPrivate\n".to_vec())]).is_err(), "extra batches cannot leak to the next picker");
        replies.borrow_mut().remove(0).send(vec![("new.txt".into(), b"NewOwner\nChosen\n".to_vec())]).unwrap();
        a.drain_inbox();
        assert_eq!(a.session.merge.headers, ["NewOwner"]);
        assert!(a.switch_document_tab(0));
        assert!(a.session.merge.headers.is_empty());
    }

    #[test]
    fn a_pending_picker_does_not_claim_unrelated_dropped_documents() {
        let replies = Replies::default();
        let mut a = browser_app(replies.clone());
        a.pick_recipient_list();
        a.run("file.new", json!({})).unwrap();
        a.services.inbox.as_ref().unwrap().lock().unwrap().push(("notes.txt".into(), b"Independent drop".to_vec()));
        a.drain_inbox();
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Independent drop"));
        assert!(a.file_dialog_open());
        replies.borrow_mut().remove(0).send(Vec::new()).unwrap();
        a.drain_inbox();
        assert!(!a.file_dialog_open() && !a.recipient_list_pending);
    }

    #[test]
    fn cancelled_or_disconnected_picker_does_not_poison_later_results() {
        for disconnect in [true, false] {
            let replies = Replies::default();
            let mut a = browser_app(replies.clone());
            a.pick_recipient_list();
            let tx = replies.borrow_mut().remove(0);
            if !disconnect {
                tx.send(Vec::new()).unwrap();
            }
            drop(tx);
            a.drain_inbox();
            assert!(!a.file_dialog_open() && !a.recipient_list_pending);
            a.services.inbox.as_ref().unwrap().lock().unwrap().push(("notes.txt".into(), b"After cancellation".to_vec()));
            a.drain_inbox();
            assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("After cancellation"));
            a.pick_recipient_list();
            replies.borrow_mut().remove(0).send(vec![("valid.txt".into(), b"Name\nAda\n".to_vec())]).unwrap();
            a.drain_inbox();
            assert_eq!(a.session.merge.headers, ["Name"]);
        }
    }

    #[test]
    fn all_files_in_a_picture_picker_batch_keep_their_origin() {
        let replies = Replies::default();
        let mut a = browser_app(replies.clone());
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255])))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        a.pick_picture();
        a.run("file.new", json!({})).unwrap();
        let files = vec![("first.png".into(), png.clone()), ("second.png".into(), png.clone())];
        replies.borrow_mut().remove(0).send(files.clone()).unwrap();
        a.drain_inbox();
        assert!(a.session.doc.media.is_empty());
        assert!(a.switch_document_tab(0));
        assert!(a.session.doc.media.is_empty());
        a.pick_picture();
        replies.borrow_mut().remove(0).send(files).unwrap();
        a.drain_inbox();
        assert!(!a.session.doc.media.is_empty(), "an unchanged origin accepts its picture batch");
    }
}

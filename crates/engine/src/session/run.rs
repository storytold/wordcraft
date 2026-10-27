use super::doc_state::{ColumnBlock, Selection};
use super::snapshot_types::RunSnapshot;
use super::*;

impl Session {
    /// Whether the command changed nothing: the document, the selection, the formatting
    /// waiting at the caret and the column selection are as they were before it ran.
    ///
    /// A command that changes nothing is not an edit: it adds no undo step, marks no
    /// unsaved changes, forces no relayout and leaves the redo stack alone (#23).
    fn unchanged(&self, doc: &Document, sel: &Selection, column: &Option<ColumnBlock>, pending: &Option<CharProps>) -> bool {
        doc.same_content(&self.doc) && *sel == self.sel && *column == self.column && *pending == self.pending
    }

    /// Run a command by id with JSON params (the single entry point for the UI, CLI, MCP and
    /// control channel). Mutating commands are undoable; a failed command leaves the document as
    /// it was; a panic inside a command becomes an error.
    pub fn run(&mut self, id: &str, params: &Value) -> Result<Value, CmdError> {
        // The document the command starts in, and the first id a document it opens can have.
        let home = self.document_id;
        let mark = document_id_mark();
        // Every dispatch starts from an unclassified outcome. Early returns (unknown, disabled,
        // protected) leave it at the default.
        self.ime_edit_outcome = ImeEditOutcome::default();
        // A rolled-back transaction must not leave a successful application-change signal from a
        // nested command that ran before the failure.
        let pending_application_change_before = self.pending_ime_application_change;
        let join = std::mem::take(&mut self.join_next);
        // The equation replacement promise is not a generic undo join: only the command that
        // actually types into the equation may consume it. Any other command discards it.
        let equation_join = std::mem::take(&mut self.equation_join_next);
        // The body replacement promise is likewise scoped: only the `text.insert` at the
        // selection the deletion left may consume it.
        let ime_text_join = self.ime_text_join.take();
        let reg = self.registry.clone();
        let Some(spec) = reg.get(id) else { return Err(CmdError::Unknown(id.to_string())) };
        let join = join
            || (equation_join && matches!(id, "equation.type" | "equation.backspace"))
            || (id == "text.insert" && ime_text_join.as_ref().is_some_and(|sel| sel == &self.sel));
        if let Some(why) = (spec.enabled)(self) {
            return Err(CmdError::Disabled(format!("{id}: {why}")));
        }
        // Restrict Editing: refused outright, or allowed as long as it only edits the exceptions.
        let guard = if spec.mutates { crate::cmd::protect::check_before(self, id, params)? } else { false };
        // Quick Parts aren't part of the document, so a refused edit puts them back itself (the Spike).
        let blocks_before = guard.then(|| self.building_blocks.clone());
        if spec.mutates && self.doc.settings.protection.as_deref() == Some("trackedChanges") {
            self.doc.settings.track_changes = true;
        }
        // Macro recording and Repeat.
        let record = !matches!(id, "tools.recordMacro" | "tools.macros" | "edit.undo" | "edit.redo" | "edit.repeat")
            && (spec.mutates || id.starts_with("caret.") || id.starts_with("select."));
        // A password never lands in a recorded macro or Repeat.
        let kept = || {
            let mut p = params.clone();
            if let Some(m) = p.as_object_mut() {
                m.remove("password");
            }
            p
        };
        if record && let Some((_, steps)) = self.recording.as_mut() {
            steps.push((id.to_string(), kept()));
        }
        if spec.mutates && !matches!(id, "edit.undo" | "edit.redo" | "edit.repeat") {
            self.last_command = Some((id.to_string(), kept()));
        }
        // Typing, caret movement and selection outside the equation commands leave the equation.
        if self.math.is_some()
            && (id.starts_with("text.") || id.starts_with("caret.") || id.starts_with("select.") || id.starts_with("edit.paste"))
            && let Some(m) = self.math.take()
        {
            let off = m.at.off + wordcraft_doc::para::OBJ.len_utf8();
            self.sel = Selection::caret(Pos { off, ..m.at });
        }
        // The undo stacks are snapshotted whole, not as a length: commands run nested commands,
        // whose checkpoints, undos and redos (and evictions at the limit) must be put back too.
        let ime_epoch_before = spec.mutates.then_some(self.ime_body_epoch);
        // Commands may leave an equation (a whole-body replacement) even after they fail; put
        // the editing session back with the document.
        let math_before = spec.mutates.then(|| self.math.clone());
        let before_doc: Option<RunSnapshot> = if spec.mutates {
            Some((
                self.doc.clone(),
                self.sel.clone(),
                self.history.clone(),
                self.redo.clone(),
                self.typing_open,
                self.undo_evicted,
                self.column.clone(),
            ))
        } else {
            None
        };
        // The formatting waiting at the caret (⌘B with nothing selected): an undo step restores
        // it, so a command that only changes it is an edit.
        let pending_before = spec.mutates.then(|| self.pending.clone());
        // Which document is open. A command that replaces the document (a mail merge) opens a
        // new one, with a new undo history: it is not an edit of this document, whatever the
        // new document holds.
        let document_id_before = self.document_id;
        // Whether the document has unsaved changes, and the revision its layout is keyed on: a
        // command that changes nothing leaves both as they were, and a failed one leaves no
        // trace.
        let dirty_before = spec.mutates.then_some(self.dirty);
        let rev_restore = spec.mutates.then_some(self.rev);
        // These mutating commands can replace the open document and discard its saved versions.
        // Keep the versions until the command and the protected-region post-check have both
        // succeeded; rejected commands must leave the old document and its snapshots together.
        // `mailings.finish` with a path saves the merged copy there and leaves the open
        // document alone, so it replaces nothing.
        let replaces_document = matches!(id, "review.compare" | "review.combine" | "mailings.envelopes" | "mailings.labels")
            || (id == "mailings.finish" && crate::p::str(params, "path").is_none());
        let versions_before = replaces_document.then(|| self.versions.clone());
        let identity_before = replaces_document.then(|| {
            (
                self.path.clone(),
                self.file_name.clone(),
                self.document_id,
                self.password.clone(),
                self.dirty,
                self.bib_style.clone(),
                self.merge.preview,
            )
        });
        let typing = spec.id == "text.insert" || spec.id == "equation.type";
        if spec.mutates && !typing {
            self.typing_open = false;
        }
        // Whether this command pushed an undo step (a no-op command's step is dropped below,
        // leaving its undo group without one).
        let mut pushed = false;
        if spec.mutates {
            let label = if typing { "Typing" } else { spec.label };
            // A joined command merges into its group's step; a group whose first command
            // changed nothing has none, so the next command in it starts one (#4001).
            if !join || self.history.is_empty() || !self.join_anchor {
                pushed = self.checkpoint(label);
            }
        } else if !spec.id.starts_with("caret.") && !spec.id.starts_with("view.") {
            // Non-mutating commands other than caret movement keep the typing group.
        } else if spec.id.starts_with("caret.") {
            self.typing_open = false;
        }
        // Only the paste right after a tracked cut makes it a move.
        if (spec.mutates || matches!(id, "edit.undo" | "edit.undoTo" | "edit.redo")) && !matches!(id, "edit.cut" | "edit.paste" | "edit.pasteSpecial")
        {
            self.pending_move = None;
        }
        let sel_before = self.sel.clone();
        // A rejected command must not persist a new proofing default, including a rejection
        // by the protected-region check after its handler has succeeded. Undo stays document-only.
        let default_language_before = self.prefs.default_language.clone();
        let run = spec.run;
        let mutates = spec.mutates;
        let ime_caret_before = self.current_ime_caret_state();
        let rev_before = self.rev;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::cmd::column::dispatch(self, id, mutates, run, params)));
        let mut result = match result {
            Ok(r) => r,
            Err(p) => {
                let msg = p.downcast_ref::<&str>().map(ToString::to_string).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                Err(CmdError::Failed(format!("{id} panicked: {msg}")))
            }
        };
        if self.document_id != home || (result.is_err() && self.order.iter().any(|d| *d >= mark)) {
            return self.finish_elsewhere(home, mark, result, before_doc, blocks_before, default_language_before, pending_application_change_before);
        }
        // Restrict Editing: outside the exceptions nothing may change.
        if result.is_ok()
            && guard
            && let Some((before, ..)) = &before_doc
            && !crate::cmd::protect::outside_unchanged(before, &self.doc, &self.author)
        {
            result = Err(CmdError::Disabled(format!("{id}: only the editable regions of this protected document can change")));
        }
        match &result {
            Ok(_) => {
                let mut no_op = false;
                if spec.mutates {
                    // Formatting changed while tracking becomes a formatting revision.
                    if let Some((before, ..)) = &before_doc {
                        crate::cmd::fmt_revisions::record(self, id, before);
                    }
                    self.doc.ensure_nonempty();
                    self.doc.prune_text_boxes();
                    // Whether the command left the document, the selection and the formatting
                    // waiting at the caret as they were. A command that replaces the document —
                    // a mail merge, a comparison — is not an edit of this one, whatever the new
                    // document holds: `replaces_document` names the commands that may replace
                    // it, and `document_id` catches any other that does (through
                    // `set_document`, which starts a new history the old one must not overwrite).
                    let unchanged = |s: &Session| match (&before_doc, &pending_before) {
                        (Some((doc, sel, _, _, _, _, column)), Some(pending)) => {
                            !replaces_document && s.document_id == document_id_before && s.unchanged(doc, sel, column, pending)
                        }
                        _ => false,
                    };
                    let changed = !unchanged(self);
                    if changed {
                        // The layout the connectors are rerouted against is the new one.
                        self.touch();
                    }
                    // Glued connectors follow the shapes they join.
                    let rerouted = crate::cmd::connectors::reroutes_after(id) && crate::cmd::connectors::reroute(self);
                    if rerouted {
                        self.touch();
                    }
                    // A command that changed nothing is not an edit (#23): it adds no undo
                    // step, marks no unsaved changes, forces no relayout and leaves the redo
                    // stack alone. What it left behind is compared with what it started from,
                    // after the reroute: a reroute that moved a connector is an edit of its own.
                    no_op = !changed && !rerouted && unchanged(self);
                    if no_op {
                        // The step the dispatch pushed is not an edit: put the stacks back as
                        // they were, so the redo stack survives and no step is evicted. The
                        // group it belonged to is left without a step of its own, so the next
                        // command in it starts one.
                        if let Some((_, _, history, redo, typing_open, evicted, _)) = &before_doc {
                            self.history = history.clone();
                            self.redo = redo.clone();
                            self.typing_open = *typing_open;
                            self.undo_evicted = *evicted;
                        }
                        if let Some(dirty) = dirty_before {
                            self.dirty = dirty;
                        }
                        if let Some(rev) = rev_restore {
                            let touched = self.rev;
                            self.restore_rev(rev);
                            crate::cmd::draw::rekey_lasso(self, touched, rev);
                        }
                        if pushed {
                            self.join_anchor = false;
                        }
                    }
                }
                self.clamp_selection();
                let mut ime_outcome = self.ime_edit_outcome;
                if ime_outcome.content_changed {
                    ime_outcome.state_changed = true;
                }
                // An unclassified mutating command is conservatively a state change. It can make
                // a frame Other, but it can never mint a receipt because `classified` is false.
                // A command that changed nothing is not an edit (#23), so it is not an IME
                // state change either: otherwise Backspace at the document start would still
                // signal an application change and move the settled IME revision.
                if !ime_outcome.classified && mutates && !no_op {
                    ime_outcome.state_changed = true;
                }
                ime_outcome.selection_changed = self.current_ime_caret_state() != ime_caret_before;
                self.ime_edit_outcome = ime_outcome;
                if ime_outcome.state_changed {
                    let caret = self.current_ime_caret_state();
                    self.ime_state_seen = Some(caret);
                    self.reset_ime_state_revision();
                }
                if !self.ime_origin && !no_op && (ime_outcome.content_changed || ime_outcome.state_changed || (!ime_outcome.classified && mutates)) {
                    self.pending_ime_application_change = true;
                }
                if sel_before.focus.story == StoryRef::Body && self.in_header_footer() {
                    self.hf_return = Some(sel_before.clone());
                }
                // A selection command that moves the insertion point ends the typing group, like
                // caret movement: typing, Select, typing are two undo steps (#419).
                if !spec.mutates && self.sel != sel_before {
                    self.typing_open = false;
                }
                if spec.mutates && !no_op && !matches!(id, "edit.undo" | "edit.redo") {
                    self.note_edit();
                }
                // Outline view: collapsed headings follow edits; the caret stays where it shows.
                crate::cmd::outline::after_command(self, &sel_before, self.rev != rev_before);
                // Extra selected objects last until something else is edited or selected; formatting
                // them keeps them selected, as in Word.
                if !matches!(id, "select.addObject" | "arrange.ungroup" | "shape.fill" | "shape.effects") && (spec.mutates || self.sel != sel_before)
                {
                    self.also_selected.clear();
                }
                self.note_typing(id, before_doc.as_ref().map(|b| &b.0), &sel_before);
            }
            Err(e) => {
                self.ime_edit_outcome = ImeEditOutcome::default();
                self.pending_ime_application_change = pending_application_change_before;
                self.autocap = None;
                self.prefs.default_language = default_language_before;
                if let Some(epoch) = ime_epoch_before {
                    self.ime_body_epoch = epoch;
                }
                if let Some(math) = math_before {
                    self.math = math;
                }
                if let Some(versions) = versions_before {
                    self.versions = versions;
                }
                if let Some((path, file_name, document_id, password, dirty, bib_style, merge_preview)) = identity_before {
                    self.path = path;
                    self.file_name = file_name;
                    self.document_id = document_id;
                    self.password = password;
                    self.dirty = dirty;
                    self.bib_style = bib_style;
                    self.merge.preview = merge_preview;
                }
                if let Some((d, s, h, r, t, ev, c)) = before_doc {
                    self.doc = d;
                    self.sel = s;
                    self.history = h;
                    self.redo = r;
                    self.typing_open = t;
                    // The steps the command pushed out are back, so they no longer count as gone.
                    self.undo_evicted = ev;
                    self.column = c;
                    // Its step is gone with them: its undo group has none to merge into.
                    if pushed {
                        self.join_anchor = false;
                    }
                }
                if let Some(dirty) = dirty_before {
                    self.dirty = dirty;
                }
                if let Some(rev) = rev_restore {
                    self.restore_rev(rev);
                }
                if let Some(b) = blocks_before {
                    self.building_blocks = b;
                }
                self.status = e.to_string();
            }
        }
        result
    }

    /// The end of [`Session::run`] for a command that ended up in another document than it
    /// started in (it opened, made, switched to or closed one) or that failed after opening one.
    /// The checks that compare the document before and after the command don't apply.
    /// Succeeded: the first document loses the undo step made for the command when its text is
    /// as it was (opening a document is no edit to undo there). Failed: the documents the command opened close again and the first document is
    /// active and as it was.
    fn finish_elsewhere(
        &mut self,
        home: u64,
        mark: u64,
        result: Result<Value, CmdError>,
        before_doc: Option<RunSnapshot>,
        blocks_before: Option<std::collections::BTreeMap<String, Fragment>>,
        default_language_before: Option<String>,
        pending_application_change_before: bool,
    ) -> Result<Value, CmdError> {
        // Another document is a state change the input method has to see, never an IME edit.
        self.ime_edit_outcome = ImeEditOutcome::default();
        match &result {
            Ok(_) => {
                self.put_back_parked(home, before_doc);
                self.clamp_selection();
                self.ime_state_seen = Some(self.current_ime_caret_state());
                self.reset_ime_state_revision();
                if !self.ime_origin {
                    self.pending_ime_application_change = true;
                }
            }
            Err(e) => {
                self.pending_ime_application_change = pending_application_change_before;
                self.autocap = None;
                self.prefs.default_language = default_language_before;
                if self.is_open(home) {
                    let _ = self.switch_to(home);
                    let opened: Vec<u64> = self.order.iter().copied().filter(|d| *d >= mark && *d != home).collect();
                    for d in opened {
                        let _ = self.close_document(d);
                    }
                    if let Some((d, s, h, r, t, ev, c)) = before_doc {
                        self.doc = d;
                        self.sel = s;
                        self.history = h;
                        self.redo = r;
                        self.typing_open = t;
                        self.undo_evicted = ev;
                        self.column = c;
                    }
                }
                if let Some(b) = blocks_before {
                    self.building_blocks = b;
                }
                self.status = e.to_string();
            }
        }
        result
    }
}

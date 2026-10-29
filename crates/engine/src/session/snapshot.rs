use super::doc_state::{MAX_UNDO, Selection, doc_bib_style};
use super::snapshot_types::{EditSnapshot, SNAPSHOT_HEAD};
use super::{Document, Session, StoryRef, next_document_id};

impl Session {
    /// Replace the document (open/new).
    pub fn set_document(&mut self, doc: Document) {
        self.autocap = None;
        let replaced = self.document_id;
        self.document_id = next_document_id();
        if let Some(at) = self.order.iter_mut().find(|d| **d == replaced) {
            *at = self.document_id;
        }
        self.note_body_replaced();
        self.bib_style = doc_bib_style(&doc);
        self.doc = doc;
        self.doc.ensure_nonempty();
        self.password = None;
        self.file_name = None;
        // The old document's equation is gone: its editing session cannot carry over.
        self.math = None;
        self.sel = Selection::caret(self.doc.start_of(StoryRef::Body));
        self.pending = None;
        self.typing = None;
        self.also_selected.clear();
        self.column = None;
        self.column_mode = false;
        self.widened = None;
        self.edit_marks.clear();
        self.new_header_parts.clear();
        self.hf_return = None;
        self.go_back_step = 0;
        self.view.outline.collapsed.clear();
        // A copy the replaced document kept stays in the recovery list.
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.recovery_slot = Default::default();
        }
        self.reset_history();
        self.clear_versions();
        self.touch();
        self.dirty = false;
        self.made_rev = self.rev;
        self.relayout();
    }

    /// A snapshot to put back with [`Session::restore`], for a caller that runs a command and
    /// may want to take it back without a trace (cheap: the document's blocks are shared).
    ///
    /// Covered: `doc`, `sel`, the undo stack (its length, and its oldest steps when it is near
    /// the limit), `redo`, the open typing group, the pending `join_next_undo`,
    /// `join_next_equation_edit` and body replacement promises, whether the typing group has a
    /// step of its own, and `dirty`. `rev`,
    /// `cache` and `layout` are derived (`restore` bumps `rev`). The other fields are not
    /// covered: a caller that lets the command change them (the view, the find state, the
    /// clipboard, macros, the file path…) saves and puts them back itself.
    ///
    /// `restore` is exact when the commands in between only add undo steps (every mutating
    /// command adds at most one; up to [`SNAPSHOT_HEAD`] may push old steps out of a full
    /// stack). Taking steps off the stack (`edit.undo`), clearing it (`file.open` into
    /// a blank document) or switching documents in between is not covered.
    ///
    /// The destructuring lists every field, so a new field does not build until someone decides
    /// whether a snapshot must cover it.
    pub fn edit_snapshot(&self) -> EditSnapshot {
        let Session {
            doc,
            sel,
            view: _,
            pending: _,
            last_highlight: _,
            path: _,
            file_name: _,
            // The custom dictionary file, like the file path: host wiring, not an edit.
            user_dict_path: _,
            // Like the dictionary: the person's theme sets, not part of the document.
            themes_dir: _,
            custom_themes: _,
            dirty,
            clipboard: _,
            // Like the clipboard: what the next paste can make a move of.
            pending_move: _,
            clipboard_text: _,
            clipboard_html: _,
            clip_history: _,
            find: _,
            goal_x: _,
            page_hint: _,
            registry: _,
            author: _,
            initials: _,
            painter: _,
            status: _,
            // Headers just created to edit: closing checks they still exist and are empty.
            new_header_parts: _,
            hf_return: _,
            history,
            undo_evicted,
            redo,
            typing_open,
            join_next,
            equation_join_next,
            ime_text_join,
            join_anchor,
            autocap: _,
            rev: _,
            protect_facts: _,
            cache: _,
            layout: _,
            // The word being typed, like `column`: valid only while `sel` is where typing left it.
            typing: _,
            originals: _,
            picture_looks: _,
            last_command: _,
            recording: _,
            macros: _,
            versions: _,
            building_blocks: _,
            autosave: _,
            bib_style: _,
            merge: _,
            ui_requests: _,
            document_id: _,
            // The untitled name and when the document was made: not edits.
            untitled: _,
            made_rev: _,
            // The other open documents: a snapshot covers the active one.
            others: _,
            order: _,
            untitled_count: _,
            ime_body_epoch,
            // The IME revision and its caret token are process-global evidence, not document
            // state: a rollback must not reuse a revision.
            ime_state_revision: _,
            ime_state_seen: _,
            ime_edit_outcome: _,
            #[cfg(any(test, feature = "test-support"))]
                ime_state_exhausted: _,
            pending_ime_application_change: _,
            ime_origin: _,
            read_aloud: _,
            prefs: _,
            system_region: _,
            keymap: _,
            ribbon: _,
            // A preference for new documents, like `prefs`: not part of this document.
            editing_lang: _,
            // Equation editing mode, like `view`: not part of the document or its history.
            math: _,
            math_latex: _,
            math_normal_text: _,
            // The save password, like the file path: not an edit to the document.
            password: _,
            #[cfg(not(target_arch = "wasm32"))]
                recovery: _,
            #[cfg(not(target_arch = "wasm32"))]
                recovery_slot: _,
            also_selected: _,
            // Column selection and a widened selection's ends, like `sel`'s shape: valid only while
            // `sel` matches them.
            column: _,
            column_mode: _,
            widened: _,
            // Go Back's places, like the caret history: not part of the document.
            edit_marks: _,
            go_back_step: _,
        } = self;
        let head = if history.len() + SNAPSHOT_HEAD > MAX_UNDO { history.iter().take(SNAPSHOT_HEAD).cloned().collect() } else { Vec::new() };
        EditSnapshot {
            doc: doc.clone(),
            sel: sel.clone(),
            history_len: history.len(),
            evicted: *undo_evicted,
            head,
            redo: redo.clone(),
            typing_open: *typing_open,
            join_next: *join_next,
            equation_join_next: *equation_join_next,
            ime_text_join: ime_text_join.clone(),
            join_anchor: *join_anchor,
            dirty: *dirty,
            ime_body_epoch: *ime_body_epoch,
        }
    }

    /// Put a snapshot back: the commands run since leave no trace in the document, the
    /// selection or undo/redo (undo steps the limit pushed out meanwhile come back). The layout
    /// is recomputed.
    pub fn restore(&mut self, snap: EditSnapshot) {
        let EditSnapshot {
            doc,
            sel,
            history_len,
            evicted,
            head,
            redo,
            typing_open,
            join_next,
            equation_join_next,
            ime_text_join,
            join_anchor,
            dirty,
            ime_body_epoch,
        } = snap;
        let gone = usize::try_from(self.undo_evicted.saturating_sub(evicted)).unwrap_or(usize::MAX);
        if gone > 0 {
            let back = gone.min(head.len());
            self.history.splice(0..0, head.into_iter().take(back));
        }
        self.undo_evicted = evicted;
        self.autocap = None;
        self.doc = doc;
        self.sel = sel;
        self.also_selected.clear();
        self.history.truncate(history_len);
        self.redo = redo;
        self.typing_open = typing_open;
        self.join_next = join_next;
        self.equation_join_next = equation_join_next;
        self.ime_text_join = ime_text_join;
        self.ime_body_epoch = ime_body_epoch;
        self.join_anchor = join_anchor;
        self.touch();
        self.dirty = dirty;
    }

    /// Make the selection valid for the current document.
    pub fn clamp_selection(&mut self) {
        self.sel.anchor = self.doc.clamp(&self.sel.anchor);
        self.sel.focus = self.doc.clamp(&self.sel.focus);
        // The equation being edited must still be there (undo, edits elsewhere).
        if let Some(m) = &self.math {
            let eq = self.doc.para_at(&m.at).and_then(|p| match p.object_at(m.at.off) {
                Some(wordcraft_doc::InlineObject::Equation { math, .. }) => Some(wordcraft_doc::math_edit::clamp(&math.nodes, &m.pos)),
                _ => None,
            });
            match eq {
                Some(pos) => {
                    if let Some(m) = self.math.as_mut() {
                        m.pos = pos;
                    }
                }
                None => self.math = None,
            }
        }
    }

    /// Why command `id` can't run now, for greying it out: its own `enabled` check, then what
    /// Restrict Editing refuses outright. `None` when it can run (or doesn't exist).
    pub fn why_disabled(&self, id: &str) -> Option<&'static str> {
        let spec = self.registry.get(id)?;
        (spec.enabled)(self).or_else(|| if spec.mutates { crate::cmd::protect::greyed_out(self, id, || self.protect_facts()) } else { None })
    }

    /// Put the revision back to `rev` after a command that changed nothing or failed. What the
    /// command cached at the revision it touched to is dropped: that number comes round again
    /// with the next edit, for a different document.
    pub(super) fn restore_rev(&mut self, rev: u64) {
        if self.rev == rev {
            return;
        }
        self.rev = rev;
        for slot in &mut self.layout {
            if slot.as_ref().is_some_and(|l| l.0 != rev) {
                *slot = None;
            }
        }
        if self.find.highlights.as_ref().is_some_and(|(r, _)| *r != rev) {
            self.find.highlights = None;
        }
        let facts = self.protect_facts.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner);
        if facts.as_ref().is_some_and(|((_, r, _), _)| *r != rev) {
            *facts = None;
        }
    }

    /// [`crate::cmd::protect::Facts`] for the document as it is now, cached per revision.
    fn protect_facts(&self) -> crate::cmd::protect::Facts {
        let key = (self.document_id, self.rev, self.author.clone());
        let mut cache = self.protect_facts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, f)) = cache.as_ref()
            && *k == key
        {
            return *f;
        }
        let f = crate::cmd::protect::Facts::of(self);
        *cache = Some((key, f));
        f
    }
}

//! Several open documents in one session. The active document lives in the session's own fields,
//! where every command works on it; the others wait as [`DocState`] values and are swapped in,
//! field by field, when they become active. Swapping moves the fields: nothing is cloned, laid
//! out again or thrown away, so a host may switch documents as often as it likes.

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde_json::json;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_layout::LayoutCache;

use super::{ColumnBlock, FindState, LayoutSlots, MathEdit, Selection, Session, TypedWord, Undo, ViewState, Widened, doc_bib_style};
use crate::CmdError;

/// Document ids are unique in the process, so an id an agent or a window holds never names
/// another document, even across sessions.
static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);

pub(super) fn next_document_id() -> u64 {
    NEXT_DOCUMENT_ID.fetch_add(1, Ordering::Relaxed)
}

/// The id the next document will get: every document made after this call has an id at least
/// this big.
pub(super) fn document_id_mark() -> u64 {
    NEXT_DOCUMENT_ID.load(Ordering::Relaxed)
}

/// An untitled document's name: `Document1`, `Document2`…
pub(super) fn untitled_name(n: u32) -> String {
    format!("Document{}", n.max(1))
}

/// An open document that isn't the active one: every per-document field of [`Session`], waiting
/// to be swapped back in. The session's app-wide state (clipboard, preferences, macros, Quick
/// Parts, the custom dictionary, Read Aloud…) is not here.
pub struct DocState {
    pub(super) doc: Document,
    pub(super) sel: Selection,
    pub(super) view: ViewState,
    pub(super) pending: Option<CharProps>,
    pub(super) path: Option<PathBuf>,
    pub(super) file_name: Option<String>,
    pub(super) dirty: bool,
    pub(super) pending_move: Option<crate::cmd::moves::PendingMove>,
    pub(super) find: FindState,
    pub(super) goal_x: Option<f32>,
    pub(super) page_hint: usize,
    pub(super) new_header_parts: Vec<u32>,
    pub(super) hf_return: Option<Selection>,
    pub(super) history: Vec<Arc<Undo>>,
    pub(super) undo_evicted: u64,
    pub(super) redo: Vec<Arc<Undo>>,
    pub(super) typing_open: bool,
    pub(super) join_next: bool,
    pub(super) equation_join_next: bool,
    pub(super) ime_text_join: Option<Selection>,
    pub(super) join_anchor: bool,
    pub(super) autocap: Option<crate::cmd::tools::AutoCap>,
    pub(super) rev: u64,
    pub(super) document_id: u64,
    pub(super) untitled: u32,
    pub(super) made_rev: u64,
    pub(super) ime_body_epoch: u64,
    pub(super) cache: LayoutCache,
    pub(super) layout: LayoutSlots,
    pub(super) typing: Option<TypedWord>,
    pub(super) originals: std::collections::HashMap<String, String>,
    pub(super) picture_looks: std::collections::HashMap<String, crate::cmd::objects::PictureLook>,
    pub(super) versions: Vec<(String, String, Document)>,
    pub(super) autosave: bool,
    pub(super) bib_style: String,
    pub(super) merge: crate::cmd::mailings::MergeState,
    pub(super) math: Option<MathEdit>,
    pub(super) password: Option<crate::io::Password>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) recovery_slot: crate::recovery::Slot,
    pub(super) also_selected: Vec<Pos>,
    pub(super) column: Option<ColumnBlock>,
    pub(super) column_mode: bool,
    pub(super) widened: Option<Widened>,
    pub(super) edit_marks: Vec<Pos>,
    pub(super) go_back_step: usize,
}

impl DocState {
    /// A fresh document state: caret at the start, no history, not dirty, a new id.
    pub(super) fn new(doc: Document, untitled: u32, view: ViewState) -> Self {
        let start = doc.start_of(StoryRef::Body);
        let bib_style = doc_bib_style(&doc);
        DocState {
            doc,
            sel: Selection::caret(start),
            view,
            pending: None,
            path: None,
            file_name: None,
            dirty: false,
            pending_move: None,
            find: FindState::default(),
            goal_x: None,
            page_hint: 0,
            new_header_parts: Vec::new(),
            hf_return: None,
            history: Vec::new(),
            undo_evicted: 0,
            redo: Vec::new(),
            typing_open: false,
            join_next: false,
            equation_join_next: false,
            ime_text_join: None,
            join_anchor: false,
            autocap: None,
            rev: 1,
            document_id: next_document_id(),
            untitled,
            made_rev: 1,
            ime_body_epoch: 0,
            cache: LayoutCache::new(),
            layout: Default::default(),
            typing: None,
            originals: Default::default(),
            picture_looks: Default::default(),
            versions: Vec::new(),
            autosave: true,
            bib_style,
            merge: Default::default(),
            math: None,
            password: None,
            #[cfg(not(target_arch = "wasm32"))]
            recovery_slot: Default::default(),
            also_selected: Vec::new(),
            column: None,
            column_mode: false,
            widened: None,
            edit_marks: Vec::new(),
            go_back_step: 0,
        }
    }

    /// The document's id ([`Session::document_id`] while it is active).
    pub fn id(&self) -> u64 {
        self.document_id
    }

    fn info(&self) -> DocInfo {
        DocInfo {
            id: self.document_id,
            title: title(self.path.as_deref(), self.file_name.as_deref(), &self.doc, self.untitled),
            path: self.path.clone(),
            dirty: self.dirty,
            active: false,
        }
    }
}

/// One open document, as [`Session::documents`] lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocInfo {
    /// The document's id: unique in the process, kept while the document is open.
    pub id: u64,
    /// The name the title bar shows: the file's name without its extension, else the name given
    /// with Rename, else the document's Title property, else `Document1`, `Document2`… (English;
    /// a front end names untitled documents in its own language).
    pub title: String,
    pub path: Option<PathBuf>,
    /// Changed since it was opened or last saved.
    pub dirty: bool,
    /// The active document, the one commands work on.
    pub active: bool,
}

fn title(path: Option<&FsPath>, file_name: Option<&str>, doc: &Document, untitled: u32) -> String {
    match (path, file_name) {
        (Some(p), _) => p.file_stem().map_or_else(|| p.to_string_lossy().to_string(), |s| s.to_string_lossy().to_string()),
        (None, Some(name)) => name.to_string(),
        (None, None) if !doc.core.title.is_empty() => doc.core.title.clone(),
        (None, None) => untitled_name(untitled),
    }
}

impl Session {
    /// Swap the active document's state with `d`'s: `d` then holds the document that was active.
    fn swap_doc(&mut self, d: &mut DocState) {
        use std::mem::swap;
        // Every field is listed, so a new one doesn't build until someone decides whether each
        // open document has its own (then it goes in `DocState` and is swapped below) or the
        // session shares it.
        let Session {
            // Per document.
            doc: _,
            sel: _,
            view: _,
            pending: _,
            path: _,
            file_name: _,
            dirty: _,
            pending_move: _,
            find: _,
            goal_x: _,
            page_hint: _,
            new_header_parts: _,
            hf_return: _,
            history: _,
            undo_evicted: _,
            redo: _,
            typing_open: _,
            join_next: _,
            equation_join_next: _,
            ime_text_join: _,
            join_anchor: _,
            autocap: _,
            rev: _,
            document_id: _,
            untitled: _,
            made_rev: _,
            ime_body_epoch: _,
            cache: _,
            layout: _,
            typing: _,
            originals: _,
            picture_looks: _,
            versions: _,
            autosave: _,
            bib_style: _,
            merge: _,
            math: _,
            password: _,
            #[cfg(not(target_arch = "wasm32"))]
                recovery_slot: _,
            also_selected: _,
            column: _,
            column_mode: _,
            widened: _,
            edit_marks: _,
            go_back_step: _,
            // Shared by every open document.
            user_dict_path: _,
            clipboard: _,
            clipboard_text: _,
            clipboard_html: _,
            clipboard_rtf: _,
            clip_history: _,
            registry: _,
            author: _,
            initials: _,
            painter: _,
            status: _,
            last_command: _,
            recording: _,
            macros: _,
            building_blocks: _,
            ui_requests: _,
            read_aloud: _,
            prefs: _,
            system_region: _,
            keymap: _,
            ribbon: _,
            editing_lang: _,
            math_latex: _,
            math_normal_text: _,
            last_highlight: _,
            // Keyed by document id and revision, so one cache serves every document.
            protect_facts: _,
            // The input method talks to the window, whichever document it shows; the IME
            // context is keyed by `document_id`.
            ime_state_revision: _,
            ime_state_seen: _,
            ime_edit_outcome: _,
            #[cfg(any(test, feature = "test-support"))]
                ime_state_exhausted: _,
            pending_ime_application_change: _,
            ime_origin: _,
            others: _,
            order: _,
            untitled_count: _,
            #[cfg(not(target_arch = "wasm32"))]
                recovery: _,
        } = self;
        let DocState {
            doc,
            sel,
            view,
            pending,
            path,
            file_name,
            dirty,
            pending_move,
            find,
            goal_x,
            page_hint,
            new_header_parts,
            hf_return,
            history,
            undo_evicted,
            redo,
            typing_open,
            join_next,
            equation_join_next,
            ime_text_join,
            join_anchor,
            autocap,
            rev,
            document_id,
            untitled,
            made_rev,
            ime_body_epoch,
            cache,
            layout,
            typing,
            originals,
            picture_looks,
            versions,
            autosave,
            bib_style,
            merge,
            math,
            password,
            #[cfg(not(target_arch = "wasm32"))]
            recovery_slot,
            also_selected,
            column,
            column_mode,
            widened,
            edit_marks,
            go_back_step,
        } = d;
        swap(&mut self.doc, doc);
        swap(&mut self.sel, sel);
        swap(&mut self.view, view);
        swap(&mut self.pending, pending);
        swap(&mut self.path, path);
        swap(&mut self.file_name, file_name);
        swap(&mut self.dirty, dirty);
        swap(&mut self.pending_move, pending_move);
        swap(&mut self.find, find);
        swap(&mut self.goal_x, goal_x);
        swap(&mut self.page_hint, page_hint);
        swap(&mut self.new_header_parts, new_header_parts);
        swap(&mut self.hf_return, hf_return);
        swap(&mut self.history, history);
        swap(&mut self.undo_evicted, undo_evicted);
        swap(&mut self.redo, redo);
        swap(&mut self.typing_open, typing_open);
        swap(&mut self.join_next, join_next);
        swap(&mut self.equation_join_next, equation_join_next);
        swap(&mut self.ime_text_join, ime_text_join);
        swap(&mut self.join_anchor, join_anchor);
        swap(&mut self.autocap, autocap);
        swap(&mut self.rev, rev);
        swap(&mut self.document_id, document_id);
        swap(&mut self.untitled, untitled);
        swap(&mut self.made_rev, made_rev);
        swap(&mut self.ime_body_epoch, ime_body_epoch);
        swap(&mut self.cache, cache);
        swap(&mut self.layout, layout);
        swap(&mut self.typing, typing);
        swap(&mut self.originals, originals);
        swap(&mut self.picture_looks, picture_looks);
        swap(&mut self.versions, versions);
        swap(&mut self.autosave, autosave);
        swap(&mut self.bib_style, bib_style);
        swap(&mut self.merge, merge);
        swap(&mut self.math, math);
        swap(&mut self.password, password);
        #[cfg(not(target_arch = "wasm32"))]
        swap(&mut self.recovery_slot, recovery_slot);
        swap(&mut self.also_selected, also_selected);
        swap(&mut self.column, column);
        swap(&mut self.column_mode, column_mode);
        swap(&mut self.widened, widened);
        swap(&mut self.edit_marks, edit_marks);
        swap(&mut self.go_back_step, go_back_step);
    }

    /// Make document `id` active without anything else a switch by the user does.
    pub(super) fn switch_to(&mut self, id: u64) -> Result<(), CmdError> {
        if id == self.document_id {
            return Ok(());
        }
        let at = self.others.iter().position(|d| d.document_id == id).ok_or_else(|| unknown(id))?;
        let mut d = self.others.swap_remove(at);
        self.swap_doc(&mut d);
        self.others.push(d);
        Ok(())
    }

    /// Take parked document `id` out of the session; false when it isn't parked here.
    fn remove_parked(&mut self, id: u64) -> bool {
        let Some(at) = self.others.iter().position(|d| d.document_id == id) else { return false };
        let d = self.others.swap_remove(at);
        self.order.retain(|d| *d != id);
        self.forget_recovery_copy(&d);
        true
    }

    /// Whether document `id` is open in this session (active or not).
    pub fn is_open(&self, id: u64) -> bool {
        id == self.document_id || self.others.iter().any(|d| d.document_id == id)
    }

    /// Every open document in the order they were opened, the active one included.
    pub fn documents(&self) -> Vec<DocInfo> {
        self.order
            .iter()
            .filter_map(|&id| {
                if id == self.document_id {
                    Some(DocInfo {
                        id,
                        title: title(self.path.as_deref(), self.file_name.as_deref(), &self.doc, self.untitled),
                        path: self.path.clone(),
                        dirty: self.dirty,
                        active: true,
                    })
                } else {
                    self.others.iter().find(|d| d.document_id == id).map(DocState::info)
                }
            })
            .collect()
    }

    /// Make document `id` the active one, the one commands work on (nothing happens when it
    /// already is). A Read Aloud reading of another document stops. An id that isn't open is an
    /// error.
    pub fn activate(&mut self, id: u64) -> Result<(), CmdError> {
        self.switch_to(id)?;
        self.stop_reading_elsewhere();
        Ok(())
    }

    /// Make document `id` the active one, as [`Session::activate`] does, but a Read Aloud
    /// reading of another document carries on: for a front end that shows each document in a
    /// window of its own, and makes each window's document active to draw it. An id that isn't
    /// open is an error.
    pub fn switch_document(&mut self, id: u64) -> Result<(), CmdError> {
        self.switch_to(id)
    }

    /// Open `doc` as another document and make it active; the one that was active stays open.
    /// The new document starts as a replaced one does ([`Session::set_document`]): caret at the
    /// start, no undo history, not dirty, no path, an untitled name (`Document2`), and the view
    /// the active document had. Returns its id.
    pub fn add_document(&mut self, doc: Document) -> u64 {
        self.untitled_count = self.untitled_count.saturating_add(1);
        self.add(doc, self.untitled_count)
    }

    /// [`Session::add_document`] for a document opened from a file: it doesn't use up an
    /// untitled name.
    pub(crate) fn add_opened_document(&mut self, doc: Document) -> u64 {
        self.add(doc, 0)
    }

    fn add(&mut self, mut doc: Document, untitled: u32) -> u64 {
        doc.ensure_nonempty();
        let mut view = self.view.clone();
        view.outline.collapsed.clear();
        let mut d = DocState::new(doc, untitled, view);
        let id = d.document_id;
        self.order.push(id);
        self.swap_doc(&mut d);
        self.others.push(d);
        self.stop_reading_elsewhere();
        id
    }

    /// A closed document's recovery copy goes with it, as its unsaved changes do.
    fn forget_recovery_copy(&mut self, d: &DocState) {
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(r), Some(id)) = (&mut self.recovery, &d.recovery_slot.id) {
            r.close(id.clone());
        }
        #[cfg(target_arch = "wasm32")]
        let _ = d;
    }

    /// What recovery reads from every open document, the active one first, with each
    /// document's slot to update. Nothing is cloned or laid out.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn recovery_docs(&mut self) -> impl Iterator<Item = crate::recovery::DocRecovery<'_>> {
        let active = crate::recovery::DocRecovery {
            id: self.document_id,
            wants_copy: self.dirty && self.password.is_none(),
            rev: self.rev,
            doc: &self.doc,
            path: self.path.as_deref(),
            slot: &mut self.recovery_slot,
        };
        std::iter::once(active).chain(self.others.iter_mut().map(|d| crate::recovery::DocRecovery {
            id: d.document_id,
            wants_copy: d.dirty && d.password.is_none(),
            rev: d.rev,
            doc: &d.doc,
            path: d.path.as_deref(),
            slot: &mut d.recovery_slot,
        }))
    }

    /// Every open document's recovery slot, with whether it should have a current copy and
    /// its revision.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn recovery_slots(&self) -> impl Iterator<Item = (bool, u64, &crate::recovery::Slot)> {
        std::iter::once((self.dirty && self.password.is_none(), self.rev, &self.recovery_slot))
            .chain(self.others.iter().map(|d| (d.dirty && d.password.is_none(), d.rev, &d.recovery_slot)))
    }

    /// Close document `id` without asking: its unsaved changes are lost (callers ask first),
    /// and so is its recovery copy.
    /// Closing the active document makes the one opened before it active (else the one after).
    /// Closing the last open document leaves a new blank one, so a session always has an
    /// active document; what a window does then is the front end's choice. An id that isn't
    /// open is an error.
    pub fn close_document(&mut self, id: u64) -> Result<(), CmdError> {
        if id != self.document_id {
            if !self.remove_parked(id) {
                return Err(unknown(id));
            }
            return Ok(());
        }
        let at = self.order.iter().position(|d| *d == id);
        let next = at.and_then(|i| i.checked_sub(1).and_then(|p| self.order.get(p)).or_else(|| self.order.get(i.saturating_add(1)))).copied();
        match next {
            Some(n) => {
                self.switch_to(n)?;
                self.remove_parked(id);
            }
            None => {
                let mut doc = crate::cmd::file::normal_document(self);
                doc.ensure_nonempty();
                self.untitled_count = self.untitled_count.saturating_add(1);
                let mut view = self.view.clone();
                view.outline.collapsed.clear();
                let mut fresh = DocState::new(doc, self.untitled_count, view);
                self.order = vec![fresh.document_id];
                self.swap_doc(&mut fresh);
                self.forget_recovery_copy(&fresh);
            }
        }
        self.stop_reading_elsewhere();
        Ok(())
    }

    /// The active document stands in for no document at all (a Start screen's, or what is left
    /// after closing the last one): an untouched blank document gives back its untitled name
    /// when it is the newest, so the first document a person sees is `Document1`, the next
    /// `Document2`.
    pub fn release_untitled_name(&mut self) {
        if self.untitled != 0 && self.untitled == self.untitled_count && self.is_untouched_blank() {
            self.untitled_count = self.untitled_count.saturating_sub(1);
            self.untitled = 0;
        }
    }

    /// Whether the active document is a blank one nobody has used: no file and no name, never
    /// changed, nothing to undo or redo and no text. Opening a file replaces such a document
    /// rather than opening beside it.
    pub fn is_untouched_blank(&self) -> bool {
        self.used_because().is_none()
    }

    /// Why the active document doesn't count as an untouched blank one, if it doesn't.
    pub(crate) fn used_because(&self) -> Option<&'static str> {
        if self.path.is_some() {
            Some("it has a file")
        } else if self.file_name.is_some() {
            Some("it has a name")
        } else if self.dirty {
            Some("it has unsaved changes")
        } else if !self.history.is_empty() || !self.redo.is_empty() {
            Some("it has undo or redo steps")
        } else if self.rev != self.made_rev {
            Some("it changed since it was made")
        } else if self.doc.paragraph_count() > 1 || !self.doc.plain_text(StoryRef::Body).trim().is_empty() {
            Some("it has text")
        } else {
            None
        }
    }

    /// Run `f` with document `id` active, then make the document that was active before active
    /// again (unless `f` closed it). For work addressed to a document other than the one a
    /// window shows: an agent's command, a result that arrives later. Read Aloud carries on.
    /// UI requests `f` makes (`ui_requests`) carry `"document": id` when that isn't the
    /// document that was active. An id that isn't open is an error and `f` doesn't run.
    pub fn with_document<R>(&mut self, id: u64, f: impl FnOnce(&mut Session) -> R) -> Result<R, CmdError> {
        let home = self.document_id;
        self.switch_to(id)?;
        // The queue drops its oldest entries when full, so `f`'s requests are told apart by
        // collecting them on their own rather than by position.
        let earlier = std::mem::take(&mut self.ui_requests);
        let r = f(self);
        let made = std::mem::replace(&mut self.ui_requests, earlier);
        for mut q in made {
            if id != home
                && let Some(o) = q.as_object_mut()
            {
                o.entry("document").or_insert(json!(id));
            }
            self.push_ui_request(q);
        }
        if self.document_id != home && self.is_open(home) {
            self.switch_to(home)?;
        }
        Ok(r)
    }

    /// The open document saved at `path`, if any: the same file reached by another spelling
    /// (`./a.docx`, a symbolic link) counts on desktop.
    pub fn find_open(&self, path: &FsPath) -> Option<u64> {
        let wanted = crate::io::canonical(path);
        let same = |p: &FsPath| p == path || wanted.is_some() && crate::io::canonical(p) == wanted;
        self.order.iter().copied().find(|&id| {
            if id == self.document_id {
                self.path.as_deref().is_some_and(same)
            } else {
                self.others.iter().find(|d| d.document_id == id).and_then(|d| d.path.as_deref()).is_some_and(same)
            }
        })
    }

    /// Stop a Read Aloud reading of a document that isn't the active one (it highlights its
    /// sentences in the document shown).
    fn stop_reading_elsewhere(&mut self) {
        if self.read_aloud.document.is_some_and(|d| d != self.document_id) && self.read_aloud.state() != crate::speech::State::Stopped {
            self.read_aloud.stop();
        }
    }

    /// Take the undo step `run` made for a command back out of the document it started in, when
    /// the command ended up in another document (it opened or made one) and left the first
    /// document's text as it was: opening a document is no edit to undo there.
    pub(super) fn put_back_parked(&mut self, id: u64, before: Option<super::RunSnapshot>) {
        let Some((doc, sel, history, redo, typing_open, evicted, column)) = before else { return };
        if !self.is_open(id) {
            return;
        }
        let _ = self.with_document(id, |s| {
            if s.doc != doc {
                return;
            }
            s.sel = sel;
            s.history = history;
            s.redo = redo;
            s.typing_open = typing_open;
            s.undo_evicted = evicted;
            s.column = column;
        });
    }
}

fn unknown(id: u64) -> CmdError {
    CmdError::Params(format!("no open document has id {id}"))
}

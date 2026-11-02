use crate::doc_state::{ColumnBlock, Selection, Undo};
use std::sync::Arc;

use wordcraft_doc::Document;
use wordcraft_layout::{DocLayout, NumeralMode, ViewMode};

/// What [`Session::restore`] needs to take commands back exactly: the document, the selection,
/// the undo and redo stacks (also the oldest undo steps the undo limit pushes out meanwhile), the
/// open typing group, the pending
/// [`Session::join_next_undo`]/[`Session::join_next_equation_edit`]/body IME replacement promises,
/// whether the group has a step of its own, and the dirty flag. See
/// [`Session::edit_snapshot`].
pub struct EditSnapshot {
    pub(super) doc: Document,
    pub(super) sel: Selection,
    pub(super) history_len: usize,
    /// `Session::undo_evicted` when the snapshot was taken.
    pub(super) evicted: u64,
    /// The oldest undo steps, kept only when the stack is near its limit (the commands that
    /// follow may push them out); at most [`SNAPSHOT_HEAD`].
    pub(super) head: Vec<Arc<Undo>>,
    pub(super) redo: Vec<Arc<Undo>>,
    pub(super) typing_open: bool,
    pub(super) join_next: bool,
    pub(super) equation_join_next: bool,
    /// The body `DeleteSurrounding` replacement promise then, with the exact caret it expects.
    pub(super) ime_text_join: Option<Selection>,
    pub(super) join_anchor: bool,
    pub(super) dirty: bool,
    /// The body's replacement epoch then: a previewed replacement that is rolled back is not a
    /// replacement after all.
    pub(super) ime_body_epoch: u64,
}

/// Undo steps that the commands run between [`Session::edit_snapshot`] and [`Session::restore`]
/// may push out of a full stack and still come back.
pub(super) const SNAPSHOT_HEAD: usize = 4;

impl EditSnapshot {
    /// The document when the snapshot was taken.
    pub fn doc(&self) -> &Document {
        &self.doc
    }
    /// The dirty flag then.
    pub fn dirty(&self) -> bool {
        self.dirty
    }
    /// The number of undo steps then.
    pub fn undo_depth(&self) -> usize {
        self.history_len
    }
}

/// What `run` keeps to take a failed mutating command back: the document, the selection, the
/// undo and redo stacks, the open typing group, the undo steps dropped so far and the column
/// block.
pub(super) type RunSnapshot = (Document, Selection, Vec<Arc<Undo>>, Vec<Arc<Undo>>, bool, u64, Option<ColumnBlock>);

/// A layout made and what it was made for: (revision, width, read height, view, read line
/// spacing, layout, proofing, outline, (deletions left out, deletions marked Hidden, body
/// deletions in balloons), numeral display, the word being typed, (hidden text shown, field
/// codes shown)).
pub(super) type LayoutEntry = (
    u64,
    f32,
    f32,
    ViewMode,
    Option<u32>,
    Arc<DocLayout>,
    bool,
    Option<wordcraft_layout::outline::OutlineView>,
    (bool, wordcraft_layout::HideDeleted, bool),
    NumeralMode,
    Option<wordcraft_layout::TypingWord>,
    (bool, bool),
);

/// The layouts last made, newest first. Two, so two windows on one document with different
/// views ([`WindowView`]) each find theirs as they draw in turn.
pub(super) type LayoutSlots = [Option<LayoutEntry>; 2];

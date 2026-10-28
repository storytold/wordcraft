//! Several windows on one document. The document, its undo history, dirty flag, path and save
//! state are shared; each window has its own [`WindowView`]: where its caret is and how it shows
//! the document. The session holds one view per document (the drawn window's, for the active
//! document); a front end keeps the other windows' views and moves them in and out with
//! [`Session::take_view`] and [`Session::put_view`] as it draws each window.

use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Pos, StoryRef};

use super::{ColumnBlock, MathEdit, Selection, Session, TypedWord, ViewState, Widened, next_math_session};

/// One window's view of a document: its selection and caret state, and how it shows the page
/// (zoom, view mode, panes, ruler…). Positions in it may be stale after edits made through
/// another window; [`Session::put_view`] makes them valid again.
pub struct WindowView {
    sel: Selection,
    view: ViewState,
    pending: Option<CharProps>,
    goal_x: Option<f32>,
    page_hint: usize,
    hf_return: Option<Selection>,
    typing_open: bool,
    join_next: bool,
    equation_join_next: bool,
    ime_text_join: Option<Selection>,
    typing: Option<TypedWord>,
    math: Option<MathEdit>,
    also_selected: Vec<Pos>,
    column: Option<ColumnBlock>,
    column_mode: bool,
    widened: Option<Widened>,
    /// The document and its revision when the view was taken out.
    document_id: u64,
    rev: u64,
}

impl WindowView {
    /// The document this view is of.
    pub fn document_id(&self) -> u64 {
        self.document_id
    }

    /// The view's selection.
    pub fn selection(&self) -> &Selection {
        &self.sel
    }

    /// How the view shows the document.
    pub fn view(&self) -> &ViewState {
        &self.view
    }
}

impl Session {
    /// Take the active document's view out of the session, for a window that stops being drawn.
    /// The session is left with a caret at the start of the document and the default view
    /// until [`Session::put_view`] puts a view back.
    pub fn take_view(&mut self) -> WindowView {
        use std::mem::{replace, take};
        let start = self.doc.start_of(StoryRef::Body);
        WindowView {
            sel: replace(&mut self.sel, Selection::caret(start)),
            view: take(&mut self.view),
            pending: self.pending.take(),
            goal_x: self.goal_x.take(),
            page_hint: take(&mut self.page_hint),
            hf_return: self.hf_return.take(),
            typing_open: take(&mut self.typing_open),
            join_next: take(&mut self.join_next),
            equation_join_next: take(&mut self.equation_join_next),
            ime_text_join: self.ime_text_join.take(),
            typing: self.typing.take(),
            math: self.math.take(),
            also_selected: take(&mut self.also_selected),
            column: self.column.take(),
            column_mode: take(&mut self.column_mode),
            widened: self.widened.take(),
            document_id: self.document_id,
            rev: self.rev,
        }
    }

    /// A copy of the active document's view, for a new window on it (View › New Window).
    pub fn copy_view(&self) -> WindowView {
        WindowView {
            sel: self.sel.clone(),
            view: self.view.clone(),
            pending: self.pending.clone(),
            goal_x: self.goal_x,
            page_hint: self.page_hint,
            hf_return: self.hf_return.clone(),
            // The new window hasn't typed: its first key starts an undo step of its own.
            typing_open: false,
            join_next: false,
            equation_join_next: false,
            ime_text_join: None,
            typing: None,
            // An equation editing session is an input method's owner: the new window's is its own.
            math: self.math.clone().and_then(|m| Some(MathEdit { session: next_math_session()?, ..m })),
            also_selected: self.also_selected.clone(),
            column: self.column.clone(),
            column_mode: self.column_mode,
            widened: self.widened.clone(),
            document_id: self.document_id,
            rev: self.rev,
        }
    }

    /// Make `v` the active document's view. When the document changed since `v` was taken
    /// (another window edited it, or `v` is of another document), its positions are clamped into
    /// the document, what only made sense for the text it was taken from (a column block, a
    /// widened table selection, more selected objects, the word being typed) is dropped, and the
    /// window's typing no longer joins its last undo step.
    pub fn put_view(&mut self, v: WindowView) {
        let WindowView {
            sel,
            view,
            pending,
            goal_x,
            page_hint,
            hf_return,
            typing_open,
            join_next,
            equation_join_next,
            ime_text_join,
            typing,
            math,
            also_selected,
            column,
            column_mode,
            widened,
            document_id,
            rev,
        } = v;
        let current = document_id == self.document_id && rev == self.rev;
        self.sel = sel;
        self.view = view;
        self.pending = pending;
        self.goal_x = goal_x;
        self.page_hint = page_hint;
        self.column_mode = column_mode;
        self.math = math;
        if current {
            self.hf_return = hf_return;
            self.typing_open = typing_open;
            self.join_next = join_next;
            self.equation_join_next = equation_join_next;
            self.ime_text_join = ime_text_join;
            self.typing = typing;
            self.also_selected = also_selected;
            self.column = column;
            self.widened = widened;
            return;
        }
        self.hf_return = hf_return.map(|s| Selection { anchor: self.doc.clamp(&s.anchor), focus: self.doc.clamp(&s.focus) });
        self.typing_open = false;
        self.join_next = false;
        self.equation_join_next = false;
        self.ime_text_join = None;
        self.typing = None;
        self.also_selected = Vec::new();
        self.column = None;
        self.widened = None;
        self.goal_x = None;
        self.clamp_selection();
    }
}

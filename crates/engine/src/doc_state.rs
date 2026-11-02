use serde::{Deserialize, Serialize};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Path, Pos, StoryRef, Table};

/// A selection: anchor (where it started) and focus (where the caret is).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Selection {
    pub anchor: Pos,
    pub focus: Pos,
}

impl Selection {
    pub fn caret(p: Pos) -> Self {
        Selection { anchor: p.clone(), focus: p }
    }
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }
    /// (start, end) in document order.
    pub fn ordered(&self) -> (Pos, Pos) {
        if self.anchor <= self.focus { (self.anchor.clone(), self.focus.clone()) } else { (self.focus.clone(), self.anchor.clone()) }
    }
}

/// The word being typed, left without a spelling squiggle until the caret leaves it (#1552).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TypedWord {
    /// Where the caret was when the word was typed.
    pub(super) at: Pos,
    /// The word's byte range in its paragraph.
    pub(super) start: usize,
    pub(super) end: usize,
}

/// A column (block) selection: the same x range on every line between two corners.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnBlock {
    /// The stream selection the block was made with (its corners). The block is current only
    /// while `Session::sel` still equals it, so any ordinary caret move or selection drops it.
    pub sel: Selection,
    /// Page x of the anchor and focus corners (points); the block spans the range between.
    pub anchor_x: f32,
    pub focus_x: f32,
    /// One `(start, end)` per line, in document order, each inside one paragraph.
    pub segments: Vec<(Pos, Pos)>,
    /// Set when the block is whole table cells (Select Cell, Row, Column or Table, or the mouse in
    /// a table's selection areas): one segment per paragraph of the cells.
    pub cells: Option<CellRange>,
    /// Set when the selection is separate pieces of text (Ctrl+drag, `select.add`, #788): each
    /// piece's `(start, end)`, sorted and apart; `segments` splits them at paragraph ends.
    pub pieces: Option<Vec<(Pos, Pos)>>,
}

/// Whole table cells selected: rows `rows.0..=rows.1` and every cell of them that reaches into
/// grid columns `cols.0..=cols.1`.
#[derive(Clone, Debug, PartialEq)]
pub struct CellRange {
    pub story: StoryRef,
    pub table: Path,
    pub rows: (usize, usize),
    pub cols: (usize, usize),
    /// The cells are one run of the text (cells of one row, or whole rows), so the stream
    /// selection between the corners holds just them and commands act on that as usual.
    /// Otherwise (part of a column) text edits and formatting go cell by cell.
    pub stream: bool,
}

impl CellRange {
    /// The selected cells of table `t` (row, cell index), in document order.
    pub fn cells(&self, t: &Table) -> Vec<(usize, usize)> {
        let ((r0, r1), (g0, g1)) = (self.rows, self.cols);
        let mut out = Vec::new();
        for (r, row) in t.rows.iter().enumerate().take(r1.saturating_add(1)).skip(r0) {
            let mut g = row.first_grid();
            for (c, cell) in row.cells.iter().enumerate() {
                let end = g.saturating_add(cell.span());
                if g <= g1 && end > g0 {
                    out.push((r, c));
                }
                g = end;
            }
        }
        out
    }
}

/// A selection Word's table rule widened to whole cells or rows (`cmd::table::extend_selection`),
/// and where it really runs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Widened {
    /// The selection it became; this is current while `Session::sel` still equals it.
    pub sel: Selection,
    /// Where the selection was really extended from and to: extending it again goes on from here.
    pub ends: Selection,
}

/// Find/replace state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FindState {
    pub query: String,
    pub replace: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Word's "Use wildcards" (#543).
    pub wildcards: bool,
    /// Take Find what literally: no `^` codes (`special: false`).
    pub literal: bool,
    pub match_prefix: bool,
    pub match_suffix: bool,
    pub ignore_punct: bool,
    pub ignore_space: bool,
    /// Matches from the last search: (start, end).
    #[serde(skip)]
    pub results: Vec<(Pos, Pos)>,
    pub current: usize,
    /// Reading Highlight: mark every match of `query` in the body.
    pub highlight: bool,
    /// Highlighted matches, for the document revision they were found in.
    #[serde(skip)]
    pub highlights: Option<(u64, Vec<(Pos, Pos)>)>,
}

#[derive(Clone)]
pub(super) struct Undo {
    pub(super) label: String,
    pub(super) doc: Document,
    pub(super) sel: Selection,
    /// Formatting waiting at the caret (Bold with nothing selected), so Undo takes it back too.
    pub(super) pending: Option<CharProps>,
    /// The column block or whole cells the selection was (undoing brings the selection back as
    /// it was).
    pub(super) column: Option<ColumnBlock>,
}

/// The number of edit locations Go Back cycles through (Word: four).
pub(super) const EDIT_MARKS: usize = 4;

/// The equation being edited.
#[derive(Clone, Debug, PartialEq)]
pub struct MathEdit {
    /// The equation object (its U+FFFC). This moves as the document is edited, so it is not an
    /// IME owner; [`Self::session`] is.
    pub at: Pos,
    /// The caret inside it.
    pub pos: wordcraft_doc::math_edit::MathPos,
    /// A stable token for this editing session, unique across documents and re-entry.
    pub session: u64,
}

/// Next stable token for an equation editing session, or `None` when the counter is exhausted.
///
/// Exhaustion fails closed: a repeated token could let a cached IME context outlive the
/// equation it described, so the caller refuses to start editing instead.
pub fn next_math_session() -> Option<u64> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.try_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |n| n.checked_add(1)).ok()
}

/// The citation style a document records (References › Style), APA when it records none or
/// one we don't know.
/// The byte range of the word the caret at `off` ends or sits in, counting apostrophes and
/// hyphens as the spelling check does; `None` unless a letter or digit comes before the caret.
pub(super) fn typed_range(text: &str, off: usize) -> Option<(usize, usize)> {
    let in_word = |c: char| wordcraft_doc::bidi::is_word_char(c) || matches!(c, '’' | '-');
    let before = text.get(..off)?;
    let start = before.char_indices().rev().take_while(|&(_, c)| in_word(c)).last().map(|(i, _)| i)?;
    let end = off.saturating_add(text.get(off..)?.chars().take_while(|&c| in_word(c)).map(char::len_utf8).sum::<usize>());
    text.get(start..off)?.chars().any(char::is_alphanumeric).then_some((start, end))
}

/// The name of `doc`'s bibliography style, APA when it names none we know.
pub(super) fn doc_bib_style(doc: &Document) -> String {
    doc.settings.bib_style.as_deref().and_then(crate::cmd::citations::find_style).map_or("APA", |st| st.name).to_string()
}

/// Most pending front-end requests a session keeps.
pub const MAX_UI_REQUESTS: usize = 64;

/// Maximum undo depth.
pub(super) const MAX_UNDO: usize = 500;

/// How many steps the Undo button's list shows (#701); `edit.undoTo` reaches no further.
pub const UNDO_LIST_MAX: usize = 100;

//! Visible logical text for IME surrounding-text queries.
//!
//! The platform input method works on what the reader sees, not on the stored run text: tracked
//! deletions are hidden, and an inline object stands for its plain-text result. The helpers here
//! build a bounded window around a selection without cloning the paragraph or the document, and
//! map both ends in both directions so a query and a delete use exactly the same text.

use std::ops::Range;

use crate::para::{OBJ, Paragraph};
use crate::{Document, Pos};

/// A bounded visible-logical window around a selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImeWindow {
    /// At most [`MAX_IME_TEXT_BYTES`] bytes of visible committed text.
    pub text: String,
    /// UTF-8 byte offset of the primary end in [`Self::text`].
    pub cursor: usize,
    /// UTF-8 byte offset of the other end in [`Self::text`].
    pub anchor: usize,
}

/// The largest visible window an IME query copies.
pub const MAX_IME_TEXT_BYTES: usize = 4000;

/// Which unit an input method's delete-surrounding counts use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteUnit {
    /// Unicode scalar values of the visible text.
    Chars,
    /// UTF-8 bytes of the visible text.
    Bytes,
}

/// A deletion request in the paragraph's stored byte offsets.
///
/// The selection itself is never part of `before` or `after`; both ranges are outside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImeDelete {
    /// Stored byte range to erase before the selection.
    pub before: Option<Range<usize>>,
    /// Stored byte range to erase after the selection.
    pub after: Option<Range<usize>>,
    /// Stored byte offset of the selection's anchor **before** the deletions.
    ///
    /// The ranges are returned in the paragraph's stored text as it is now; the caller applies
    /// them (and shifts these offsets past `before`) itself.
    pub anchor: usize,
    /// Stored byte offset of the selection's focus **before** the deletions.
    pub focus: usize,
}

/// One paragraph's visible logical text.
///
/// Deleted runs are skipped and inline objects contribute their plain-text result. Iteration is
/// lazy: no full visible copy is made.
pub struct VisibleParagraph<'a> {
    para: &'a Paragraph,
    deleted: Vec<Range<usize>>,
}

impl<'a> VisibleParagraph<'a> {
    /// A view of `para` with tracked deletions hidden.
    pub fn new(para: &'a Paragraph) -> Self {
        let mut deleted = para.deleted_ranges();
        deleted.sort_unstable_by_key(|r| r.start);
        Self { para, deleted }
    }

    /// Whether `off` falls inside a tracked deletion, advancing `next` past the runs that end
    /// before it.
    ///
    /// The ranges are sorted, so one monotone cursor makes a whole scan O(text + ranges) instead
    /// of O(text × ranges): alternating tracked revisions used to make every character scan every
    /// range.
    fn is_deleted_at(&self, next: &mut usize, off: usize) -> bool {
        while let Some(r) = self.deleted.get(*next) {
            if r.end > off {
                break;
            }
            *next = next.saturating_add(1);
        }
        self.deleted.get(*next).is_some_and(|r| r.start <= off && off < r.end)
    }

    /// Call `f` for every visible piece: `(stored byte start, piece, stored byte len)`.
    fn for_each_piece(&self, mut f: impl FnMut(usize, &str, usize)) {
        self.for_each_piece_while(|start, piece, stored| {
            f(start, piece, stored);
            true
        });
    }

    /// Call `f` for visible pieces until it returns `false`.
    fn for_each_piece_while(&self, mut f: impl FnMut(usize, &str, usize) -> bool) {
        let mut object = 0usize;
        let mut deleted = 0usize;
        for (i, c) in self.para.text.char_indices() {
            let is_object = c == OBJ;
            if is_object {
                object += 1;
            }
            if self.is_deleted_at(&mut deleted, i) {
                continue;
            }
            let keep_going = if is_object {
                let piece = self.para.objects.get(object - 1).map_or("", |o| o.plain_text());
                f(i, piece, OBJ.len_utf8())
            } else {
                let mut buf = [0u8; 4];
                f(i, c.encode_utf8(&mut buf), c.len_utf8())
            };
            if !keep_going {
                break;
            }
        }
    }

    /// Total visible UTF-8 bytes.
    pub fn visible_len(&self) -> usize {
        let mut len = 0usize;
        self.for_each_piece(|_, piece, _| len = len.saturating_add(piece.len()));
        len
    }

    /// Visible offset of a stored paragraph byte offset.
    ///
    /// `None` when the stored offset is not a character boundary, points inside a tracked
    /// deletion, or would split an inline object's plain-text result.
    pub fn visible_offset(&self, stored: usize) -> Option<usize> {
        if stored > self.para.text.len() || !self.para.text.is_char_boundary(stored) {
            return None;
        }
        let mut visible = 0usize;
        let mut found = false;
        let mut object = 0usize;
        let mut deleted = 0usize;
        for (i, c) in self.para.text.char_indices() {
            if i == stored {
                found = true;
                break;
            }
            if i > stored {
                break;
            }
            let is_object = c == OBJ;
            if is_object {
                object += 1;
            }
            if self.is_deleted_at(&mut deleted, i) {
                // A position at the start of a deleted run is still a visible position; one
                // inside it is not.
                continue;
            }
            if is_object {
                let piece = self.para.objects.get(object - 1).map_or("", |o| o.plain_text());
                visible = visible.saturating_add(piece.len());
            } else {
                visible = visible.saturating_add(c.len_utf8());
            }
        }
        if !found {
            // The end of the paragraph, or a deleted run at its end.
            let mut next = 0usize;
            return (!self.is_deleted_at(&mut next, stored)).then_some(visible);
        }
        if stored < self.para.text.len() && self.is_deleted_at(&mut deleted, stored) {
            // The start of a deleted run is still a visible caret position; only a position
            // strictly inside one is unrepresentable.
            let at_start = self.deleted.iter().any(|r| r.start == stored);
            return at_start.then_some(visible);
        }
        Some(visible)
    }

    /// Stored paragraph byte offset for a visible offset.
    ///
    /// `None` when the visible offset falls inside an inline object's plain-text result or is
    /// not a boundary of the visible representation.
    pub fn stored_offset(&self, visible: usize) -> Option<usize> {
        let mut acc = 0usize;
        let mut answer = None;
        let mut object = 0usize;
        let mut deleted = 0usize;
        for (i, c) in self.para.text.char_indices() {
            let is_object = c == OBJ;
            if is_object {
                object += 1;
            }
            if self.is_deleted_at(&mut deleted, i) {
                continue;
            }
            let piece_len = if is_object { self.para.objects.get(object - 1).map_or(0, |o| o.plain_text().len()) } else { c.len_utf8() };
            if visible == acc {
                answer = Some(i);
                break;
            }
            if visible < acc.saturating_add(piece_len) {
                // Inside a character (or an object's expansion): no stored boundary there.
                return None;
            }
            acc = acc.saturating_add(piece_len);
        }
        answer.or_else(|| (visible == acc).then_some(self.para.text.len()))
    }

    /// Stored offset for the **start** of a visible range, skipping zero-length objects at
    /// the same boundary.
    ///
    /// A content control's markers are inline objects with no plain text, so they share a
    /// visible offset with the content just inside (or after) them. A range start must map
    /// after a control-start marker and after a control-end marker, never onto the marker
    /// itself: an IME deletion must not break the control. Non-empty object expansions still
    /// map to the object's own stored offset, so a range that reaches one deletes it whole.
    pub fn stored_offset_after(&self, visible: usize) -> Option<usize> {
        let mut acc = 0usize;
        let mut object = 0usize;
        let mut deleted = 0usize;
        for (i, c) in self.para.text.char_indices() {
            let is_object = c == OBJ;
            if is_object {
                object += 1;
            }
            if self.is_deleted_at(&mut deleted, i) {
                continue;
            }
            let piece_len = if is_object { self.para.objects.get(object - 1).map_or(0, |o| o.plain_text().len()) } else { c.len_utf8() };
            if visible == acc && piece_len > 0 {
                return Some(i);
            }
            if visible < acc.saturating_add(piece_len) {
                return None;
            }
            acc = acc.saturating_add(piece_len);
        }
        (visible == acc).then_some(self.para.text.len())
    }

    /// Stored offset for a visible offset, preferring the position **before** a tracked
    /// deletion that starts there. Use this for the end of a range so a deletion run just
    /// after the caret is never drawn into it.
    pub fn stored_offset_before(&self, visible: usize) -> Option<usize> {
        let mut acc = 0usize;
        let mut object = 0usize;
        let mut deleted = 0usize;
        for (i, c) in self.para.text.char_indices() {
            if visible == acc {
                return Some(i);
            }
            let is_object = c == OBJ;
            if is_object {
                object += 1;
            }
            if self.is_deleted_at(&mut deleted, i) {
                continue;
            }
            let piece_len = if is_object { self.para.objects.get(object - 1).map_or(0, |o| o.plain_text().len()) } else { c.len_utf8() };
            if visible < acc.saturating_add(piece_len) {
                return None;
            }
            acc = acc.saturating_add(piece_len);
        }
        (visible == acc).then_some(self.para.text.len())
    }

    /// Round a visible byte offset forward to the next visible boundary (inward for a start).
    pub fn ceil_boundary(&self, visible: usize) -> usize {
        let mut acc = 0usize;
        let mut out = None;
        self.for_each_piece(|_, piece, _| {
            if out.is_some() {
                return;
            }
            if acc >= visible {
                out = Some(acc);
                return;
            }
            acc = acc.saturating_add(piece.len());
            if acc >= visible {
                out = Some(acc);
            }
        });
        out.unwrap_or(acc)
    }

    /// Round a visible byte offset backward to the previous visible boundary (inward for an end).
    pub fn floor_boundary(&self, visible: usize) -> usize {
        let mut acc = 0usize;
        let mut out = 0usize;
        self.for_each_piece(|_, piece, _| {
            let next = acc.saturating_add(piece.len());
            if next > visible {
                return;
            }
            acc = next;
            out = acc;
        });
        out
    }

    /// A bounded window with `cursor` and `anchor` as visible byte offsets.
    ///
    /// The selection is never split and the window never exceeds [`MAX_IME_TEXT_BYTES`]. A
    /// selection wider than that, an invalid offset, or an offset that cannot be represented as
    /// visible text returns `None` (unavailable, not fabricated).
    pub fn window(&self, cursor: usize, anchor: usize, max_bytes: usize) -> Option<ImeWindow> {
        // The public API caps its own budget: a caller cannot ask for an unbounded copy.
        let max_bytes = max_bytes.min(MAX_IME_TEXT_BYTES);
        let (sel_start, sel_end) = (cursor.min(anchor), cursor.max(anchor));
        let selection_len = sel_end - sel_start;
        if selection_len > max_bytes {
            return None;
        }
        // Spend the budget around the selection and stop walking at its end: the paragraph is
        // never copied and the work is bounded by the caret's distance from the paragraph start.
        let before = (max_bytes - selection_len) / 2;
        let window_start = sel_start.saturating_sub(before);
        let window_end = window_start.saturating_add(max_bytes);
        let mut text = String::with_capacity(window_end.saturating_sub(window_start).min(max_bytes));
        let mut first_kept = None;
        let mut acc = 0usize;
        self.for_each_piece_while(|_, piece, _| {
            for (k, c) in piece.char_indices() {
                let start = acc.saturating_add(k);
                let end = start.saturating_add(c.len_utf8());
                if end > window_end {
                    return false;
                }
                if start >= window_start {
                    if first_kept.is_none() {
                        first_kept = Some(start);
                    }
                    text.push(c);
                }
            }
            acc = acc.saturating_add(piece.len());
            acc < window_end
        });
        let Some(first) = first_kept else {
            // An empty document is an available empty context, never unavailable.
            return (cursor == 0 && anchor == 0).then_some(ImeWindow { text: String::new(), cursor: 0, anchor: 0 });
        };
        Some(ImeWindow { text, cursor: cursor.saturating_sub(first), anchor: anchor.saturating_sub(first) })
    }
}

impl Document {
    /// Bounded visible logical text around `cursor`/`anchor` in one paragraph.
    ///
    /// A selection across paragraphs or blocks is unrepresentable and returns `None`.
    pub fn ime_surrounding(&self, cursor: &Pos, anchor: &Pos, max_bytes: usize) -> Option<ImeWindow> {
        if cursor.story != anchor.story || cursor.path != anchor.path {
            return None;
        }
        let para = self.para_at(cursor)?;
        let view = VisibleParagraph::new(para);
        let c = view.visible_offset(cursor.off)?;
        let a = view.visible_offset(anchor.off)?;
        view.window(c, a, max_bytes.min(MAX_IME_TEXT_BYTES))
    }

    /// Delete-surrounding ranges in the stored paragraph text.
    ///
    /// Counts exclude the selection, tracked deletions and any pre-edit (which never lives in
    /// the model here). Byte counts that end inside a scalar leave it whole: a start rounds
    /// forward and an end rounds backward; a byte count beyond the visible text is refused.
    /// Character counts clamp at the paragraph's ends, as the web text agent's do. A range that
    /// cannot be represented in stored text returns `None` instead of guessing; so does a
    /// request with nothing to erase (`0` on both sides).
    pub fn ime_delete_surrounding(&self, cursor: &Pos, anchor: &Pos, before: usize, after: usize, unit: DeleteUnit) -> Option<ImeDelete> {
        if cursor.story != anchor.story || cursor.path != anchor.path {
            return None;
        }
        if before == 0 && after == 0 {
            // Nothing to erase: don't build the boundary table (or walk a long paragraph) to
            // discover it.
            return None;
        }
        let para = self.para_at(cursor)?;
        let view = VisibleParagraph::new(para);
        let c = view.visible_offset(cursor.off)?;
        let a = view.visible_offset(anchor.off)?;
        let (sel_start, sel_end) = (c.min(a), c.max(a));

        let (before_start, after_end) = match unit {
            DeleteUnit::Bytes => {
                let total = view.visible_len();
                if before > sel_start || after > total - sel_end {
                    return None;
                }
                (view.ceil_boundary(sel_start.saturating_sub(before)), view.floor_boundary(sel_end.saturating_add(after)))
            }
            DeleteUnit::Chars => char_boundaries(&view, sel_start, sel_end, before, after)?,
        };

        // Start a range after a deletion run and end it before one, so tracked deletions are
        // never part of the erased stored range.
        let before_start = view.stored_offset_after(before_start)?;
        let before_end = view.stored_offset_before(sel_start)?;
        let after_start = view.stored_offset_after(sel_end)?;
        let after_end = view.stored_offset_before(after_end)?;
        let before_range = (before_start < before_end).then_some(before_start..before_end);
        let after_range = (after_start < after_end).then_some(after_start..after_end);
        Some(ImeDelete { before: before_range, after: after_range, anchor: anchor.off, focus: cursor.off })
    }
}

/// The visible byte boundaries `before` characters back from `sel_start` and `after`
/// characters on from `sel_end`.
///
/// Two walks and constant memory replace the old per-character `Vec` that held every boundary
/// (a hostile count could make it allocate the whole paragraph). Character counts clamp at the
/// paragraph's ends; `None` when either selection end is not a boundary of the visible text.
fn char_boundaries(view: &VisibleParagraph<'_>, sel_start: usize, sel_end: usize, before: usize, after: usize) -> Option<(usize, usize)> {
    let mut acc = 0usize;
    let mut chars = 0usize;
    let mut start_index = None;
    let mut end_index = None;
    view.for_each_piece(|_, piece, _| {
        for (k, _) in piece.char_indices() {
            let at = acc.saturating_add(k);
            if at == sel_start {
                start_index = Some(chars);
            }
            if at == sel_end {
                end_index = Some(chars);
            }
            chars = chars.saturating_add(1);
        }
        acc = acc.saturating_add(piece.len());
    });
    if sel_start == acc {
        start_index = Some(chars);
    }
    if sel_end == acc {
        end_index = Some(chars);
    }
    let start_index = start_index?;
    let end_index = end_index?;
    let target_before = start_index.saturating_sub(before);
    let target_after = end_index.saturating_add(after).min(chars);

    let mut acc = 0usize;
    let mut index = 0usize;
    let mut before_start = None;
    let mut after_end = (target_after == 0).then_some(0);
    view.for_each_piece(|_, piece, _| {
        for (k, _) in piece.char_indices() {
            let at = acc.saturating_add(k);
            if index == target_before && before_start.is_none() {
                before_start = Some(at);
            }
            if index == target_after && after_end.is_none() {
                after_end = Some(at);
            }
            index = index.saturating_add(1);
        }
        acc = acc.saturating_add(piece.len());
    });
    if index == target_before && before_start.is_none() {
        before_start = Some(acc);
    }
    if index == target_after && after_end.is_none() {
        after_end = Some(acc);
    }
    Some((before_start?, after_end?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::para::Run;
    use crate::props::CharProps;
    use crate::{InlineObject, StoryRef};

    /// Builds a paragraph with one run per `(text, deleted)` part; deleted parts carry revision 0.
    fn para_with_runs(parts: &[(&str, bool)]) -> Paragraph {
        let mut p = Paragraph::new();
        for (text, deleted) in parts {
            let mut props = CharProps::default();
            if *deleted {
                props.del = Some(0);
            }
            p.text.push_str(text);
            p.runs.push(Run { len: text.len(), props });
        }
        p
    }

    /// A tracked deletion is hidden from the IME view but still maps stored offsets: its start is a
    /// visible caret position, its interior is not, and a window concatenates only visible text.
    #[test]
    fn tracked_deletions_are_hidden_and_offsets_still_map() {
        let p = para_with_runs(&[("ab", false), ("DEL", true), ("cd", false)]);
        let view = VisibleParagraph::new(&p);
        assert_eq!(view.visible_len(), 4);
        assert_eq!(view.visible_offset(0), Some(0));
        assert_eq!(view.visible_offset(2), Some(2), "the start of a deleted run is a visible caret position");
        assert_eq!(view.visible_offset(3), None, "inside a tracked deletion");
        assert_eq!(view.visible_offset(5), Some(2));
        assert_eq!(view.visible_offset(7), Some(4));
        assert_eq!(view.stored_offset(2), Some(5));
        assert_eq!(view.stored_offset(4), Some(7));
        assert_eq!(view.window(2, 2, 4000).map(|w| w.text), Some("abcd".into()));
    }

    /// An inline object contributes its plain text to visible length and boundary mapping; offsets
    /// strictly inside that expansion have no stored boundary.
    #[test]
    fn objects_contribute_their_plain_text_and_split_mapping_stops() {
        let mut p = Paragraph::new();
        p.text = format!("a{OBJ}b");
        p.runs = vec![Run { len: p.text.len(), props: CharProps::default() }];
        p.objects.push(InlineObject::Equation { linear: "x+y".into(), display: false, math: Default::default() });
        let view = VisibleParagraph::new(&p);
        assert_eq!(view.visible_len(), 5);
        assert_eq!(view.visible_offset(1), Some(1));
        assert_eq!(view.visible_offset(4), Some(4));
        assert_eq!(view.stored_offset(1), Some(1));
        assert_eq!(view.stored_offset(4), Some(4));
        assert_eq!(view.stored_offset(2), None, "inside the object's text");
    }

    /// An empty paragraph yields an available empty IME context at offset 0; an empty document
    /// yields an available empty context too, though this test asserts only its text, not its offsets.
    #[test]
    fn an_empty_document_is_available_and_empty() {
        let p = Paragraph::new();
        let view = VisibleParagraph::new(&p);
        let w = view.window(0, 0, MAX_IME_TEXT_BYTES).expect("an available empty context");
        assert_eq!((w.text.as_str(), w.cursor, w.anchor), ("", 0, 0));
        let doc = Document::new();
        let w = doc.ime_surrounding(&Pos::body(0, 0), &Pos::body(0, 0), MAX_IME_TEXT_BYTES).expect("available");
        assert_eq!(w.text, "");
    }

    /// A window is capped at the IME byte budget while keeping cursor and anchor inside its text; a
    /// selection wider than the budget is unavailable.
    #[test]
    fn bounded_window_keeps_the_selection_and_rejects_too_wide_ones() {
        let p = Paragraph::with_text(&"a".repeat(10_000), CharProps::default());
        let view = VisibleParagraph::new(&p);
        let w = view.window(5000, 5000, MAX_IME_TEXT_BYTES).expect("a bounded window");
        assert!(w.text.len() <= MAX_IME_TEXT_BYTES);
        assert_eq!(w.cursor, w.anchor);
        assert!(w.cursor <= w.text.len());
        assert!(view.window(0, 4001, MAX_IME_TEXT_BYTES).is_none());
    }

    /// Byte deletion rounds its start to a visible UTF-8 boundary, leaves partially covered scalar
    /// values whole, and refuses counts that run past the paragraph.
    #[test]
    fn byte_deletion_rounds_inward_and_leaves_partial_scalars_whole() {
        let doc = Document::from_text("aé👍b");
        let caret = Pos::body(0, 8);
        let one = doc.ime_delete_surrounding(&caret, &caret, 1, 0, DeleteUnit::Bytes).expect("b");
        assert_eq!(one.before, Some(7..8));
        // Two bytes start inside the emoji: it is left whole, only `b` goes.
        let two = doc.ime_delete_surrounding(&caret, &caret, 2, 0, DeleteUnit::Bytes).expect("b");
        assert_eq!(two.before, Some(7..8));
        // Six bytes start inside `é`: it stays whole, the emoji and `b` go.
        let six = doc.ime_delete_surrounding(&caret, &caret, 6, 0, DeleteUnit::Bytes).expect("emoji and b");
        assert_eq!(six.before, Some(3..8));
        // A count beyond the paragraph is refused, not clipped to delete a whole side.
        assert!(doc.ime_delete_surrounding(&caret, &caret, 0, 1, DeleteUnit::Bytes).is_none());
        assert!(doc.ime_delete_surrounding(&caret, &caret, 99, 0, DeleteUnit::Bytes).is_none());
    }

    /// Surrounding deletion skips the selected range on both sides and returns the selection's
    /// original reversed anchor and focus.
    #[test]
    fn byte_deletion_excludes_a_reversed_selection() {
        let doc = Document::from_text("abcd");
        let cursor = Pos::body(0, 1);
        let anchor = Pos::body(0, 3);
        let del = doc.ime_delete_surrounding(&cursor, &anchor, 1, 1, DeleteUnit::Bytes).expect("a deletion");
        assert_eq!(del.before, Some(0..1), "only the character before the selection");
        assert_eq!(del.after, Some(3..4), "only the character after the selection");
        assert_eq!((del.anchor, del.focus), (3, 1), "the reversed selection is preserved");
    }

    /// Tracked deletions are absent from windows and skipped when deleting around a caret, so an IME
    /// edit cannot remove them.
    #[test]
    fn tracked_deletions_are_not_counted_for_deletion_or_query() {
        let mut doc = Document::from_text("abDELcd");
        let p = para_with_runs(&[("ab", false), ("DEL", true), ("cd", false)]);
        doc.body = vec![std::sync::Arc::new(crate::Block::Para(p))];
        let view = VisibleParagraph::new(doc.para_at(&Pos::body(0, 0)).expect("paragraph"));
        assert_eq!(view.window(2, 2, 100).map(|w| w.text), Some("abcd".into()));
        let caret = Pos::body(0, 7);
        let del = doc.ime_delete_surrounding(&caret, &caret, 1, 0, DeleteUnit::Bytes).expect("c");
        assert_eq!(del.before, Some(6..7), "the stored deletion run is skipped");
    }

    /// A selection spanning paragraphs has no single surrounding context: both the query and
    /// surrounding deletion return `None`.
    #[test]
    fn cross_paragraph_selection_is_unavailable() {
        let doc = Document::from_text("one\ntwo");
        let cursor = Pos::body(0, 1);
        let anchor = Pos::body(1, 1);
        assert!(doc.ime_surrounding(&cursor, &anchor, MAX_IME_TEXT_BYTES).is_none());
        assert!(doc.ime_delete_surrounding(&cursor, &anchor, 1, 0, DeleteUnit::Chars).is_none());
    }

    /// An object's plain-text result stands for the object in the visible text; a deletion
    /// that starts inside that expansion leaves the object whole (byte) or is refused (chars).
    #[test]
    fn object_expansion_is_never_partly_deleted() {
        let mut doc = Document::from_text("");
        let mut p = Paragraph::new();
        p.text = format!("a{OBJ}b");
        p.runs = vec![Run { len: p.text.len(), props: CharProps::default() }];
        p.objects.push(InlineObject::Equation { linear: "x+y".into(), display: false, math: Default::default() });
        doc.body = vec![std::sync::Arc::new(crate::Block::Para(p))];
        let caret = Pos::body(0, 5);
        // Two bytes before the caret end inside `x+y` (`y` is the last byte): the start rounds
        // forward past the object, so only `b` goes.
        let two = doc.ime_delete_surrounding(&caret, &caret, 2, 0, DeleteUnit::Bytes).expect("b only");
        assert_eq!(two.before, Some(4..5), "stored UTF-8 offsets keep the object whole");
        let mut remaining = format!("a{OBJ}b");
        remaining.replace_range(two.before.clone().unwrap(), "");
        assert_eq!(remaining, format!("a{OBJ}"));
        // A character that lands inside the expansion has no stored boundary: refused.
        assert!(doc.ime_delete_surrounding(&caret, &caret, 2, 0, DeleteUnit::Chars).is_none());
        // A count reaching before the object erases the object as a whole.
        let five = doc.ime_delete_surrounding(&caret, &caret, 5, 0, DeleteUnit::Bytes).expect("object and b");
        assert_eq!(five.before, Some(0..5));
    }

    /// Character counts on the right side clamp at the paragraph's end and never cross into the
    /// next paragraph.
    #[test]
    fn trailing_character_deletion_clamps_at_the_paragraph_end() {
        let doc = Document::from_text("ab\ncd");
        let caret = Pos::body(0, 1);
        let del = doc.ime_delete_surrounding(&caret, &caret, 0, 99, DeleteUnit::Chars).expect("b only");
        assert_eq!(del.after, Some(1..2));
        let empty = doc.ime_delete_surrounding(&caret, &caret, 0, 0, DeleteUnit::Chars);
        assert!(empty.is_none(), "nothing to erase");
    }

    /// Sorted tracked deletions are scanned with one cursor: alternating revisions stay correct
    /// (and linear) instead of each character scanning every range.
    #[test]
    fn alternating_tracked_revisions_map_correctly() {
        // Equal-length visible/deleted pieces, so the expected visible byte count is exact.
        let parts: Vec<(String, bool)> =
            (0..2000).map(|i| (if i % 2 == 0 { format!("v{i:04};") } else { format!("D{i:04};") }, i % 2 == 1)).collect();
        let parts: Vec<(&str, bool)> = parts.iter().map(|(t, d)| (t.as_str(), *d)).collect();
        let p = para_with_runs(&parts);
        let view = VisibleParagraph::new(&p);
        assert_eq!(view.visible_len(), p.text.len() - 6 * 1000);
        assert_eq!(view.visible_offset(p.text.len()), Some(view.visible_len()));
        let w = view.window(0, 0, 100).expect("a window");
        assert!(w.text.len() <= 100);
        assert!(!w.text.contains('D'), "tracked deletions never show");
    }

    /// The public window never copies more than the IME cap, whatever a caller asks for.
    #[test]
    fn window_budget_is_capped() {
        let p = Paragraph::with_text(&"a".repeat(10_000), CharProps::default());
        let view = VisibleParagraph::new(&p);
        let w = view.window(0, 0, usize::MAX).expect("a bounded window");
        assert_eq!(w.text.len(), MAX_IME_TEXT_BYTES);
        let doc = Document::from_text(&"a".repeat(10_000));
        let w = doc.ime_surrounding(&Pos::body(0, 0), &Pos::body(0, 0), usize::MAX).expect("bounded");
        assert_eq!(w.text.len(), MAX_IME_TEXT_BYTES);
    }

    /// A caret in a table-cell paragraph maps against that cell's text, not the body around the table.
    #[test]
    fn table_cell_paragraph_maps_inside_its_cell() {
        let mut doc = Document::from_text("outside");
        let mut table = crate::Table::new(1, 1, 100.0);
        table.rows[0].cells[0].blocks = vec![crate::para_block(Paragraph::with_text("cell", CharProps::default()))];
        doc.body.push(std::sync::Arc::new(crate::Block::Table(table)));
        let path = crate::Path(vec![1, 0, 0, 0]);
        let cursor = Pos { story: StoryRef::Body, path: path.clone(), off: 4 };
        let anchor = Pos { story: StoryRef::Body, path, off: 2 };
        let w = doc.ime_surrounding(&cursor, &anchor, MAX_IME_TEXT_BYTES).expect("the cell text");
        assert_eq!(w.text, "cell");
        assert_eq!(w.cursor, 4);
        assert_eq!(w.anchor, 2);
    }

    /// Content-control markers are zero-length visible objects. A deletion range starts after
    /// the marker and ends before the next one, so an ordinary IME edit never removes a marker
    /// or leaves a half-paired control.
    #[test]
    fn control_markers_are_not_included_in_delete_ranges() {
        use crate::control::{ContentControl, ControlKind};
        let mut p = Paragraph::new();
        p.text = format!("pre{OBJ}Ada{OBJ}X");
        p.runs = vec![Run { len: p.text.len(), props: CharProps::default() }];
        let mut ctl = ContentControl::new(ControlKind::RichText);
        ctl.placeholder_text = "Enter name".into();
        p.objects.push(InlineObject::ControlStart { control: Box::new(ctl) });
        p.objects.push(InlineObject::ControlEnd);
        let mut doc = Document::new();
        doc.body = vec![std::sync::Arc::new(crate::Block::Para(p))];
        // Stored positions are UTF-8 byte offsets; each OBJ marker occupies three bytes.
        let cursor = Pos::body(0, 9);
        let del = doc.ime_delete_surrounding(&cursor, &cursor, 3, 1, DeleteUnit::Bytes).expect("a deletion");
        assert_eq!(del.before, Some(6..9), "the control-start marker and `pre` stay outside");
        assert_eq!(del.after, Some(12..13), "the control-end marker stays outside; only `X` goes");
        assert_eq!((del.anchor, del.focus), (9, 9));
    }
}

//! Saved selections follow a member's edit (chat): the owner's selection and the other
//! members' selections are positions (paragraph path + byte offset) taken before the edit; this
//! maps them onto the document after it.
//!
//! Per story: paragraphs equal at the start and at the end of the story are unchanged (a
//! paragraph after the edit keeps its offset and moves by the number of paragraphs added or
//! removed). Positions in the changed paragraphs are mapped through the changed text (the
//! paragraphs joined with a separator): before the first changed character they stay, after the
//! last one they move by the length difference, inside the change they go to its start. Text
//! inserted exactly at a selection's start goes before the selection (it keeps covering the text
//! it covered); at a caret or a selection's end it goes after.

use wordcraft_doc::{Document, Path, Pos};
use wordcraft_engine::Selection;

/// `sel` (valid in `before`) mapped onto `after`.
pub fn shift_selection(before: &Document, after: &Document, sel: &Selection) -> Selection {
    if sel.anchor == sel.focus {
        let p = shift_pos(before, after, &sel.anchor);
        return Selection { anchor: p.clone(), focus: p };
    }
    let anchor_is_start = sel.ordered().0 == sel.anchor;
    Selection { anchor: shift(before, after, &sel.anchor, anchor_is_start), focus: shift(before, after, &sel.focus, !anchor_is_start) }
}

/// Separator between paragraphs in the joined text (not a character paragraphs hold).
const SEP: char = '\u{1}';

fn shift_pos(before: &Document, after: &Document, p: &Pos) -> Pos {
    shift(before, after, p, false)
}

/// `start`: the position starts a selection, so text inserted exactly there goes before it.
fn shift(before: &Document, after: &Document, p: &Pos, start: bool) -> Pos {
    let (Some(sb), Some(sa)) = (before.story(p.story), after.story(p.story)) else { return p.clone() };
    if sb.len() == sa.len() && sb.iter().zip(sa.iter()).all(|(x, y)| std::sync::Arc::ptr_eq(x, y)) {
        return p.clone();
    }
    let bp = before.para_paths(p.story);
    let ap = after.para_paths(p.story);
    let Some(i) = bp.iter().position(|q| *q == p.path) else { return p.clone() };
    let pb = |k: usize| bp.get(k).and_then(|q| before.para(p.story, q));
    let pa = |k: usize| ap.get(k).and_then(|q| after.para(p.story, q));
    let n = bp.len().min(ap.len());
    let mut head: usize = 0;
    while head < n && pb(head) == pa(head) {
        head = head.saturating_add(1);
    }
    let mut tail: usize = 0;
    while tail < n.saturating_sub(head)
        && bp.len().checked_sub(tail.saturating_add(1)).and_then(pb) == ap.len().checked_sub(tail.saturating_add(1)).and_then(pa)
    {
        tail = tail.saturating_add(1);
    }
    let at = |path: &Path, off: usize| Pos { story: p.story, path: path.clone(), off };
    if i < head {
        return ap.get(i).map(|q| at(q, p.off)).unwrap_or_else(|| p.clone());
    }
    let (b_end, a_end) = (bp.len().saturating_sub(tail), ap.len().saturating_sub(tail));
    if i >= b_end {
        let Some(j) = i.saturating_add(ap.len()).checked_sub(bp.len()) else { return p.clone() };
        return ap.get(j).map(|q| at(q, p.off)).unwrap_or_else(|| p.clone());
    }
    // In the changed region: map through the joined text.
    let joined = |paths: &[Path], doc: &Document| -> String {
        paths.iter().map(|q| doc.para(p.story, q).map(|x| x.text.as_str()).unwrap_or("")).collect::<Vec<_>>().join(&SEP.to_string())
    };
    let rb = bp.get(head..b_end).unwrap_or(&[]);
    let ra = ap.get(head..a_end).unwrap_or(&[]);
    let (tb, ta) = (joined(rb, before), joined(ra, after));
    let g = rb
        .iter()
        .take(i.saturating_sub(head))
        .fold(p.off, |g, q| g.saturating_add(before.para(p.story, q).map(|x| x.len()).unwrap_or(0).saturating_add(1)));
    let cp = common_prefix(&tb, &ta);
    let cs = common_suffix(&tb, &ta).min(tb.len().min(ta.len()).saturating_sub(cp));
    let moved = |g: usize| g.saturating_add(ta.len()).saturating_sub(tb.len());
    let changed_end = tb.len().saturating_sub(cs);
    let g2 = if start && g >= changed_end {
        moved(g)
    } else if g <= cp {
        g
    } else if g >= changed_end {
        moved(g)
    } else {
        cp
    };
    if ra.is_empty() {
        // Paragraphs removed: the start of the next one (or the end of the previous one).
        return match ap.get(head) {
            Some(q) => at(q, 0),
            None => ap.last().map(|q| at(q, after.para(p.story, q).map(|x| x.len()).unwrap_or(0))).unwrap_or_else(|| p.clone()),
        };
    }
    let mut left = g2;
    for q in ra {
        let len = after.para(p.story, q).map(|x| x.len()).unwrap_or(0);
        if left <= len {
            return at(q, left);
        }
        left = left.saturating_sub(len.saturating_add(1));
    }
    ra.last().map(|q| at(q, after.para(p.story, q).map(|x| x.len()).unwrap_or(0))).unwrap_or_else(|| p.clone())
}

/// Bytes the two strings share at the start (on a character boundary).
fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices().zip(b.chars()).find(|((_, x), y)| x != y).map(|((i, _), _)| i).unwrap_or(a.len().min(b.len()))
}

/// Bytes the two strings share at the end (on a character boundary).
fn common_suffix(a: &str, b: &str) -> usize {
    let mut n: usize = 0;
    for (x, y) in a.chars().rev().zip(b.chars().rev()) {
        if x != y {
            break;
        }
        n = n.saturating_add(x.len_utf8());
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{CharProps, StoryRef};

    fn doc(text: &str) -> Document {
        Document::from_text(text)
    }

    fn pos(block: usize, off: usize) -> Pos {
        Pos::body(block, off)
    }

    #[test]
    fn same_paragraph_shifts_after_the_edit_only() {
        let before = doc("Price: 1,000. Term: 12.\nSecond.");
        let mut after = before.clone();
        let _ = after.insert_text(&pos(0, 0), "New ", &CharProps::default());
        assert_eq!(shift_pos(&before, &after, &pos(0, 14)), pos(0, 18));
        assert_eq!(shift_pos(&before, &after, &pos(0, 0)), pos(0, 0));
        assert_eq!(shift_pos(&before, &after, &pos(1, 3)), pos(1, 3));
    }

    #[test]
    fn inserted_and_removed_paragraphs_move_the_index() {
        let before = doc("One.\nTwo.\nThree café.");
        let mut after = before.clone();
        let _ = after.split_paragraph(&pos(0, 4));
        let _ = after.insert_text(&pos(1, 0), "New.", &CharProps::default());
        assert_eq!(shift_pos(&before, &after, &pos(2, 2)), pos(3, 2));
        assert_eq!(shift_pos(&before, &after, &pos(0, 1)), pos(0, 1));
        let mut removed = before.clone();
        let _ = removed.remove_block(StoryRef::Body, &Path::top(0));
        assert_eq!(shift_pos(&before, &removed, &pos(2, 2)), pos(1, 2));
        assert_eq!(shift_pos(&before, &removed, &pos(0, 2)), pos(0, 0));
    }

    #[test]
    fn a_selection_keeps_its_text_when_text_is_inserted_at_its_start() {
        let before = doc("Term: 12 months.");
        let mut after = before.clone();
        let _ = after.insert_text(&pos(0, 6), "24 months", &CharProps::default());
        let sel = Selection { anchor: pos(0, 6), focus: pos(0, 15) };
        assert_eq!(shift_selection(&before, &after, &sel), Selection { anchor: pos(0, 15), focus: pos(0, 24) });
        let back = Selection { anchor: pos(0, 15), focus: pos(0, 6) };
        assert_eq!(shift_selection(&before, &after, &back), Selection { anchor: pos(0, 24), focus: pos(0, 15) });
        // A caret at the insertion point stays before it.
        let caret = Selection { anchor: pos(0, 6), focus: pos(0, 6) };
        assert_eq!(shift_selection(&before, &after, &caret), caret);
    }

    #[test]
    fn inside_a_replaced_range_goes_to_its_start() {
        let before = doc("Alpha beta gamma.");
        let mut after = before.clone();
        if let Ok(p) = after.para_mut(StoryRef::Body, &Path::top(0)) {
            let _ = p.delete(6, 10);
            let _ = p.insert_text(6, "LONG BETA", &CharProps::default());
        }
        assert_eq!(shift_pos(&before, &after, &pos(0, 8)), pos(0, 6));
        assert_eq!(shift_pos(&before, &after, &pos(0, 11)), pos(0, 16));
    }
}

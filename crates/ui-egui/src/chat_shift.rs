//! Saved selections follow a member's edit (chat add-in): the owner's selection and the other
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
    let mut head = 0;
    while head < n && pb(head) == pa(head) {
        head += 1;
    }
    let mut tail = 0;
    while tail < n - head && pb(bp.len() - 1 - tail) == pa(ap.len() - 1 - tail) {
        tail += 1;
    }
    let at = |path: &Path, off: usize| Pos { story: p.story, path: path.clone(), off };
    if i < head {
        return ap.get(i).map(|q| at(q, p.off)).unwrap_or_else(|| p.clone());
    }
    if i >= bp.len() - tail {
        let j = i + ap.len() - bp.len();
        return ap.get(j).map(|q| at(q, p.off)).unwrap_or_else(|| p.clone());
    }
    // In the changed region: map through the joined text.
    let joined = |paths: &[Path], doc: &Document| -> String {
        paths.iter().map(|q| doc.para(p.story, q).map(|x| x.text.as_str()).unwrap_or("")).collect::<Vec<_>>().join(&SEP.to_string())
    };
    let rb = bp.get(head..bp.len() - tail).unwrap_or(&[]);
    let ra = ap.get(head..ap.len() - tail).unwrap_or(&[]);
    let (tb, ta) = (joined(rb, before), joined(ra, after));
    let g: usize = rb.iter().take(i - head).map(|q| before.para(p.story, q).map(|x| x.len()).unwrap_or(0) + 1).sum::<usize>() + p.off;
    let cp = common_prefix(&tb, &ta);
    let cs = common_suffix(&tb, &ta).min(tb.len().min(ta.len()) - cp);
    let moved = |g: usize| (g + ta.len()).saturating_sub(tb.len());
    let g2 = if start && g >= tb.len() - cs {
        moved(g)
    } else if g <= cp {
        g
    } else if g >= tb.len() - cs {
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
        left -= len + 1;
    }
    ra.last().map(|q| at(q, after.para(p.story, q).map(|x| x.len()).unwrap_or(0))).unwrap_or_else(|| p.clone())
}

/// Bytes the two strings share at the start (on a character boundary).
fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices().zip(b.chars()).find(|((_, x), y)| x != y).map(|((i, _), _)| i).unwrap_or(a.len().min(b.len()))
}

/// Bytes the two strings share at the end (on a character boundary).
fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.chars().rev().zip(b.chars().rev()) {
        if x != y {
            break;
        }
        n += x.len_utf8();
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
        let before = doc("Valor: 1.000. Prazo: 12.\nSegundo.");
        let mut after = before.clone();
        let _ = after.insert_text(&pos(0, 0), "Novo ", &CharProps::default());
        assert_eq!(shift_pos(&before, &after, &pos(0, 14)), pos(0, 19));
        assert_eq!(shift_pos(&before, &after, &pos(0, 0)), pos(0, 0));
        assert_eq!(shift_pos(&before, &after, &pos(1, 3)), pos(1, 3));
    }

    #[test]
    fn inserted_and_removed_paragraphs_move_the_index() {
        let before = doc("Um.\nDois.\nTrês.");
        let mut after = before.clone();
        let _ = after.split_paragraph(&pos(0, 3));
        let _ = after.insert_text(&pos(1, 0), "Novo.", &CharProps::default());
        assert_eq!(shift_pos(&before, &after, &pos(2, 2)), pos(3, 2));
        assert_eq!(shift_pos(&before, &after, &pos(0, 1)), pos(0, 1));
        let mut removed = before.clone();
        let _ = removed.remove_block(StoryRef::Body, &Path::top(0));
        assert_eq!(shift_pos(&before, &removed, &pos(2, 2)), pos(1, 2));
        assert_eq!(shift_pos(&before, &removed, &pos(0, 2)), pos(0, 0));
    }

    #[test]
    fn a_selection_keeps_its_text_when_text_is_inserted_at_its_start() {
        let before = doc("Prazo: 12 meses.");
        let mut after = before.clone();
        let _ = after.insert_text(&pos(0, 7), "24 meses", &CharProps::default());
        let sel = Selection { anchor: pos(0, 7), focus: pos(0, 15) };
        assert_eq!(shift_selection(&before, &after, &sel), Selection { anchor: pos(0, 15), focus: pos(0, 23) });
        let back = Selection { anchor: pos(0, 15), focus: pos(0, 7) };
        assert_eq!(shift_selection(&before, &after, &back), Selection { anchor: pos(0, 23), focus: pos(0, 15) });
        // A caret at the insertion point stays before it.
        let caret = Selection { anchor: pos(0, 7), focus: pos(0, 7) };
        assert_eq!(shift_selection(&before, &after, &caret), caret);
    }

    #[test]
    fn inside_a_replaced_range_goes_to_its_start() {
        let before = doc("Alfa beta gama.");
        let mut after = before.clone();
        if let Ok(p) = after.para_mut(StoryRef::Body, &Path::top(0)) {
            let _ = p.delete(5, 9);
            let _ = p.insert_text(5, "BETA LONGA", &CharProps::default());
        }
        assert_eq!(shift_pos(&before, &after, &pos(0, 7)), pos(0, 5));
        assert_eq!(shift_pos(&before, &after, &pos(0, 10)), pos(0, 16));
    }
}

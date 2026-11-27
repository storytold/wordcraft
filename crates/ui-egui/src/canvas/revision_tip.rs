//! The hover tip on a tracked change (#7887): what kind of change it is, who made it and when.

use super::{DocLayout, Pos, WordApp};
use wordcraft_doc::props::CharProps;
use wordcraft_layout::display::MarkupOptions;

/// The tip for the tracked change under the pointer at (x, y) on `page`, where `pos` is the caret
/// position the point hits: its kind, its author and its time, a line each. Only changes the page
/// marks count: none with markup hidden, insertions, deletions and moves while Show Markup shows
/// them, formatting changes while it shows formatting. As with a link, the point must be on a
/// character, not beside, above or below it: on the changed character itself, or anywhere in the
/// text of a paragraph whose own formatting changed (its alignment, indents…) when the character
/// has no change of its own.
pub(super) fn revision_under(app: &WordApp, layout: &DocLayout, page: usize, x: f32, y: f32, pos: &Pos) -> Option<String> {
    let session = &app.session;
    if !session.view.show_markup {
        return None;
    }
    let markup = &session.prefs.markup;
    let doc = &session.doc;
    let para = doc.para_at(pos)?;
    let para_change = para.props.fmt_change.as_ref().filter(|_| markup.formatting).map(|c| (c.rev, tl!("Formatted")));
    let at = wordcraft_geom::Point::new(x, y);
    // The caret falls between characters: the one under the point is on either side of it.
    [(pos.off, para.next_boundary(pos.off)), (para.prev_boundary(pos.off), pos.off)].into_iter().filter(|(a, b)| a < b).find_map(|(a, b)| {
        let (rev, kind) = change_of(para.props_of_char(a), markup, |i| doc.move_name(Some(i)).is_some()).or(para_change)?;
        let rev = doc.revisions.get(usize::try_from(rev).ok()?)?;
        let (from, to) = (Pos { off: a, ..pos.clone() }, Pos { off: b, ..pos.clone() });
        let hit = layout.selection_rects(doc, &from, &to, page).into_iter().any(|(pi, r)| pi == page && r.contains(at));
        hit.then(|| tip(kind, &rev.author, &crate::local_time::revision_time(rev)))
    })
}

/// The revision a character's marks point to and what to call it, when the markup shown marks it.
/// A deletion lies over an insertion it removes (inserted, then deleted by someone else): the
/// deletion is the change the page shows.
fn change_of(props: &CharProps, markup: &MarkupOptions, is_move: impl Fn(u32) -> bool) -> Option<(u32, &'static str)> {
    if markup.insertions_deletions {
        if let Some(i) = props.del {
            return Some((i, if is_move(i) { tl!("Moved from") } else { tl!("Deleted") }));
        }
        if let Some(i) = props.ins {
            return Some((i, if is_move(i) { tl!("Moved to") } else { tl!("Inserted") }));
        }
    }
    props.fmt_change.as_ref().filter(|_| markup.formatting).map(|c| (c.rev, tl!("Formatted")))
}

/// The tip's lines: the change's kind, its author (or "Author" when the file names none, as the
/// balloons do) and its time, left out when the file gives none.
fn tip(kind: &str, author: &str, when: &str) -> String {
    let author = if author.trim().is_empty() { tl!("Author") } else { author };
    let mut lines = vec![kind, author];
    if !when.is_empty() {
        lines.push(when);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::props::PropChange;

    /// A deletion over an insertion is the change shown; formatting counts only while the markup
    /// shows formatting, and insertions and deletions only while it shows them.
    #[test]
    fn the_shown_markup_picks_the_change() {
        let all = MarkupOptions { insertions_deletions: true, formatting: true, ..Default::default() };
        let no_move = |_| false;
        let both = CharProps { ins: Some(0), del: Some(1), ..Default::default() };
        assert_eq!(change_of(&both, &all, no_move), Some((1, "Deleted")));
        assert_eq!(change_of(&both, &all, |i| i == 1), Some((1, "Moved from")));
        let inserted = CharProps { ins: Some(2), ..Default::default() };
        assert_eq!(change_of(&inserted, &all, no_move), Some((2, "Inserted")));
        assert_eq!(change_of(&inserted, &all, |_| true), Some((2, "Moved to")));
        let formatted = CharProps { fmt_change: PropChange::boxed(3, CharProps::default()), ..Default::default() };
        assert_eq!(change_of(&formatted, &all, no_move), Some((3, "Formatted")));
        assert_eq!(change_of(&formatted, &MarkupOptions { formatting: false, ..all.clone() }, no_move), None);
        assert_eq!(change_of(&inserted, &MarkupOptions { insertions_deletions: false, ..all.clone() }, no_move), None);
        assert_eq!(change_of(&CharProps::default(), &all, no_move), None);
    }

    /// The tip's lines; an unnamed author reads "Author", and a missing time is left out.
    #[test]
    fn the_tip_names_kind_author_and_time() {
        assert_eq!(tip("Inserted", "Ada", "2026-11-20 14:05"), "Inserted\nAda\n2026-11-20 14:05");
        assert_eq!(tip("Deleted", " ", ""), "Deleted\nAuthor");
    }
}

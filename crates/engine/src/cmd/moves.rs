//! Tracked moves (Review › Track Changes, ECMA-376 §17.13.5.21–28): with Track Changes on, a
//! paragraph or sentence cut and pasted elsewhere is recorded as a move, its source (`w:moveFrom`,
//! a deletion revision) and destination (`w:moveTo`, an insertion revision) linked by a move
//! name. Accepting or rejecting either end resolves both.

use std::collections::BTreeSet;

use wordcraft_doc::edit::Fragment;
use wordcraft_doc::{Pos, Revision, RevisionKind, StoryRef};

use super::{now_iso, track_delete_as};
use crate::{CmdError, Selection, Session};

/// The deletion the last tracked cut recorded, which the paste right after it makes a move.
#[derive(Clone, Debug)]
pub struct PendingMove {
    /// Its revision (made for this cut alone).
    pub rev: u32,
    /// The text that was cut.
    pub text: String,
    pub author: String,
}

/// Whether cut text is big enough to be moved rather than deleted and inserted, as Word does: a
/// whole paragraph (it ends in, or holds, a paragraph mark) or at least a sentence.
pub fn move_sized(text: &str) -> bool {
    let t = text.trim_end_matches([' ', '\u{a0}']);
    if t.contains('\n') {
        return true;
    }
    let t = t.trim_end_matches(['"', '\'', '\u{201d}', '\u{2019}', ')']);
    t.ends_with(['.', '!', '?', '\u{2026}']) && t.split_whitespace().nth(1).is_some()
}

/// Every story.
fn stories(s: &Session) -> Vec<StoryRef> {
    std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect()
}

/// Whether some text or paragraph mark carries revision `rev` as its insertion or deletion.
fn in_use(s: &Session, rev: u32) -> bool {
    stories(s).into_iter().any(|st| {
        s.doc.para_paths(st).iter().any(|path| {
            s.doc.para(st, path).is_some_and(|p| {
                p.mark.ins == Some(rev) || p.mark.del == Some(rev) || p.runs.iter().any(|r| r.props.ins == Some(rev) || r.props.del == Some(rev))
            })
        })
    })
}

/// Cut's deletion under Track Changes: a cut big enough to be moved gets a revision of its own
/// and is remembered, so the paste right after it can make it a move. Returns `false` (nothing
/// done) when it isn't such a cut.
pub fn cut_tracked(s: &mut Session) -> Result<bool, CmdError> {
    let (a, b) = s.sel.ordered();
    if !s.doc.settings.track_changes || a == b || a.story != b.story || !move_sized(&s.clipboard_text) {
        return Ok(false);
    }
    s.doc.revisions.push(Revision::new(RevisionKind::Delete, s.author.clone(), now_iso()));
    let rid = (s.doc.revisions.len() - 1) as u32;
    let at = track_delete_as(s, &a, &b, rid)?;
    s.sel = Selection::caret(at);
    // Only the author's own insertions were cut: they are gone, not moved.
    if in_use(s, rid) {
        s.pending_move = Some(PendingMove { rev: rid, text: s.clipboard_text.clone(), author: s.author.clone() });
    }
    Ok(true)
}

/// A tracked paste of what the last command cut: makes that cut the source of a move and
/// returns the revision for the pasted text (the destination). `None` = an ordinary insertion.
pub fn take_paste(s: &mut Session, frag: &Fragment) -> Option<u32> {
    let m = s.pending_move.take()?;
    if m.author != s.author || frag.plain_text() != m.text || !in_use(s, m.rev) {
        return None;
    }
    let src = s.doc.revisions.get(m.rev as usize).filter(|r| r.kind == RevisionKind::Delete && r.move_name.is_none())?;
    let date = src.date.clone();
    let taken: BTreeSet<&str> = s.doc.revisions.iter().filter_map(|r| r.move_name.as_deref()).collect();
    let name = (1..).map(|n: u64| format!("move{n}")).find(|n| !taken.contains(n.as_str()))?;
    if let Some(r) = s.doc.revisions.get_mut(m.rev as usize) {
        r.move_name = Some(name.clone());
    }
    s.doc.revisions.push(Revision { kind: RevisionKind::Insert, author: s.author.clone(), date, move_name: Some(name) });
    Some((s.doc.revisions.len() - 1) as u32)
}

/// The move names of the insertions and deletions in `a..b` (and of the paragraph marks it
/// covers; `caret_mark` = the mark of the paragraph at that path, for a caret before it).
pub fn names_in(s: &Session, a: &Pos, b: &Pos, caret_mark: Option<&wordcraft_doc::Path>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |r: Option<u32>| {
        if let Some(n) = s.doc.move_name(r) {
            out.insert(n.to_string());
        }
    };
    for path in s.doc.paths_between(a, b) {
        let Some(p) = s.doc.para(a.story, &path) else { continue };
        let from = if path == a.path { a.off } else { 0 };
        let to = if path == b.path { b.off } else { p.len() };
        for (r, c) in p.run_ranges() {
            if (r.end > from && r.start < to) || (from == to && r.start <= from && from <= r.end) {
                add(c.ins);
                add(c.del);
            }
        }
        if path != b.path || caret_mark == Some(&path) {
            add(p.mark.ins);
            add(p.mark.del);
        }
    }
    out
}

/// Accept or reject every end of the moves named `names`: accepting removes the source and
/// keeps the destination, rejecting removes the destination and restores the source.
pub fn resolve(s: &mut Session, names: &BTreeSet<String>, accept: bool) -> Result<(), CmdError> {
    if names.is_empty() {
        return Ok(());
    }
    let named = |s: &Session, r: Option<u32>| s.doc.move_name(r).is_some_and(|n| names.contains(n));
    for st in stories(s) {
        for path in s.doc.para_paths(st).into_iter().rev() {
            let Some(p) = s.doc.para(st, &path) else { continue };
            let ranges: Vec<(usize, usize, bool, bool)> =
                p.run_ranges().map(|(r, c)| (r.start, r.end, named(s, c.ins), named(s, c.del))).filter(|(_, _, ins, del)| *ins || *del).collect();
            let (mark_ins, mark_del) = (named(s, p.mark.ins), named(s, p.mark.del));
            if ranges.is_empty() && !mark_ins && !mark_del {
                continue;
            }
            let para = s.doc.para_mut(st, &path)?;
            for (a, b, ins, del) in ranges.into_iter().rev() {
                if (del && accept) || (ins && !accept) {
                    para.delete(a, b)?;
                } else {
                    para.format(a, b, &|c| {
                        if ins {
                            c.ins = None;
                        }
                        if del {
                            c.del = None;
                        }
                    })?;
                }
            }
            if !mark_ins && !mark_del {
                continue;
            }
            // A moved paragraph mark that goes away joins the next paragraph to this one.
            if ((mark_del && accept) || (mark_ins && !accept)) && super::join_next_para(s, st, &path)? {
                continue;
            }
            let para = s.doc.para_mut(st, &path)?;
            if mark_ins {
                para.mark.ins = None;
            }
            if mark_del {
                para.mark.del = None;
            }
            para.touch();
        }
    }
    Ok(())
}

/// Whether the body has moved text (cheap: stops at the first).
pub fn has_moves(doc: &wordcraft_doc::Document) -> bool {
    if !doc.revisions.iter().any(|r| r.move_name.is_some()) {
        return false;
    }
    doc.para_paths(StoryRef::Body).iter().any(|path| {
        doc.para(StoryRef::Body, path).is_some_and(|p| {
            doc.move_name(p.mark.ins).is_some()
                || doc.move_name(p.mark.del).is_some()
                || p.runs.iter().any(|r| doc.move_name(r.props.ins).is_some() || doc.move_name(r.props.del).is_some())
        })
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wordcraft_doc::{Document, Pos};

    use crate::Session;

    fn texts(s: &Session) -> Vec<String> {
        s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect()
    }

    /// Three paragraphs; with Track Changes on, the second (with its mark) cut and pasted
    /// before the first.
    fn moved() -> Session {
        let mut s = Session::new(Document::new());
        s.run("document.setText", &json!({"text": "First one.\nSecond one.\nThird one."})).unwrap();
        s.run("file.setAuthor", &json!({"name": "Ana"})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}})).unwrap();
        s.run("edit.cut", &json!({})).unwrap();
        s.run("caret.set", &json!({"pos": {"block": 0, "off": 0}})).unwrap();
        s.run("edit.paste", &json!({})).unwrap();
        s
    }

    fn kinds(s: &mut Session) -> Vec<(String, Value)> {
        let ch = s.run("review.changes", &json!({})).unwrap();
        ch.as_array().unwrap().iter().map(|c| (c["kind"].as_str().unwrap().to_string(), c["move"].clone())).collect()
    }

    #[test]
    fn a_tracked_cut_and_paste_of_a_paragraph_is_a_move() {
        let mut s = moved();
        assert_eq!(texts(&s), ["Second one.", "First one.", "Second one.", "Third one."]);
        let p = |i: usize| s.doc.para_at(&Pos::body(i, 0)).unwrap().clone();
        let name = |r: Option<u32>| s.doc.move_name(r).map(str::to_string);
        let (to, from) = (p(0), p(2));
        assert_eq!(name(to.runs[0].props.ins).as_deref(), Some("move1"));
        assert_eq!(name(to.mark.ins).as_deref(), Some("move1"), "the pasted paragraph mark is moved too");
        assert_eq!(name(from.runs[0].props.del).as_deref(), Some("move1"));
        assert_eq!(name(from.mark.del).as_deref(), Some("move1"));
        assert_eq!(p(1).runs[0].props.ins, None);
        let k = kinds(&mut s);
        assert!(k.iter().all(|(k, m)| k.starts_with("move") && m == "move1"), "{k:?}");
        // Saved and opened again, it is still a move.
        let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
        let b0 = back.body[0].as_para().unwrap();
        assert!(back.move_name(b0.runs[0].props.ins).is_some() && back.move_name(b0.mark.ins).is_some());
        let b2 = back.body[2].as_para().unwrap();
        assert_eq!(back.move_name(b2.runs[0].props.del), back.move_name(b0.runs[0].props.ins));

        // A word is no move: a deletion and an insertion.
        let mut s = Session::new(Document::new());
        s.run("document.setText", &json!({"text": "one two"})).unwrap();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 4}})).unwrap();
        s.run("edit.cut", &json!({})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("edit.paste", &json!({})).unwrap();
        let k: Vec<String> = kinds(&mut s).into_iter().map(|(k, _)| k).collect();
        assert_eq!(k, ["delete", "insert"]);
    }

    #[test]
    fn accepting_one_end_of_a_move_accepts_both() {
        let mut s = moved();
        // The caret in the text moved away.
        s.run("caret.set", &json!({"pos": {"block": 2, "off": 3}})).unwrap();
        s.run("review.accept", &json!({})).unwrap();
        assert_eq!(texts(&s), ["Second one.", "First one.", "Third one."]);
        assert!(kinds(&mut s).is_empty());
    }

    #[test]
    fn rejecting_a_move_puts_the_text_back() {
        let mut s = moved();
        // The caret in the moved text.
        s.run("caret.set", &json!({"pos": {"block": 0, "off": 3}})).unwrap();
        s.run("review.reject", &json!({})).unwrap();
        assert_eq!(texts(&s), ["First one.", "Second one.", "Third one."]);
        assert!(kinds(&mut s).is_empty());
    }
}

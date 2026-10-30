use super::{
    BTreeMap, BTreeSet, CmdError, CmdResult, Comment, InlineObject, Paragraph, PartKind, Pos, Selection, Session, StoryRef, Value, json, now_iso, p,
    para_block, pos_json, sel_result,
};

pub(super) fn new_comment(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::str(v, "text").unwrap_or("").to_string();
    let (a, b) = s.sel.ordered();
    let (a, b) = if a == b {
        // Comment on the word at the caret.
        match s.doc.para_at(&a) {
            Some(para) if !para.is_empty() => {
                let (x, y) = para.word_at(a.off);
                let y = para.text.get(x..y).map(|w| x + w.trim_end().len()).unwrap_or(y);
                (Pos { off: x, ..a.clone() }, Pos { off: y, ..a })
            }
            _ => (a.clone(), a),
        }
    } else {
        (a, b)
    };
    let part = s.doc.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text(&text, Default::default()))])?;
    let id = comment_id(s)?;
    let initials = s.user_initials();
    let now = now_iso();
    s.doc.comments.insert(
        id,
        Comment {
            author: s.author.clone(),
            initials,
            date: now.clone(),
            date_utc: Some(now),
            parent: None,
            resolved: false,
            part,
            ..Default::default()
        },
    );
    let props = Default::default();
    s.doc.insert_object(&b, InlineObject::CommentEnd { id }, &props)?;
    s.doc.insert_object(&a, InlineObject::CommentStart { id }, &props)?;
    s.view.comments_pane = true;
    Ok(json!({"id": id, "story": part}))
}

/// An omitted ID permits caret targeting; an explicit malformed value must not.
/// Unlike the general numeric parameter helper, comment IDs never coerce floats.
pub(super) fn validate_comment_id(v: &Value) -> Result<(), CmdError> {
    if v.get("id").is_some_and(|id| id.as_u64().is_none_or(|n| u32::try_from(n).is_err())) {
        return Err(CmdError::Params("`id` must be a non-negative 32-bit JSON integer".into()));
    }
    Ok(())
}

pub(super) fn reply(s: &mut Session, v: &Value) -> CmdResult {
    validate_comment_id(v)?;
    let parent = p::u64(v, "id").map(|x| x as u32).ok_or_else(|| CmdError::Params("`id` required".into()))?;
    if !s.doc.comments.contains_key(&parent) {
        return Err(CmdError::Params("no such comment".into()));
    }
    let text = p::req_str(v, "text")?;
    let part = s.doc.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text(text, Default::default()))])?;
    let id = comment_id(s)?;
    let initials = s.user_initials();
    let now = now_iso();
    s.doc.comments.insert(
        id,
        Comment {
            author: s.author.clone(),
            initials,
            date: now.clone(),
            date_utc: Some(now),
            parent: Some(parent),
            resolved: false,
            part,
            ..Default::default()
        },
    );
    Ok(json!({"id": id}))
}

/// An id no comment has: one past the largest, or the lowest free one once a file used `u32::MAX`.
fn comment_id(s: &Session) -> Result<u32, CmdError> {
    let ids = &s.doc.comments;
    wordcraft_doc::free_id(ids.keys().next_back().copied(), ids.keys().copied(), 0).ok_or_else(|| CmdError::Failed("no free comment id".into()))
}

/// Comments can't be written or edited in a document protected as read only or for filling in
/// forms (the comment fields are read-only there, #1348).
pub(super) fn comments_change(s: &Session) -> Option<&'static str> {
    match s.doc.settings.protection.as_deref() {
        Some("readOnly" | "forms") => Some("the document is protected"),
        _ => None,
    }
}

/// Replace a comment's (or reply's) text; each line becomes a paragraph.
pub(super) fn edit_comment(s: &mut Session, v: &Value) -> CmdResult {
    validate_comment_id(v)?;
    let id = p::u64(v, "id").and_then(|x| u32::try_from(x).ok()).ok_or_else(|| CmdError::Params("`id` required".into()))?;
    let text = p::req_str(v, "text")?;
    let part = s.doc.comments.get(&id).map(|c| c.part).ok_or_else(|| CmdError::Params("no such comment".into()))?;
    let blocks = text.split('\n').take(10_000).map(|l| para_block(Paragraph::with_text(l.trim_end_matches('\r'), Default::default()))).collect();
    s.doc.set_story(StoryRef::Part(part), blocks)?;
    Ok(json!({"id": id}))
}

/// The innermost comment whose range holds the caret.
pub fn comment_range_at_caret(s: &Session) -> Option<u32> {
    let f = &s.sel.focus;
    let p = s.doc.para_at(f)?;
    let mut open: Vec<u32> = Vec::new();
    for off in p.object_offsets() {
        if off > f.off {
            break;
        }
        match p.object_at(off) {
            Some(InlineObject::CommentStart { id }) => open.push(*id),
            Some(InlineObject::CommentEnd { id }) => open.retain(|x| x != id),
            _ => {}
        }
    }
    open.last().copied()
}

/// Comment anchored around the caret, else the first one in its paragraph.
pub(super) fn comment_at_caret(s: &Session) -> Option<u32> {
    let f = &s.sel.focus;
    let p = s.doc.para_at(f)?;
    comment_range_at_caret(s).or_else(|| {
        p.object_offsets().into_iter().find_map(|o| match p.object_at(o) {
            Some(InlineObject::CommentStart { id }) | Some(InlineObject::CommentEnd { id }) => Some(*id),
            _ => None,
        })
    })
}

/// Removes the comments' anchors from every story: the body, headers, footers, notes and text
/// boxes (#1345).
fn remove_anchors(s: &mut Session, ids: &BTreeSet<u32>) -> Result<(), CmdError> {
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    for story in stories {
        for path in s.doc.para_paths(story) {
            let offs: Vec<usize> = s
                .doc
                .para(story, &path)
                .map(|p| {
                    p.object_offsets()
                        .into_iter()
                        .filter(|o| matches!(p.object_at(*o), Some(InlineObject::CommentStart { id }) | Some(InlineObject::CommentEnd { id }) if ids.contains(id)))
                        .collect()
                })
                .unwrap_or_default();
            if offs.is_empty() {
                continue;
            }
            let para = s.doc.para_mut(story, &path)?;
            for o in offs.into_iter().rev() {
                para.delete(o, o + wordcraft_doc::para::OBJ.len_utf8())?;
            }
        }
    }
    Ok(())
}

/// The comment and every reply below it, replies to replies included (#1345). A parent cycle
/// from a hostile file ends the walk instead of looping.
pub(super) fn thread_of(s: &Session, id: u32) -> Vec<u32> {
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (k, c) in &s.doc.comments {
        if let Some(parent) = c.parent {
            children.entry(parent).or_default().push(*k);
        }
    }
    let mut seen = BTreeSet::from([id]);
    let mut ids = vec![id];
    let mut i = 0;
    while let Some(&at) = ids.get(i) {
        for k in children.get(&at).into_iter().flatten() {
            if seen.insert(*k) {
                ids.push(*k);
            }
        }
        i += 1;
    }
    ids
}

pub(super) fn delete_comment(s: &mut Session, v: &Value) -> CmdResult {
    validate_comment_id(v)?;
    let ids: Vec<u32> = if p::bool(v, "all").unwrap_or(false) {
        s.doc.comments.keys().copied().collect()
    } else {
        let id = p::u64(v, "id").map(|x| x as u32).or_else(|| comment_at_caret(s)).ok_or_else(|| CmdError::Params("no comment here".into()))?;
        thread_of(s, id)
    };
    remove_anchors(s, &ids.iter().copied().collect())?;
    for id in &ids {
        if let Some(c) = s.doc.comments.remove(id) {
            s.doc.parts.remove(&c.part);
        }
    }
    s.clamp_selection();
    Ok(json!({"deleted": ids}))
}

/// Comments in reading order with their anchors: every story the reader sees, in
/// `Document::shown_stories` order (#2289). A comment anchored only in a story nothing shows
/// (a removed header) has no anchor.
pub fn comment_list(s: &Session) -> Vec<(u32, Option<Pos>)> {
    let mut found: Vec<(u32, Option<Pos>)> = Vec::new();
    // The balloons ask every frame; most documents have no comments.
    if s.doc.comments.is_empty() {
        return found;
    }
    for story in s.doc.shown_stories() {
        for path in s.doc.para_paths(story) {
            let Some(p) = s.doc.para(story, &path) else { continue };
            for o in p.object_offsets() {
                if let Some(InlineObject::CommentStart { id }) = p.object_at(o) {
                    found.push((*id, Some(Pos { story, path: path.clone(), off: o })));
                }
            }
        }
    }
    for id in s.doc.comments.keys() {
        if !found.iter().any(|(x, _)| x == id) {
            found.push((*id, None));
        }
    }
    found
}

pub(super) fn list_comments(s: &mut Session, _: &Value) -> CmdResult {
    let list = comment_list(s);
    Ok(Value::Array(
        list.iter()
            .filter_map(|(id, pos)| {
                let c = s.doc.comments.get(id)?;
                Some(json!({
                    "id": id, "author": c.author, "date": c.date, "resolved": c.resolved, "parent": c.parent,
                    "text": s.doc.plain_text(StoryRef::Part(c.part)),
                    "anchor": pos.as_ref().map(pos_json),
                }))
            })
            .collect(),
    ))
}

pub(super) fn nav_comment(s: &mut Session, dir: i32) -> CmdResult {
    let list: Vec<Pos> = comment_list(s).into_iter().filter_map(|(_, p)| p).collect();
    // Reading order: the story's place in `shown_stories`, then the place in the story. Part ids
    // follow creation, not reading, so `Pos` order won't do across stories.
    let rank: BTreeMap<StoryRef, usize> = s.doc.shown_stories().into_iter().enumerate().map(|(i, st)| (st, i)).collect();
    let key = |p: &Pos| rank.get(&p.story).map(|i| (*i, p.path.clone(), p.off));
    let caret = key(&s.sel.focus);
    let t = if dir > 0 {
        list.iter().find(|p| caret.is_some() && key(p) > caret).or(list.first())
    } else {
        list.iter().rev().find(|p| caret.is_some() && key(p) < caret).or(list.last())
    };
    if let Some(p) = t {
        s.sel = Selection::caret(p.clone());
    }
    sel_result(s)
}

use super::{CharProps, CmdError, CmdResult, InlineObject, Pos, Selection, Session, StoryRef, Value, json, p, sel_result};

/// What a link can point to inside the document (Insert › Link › Place in This Document): the top
/// of the document, its headings (body block, level, text) and its bookmarks, without the hidden
/// ones (names starting with `_`).
pub(super) fn link_targets(s: &mut Session, _: &Value) -> CmdResult {
    let headings: Vec<Value> = crate::cmd::references::headings(s)
        .into_iter()
        .map(|(block, level, text, _)| json!({"block": block, "level": level, "text": text}))
        .collect();
    let mut bookmarks: Vec<String> = Vec::new();
    for (name, _) in s.doc.bookmarks() {
        if !name.starts_with('_') && !bookmarks.contains(&name) {
            bookmarks.push(name);
        }
    }
    Ok(json!({"top": true, "headings": headings, "bookmarks": bookmarks}))
}

/// The address and the default text of a link to a place in this document, from `top`, `bookmark`
/// or `heading` (a heading gets a hidden `_Toc` bookmark, as in Word); None when none is given.
fn place_target(s: &mut Session, v: &Value) -> Result<Option<(String, String)>, CmdError> {
    if p::bool(v, "top") == Some(true) {
        return Ok(Some(("#_top".into(), "Top of the Document".into())));
    }
    if let Some(name) = p::str(v, "bookmark") {
        if !s.doc.bookmarks().iter().any(|(n, _)| n == name) {
            return Err(CmdError::Params(format!("no bookmark `{name}`")));
        }
        return Ok(Some((format!("#{name}"), name.to_string())));
    }
    if let Some(block) = p::u64(v, "heading") {
        let Some((i, _, text, lead)) = crate::cmd::references::headings(s).into_iter().find(|h| h.0 as u64 == block) else {
            return Err(CmdError::Params(format!("block {block} is not a heading")));
        };
        let name = crate::cmd::references::toc_bookmark(s, i, lead)?;
        return Ok(Some((format!("#{name}"), text)));
    }
    Ok(None)
}

pub(super) fn link(s: &mut Session, v: &Value) -> CmdResult {
    let (url, label) = match p::str(v, "url") {
        Some(url) => (url.to_string(), url.to_string()),
        None => match place_target(s, v)? {
            Some(t) => t,
            None => {
                s.push_ui_request(json!({"open": "link"}));
                return sel_result(s);
            }
        },
    };
    if url.is_empty() || url.len() > 4096 {
        return Err(CmdError::Params("bad url".into()));
    }
    let style = Some("Hyperlink".to_string());
    if s.sel.is_collapsed() {
        let text = p::str(v, "text").unwrap_or(&label).to_string();
        let mut props = s.typing_props();
        props.link = Some(url.clone());
        props.style = style;
        let at = s.sel.focus.clone();
        let end = s.doc.insert_text(&at, &text, &props)?;
        s.sel = Selection::caret(end);
        s.pending = Some(CharProps::default());
    } else {
        let (a, b) = s.sel.ordered();
        s.doc.format_range(&a, &b, &|c| {
            c.link = Some(url.clone());
            c.style = style.clone();
        })?;
    }
    sel_result(s)
}

/// Whether `name` can name a bookmark: starts with a letter or underscore, contains only alphanumeric characters
/// or underscores, and is at most 40 characters (#3491). Word allows leading underscores for hidden bookmarks (_Toc, _Ref).
pub fn bookmark_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_') && name.chars().count() <= 40
}

/// Where bookmark `name` is: see [`bookmark_span`]. `None` when there is no such bookmark.
pub fn bookmark_range(doc: &wordcraft_doc::Document, name: &str) -> Option<(Pos, Pos)> {
    let (_, start) = doc.bookmarks().into_iter().find(|(n, _)| n == name)?;
    Some(bookmark_span(doc, name, &start))
}

/// The text of bookmark `name` whose start mark is at `start`: from just after that mark to its
/// end mark — in a later paragraph when it spans several (the start again when it has no end).
pub fn bookmark_span(doc: &wordcraft_doc::Document, name: &str, start: &Pos) -> (Pos, Pos) {
    let from = Pos { off: start.off.saturating_add(wordcraft_doc::para::OBJ.len_utf8()), ..start.clone() };
    let end = doc.para_paths(start.story).into_iter().skip_while(|p| *p != start.path).find_map(|path| {
        let para = doc.para(start.story, &path)?;
        let off = para.object_offsets().into_iter().find(|o| {
            (path != start.path || *o > start.off) && matches!(para.object_at(*o), Some(InlineObject::BookmarkEnd { name: n }) if n == name)
        })?;
        Some(Pos { story: start.story, path, off })
    });
    let end = end.unwrap_or_else(|| from.clone());
    (from, end)
}

/// Remove bookmark `name`'s start and end marks, in whichever paragraphs and stories they are,
/// keeping the selection on the same text. Returns whether the bookmark was there.
fn remove_bookmark(s: &mut Session, name: &str) -> Result<bool, CmdError> {
    let mark_len = wordcraft_doc::para::OBJ.len_utf8();
    let mut found = false;
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    for story in stories {
        for path in s.doc.para_paths(story) {
            let Some(p) = s.doc.para(story, &path) else { continue };
            let mut marks = Vec::new();
            for off in p.object_offsets() {
                match p.object_at(off) {
                    Some(InlineObject::BookmarkStart { name: n }) if n == name => {
                        found = true;
                        marks.push(off);
                    }
                    Some(InlineObject::BookmarkEnd { name: n }) if n == name => marks.push(off),
                    _ => {}
                }
            }
            if marks.is_empty() {
                continue;
            }
            let para = s.doc.para_mut(story, &path)?;
            for off in marks.into_iter().rev() {
                para.delete(off, off + mark_len)?;
                for at in [&mut s.sel.anchor, &mut s.sel.focus] {
                    if at.story == story && at.path == path && at.off > off {
                        at.off = at.off.saturating_sub(mark_len);
                    }
                }
            }
        }
    }
    s.clamp_selection();
    Ok(found)
}

pub(super) fn delete_bookmark(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim();
    if !remove_bookmark(s, name)? {
        return Err(CmdError::Params(format!("no bookmark `{name}`")));
    }
    sel_result(s)
}

pub(super) fn bookmark(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    if !bookmark_name_ok(&name) {
        return Err(CmdError::Params("bookmark names start with a letter, have no spaces and at most 40 characters".into()));
    }
    // Replace an existing bookmark of the same name.
    remove_bookmark(s, &name)?;
    let (a, b) = s.sel.ordered();
    let props = CharProps::default();
    let end = s.doc.insert_object(&b, InlineObject::BookmarkEnd { name: name.clone() }, &props)?;
    s.doc.insert_object(&a, InlineObject::BookmarkStart { name: name.clone() }, &props)?;
    let _ = end;
    sel_result(s)
}

/// A link the caret is on or the selection lies inside: all of it, however its formatting splits
/// it, within one paragraph.
pub struct LinkSpan {
    pub url: String,
    /// What it shows, without any picture or field in it.
    pub text: String,
    /// It holds a picture or field, so its text can't be retyped as plain text.
    pub has_objects: bool,
    from: Pos,
    to: Pos,
}

/// The link to edit (Word's Edit Hyperlink, #4974). At a caret the character after it counts
/// first, then the one before, so the end of a link is still on it.
pub fn link_at_selection(s: &Session) -> Option<LinkSpan> {
    let (a, b) = s.sel.ordered();
    if a.story != b.story || a.path != b.path {
        return None;
    }
    let p = s.doc.para_at(&a)?;
    let linked = |off: usize| p.props_of_char(off).link.is_some();
    let off = if a.off < p.len() && linked(a.off) {
        a.off
    } else if a == b {
        p.text.get(..a.off)?.char_indices().next_back()?.0
    } else {
        return None;
    };
    let url = p.props_of_char(off).link.clone()?;
    let (mut start, mut end) = (0, None);
    for (range, props) in p.run_ranges() {
        if props.link.as_deref() == Some(url.as_str()) {
            if range.end > off {
                end = Some(range.end);
            }
        } else if end.is_some() {
            break;
        } else {
            start = range.end;
        }
    }
    let end = end.filter(|e| b.off <= *e)?;
    let shown = p.text.get(start..end)?;
    Some(LinkSpan {
        text: shown.replace(wordcraft_doc::para::OBJ, ""),
        has_objects: shown.contains(wordcraft_doc::para::OBJ),
        url,
        from: Pos { off: start, ..a.clone() },
        to: Pos { off: end, ..a },
    })
}

/// Point the link at the caret somewhere else, and retype its text when `text` differs. The rest
/// of its formatting stays.
pub(super) fn edit_link(s: &mut Session, v: &Value) -> CmdResult {
    let none = || CmdError::Params("no link at the caret".into());
    link_at_selection(s).ok_or_else(none)?;
    // A heading's new `_Toc` bookmark can move the link, so it's read again after this.
    let (url, label) = match p::str(v, "url") {
        Some(url) => (url.to_string(), url.to_string()),
        None => place_target(s, v)?.ok_or_else(|| CmdError::Params("a target is required".into()))?,
    };
    if url.is_empty() || url.len() > 4096 {
        return Err(CmdError::Params("bad url".into()));
    }
    let span = link_at_selection(s).ok_or_else(none)?;
    let text = p::str(v, "text").map(|t| if t.is_empty() { label.as_str() } else { t }).filter(|t| *t != span.text);
    let (mut from, mut to) = (span.from, span.to);
    crate::cmd::controls::check_edit(s, &from, &to)?;
    if let Some(text) = text {
        if span.has_objects || text.contains(['\n', '\r', wordcraft_doc::para::OBJ]) {
            return Err(CmdError::Params("this text can't replace the link's".into()));
        }
        let first = s.doc.para_at(&from).map(|p| p.props_of_char(from.off).clone());
        s.sel = Selection { anchor: from.clone(), focus: to };
        s.pending = first;
        crate::cmd::paste_text(s, text)?;
        to = s.sel.focus.clone();
        from = Pos { off: to.off.saturating_sub(text.len()), ..to.clone() };
        s.pending = Some(CharProps::default());
    }
    s.doc.format_range(&from, &to, &|c| c.link = Some(url.clone()))?;
    sel_result(s)
}

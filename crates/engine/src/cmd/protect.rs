//! Restrict Editing (Review › Protect, #412): editing restrictions (read only, comments, tracked
//! changes, filling in forms), formatting limited to a selection of styles, exceptions — ranges
//! everyone may still edit (`w:permStart w:edGrp="everyone"`) — and an optional password that
//! guards Stop Protection (ECMA-376 §17.15.1.29; the hash is in `wordcraft_docx`).
//!
//! Enforcement lives in [`crate::Session::run`]: [`check_before`] refuses commands the
//! restrictions forbid outright, and while exceptions exist [`outside_unchanged`] checks after the
//! command that nothing outside them changed (the command is rolled back otherwise).

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, PermRange, Pos, StoryRef};

use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The editing restrictions `w:edit` names.
const MODES: [&str; 5] = ["none", "readOnly", "comments", "trackedChanges", "forms"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("review.restrict", "Restrict Editing", "Review › Protect", restrict).params(
            r#"{"mode"?: "none|readOnly|comments|trackedChanges|forms" (start enforcing; "none" stops), "formatting"?: bool (limit formatting to unlocked styles), "lockedStyles"?: [style id or name] (the styles that can't be applied; every other style is unlocked), "password"?: string (set when enforcing; required to stop a password-protected document), "stop"?: bool, "exception"?: "add" (the selection stays editable for everyone) | "remove" (exceptions touching the selection) | "removeAll"} → state"#,
        ),
    ]
}

/// The protection state, as `review.restrict` returns it and the Restrict Editing pane shows it.
pub fn state(s: &Session) -> Value {
    let st = &s.doc.settings;
    let regions: Vec<Value> =
        s.doc.perm_ranges().iter().filter(|r| editable_by(r, &s.author)).map(|r| json!({"start": r.start, "end": r.end})).collect();
    json!({
        "protection": st.protection,
        "formatting": st.protect_formatting,
        "password": !st.protect_hash.is_empty(),
        "lockedStyles": s.doc.styles.styles.iter().filter(|x| x.locked).map(|x| x.id.clone()).collect::<Vec<_>>(),
        "exceptions": regions.len(),
        "regions": regions,
    })
}

/// Is the document's editing or formatting restricted?
pub fn enforced(doc: &Document) -> bool {
    doc.settings.protection.is_some()
}

/// May `author` edit inside the range?
fn editable_by(r: &PermRange, author: &str) -> bool {
    r.everyone() || (!r.editor.is_empty() && r.editor.eq_ignore_ascii_case(author.trim()))
}

fn restrict(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(e) = p::str(v, "exception") {
        if enforced(&s.doc) {
            return Err(CmdError::Disabled("stop protection before changing the exceptions".into()));
        }
        match e {
            "add" => add_exception(s)?,
            "remove" => remove_exceptions(s, false)?,
            "removeAll" => remove_exceptions(s, true)?,
            _ => return Err(CmdError::Params("`exception` is \"add\", \"remove\" or \"removeAll\"".into())),
        }
        return Ok(state(s));
    }
    if let Some(list) = v.get("lockedStyles") {
        let Some(list) = list.as_array() else { return Err(CmdError::Params("`lockedStyles` is a list of style ids or names".into())) };
        if enforced(&s.doc) {
            return Err(CmdError::Disabled("stop protection before changing the formatting restrictions".into()));
        }
        let names: Vec<&str> = list.iter().filter_map(Value::as_str).take(10_000).collect();
        for st in &mut s.doc.styles.styles {
            st.locked = names.iter().any(|n| *n == st.id || n.eq_ignore_ascii_case(&st.name));
        }
    }
    let mode = p::str(v, "mode");
    if let Some(m) = mode
        && !MODES.contains(&m)
    {
        return Err(CmdError::Params(format!("unknown `mode` {m:?}: none, readOnly, comments, trackedChanges or forms")));
    }
    let formatting = p::bool(v, "formatting");
    let stop = p::bool(v, "stop") == Some(true) || (mode == Some("none") && formatting != Some(true));
    if stop {
        return stop_protection(s, p::str(v, "password").unwrap_or(""));
    }
    if mode.is_none() && formatting.is_none() {
        // Only the locked styles changed (or nothing): report the state.
        return Ok(state(s));
    }
    if enforced(&s.doc) && !s.doc.settings.protect_hash.is_empty() {
        return Err(CmdError::Disabled("the document is protected with a password: stop protection first".into()));
    }
    let formatting = formatting.unwrap_or(false);
    let mode = mode.unwrap_or("none");
    if mode == "none" && !formatting {
        return stop_protection(s, "");
    }
    let hash = match p::str(v, "password").filter(|p| !p.is_empty()) {
        Some(pw) => wordcraft_docx::protection_hash(pw, wordcraft_docx::DEFAULT_SPIN_COUNT).map_err(|e| CmdError::Params(e.to_string()))?,
        None => Vec::new(),
    };
    let st = &mut s.doc.settings;
    st.protection = Some(mode.to_string());
    st.protect_formatting = formatting;
    st.protect_hash = hash;
    if mode == "trackedChanges" {
        st.track_changes = true;
    }
    Ok(state(s))
}

/// Stop Protection: with the document's password when it has one. A password in the older form
/// WordCraft can't check is accepted, and the result says so (`verified: false`).
fn stop_protection(s: &mut Session, password: &str) -> CmdResult {
    let mut verified = true;
    if enforced(&s.doc) && !s.doc.settings.protect_hash.is_empty() {
        match wordcraft_docx::check_protection_password(&s.doc.settings.protect_hash, password) {
            Some(true) => {}
            Some(false) => return Err(CmdError::Params("the password is incorrect".into())),
            None if password.is_empty() => return Err(CmdError::Params("type the document's password to stop protection".into())),
            None => verified = false,
        }
    }
    let st = &mut s.doc.settings;
    st.protection = None;
    st.protect_formatting = false;
    st.protect_hash.clear();
    let mut r = state(s);
    r["verified"] = json!(verified);
    Ok(r)
}

/// Make the selection an exception everyone may edit.
fn add_exception(s: &mut Session) -> Result<(), CmdError> {
    let (a, b) = s.sel.ordered();
    if a == b || a.story != b.story {
        return Err(CmdError::Params("select the text that stays editable".into()));
    }
    let id = s.doc.perm_ranges().iter().map(|r| r.id).max().map_or(0, |m| m.saturating_add(1));
    let props = CharProps::default();
    // The end first: inserting it can't move the start.
    s.doc.insert_object(&b, InlineObject::PermEnd { id }, &props)?;
    let after_start = s.doc.insert_object(&a, InlineObject::PermStart { id, group: "everyone".into(), editor: String::new() }, &props)?;
    let mut end = b.clone();
    if end.path == a.path && end.story == a.story {
        end.off += OBJ.len_utf8();
    }
    s.sel = crate::Selection { anchor: after_start, focus: end };
    Ok(())
}

/// Remove the exceptions that touch the selection (or all of them).
fn remove_exceptions(s: &mut Session, all: bool) -> Result<(), CmdError> {
    let (a, b) = s.sel.ordered();
    let ids: Vec<u32> =
        s.doc.perm_ranges().iter().filter(|r| all || (r.start.story == a.story && r.start <= b && a <= r.end)).map(|r| r.id).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    for story in stories {
        for path in s.doc.para_paths(story) {
            let Some(p) = s.doc.para(story, &path) else { continue };
            let marks: Vec<usize> = p
                .object_offsets()
                .into_iter()
                .filter(
                    |off| matches!(p.object_at(*off), Some(InlineObject::PermStart { id, .. } | InlineObject::PermEnd { id }) if ids.contains(id)),
                )
                .collect();
            if marks.is_empty() {
                continue;
            }
            let para = s.doc.para_mut(story, &path)?;
            for off in marks.into_iter().rev() {
                para.delete(off, off + OBJ.len_utf8())?;
            }
        }
    }
    s.clamp_selection();
    Ok(())
}

/// Commands always allowed in a protected document: protection itself, file commands, undo.
fn always_allowed(id: &str) -> bool {
    id == "review.restrict" || id.starts_with("file.") || id == "edit.undo" || id == "edit.redo"
}

/// What a protected document lets command `id` do before it runs: `Err` refuses it; `Ok(true)`
/// lets it run as long as it only changes the exceptions (checked afterwards with
/// [`outside_unchanged`]).
pub(crate) fn check_before(s: &Session, id: &str, params: &Value) -> Result<bool, CmdError> {
    let Some(mode) = s.doc.settings.protection.as_deref() else { return Ok(false) };
    if always_allowed(id) {
        return Ok(false);
    }
    if s.doc.settings.protect_formatting {
        check_formatting(s, id, params)?;
    }
    let comment_ok = id.starts_with("review.") && (id.contains("Comment") || id == "review.reply");
    let exceptions = || s.doc.perm_ranges().iter().any(|r| editable_by(r, &s.author));
    match mode {
        "readOnly" if exceptions() => Ok(true),
        "readOnly" | "forms" => Err(CmdError::Disabled(format!("{id}: the document is protected (read only)"))),
        "comments" if comment_ok => Ok(false),
        "comments" if exceptions() => Ok(true),
        "comments" => Err(CmdError::Disabled(format!("{id}: only comments are allowed in this document"))),
        _ => Ok(false),
    }
}

/// Formatting limited to a selection of styles: no direct formatting, and no locked style.
fn check_formatting(s: &Session, id: &str, params: &Value) -> Result<(), CmdError> {
    let style = match id {
        "para.style" | "format.charStyle" => p::str(params, "style"),
        "para.normal" => Some("Normal"),
        "para.heading1" => Some("Heading1"),
        "para.heading2" => Some("Heading2"),
        "para.heading3" => Some("Heading3"),
        _ => None,
    };
    if let Some(name) = style {
        let locked = s.doc.styles.styles.iter().any(|st| st.locked && (st.id == name || st.name.eq_ignore_ascii_case(name)));
        return if locked { Err(CmdError::Disabled(format!("{id}: the style {name:?} is locked in this document"))) } else { Ok(()) };
    }
    let direct = (id.starts_with("format.") && !matches!(id, "format.changeCase" | "format.state" | "format.fontDialog"))
        || (id.starts_with("para.") && !matches!(id, "para.sort" | "para.dialog"))
        || id.starts_with("styles.create")
        || id.starts_with("styles.modify")
        || id.starts_with("styles.update")
        || id.starts_with("styles.delete");
    if direct {
        return Err(CmdError::Disabled(format!("{id}: formatting is limited to the document's styles")));
    }
    Ok(())
}

/// Did a command leave everything outside the exceptions as it was? Compares the text, run
/// formatting and paragraph formatting outside the ranges `author` may edit, in every story.
pub(crate) fn outside_unchanged(before: &Document, after: &Document, author: &str) -> bool {
    outside(before, author) == outside(after, author)
}

/// One piece of protected content: text with its formatting, or a paragraph mark's formatting.
#[derive(PartialEq)]
enum Piece {
    Text(String, CharProps),
    Object(Option<InlineObject>),
    Mark(Box<wordcraft_doc::ParaProps>, CharProps),
}

/// Everything outside the editable ranges, in order.
fn outside(doc: &Document, author: &str) -> Vec<Piece> {
    let editable: Vec<u32> = doc.perm_ranges().iter().filter(|r| editable_by(r, author)).map(|r| r.id).collect();
    let mut out = Vec::new();
    let stories = std::iter::once(StoryRef::Body).chain(doc.parts.keys().map(|k| StoryRef::Part(*k)));
    for story in stories {
        let mut open: Vec<u32> = Vec::new();
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            let mut k = 0;
            for (r, props) in p.run_ranges() {
                let Some(text) = p.text.get(r.clone()) else { continue };
                let mut piece = String::new();
                for (i, c) in text.char_indices() {
                    if c == OBJ {
                        let obj = p.objects.get(k);
                        k += 1;
                        match obj {
                            Some(InlineObject::PermStart { id, .. }) if editable.contains(id) => {
                                flush(&mut out, &mut piece, props, &open);
                                open.push(*id);
                                continue;
                            }
                            Some(InlineObject::PermEnd { id }) if editable.contains(id) => {
                                flush(&mut out, &mut piece, props, &open);
                                if let Some(j) = open.iter().rposition(|x| x == id) {
                                    open.remove(j);
                                }
                                continue;
                            }
                            _ => {}
                        }
                        if open.is_empty() {
                            flush(&mut out, &mut piece, props, &open);
                            out.push(Piece::Object(obj.cloned()));
                        }
                        let _ = i;
                        continue;
                    }
                    if open.is_empty() {
                        piece.push(c);
                    }
                }
                flush(&mut out, &mut piece, props, &open);
            }
            if open.is_empty() {
                out.push(Piece::Mark(Box::new(p.props.clone()), p.mark.clone()));
            }
        }
    }
    out
}

fn flush(out: &mut Vec<Piece>, piece: &mut String, props: &CharProps, open: &[u32]) {
    if !piece.is_empty() && open.is_empty() {
        out.push(Piece::Text(std::mem::take(piece), props.clone()));
    }
    piece.clear();
}

/// Did undo or redo take protection away (it may only be lifted with Stop Protection)?
pub(crate) fn protection_lifted(before: &Document, after: &Document) -> bool {
    let (b, a) = (&before.settings, &after.settings);
    b.protection.is_some() && (a.protection != b.protection || a.protect_formatting != b.protect_formatting || a.protect_hash != b.protect_hash)
}

/// The first position in an editable range after `from` (wrapping round), for Find Next Region
/// I Can Edit; with it, the range's end.
pub fn next_region(s: &Session, from: &Pos) -> Option<(Pos, Pos)> {
    let ranges: Vec<PermRange> = s.doc.perm_ranges().into_iter().filter(|r| editable_by(r, &s.author)).collect();
    let next = ranges.iter().find(|r| r.start > *from).or_else(|| ranges.first())?;
    Some((next.start.clone(), next.end.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Selection;

    fn select(s: &mut Session, text: &str) {
        s.run("select.text", &json!({"text": text})).unwrap();
    }

    #[test]
    fn read_only_with_an_exception_allows_edits_only_inside_it() {
        let mut s = Session::new(Document::from_text("Locked intro.\nFill in here.\nLocked end."));
        select(&mut s, "Fill in here.");
        s.run("review.restrict", &json!({"exception": "add"})).unwrap();
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        // Inside the exception: typing works.
        select(&mut s, "here");
        s.run("text.insert", &json!({"text": "there"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("Fill in there."));
        // Outside it: refused, and the document is as it was.
        let before = s.doc.plain_text(StoryRef::Body);
        select(&mut s, "intro");
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        assert!(s.run("format.bold", &json!({})).is_err());
        // Backspace at the start of the exception would delete outside it.
        let start = s.doc.perm_ranges()[0].start.clone();
        s.sel = Selection::caret(start);
        assert!(s.run("text.backspace", &json!({})).is_err());
        assert_eq!(s.doc.plain_text(StoryRef::Body), before);
        // Undo can't lift the protection.
        for _ in 0..5 {
            let _ = s.run("edit.undo", &json!({}));
        }
        assert_eq!(s.doc.settings.protection.as_deref(), Some("readOnly"));
    }

    #[test]
    fn password_guards_stop_protection_and_survives_docx() {
        let mut s = Session::new(Document::from_text("Secret plan.\nEditable part."));
        select(&mut s, "Editable part.");
        s.run("review.restrict", &json!({"exception": "add"})).unwrap();
        s.run("review.restrict", &json!({"mode": "readOnly", "password": "pw1", "formatting": true, "lockedStyles": ["Heading1"]})).unwrap();
        let bytes = crate::io::save_bytes("x.docx", &s.doc).unwrap();
        let doc = crate::io::open_bytes("x.docx", &bytes).unwrap();
        let st = &doc.settings;
        assert_eq!(st.protection.as_deref(), Some("readOnly"));
        assert!(st.protect_formatting);
        assert!(st.protect_hash.iter().any(|(k, v)| k == "w:algorithmName" && v == "SHA-512"));
        assert!(doc.styles.get("Heading1").is_some_and(|x| x.locked));
        let r = doc.perm_ranges();
        assert_eq!(r.len(), 1);
        assert!(r[0].everyone());
        let mut t = Session::new(doc);
        assert!(t.run("review.restrict", &json!({"stop": true, "password": "wrong"})).is_err());
        assert!(t.run("review.restrict", &json!({"mode": "none"})).is_err());
        assert!(t.run("para.heading1", &json!({})).is_err(), "locked style");
        let v = t.run("review.restrict", &json!({"stop": true, "password": "pw1"})).unwrap();
        assert_eq!(v["verified"], true);
        assert!(t.doc.settings.protection.is_none());
        // The password isn't kept for Repeat or macros.
        assert!(t.last_command.as_ref().is_none_or(|(_, p)| p.get("password").is_none()));
    }
}

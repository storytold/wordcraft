//! Working with fields like Word's field keys: show codes instead of results (Alt+F9, Shift+F9),
//! edit a field's code, unlink a field to plain text (Ctrl+Shift+F9), lock and unlock it against
//! updates (Ctrl+F11, Ctrl+Shift+F11) and move from field to field (F11, Shift+F11).
//!
//! Two kinds of field are handled: a simple field ([`InlineObject::Field`], one object whose result
//! is cached text) and a range field ([`InlineObject::FieldStart`] … [`InlineObject::FieldEnd`],
//! whose result is the ordinary content between the markers, e.g. a Zotero citation). Showing the
//! code instead of the result applies to simple fields; range fields keep showing their result.

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Pos, StoryRef};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// Longest field code accepted.
const MAX_CODE: usize = 4096;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("view.fieldCodes", "Field Codes", "View › Show", |s, v| {
            s.view.field_codes = p::bool(v, "value").unwrap_or(!s.view.field_codes);
            s.relayout();
            Ok(json!({"value": s.view.field_codes}))
        })
        .key("Alt+F9")
        .params(r#"{"value"?: bool}"#)
        .pure(),
        CommandSpec::new("fields.toggleCode", "Toggle Field Codes", "Insert › Text › Quick Parts", toggle_code).key("Shift+F9"),
        CommandSpec::new("fields.setCode", "Edit Field Code", "Insert › Text › Quick Parts", set_code).params(r#"{"code": string}"#),
        CommandSpec::new("fields.unlink", "Unlink Fields", "Insert › Text › Quick Parts", unlink).key("Mod+Shift+F9"),
        CommandSpec::new("fields.lock", "Lock Fields", "Insert › Text › Quick Parts", |s, _| set_locked(s, true)).key("Mod+F11"),
        CommandSpec::new("fields.unlock", "Unlock Fields", "Insert › Text › Quick Parts", |s, _| set_locked(s, false)).key("Mod+Shift+F11"),
        CommandSpec::new("fields.next", "Next Field", "Insert › Text › Quick Parts", |s, _| go(s, true)).key("F11").pure(),
        CommandSpec::new("fields.previous", "Previous Field", "Insert › Text › Quick Parts", |s, _| go(s, false)).key("Shift+F11").pure(),
        CommandSpec::new("fields.selected", "Selected Fields", "Insert › Text › Quick Parts", |s, _| {
            Ok(Value::Array(selected(s).iter().map(|f| f.json()).collect()))
        })
        .pure(),
    ]
}

/// A field in the document: where its object (or start marker) is, and for a range field where
/// its end marker is.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldAt {
    pub at: Pos,
    /// The `FieldEnd` of a range field; `None` for a simple field.
    pub end: Option<Pos>,
    pub instr: String,
    pub locked: bool,
    /// Simple fields: shows its code instead of its result (relative to View › Field Codes).
    pub code: bool,
}

impl FieldAt {
    fn json(&self) -> Value {
        json!({
            "pos": super::pos_json(&self.at),
            "instr": self.instr,
            "locked": self.locked,
            "range": self.end.is_some(),
            "code": self.code,
        })
    }
}

/// Every field of a story in document order: simple fields and range fields (by their start).
pub fn story_fields(s: &Session, story: StoryRef) -> Vec<FieldAt> {
    let mut out = Vec::new();
    for path in s.doc.para_paths(story) {
        let Some(para) = s.doc.para(story, &path) else { continue };
        for off in para.object_offsets() {
            if let Some(InlineObject::Field { instr, locked, code, .. }) = para.object_at(off) {
                out.push(FieldAt { at: Pos::new(story, path.clone(), off), end: None, instr: instr.clone(), locked: *locked, code: *code });
            }
        }
    }
    for r in s.doc.field_ranges(story) {
        out.push(FieldAt { at: r.start, end: Some(r.end), instr: r.instr, locked: r.locked, code: false });
    }
    out.sort_by(|a, b| a.at.cmp(&b.at));
    out
}

/// The fields the selection holds: every field starting inside it; with a caret (or a selection
/// holding none), the simple field right after or right before the caret, else the innermost
/// range field around it.
pub fn selected(s: &Session) -> Vec<FieldAt> {
    let (a, b) = s.sel.ordered();
    let all = story_fields(s, a.story);
    if a != b {
        let inside: Vec<FieldAt> = all.iter().filter(|f| f.at >= a && f.at < b).cloned().collect();
        if !inside.is_empty() {
            return inside;
        }
    }
    let width = OBJ.len_utf8();
    let simple = |off: usize| all.iter().find(|f| f.end.is_none() && f.at.path == a.path && f.at.off == off).cloned();
    if let Some(f) = simple(a.off).or_else(|| a.off.checked_sub(width).and_then(simple)) {
        return vec![f];
    }
    // The innermost range field around the caret: the last-starting one that contains it.
    all.into_iter().rfind(|f| f.end.as_ref().is_some_and(|e| f.at < a && a <= *e)).into_iter().collect()
}

fn none_selected() -> CmdError {
    CmdError::Failed("there is no field in the selection".into())
}

/// Change the object a field's position names (the simple field or the range field's start).
fn with_field(s: &mut Session, at: &Pos, f: impl FnOnce(&mut InlineObject)) -> Result<(), CmdError> {
    let para = s.doc.para_mut(at.story, &at.path)?;
    if let Some(o) = para.object_at_mut(at.off) {
        f(o);
    }
    para.touch();
    Ok(())
}

/// Shift+F9: the selected simple fields switch between showing their code and their result.
fn toggle_code(s: &mut Session, _: &Value) -> CmdResult {
    let fields: Vec<FieldAt> = selected(s).into_iter().filter(|f| f.end.is_none()).collect();
    if fields.is_empty() {
        return Err(none_selected());
    }
    for f in &fields {
        with_field(s, &f.at, |o| {
            if let InlineObject::Field { code, .. } = o {
                *code = !*code;
            }
        })?;
    }
    Ok(json!({"fields": fields.len()}))
}

/// Selected simple fields show their results again (after F9).
pub fn show_results(s: &mut Session) -> Result<(), CmdError> {
    for f in selected(s).into_iter().filter(|f| f.code) {
        with_field(s, &f.at, |o| {
            if let InlineObject::Field { code, .. } = o {
                *code = false;
            }
        })?;
    }
    Ok(())
}

/// Set the selected field's code (the first one, when several are selected), then update fields
/// so its result follows.
fn set_code(s: &mut Session, v: &Value) -> CmdResult {
    let code = p::req_str(v, "code")?.trim().to_string();
    if code.is_empty() || code.len() > MAX_CODE || code.contains(OBJ) {
        return Err(CmdError::Params("bad field code".into()));
    }
    let Some(f) = selected(s).into_iter().next() else { return Err(none_selected()) };
    with_field(s, &f.at, |o| match o {
        InlineObject::Field { instr, .. } | InlineObject::FieldStart { instr, .. } => *instr = code.clone(),
        _ => {}
    })?;
    super::references::update_fields(s)?;
    Ok(json!({"instr": code}))
}

/// Ctrl+F11 / Ctrl+Shift+F11: lock or unlock the selected fields (a locked field keeps its result
/// when fields update).
fn set_locked(s: &mut Session, value: bool) -> CmdResult {
    let fields = selected(s);
    if fields.is_empty() {
        return Err(none_selected());
    }
    for f in &fields {
        with_field(s, &f.at, |o| match o {
            InlineObject::Field { locked, .. } | InlineObject::FieldStart { locked, .. } => *locked = value,
            _ => {}
        })?;
    }
    Ok(json!({"fields": fields.len(), "locked": value}))
}

/// The text a simple field shows now (page numbers from the current layout).
fn shown_text(s: &mut Session, f: &FieldAt) -> String {
    let Some(InlineObject::Field { instr, result, .. }) = s.doc.para_at(&f.at).and_then(|p| p.object_at(f.at.off)).cloned() else {
        return String::new();
    };
    let l = s.layout();
    let page = l.caret(&f.at).and_then(|c| l.pages.get(c.page));
    let ctx = wordcraft_layout::fields::FieldCtx {
        page: page.map(|p| p.number).unwrap_or(1),
        pages: u32::try_from(l.pages.len()).unwrap_or(u32::MAX),
        section_pages: u32::try_from(l.pages.len()).unwrap_or(u32::MAX),
        section: 1,
        title: s.doc.core.title.as_str().into(),
        author: s.doc.core.creator.as_str().into(),
        filename: s.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default().into(),
        ..Default::default()
    };
    wordcraft_layout::fields::field_text(&instr, &result, &ctx).0
}

/// Ctrl+Shift+F9: replace the selected fields with what they show (a simple field becomes its
/// result text; a range field loses its markers and keeps its content).
fn unlink(s: &mut Session, _: &Value) -> CmdResult {
    let fields = selected(s);
    if fields.is_empty() {
        return Err(none_selected());
    }
    let texts: Vec<String> = fields.iter().map(|f| if f.end.is_none() { shown_text(s, f) } else { String::new() }).collect();
    // Markers to remove and text to put in their place, last first so earlier positions hold.
    let mut edits: Vec<(Pos, String)> = Vec::new();
    for (f, t) in fields.iter().zip(texts) {
        if let Some(end) = &f.end {
            edits.push((end.clone(), String::new()));
        }
        edits.push((f.at.clone(), t));
    }
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    edits.dedup_by(|a, b| a.0 == b.0);
    let width = OBJ.len_utf8();
    for (at, text) in &edits {
        let para = s.doc.para_mut(at.story, &at.path)?;
        if para.object_at(at.off).is_none() {
            continue;
        }
        let props = para.props_of_char(at.off).clone();
        para.delete(at.off, at.off + width)?;
        para.insert_text(at.off, text, &props)?;
    }
    s.clamp_selection();
    Ok(json!({"fields": fields.len()}))
}

/// F11 / Shift+F11: select the next or previous field in the story (wrapping around).
fn go(s: &mut Session, forward: bool) -> CmdResult {
    let (a, _) = s.sel.ordered();
    let all = story_fields(s, a.story);
    let target = if forward {
        let after = |f: &&FieldAt| if s.sel.is_collapsed() { f.at >= a } else { f.at > a };
        all.iter().find(after).or(all.first())
    } else {
        all.iter().rev().find(|f| f.at < a).or(all.last())
    };
    let Some(f) = target else { return Err(CmdError::Failed("the document has no fields".into())) };
    let width = OBJ.len_utf8();
    let end = match &f.end {
        Some(e) => Pos { off: e.off + width, ..e.clone() },
        None => Pos { off: f.at.off + width, ..f.at.clone() },
    };
    s.sel = Selection { anchor: f.at.clone(), focus: end };
    sel_result(s)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::para::InlineObject;

    use crate::Session;

    fn session_with_fields() -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Made by "})).unwrap();
        s.run("insert.field", &json!({"instr": "AUTHOR", "result": "Old"})).unwrap();
        s.run("text.insert", &json!({"text": " on "})).unwrap();
        s.run("insert.field", &json!({"instr": "TITLE", "result": "T"})).unwrap();
        s.doc.core.creator = "Ada".into();
        s
    }

    fn page_text(s: &mut Session) -> String {
        let l = s.layout();
        let mut out = String::new();
        for page in &l.pages {
            for it in &page.items {
                if let wordcraft_layout::Placed::Lines { para, .. } = it {
                    for (_, shown, _) in &para.shown {
                        out.push_str(shown);
                        out.push('|');
                    }
                }
            }
        }
        out
    }

    /// Alt+F9 shows every field's code in braces; Shift+F9 flips just the selected one; F9 shows
    /// its result again.
    #[test]
    fn field_codes_show_in_place_of_results() {
        let mut s = session_with_fields();
        s.run("caret.docStart", &json!({})).unwrap();
        s.run("fields.next", &json!({})).unwrap();
        assert!(!page_text(&mut s).contains("{ AUTHOR }"));
        s.run("view.fieldCodes", &json!({})).unwrap();
        let shown = page_text(&mut s);
        assert!(shown.contains("{ AUTHOR }") && shown.contains("{ TITLE }"), "{shown}");
        s.run("view.fieldCodes", &json!({"value": false})).unwrap();
        // Shift+F9 on the selected (first) field only.
        s.run("fields.toggleCode", &json!({})).unwrap();
        let shown = page_text(&mut s);
        assert!(shown.contains("{ AUTHOR }") && !shown.contains("{ TITLE }"), "{shown}");
        // F9 updates and shows the result again.
        s.run("references.updateFields", &json!({})).unwrap();
        let shown = page_text(&mut s);
        assert!(!shown.contains("{ AUTHOR }") && shown.contains("Ada"), "{shown}");
        // The toggle is undoable and never saved to files.
        s.run("fields.toggleCode", &json!({})).unwrap();
        assert!(serde_json::to_string(&s.doc).unwrap().contains("\"code\":true"));
        assert!(s.undo());
        assert!(!page_text(&mut s).contains("{ AUTHOR }"));
    }

    /// Editing a code changes the instruction and the result; lock keeps a result through updates;
    /// unlink turns the field into its text in one undo step; F11/Shift+F11 walk the fields.
    #[test]
    fn edit_lock_unlink_and_walk_fields() {
        let mut s = session_with_fields();
        s.run("caret.docStart", &json!({})).unwrap();
        s.run("fields.next", &json!({})).unwrap();
        assert_eq!(s.run("fields.selected", &json!({})).unwrap()[0]["instr"], "AUTHOR");
        s.run("fields.next", &json!({})).unwrap();
        assert_eq!(s.run("fields.selected", &json!({})).unwrap()[0]["instr"], "TITLE");
        s.run("fields.next", &json!({})).unwrap();
        assert_eq!(s.run("fields.selected", &json!({})).unwrap()[0]["instr"], "AUTHOR", "wraps around");
        s.run("fields.previous", &json!({})).unwrap();
        assert_eq!(s.run("fields.selected", &json!({})).unwrap()[0]["instr"], "TITLE");
        s.run("fields.previous", &json!({})).unwrap();

        // Edit the code: the result follows.
        s.doc.core.title = "Report".into();
        s.run("fields.setCode", &json!({"code": "TITLE"})).unwrap();
        let objs = |s: &Session| s.doc.body.first().and_then(|b| b.as_para()).map(|p| p.objects.clone()).unwrap_or_default();
        assert!(matches!(objs(&s).first(), Some(InlineObject::Field { instr, result, .. }) if instr == "TITLE" && result == "Report"));
        assert!(s.run("fields.setCode", &json!({"code": ""})).is_err());

        // A locked field keeps its result.
        s.run("fields.lock", &json!({})).unwrap();
        s.doc.core.title = "Changed".into();
        s.run("references.updateFields", &json!({})).unwrap();
        assert!(matches!(objs(&s).first(), Some(InlineObject::Field { result, locked: true, .. }) if result == "Report"));
        s.run("fields.unlock", &json!({})).unwrap();
        s.run("references.updateFields", &json!({})).unwrap();
        assert!(matches!(objs(&s).first(), Some(InlineObject::Field { result, locked: false, .. }) if result == "Changed"));

        // Unlink: the field becomes its text; Undo brings it back.
        s.run("fields.unlink", &json!({})).unwrap();
        assert_eq!(objs(&s).len(), 1);
        assert!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).starts_with("Made by Changed on "));
        assert!(s.undo());
        assert_eq!(objs(&s).len(), 2);
        // Nothing selected: an error, not a no-op undo step.
        s.run("caret.docStart", &json!({})).unwrap();
        assert!(s.run("fields.lock", &json!({})).is_err());
    }
}

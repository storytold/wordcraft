//! Restrict Editing (Review › Protect, #412): editing restrictions (read only, comments, tracked
//! changes, filling in forms), formatting limited to a selection of styles, exceptions — ranges
//! everyone may still edit (`w:permStart w:edGrp="everyone"`) — and an optional password that
//! guards Stop Protection (ECMA-376 §17.15.1.29; the hash is in `wordcraft_docx`).
//!
//! Enforcement lives in [`crate::Session::run`]: [`check_before`] refuses commands the
//! restrictions forbid outright, and while exceptions exist [`outside_unchanged`] checks after the
//! command that nothing outside them changed (the command is rolled back otherwise).

use serde_json::{Value, json};
use std::collections::HashSet;
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::props::{CellProps, CharProps, RowProps, TableProps};
use wordcraft_doc::{Block, Blocks, Document, Path, PermRange, Pos, StoryRef};

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
        "formFields": s.doc.fillable_controls(),
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

/// The error Stop Protection reports when the document's password hash uses an algorithm or form
/// WordCraft can't check; the protection stays on.
pub const UNSUPPORTED_PASSWORD: &str = "This password type isn't supported";

/// Stop Protection: with the document's password when it has one. The password is checked like
/// Word checks it — the current hash form and the legacy one (ECMA-376 Part 4 §14.8.1); a hash
/// WordCraft can't check refuses ([`UNSUPPORTED_PASSWORD`]) rather than unlocking.
fn stop_protection(s: &mut Session, password: &str) -> CmdResult {
    if enforced(&s.doc) {
        use wordcraft_docx::PasswordCheck;
        match wordcraft_docx::check_protection_password(&s.doc.settings.protect_hash, password) {
            PasswordCheck::NoPassword | PasswordCheck::Correct => {}
            PasswordCheck::Incorrect if password.is_empty() => {
                return Err(CmdError::Params("type the document's password to stop protection".into()));
            }
            PasswordCheck::Incorrect => return Err(CmdError::Params("the password is incorrect".into())),
            PasswordCheck::Unsupported => return Err(CmdError::Disabled(UNSUPPORTED_PASSWORD.into())),
        }
    }
    let st = &mut s.doc.settings;
    st.protection = None;
    st.protect_formatting = false;
    st.protect_hash.clear();
    Ok(state(s))
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
    // Word blocks content-control property edits until Stop Protection: changing a control's
    // lock flags would reshape what forms mode lets through. The param's presence is the
    // trigger — absent, the command keeps the current lock (as `check_formatting` inspects
    // params); other properties (title, tag, placeholder…) are unaffected.
    if id == "developer.properties" && (params.get("lockEdit").is_some() || params.get("lockDelete").is_some()) {
        return Err(CmdError::Disabled(format!("{id}: stop protection before changing a content control's locks")));
    }
    if s.doc.settings.protect_formatting {
        check_formatting(s, id, params)?;
    }
    let comment_ok = id.starts_with("review.") && (id.contains("Comment") || id == "review.reply");
    let exceptions = || s.doc.perm_ranges().iter().any(|r| editable_by(r, &s.author));
    match mode {
        "readOnly" if exceptions() => Ok(true),
        // Filling in forms: the interiors of controls that allow it stay editable.
        "forms" if s.doc.fillable_controls() > 0 => Ok(true),
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
        "para.style" | "format.charStyle" | "table.style" => p::str(params, "style"),
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
        || id.starts_with("styles.delete")
        // Direct formatting outside the prefixes: Paste Formatting, and table and cell formatting.
        || matches!(
            id,
            "edit.pasteFormat"
                | "table.shading"
                | "table.borders"
                | "table.borderPainter"
                | "table.cellAlign"
                | "table.cellMargins"
                | "table.textDirection"
                | "table.newStyle"
                | "table.modifyStyle"
                | "table.deleteStyle"
                | "tools.updateStyles"
        )
        // Attach Template updates the styles from it when asked (`linkStyles` is only a setting).
        || (id == "tools.attachTemplate" && p::bool(params, "update") == Some(true));
    if direct {
        return Err(CmdError::Disabled(format!("{id}: formatting is limited to the document's styles")));
    }
    Ok(())
}

/// Which parts of a protected document stay editable: the `w:permStart` ranges `author` may
/// edit (readOnly, comments), or — filling in forms — the interiors of the content controls
/// whose `w:lock` allows editing them.
enum HoleMode {
    Perms(String),
    Controls,
}

/// Did a command leave everything outside the editable regions as it was? Compares the text,
/// run formatting and paragraph formatting outside the ranges `author` may edit, in every story,
/// and the formatting of every table, row and cell that holds any of that protected content.
pub(crate) fn outside_unchanged(before: &Document, after: &Document, author: &str) -> bool {
    // Protection can't change mid-command (`protection_lifted` guards undo and redo).
    let mode = if Some("forms") == before.settings.protection.as_deref() { HoleMode::Controls } else { HoleMode::Perms(author.into()) };
    outside(before, &mode) == outside(after, &mode)
}

/// One piece of protected content: text with its formatting, a paragraph mark's formatting, or
/// the formatting of a table, row or cell holding protected content.
#[derive(PartialEq, Debug)]
enum Piece {
    Text(String, CharProps),
    Object(Option<InlineObject>),
    Mark(Box<wordcraft_doc::ParaProps>, CharProps),
    Table(Box<TableProps>, Vec<f32>),
    Row(Box<RowProps>),
    Cell(Box<CellProps>),
}

/// The control markers of a story that pair, bracket-style — the start and end positions of
/// the doc crate's [`Document::control_ranges`], which pairs per block list (never across a
/// table-cell boundary) and skips controls wrapped around tables. These are the same markers
/// [`outside`] walks and [`Document::fillable_controls`] counts, so the walk and the count
/// agree. A start that never closes (or an end nothing opened) is an orphan, not a hole: it
/// stays ordinary protected content, and the text after it stays protected.
fn paired_control_markers(doc: &Document, story: StoryRef) -> HashSet<(Path, usize)> {
    doc.control_ranges(story).into_iter().flat_map(|r| [(r.start.path, r.start.off), (r.end.path, r.end.off)]).collect()
}

/// Everything outside the editable regions, in order.
fn outside(doc: &Document, mode: &HoleMode) -> Vec<Piece> {
    let editable: Vec<u32> = match mode {
        HoleMode::Perms(author) => doc.perm_ranges().iter().filter(|r| editable_by(r, author)).map(|r| r.id).collect(),
        HoleMode::Controls => Vec::new(),
    };
    let mut out = Vec::new();
    let stories = std::iter::once(StoryRef::Body).chain(doc.parts.keys().map(|k| StoryRef::Part(*k)));
    for story in stories {
        let mut open: Vec<u32> = Vec::new();
        let mut fillable: Vec<bool> = Vec::new();
        // In forms mode, only markers that pair open and close holes (orphans stay content).
        let paired = if matches!(mode, HoleMode::Controls) { paired_control_markers(doc, story) } else { HashSet::new() };
        // The perm range or fillable control the walk is inside, innermost last.
        let in_hole = |open: &[u32], fillable: &[bool]| match mode {
            HoleMode::Perms(_) => !open.is_empty(),
            HoleMode::Controls => fillable.last() == Some(&true),
        };
        // The paragraphs with protected content, and every table, row and cell above them.
        let mut protected: HashSet<Vec<u32>> = HashSet::new();
        let mut holding: HashSet<Vec<u32>> = HashSet::new();
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            let pieces = out.len();
            let mut k = 0;
            for (r, props) in p.run_ranges() {
                let Some(text) = p.text.get(r.clone()) else { continue };
                let mut piece = String::new();
                for (i, c) in text.char_indices() {
                    if c == OBJ {
                        let obj = p.objects.get(k);
                        k += 1;
                        // A marker that opens or closes a hole moves the walk's stack instead.
                        let mut boundary = false;
                        match mode {
                            HoleMode::Perms(_) => match obj {
                                Some(InlineObject::PermStart { id, .. }) if editable.contains(id) => {
                                    flush(&mut out, &mut piece, props, in_hole(&open, &fillable));
                                    open.push(*id);
                                    boundary = true;
                                }
                                Some(InlineObject::PermEnd { id }) if editable.contains(id) => {
                                    flush(&mut out, &mut piece, props, in_hole(&open, &fillable));
                                    if let Some(j) = open.iter().rposition(|x| x == id) {
                                        open.remove(j);
                                    }
                                    boundary = true;
                                }
                                _ => {}
                            },
                            HoleMode::Controls => match obj {
                                Some(InlineObject::ControlStart { control }) if paired.contains(&(path.clone(), r.start + i)) => {
                                    flush(&mut out, &mut piece, props, in_hole(&open, &fillable));
                                    fillable.push(control.lock.can_edit());
                                    boundary = true;
                                }
                                Some(InlineObject::ControlEnd) if paired.contains(&(path.clone(), r.start + i)) => {
                                    flush(&mut out, &mut piece, props, in_hole(&open, &fillable));
                                    fillable.pop();
                                    boundary = true;
                                }
                                _ => {}
                            },
                        }
                        if !boundary && !in_hole(&open, &fillable) {
                            flush(&mut out, &mut piece, props, false);
                            out.push(Piece::Object(obj.cloned()));
                        }
                        continue;
                    }
                    if !in_hole(&open, &fillable) {
                        piece.push(c);
                    }
                }
                flush(&mut out, &mut piece, props, in_hole(&open, &fillable));
            }
            if !in_hole(&open, &fillable) {
                out.push(Piece::Mark(Box::new(p.props.clone()), p.mark.clone()));
            }
            let ancestors = (1..path.0.len()).filter_map(|n| path.0.get(..n));
            if out.len() > pieces {
                protected.extend(ancestors.clone().map(<[u32]>::to_vec));
            }
            holding.extend(ancestors.map(<[u32]>::to_vec));
        }
        if let Some(blocks) = doc.story(story) {
            structure(blocks, &mut Vec::new(), &protected, &holding, &mut out, 0);
        }
    }
    out
}

/// The formatting of every table, row and cell in `blocks` that holds protected content (or no
/// paragraph at all): an exception covering only some of it can't reformat the rest.
fn structure(blocks: &Blocks, prefix: &mut Vec<u32>, protected: &HashSet<Vec<u32>>, holding: &HashSet<Vec<u32>>, out: &mut Vec<Piece>, depth: usize) {
    // `Document::para_paths` stops at the same depth.
    if depth > 16 {
        return;
    }
    let guarded = |at: &Vec<u32>| protected.contains(at) || !holding.contains(at);
    for (i, b) in blocks.iter().enumerate() {
        let Block::Table(t) = &**b else { continue };
        prefix.push(u32::try_from(i).unwrap_or(u32::MAX));
        if guarded(prefix) {
            out.push(Piece::Table(Box::new(t.props.clone()), t.grid.clone()));
        }
        for (r, row) in t.rows.iter().enumerate() {
            prefix.push(u32::try_from(r).unwrap_or(u32::MAX));
            if guarded(prefix) {
                out.push(Piece::Row(Box::new(row.props.clone())));
            }
            for (c, cell) in row.cells.iter().enumerate() {
                prefix.push(u32::try_from(c).unwrap_or(u32::MAX));
                if guarded(prefix) {
                    out.push(Piece::Cell(Box::new(cell.props.clone())));
                }
                structure(&cell.blocks, prefix, protected, holding, out, depth + 1);
                prefix.pop();
            }
            prefix.pop();
        }
        prefix.pop();
    }
}

fn flush(out: &mut Vec<Piece>, piece: &mut String, props: &CharProps, in_hole: bool) {
    if !piece.is_empty() && !in_hole {
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
/// I Can Edit; with it, the range's end. Filling in forms: the next fillable control's content.
pub fn next_region(s: &Session, from: &Pos) -> Option<(Pos, Pos)> {
    if s.doc.settings.protection.as_deref() == Some("forms") {
        let stories = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k)));
        let mut ranges: Vec<(Pos, Pos)> =
            stories.flat_map(|story| s.doc.control_ranges(story)).filter(|r| r.control.lock.can_edit()).map(|r| (r.content_start(), r.end)).collect();
        ranges.sort_by(|a, b| a.0.cmp(&b.0));
        let next = ranges.iter().find(|r| r.0 > *from).or_else(|| ranges.first())?;
        return Some((next.0.clone(), next.1.clone()));
    }
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

    /// Table, row and cell formatting reaches protected text, so an exception inside one cell
    /// doesn't let table commands run there change the table, the row or the cell (#1327).
    #[test]
    fn read_only_exception_in_a_cell_does_not_open_the_table() {
        let mut s = Session::new(Document::from_text("Intro."));
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        for (i, t) in ["Alpha", "Bravo", "Charlie", "Delta"].iter().enumerate() {
            if i > 0 {
                s.run("text.tab", &json!({})).unwrap();
            }
            s.run("text.insert", &json!({"text": t})).unwrap();
        }
        select(&mut s, "Alpha");
        s.run("review.restrict", &json!({"exception": "add"})).unwrap();
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        // Typing inside the exception still works.
        select(&mut s, "lph");
        s.run("text.insert", &json!({"text": "LPH"})).unwrap();
        select(&mut s, "LPH");
        let before = s.doc.clone();
        for (id, v) in [
            ("table.properties", json!({"align": "center"})),
            ("table.rowHeight", json!({"height": 40})),
            ("table.shading", json!({"color": "FF0000"})),
            ("table.cellAlign", json!({"value": "bottomRight"})),
            ("table.borders", json!({"kind": "none"})),
        ] {
            let e = s.run(id, &v).unwrap_err().to_string();
            assert!(e.contains("only the editable regions"), "{id}: {e}");
            assert_eq!(s.doc, before, "{id}");
        }
    }

    /// A table wholly inside an exception stays formattable: its table, rows and cells hold no
    /// protected content.
    #[test]
    fn read_only_exception_around_a_table_lets_it_be_formatted() {
        let mut s = Session::new(Document::from_text("Locked.\nStart."));
        s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        s.run("text.insert", &json!({"text": "Alpha"})).unwrap();
        select(&mut s, "Start.");
        let anchor = s.sel.ordered().0;
        s.sel = Selection { anchor, focus: s.doc.end_of(StoryRef::Body) };
        s.run("review.restrict", &json!({"exception": "add"})).unwrap();
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        select(&mut s, "Alpha");
        s.run("table.properties", &json!({"align": "center"})).unwrap();
        s.run("table.rowHeight", &json!({"height": 40})).unwrap();
        s.run("table.shading", &json!({"color": "FF0000"})).unwrap();
        // The text before the exception is still protected.
        select(&mut s, "Locked");
        assert!(s.run("format.bold", &json!({})).is_err());
    }

    /// Formatting limited to styles: attaching a template refuses to update the styles from it
    /// (it would bypass the Update Styles block); attaching without updating still works.
    #[test]
    fn formatting_restriction_refuses_attach_template_update() {
        let mut t = Document::from_text("Template.");
        if let Some(st) = t.styles.styles.iter_mut().find(|st| st.id == "Heading1") {
            st.chr.size = Some(31.0);
        }
        let data = crate::cmd::insert::base64_encode(&crate::io::save_bytes("t.docx", &t).unwrap());
        let mut s = Session::new(Document::from_text("Text."));
        s.run("review.restrict", &json!({"mode": "none", "formatting": true})).unwrap();
        let before = s.doc.clone();
        let e = s.run("tools.attachTemplate", &json!({"path": "Report.dotx", "update": true, "data": data})).unwrap_err().to_string();
        assert!(e.contains("formatting is limited"), "{e}");
        assert_eq!(s.doc, before);
        s.run("tools.attachTemplate", &json!({"path": "Report.dotx"})).unwrap();
        assert_eq!(s.doc.settings.attached_template.as_deref(), Some("Report.dotx"));
    }

    /// Formatting limited to styles: Paste Formatting and table formatting are direct
    /// formatting, and a locked table style can't be applied (#1327).
    #[test]
    fn formatting_restriction_covers_paste_formatting_and_tables() {
        let mut s = Session::new(Document::from_text("Bold words.\nplain words."));
        s.run("insert.table", &json!({"rows": 1, "cols": 1})).unwrap();
        s.run("text.insert", &json!({"text": "Cell"})).unwrap();
        select(&mut s, "Bold");
        s.run("format.bold", &json!({})).unwrap();
        s.run("edit.copyFormat", &json!({})).unwrap();
        s.run("review.restrict", &json!({"formatting": true, "lockedStyles": ["TableGrid"]})).unwrap();
        s.run("review.restrict", &json!({"mode": "none", "formatting": true})).unwrap();
        assert!(s.doc.settings.protect_formatting);
        select(&mut s, "plain");
        let before = s.doc.clone();
        assert!(s.run("edit.pasteFormat", &json!({})).is_err(), "edit.pasteFormat");
        select(&mut s, "Cell");
        for (id, v) in [
            ("table.shading", json!({"color": "FF0000"})),
            ("table.borders", json!({"kind": "none"})),
            ("table.cellAlign", json!({"value": "bottomRight"})),
            ("table.style", json!({"style": "TableGrid"})),
        ] {
            let e = s.run(id, &v).unwrap_err().to_string();
            assert!(e.contains("formatting is limited") || e.contains("is locked"), "{id}: {e}");
        }
        assert_eq!(s.doc, before);
    }

    /// Read only, no exceptions: going to a header the document doesn't have doesn't create
    /// one, and loading or filtering recipients doesn't rewrite the merge fields' shown results
    /// (both used to change the document from a non-mutating command, #1327).
    #[test]
    fn read_only_navigation_and_recipients_leave_the_document_alone() {
        let mut s = Session::new(Document::from_text("Dear name,"));
        select(&mut s, "name");
        s.run("mailings.insertField", &json!({"field": "Name"})).unwrap();
        s.run("mailings.recipients", &json!({"rows": [{"Name": "Ada"}, {"Name": "Bob"}]})).unwrap();
        s.run("mailings.preview", &json!({"value": true})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("Ada"));
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        let (before, depth) = (s.doc.clone(), s.undo_depth());
        assert!(s.run("insert.editHeader", &json!({})).is_err());
        assert!(s.run("insert.editFooter", &json!({})).is_err());
        let _ = s.run("mailings.recipients", &json!({"rows": [{"Name": "Cyd"}]}));
        let _ = s.run("mailings.filterRecipients", &json!({"rules": [{"field": "Name", "op": "equal", "value": "Cyd"}]}));
        let _ = s.run("mailings.sortRecipients", &json!({"field": "Name", "descending": true}));
        let _ = s.run("mailings.editRecipients", &json!({"rows": [{"Name": "Dee"}]}));
        assert!(s.run("mailings.next", &json!({})).is_err());
        assert_eq!(s.doc, before);
        assert_eq!(s.undo_depth(), depth);
    }

    #[test]
    fn forms_mode_allows_edits_only_inside_fillable_controls() {
        let mut s = Session::new(Document::from_text("Locked intro.\nPick size.\nFill name.\nLocked end."));
        select(&mut s, "Pick size.");
        s.run("developer.dropDown", &json!({"tag": "size", "items": ["S", "M", "L"]})).unwrap();
        // A control that takes typing (a drop-down list doesn't: only its entries can be chosen).
        select(&mut s, "Fill name.");
        s.run("developer.plainText", &json!({"tag": "name"})).unwrap();
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        assert_eq!(s.doc.settings.protection.as_deref(), Some("forms"));
        // Filling the drop-down by tag (the caret is outside the control): allowed.
        s.run("developer.chooseItem", &json!({"control": "size", "value": "M"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("\nM\n"));
        // Typing inside the control's content: allowed (text.insert replaces the selection).
        select(&mut s, "Fill");
        s.run("text.insert", &json!({"text": "Filled"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("Filled name."));
        // Typing and formatting outside: refused, document unchanged.
        let before = s.doc.plain_text(StoryRef::Body);
        select(&mut s, "intro");
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        assert!(s.run("format.bold", &json!({})).is_err());
        assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    }

    #[test]
    fn forms_mode_without_fillable_controls_stays_locked() {
        let mut s = Session::new(Document::from_text("No controls here."));
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        select(&mut s, "controls");
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    }

    /// A content-locked control stays frozen in forms mode, next to a fillable one: its content
    /// takes no entries or deletion, while the fillable control's interior takes typing. (The
    /// locks are set before Restrict Editing: once no control is fillable, protection refuses
    /// every command outright, `developer.properties` included.)
    #[test]
    fn content_locked_control_stays_frozen_in_forms_mode() {
        let mut s = Session::new(Document::from_text("Fill me [combo here] now."));
        select(&mut s, "combo here");
        s.run("developer.comboBox", &json!({"tag": "deg", "items": ["BSc", "MSc"]})).unwrap();
        select(&mut s, "now");
        s.run("developer.richText", &json!({"tag": "free"})).unwrap();
        s.run("developer.properties", &json!({"control": "deg", "lockEdit": true, "lockDelete": true})).unwrap();
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        // The content-locked control's own content is frozen: refused.
        assert!(s.run("developer.chooseItem", &json!({"control": "deg", "value": "MSc"})).is_err());
        // Deleting a lockDelete control is still refused under forms mode.
        assert!(s.run("developer.removeControl", &json!({"control": "deg"})).is_err());
        // Typing in the fillable control next to it: allowed.
        select(&mut s, "now");
        s.run("text.insert", &json!({"text": "today"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("today"));
        // Text right next to the controls, inside no fillable control: refused, unchanged.
        let before = s.doc.plain_text(StoryRef::Body);
        select(&mut s, "Fill me");
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    }

    /// Word blocks content-control property edits until Stop Protection: while any restriction
    /// is enforced, `developer.properties` refuses to change a control's lock flags — the
    /// presence of `lockEdit`/`lockDelete` is the trigger, whatever their value (absent, they
    /// keep the current lock). Other properties (title, tag…) stay editable.
    #[test]
    fn lock_property_changes_are_refused_under_protection() {
        let mut s = Session::new(Document::from_text("Fill [pick] now."));
        select(&mut s, "pick");
        s.run("developer.richText", &json!({"tag": "free"})).unwrap();
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        // A lock flag on a fillable control: refused, whatever the value.
        assert!(s.run("developer.properties", &json!({"control": "free", "lockEdit": true})).is_err(), "lockEdit: true");
        assert!(s.run("developer.properties", &json!({"control": "free", "lockDelete": true})).is_err(), "lockDelete: true");
        assert!(s.run("developer.properties", &json!({"control": "free", "lockEdit": false})).is_err(), "lockEdit: false");
        // Other properties still work, and the control stays fillable.
        s.run("developer.properties", &json!({"control": "free", "title": "Renamed"})).unwrap();
        let titled = s.doc.control_ranges(StoryRef::Body).iter().find(|r| r.control.tag == "free").map(|r| r.control.title.clone());
        assert_eq!(titled, Some("Renamed".into()));
        // Any enforced mode refuses it too: read only, even with an exception that allows edits.
        s.run("review.restrict", &json!({"stop": true})).unwrap();
        select(&mut s, "now");
        s.run("review.restrict", &json!({"exception": "add"})).unwrap();
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        assert!(s.run("developer.properties", &json!({"control": "free", "lockEdit": true})).is_err());
        // Without protection, the lock flag works again (and makes the control unfillable).
        s.run("review.restrict", &json!({"stop": true})).unwrap();
        s.run("developer.properties", &json!({"control": "free", "lockEdit": true})).unwrap();
        assert_eq!(s.doc.fillable_controls(), 0);
    }

    /// The review's bypass: with forms protection enforced, a recipient could clear a control's
    /// locks through `developer.properties` — the walk never compares the marker payload the
    /// command changes, and an empty control has no content to flip either. Refused now.
    #[test]
    fn forms_mode_cannot_clear_a_controls_lock_by_properties() {
        use wordcraft_doc::control::{ContentControl, ControlKind, ControlLock, ListItem};
        let mut s = Session::new(Document::from_text("Fill now."));
        select(&mut s, "now");
        s.run("developer.richText", &json!({"tag": "free"})).unwrap();
        // An empty content-locked combo box (adjacent markers, nothing between): only the
        // payload `developer.properties` changes differs — the bypass's shape.
        let at = s.doc.end_of(StoryRef::Body);
        let props = CharProps::default();
        let mut ctl = ContentControl::new(ControlKind::ComboBox {
            items: vec![ListItem { display: "BSc".into(), value: "BSc".into() }],
            last_value: String::new(),
        });
        ctl.tag = "deg".into();
        ctl.lock = ControlLock::ContentLocked;
        let start = s.doc.insert_object(&at, InlineObject::ControlStart { control: Box::new(ctl) }, &props).unwrap();
        s.doc.insert_object(&start, InlineObject::ControlEnd, &props).unwrap();
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        assert_eq!(s.doc.fillable_controls(), 1, "only the rich-text control is fillable");
        // Clearing "cannot be edited" (or "cannot be deleted") by properties: refused.
        assert!(s.run("developer.properties", &json!({"control": "deg", "lockEdit": false})).is_err(), "clearing lockEdit");
        assert!(s.run("developer.properties", &json!({"control": "deg", "lockDelete": false})).is_err(), "clearing lockDelete");
        // The control is still content-locked: its content takes no entries.
        assert!(s.run("developer.chooseItem", &json!({"control": "deg", "value": "BSc"})).is_err());
        // After Stop Protection, the lock can be changed again.
        s.run("review.restrict", &json!({"stop": true})).unwrap();
        s.run("developer.properties", &json!({"control": "deg", "lockEdit": false})).unwrap();
        assert_eq!(s.doc.fillable_controls(), 2);
    }

    /// Locks nest: innermost wins. A locked control inside a fillable one keeps its own content
    /// frozen while the control around it stays fillable. (Typing into the locked control is
    /// already refused by the control rules; the protection walk is what refuses formatting.)
    #[test]
    fn forms_mode_nested_locks_innermost_wins() {
        let mut s = Session::new(Document::from_text("Alpha Beta."));
        select(&mut s, "Alpha Beta.");
        s.run("developer.richText", &json!({"tag": "outer"})).unwrap();
        select(&mut s, "Beta");
        s.run("developer.plainText", &json!({"tag": "inner"})).unwrap();
        s.run("developer.properties", &json!({"control": "inner", "lockEdit": true})).unwrap();
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        // The walk: the fillable control's content is a hole, the locked one's is protected.
        let texts: Vec<String> = outside(&s.doc, &HoleMode::Controls)
            .into_iter()
            .map(|p| match p {
                Piece::Text(t, _) => t,
                _ => String::new(),
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("Beta")), "the locked control's content is protected");
        assert!(!texts.iter().any(|t| t.contains("Alpha")), "the fillable control's content is a hole");
        // Formatting the fillable control's content: allowed.
        select(&mut s, "Alpha");
        s.run("format.bold", &json!({})).unwrap();
        // Formatting and typing the inner locked control's content: refused, unchanged.
        let before = s.doc.plain_text(StoryRef::Body);
        select(&mut s, "Beta");
        assert!(s.run("format.bold", &json!({})).is_err());
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    }

    /// Filling in forms: typing into an empty control's interior is allowed, typing outside any
    /// control is refused and leaves the document as it was.
    #[test]
    fn forms_mode_fills_an_empty_control() {
        let mut s = Session::new(Document::from_text("Intro.\nMiddle.\nTail."));
        select(&mut s, "Middle.");
        s.run("developer.richText", &json!({"tag": "free"})).unwrap();
        // An empty plain-text control at the caret (it shows its placeholder).
        let end = s.doc.end_of(StoryRef::Body);
        s.sel = Selection::caret(end);
        s.run("developer.plainText", &json!({"tag": "name"})).unwrap();
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        // Typing into the empty control's interior: allowed.
        let at = s.doc.control_ranges(StoryRef::Body).iter().find(|r| r.control.tag == "name").map(|r| r.content_start()).unwrap();
        s.sel = Selection::caret(at);
        s.run("text.insert", &json!({"text": "Ada"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("Ada"));
        // Typing in the rich-text control's content: allowed; outside: refused, unchanged.
        select(&mut s, "Middle");
        s.run("text.insert", &json!({"text": "!"})).unwrap();
        let before = s.doc.plain_text(StoryRef::Body);
        select(&mut s, "Intro");
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    }

    /// A control inside a table cell is a hole: filling it by tag works, and the cell next to
    /// it — outside any control — stays locked. (Controls wrapped around whole rows or cells,
    /// whose markers aren't the cell paragraph's text, are not holes; a limit for now.)
    #[test]
    fn forms_mode_fills_a_control_in_a_table_cell() {
        let mut s = Session::new(Document::from_text("Form."));
        s.run("insert.table", &json!({"rows": 1, "cols": 2})).unwrap();
        // The caret is in the first cell: give both cells text, then a drop-down in the second.
        s.run("text.insert", &json!({"text": "Size"})).unwrap();
        s.run("text.tab", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "Choice"})).unwrap();
        select(&mut s, "Choice");
        s.run("developer.dropDown", &json!({"tag": "size", "items": ["S", "M"]})).unwrap();
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        // Filling the cell's drop-down by tag: allowed.
        s.run("developer.chooseItem", &json!({"control": "size", "value": "M"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("M"));
        // Typing in the neighbouring cell, outside the control: refused, unchanged.
        let before = s.doc.plain_text(StoryRef::Body);
        select(&mut s, "Size");
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    }

    /// Discoverability in forms mode: the state counts the fillable controls (`formFields`),
    /// and Find Next Region I Can Edit walks their contents in order, wrapping round. Other
    /// modes keep the perm-range behaviour.
    #[test]
    fn form_fields_state_and_find_next_region() {
        let mut s = Session::new(Document::from_text("Alpha.\nBeta.\n"));
        select(&mut s, "Alpha.");
        s.run("developer.richText", &json!({"tag": "first"})).unwrap();
        select(&mut s, "Beta.");
        s.run("developer.richText", &json!({"tag": "second"})).unwrap();
        assert_eq!(state(&s)["formFields"], 2, "the count is reported in every mode");
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        assert_eq!(state(&s)["formFields"], 2);
        let ranges = s.doc.control_ranges(StoryRef::Body);
        let first = ranges.iter().find(|r| r.control.tag == "first").unwrap();
        let second = ranges.iter().find(|r| r.control.tag == "second").unwrap();
        let (start, end) = next_region(&s, &s.doc.start_of(StoryRef::Body)).unwrap();
        assert_eq!(start, first.content_start());
        assert_eq!(end, first.end);
        let (start, end) = next_region(&s, &first.end).unwrap();
        assert_eq!(start, second.content_start());
        assert_eq!(end, second.end);
        let (start, end) = next_region(&s, &second.end).unwrap();
        assert_eq!(start, first.content_start(), "wraps round to the first control");
        assert_eq!(end, first.end);
        // Without forms mode, Find Next Region is about perm ranges: none here, nothing to find.
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        assert_eq!(next_region(&s, &s.doc.start_of(StoryRef::Body)), None);
    }

    /// Malformed control markers — an orphan end, a hostile pile of nested starts that never
    /// close — keep forms mode conservative: no panic or loop, locked text stays locked (the
    /// pairing comes from the doc crate's per-block-list scan, so unclosed markers are orphans
    /// whatever their depth, and markers never pair across a table-cell boundary).
    #[test]
    fn forms_mode_with_malformed_markers_stays_conservative() {
        use wordcraft_doc::control::{ContentControl, ControlKind};
        let mut s = Session::new(Document::from_text("Locked.\nPick.\nEnd."));
        select(&mut s, "Pick.");
        s.run("developer.dropDown", &json!({"tag": "size", "items": ["S", "M"]})).unwrap();
        s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
        // An orphan end marker at the end of the first paragraph…
        let props = CharProps::default();
        let paths = s.doc.para_paths(StoryRef::Body);
        let at = Pos { story: StoryRef::Body, path: paths[0].clone(), off: s.doc.para(StoryRef::Body, &paths[0]).map(|p| p.len()).unwrap_or(0) };
        s.doc.insert_object(&at, InlineObject::ControlEnd, &props).unwrap();
        // …and a hostile pile of nested starts, none of them closed (deeper than any cap).
        let mut at = Pos { story: StoryRef::Body, path: paths[2].clone(), off: 0 };
        for _ in 0..24 {
            at = s
                .doc
                .insert_object(&at, InlineObject::ControlStart { control: Box::new(ContentControl::new(ControlKind::RichText)) }, &props)
                .unwrap();
        }
        // Only the drop-down is a whole control; the unpaired markers don't count as fillable.
        assert_eq!(s.doc.fillable_controls(), 1);
        s.run("developer.chooseItem", &json!({"control": "size", "value": "M"})).unwrap();
        let before = s.doc.plain_text(StoryRef::Body);
        select(&mut s, "Locked");
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        // Text after the unclosed fillable starts is still locked: they are orphans, not a hole.
        select(&mut s, "End");
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        assert_eq!(s.doc.plain_text(StoryRef::Body), before);
        // An orphan end mid-paragraph: the text after it, in the same paragraph, stays locked.
        let mid = Pos { story: StoryRef::Body, path: paths[0].clone(), off: 4 };
        s.doc.insert_object(&mid, InlineObject::ControlEnd, &props).unwrap();
        select(&mut s, "ed.");
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        assert_eq!(s.doc.plain_text(StoryRef::Body), before);
        // The walk compares the changed character: the pieces of the malformed document and of a
        // clone with one character added outside any control differ.
        let mut changed = s.doc.clone();
        let at = Pos { story: StoryRef::Body, path: paths[0].clone(), off: 0 };
        changed.insert_text(&at, "x", &CharProps::default()).unwrap();
        assert_ne!(outside(&s.doc, &HoleMode::Controls), outside(&changed, &HoleMode::Controls));
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
        t.run("review.restrict", &json!({"stop": true, "password": "pw1"})).unwrap();
        assert!(t.doc.settings.protection.is_none());
        // The password isn't kept for Repeat or macros.
        assert!(t.last_command.as_ref().is_none_or(|(_, p)| p.get("password").is_none()));
    }

    #[test]
    fn forms_protection_survives_save_and_password_guards_stop() {
        let mut s = Session::new(Document::from_text("Form: [pick]."));
        select(&mut s, "pick");
        s.run("developer.dropDown", &json!({"tag": "opt", "items": ["A", "B"]})).unwrap();
        s.run("review.restrict", &json!({"mode": "forms", "password": "pw"})).unwrap();
        s.run("developer.chooseItem", &json!({"control": "opt", "value": "B"})).unwrap();
        let bytes = crate::io::save_bytes("f.docx", &s.doc).unwrap();
        let mut t = Session::new(crate::io::open_bytes("f.docx", &bytes).unwrap());
        assert_eq!(t.doc.settings.protection.as_deref(), Some("forms"));
        assert!(t.doc.plain_text(StoryRef::Body).contains('B'));
        // Still enforced after reopen: outside edit refused, wrong password refused.
        select(&mut t, "Form");
        assert!(t.run("text.insert", &json!({"text": "x"})).is_err());
        assert!(t.run("review.restrict", &json!({"stop": true, "password": "no"})).is_err());
        // Right password lifts it; then outside edits work. Undo never lifts it.
        t.run("review.restrict", &json!({"stop": true, "password": "pw"})).unwrap();
        select(&mut t, "Form");
        t.run("text.insert", &json!({"text": "x"})).unwrap();
    }

    /// A Word file with the legacy hash (`w:cryptAlgorithmSid`): a wrong password is rejected, the
    /// right one unlocks; a hash WordCraft can't check never unlocks.
    #[test]
    fn legacy_password_is_verified_and_unknown_forms_refused() {
        let protected = |sid: &str| {
            let mut doc = Document::from_text("Locked.");
            doc.settings.protection = Some("readOnly".into());
            doc.settings.protect_hash = [
                ("w:cryptProviderType", "rsaFull"),
                ("w:cryptAlgorithmClass", "hash"),
                ("w:cryptAlgorithmType", "typeAny"),
                ("w:cryptAlgorithmSid", sid),
                ("w:cryptSpinCount", "100000"),
                // SHA-1 of salt 00…0F + Word's legacy key for "Example" (ECMA-376 Part 4 §14.8.1).
                ("w:hash", "pEWelKoH6SVpvMmyPAJ8bHA/g/s="),
                ("w:salt", "AAECAwQFBgcICQoLDA0ODw=="),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
            Session::new(doc)
        };
        let mut s = protected("4");
        assert!(s.run("review.restrict", &json!({"stop": true, "password": "wrong"})).is_err());
        assert!(s.run("review.restrict", &json!({"stop": true, "password": ""})).is_err());
        assert!(super::enforced(&s.doc));
        s.run("review.restrict", &json!({"stop": true, "password": "Example"})).unwrap();
        assert!(!super::enforced(&s.doc));
        // MD2 isn't supported: refused whatever the password.
        let mut s = protected("1");
        let e = s.run("review.restrict", &json!({"stop": true, "password": "anything"})).unwrap_err();
        assert!(e.to_string().contains(super::UNSUPPORTED_PASSWORD));
        assert!(super::enforced(&s.doc));
    }
}

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

/// Is the document's editing restricted? The formatting-only protection stores the mode "none"
/// (`w:documentProtection w:edit="none"` with `w:formatting="1"`), so it doesn't count here: it
/// limits the styles, not the editing.
pub fn editing_restricted(doc: &Document) -> bool {
    doc.settings.protection.as_deref().is_some_and(|m| m != "none")
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
    let explicit_stop = p::bool(v, "stop") == Some(true);
    // Stopping the protection is explicit: the "none" mode is the shorthand for it, but on a
    // protected document only `{"stop": true}` stops it — a command that merely names a mode
    // can't strip a user's protection (Word asks for Stop Protection first).
    if mode == Some("none") && formatting != Some(true) && !explicit_stop && enforced(&s.doc) {
        return Err(CmdError::Disabled("the document is protected: stop protection first (\"stop\": true)".into()));
    }
    let stop = explicit_stop || (mode == Some("none") && formatting != Some(true));
    if stop {
        return stop_protection(s, p::str(v, "password").unwrap_or(""));
    }
    if mode.is_none() && formatting.is_none() {
        // Only the locked styles changed (or nothing): report the state.
        return Ok(state(s));
    }
    // The protection of a protected document doesn't change: Word asks for Stop Protection first.
    // (The formatting-only protection isn't one: it limits the styles, not the editing — but a
    // password on it still guards it, so it can't be replaced either.)
    if editing_restricted(&s.doc) || !s.doc.settings.protect_hash.is_empty() {
        let why = if s.doc.settings.protect_hash.is_empty() {
            "the document is protected: stop protection first"
        } else {
            "the document is protected with a password: stop protection first"
        };
        return Err(CmdError::Disabled(why.into()));
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

/// The Developer tab's commands that insert a content control.
const CONTROL_INSERTS: [&str; 9] = [
    "developer.richText",
    "developer.plainText",
    "developer.picture",
    "developer.buildingBlock",
    "developer.checkBox",
    "developer.comboBox",
    "developer.dropDown",
    "developer.datePicker",
    "developer.repeatingSection",
];

/// Commands always allowed in a protected document: protection itself, undo, and the file
/// commands that don't change the open document. A `file.*` command that does change it is
/// checked like any other, so the protection holds for it too: the privacy options and the
/// properties are document settings, and the inspector removes content (#1327 follow-up).
fn always_allowed(id: &str, params: &Value) -> bool {
    if id == "review.restrict" || id == "edit.undo" || id == "edit.redo" {
        return true;
    }
    if !id.starts_with("file.") {
        return false;
    }
    match id {
        "file.privacyOptions" | "file.properties" => false,
        // The user name says who may edit the document and who the revisions belong to: it is
        // checked like any other change, not waved through.
        "file.setAuthor" => false,
        // The inspector reads the document; its removals change it.
        "file.inspect" => params.get("remove").and_then(Value::as_array).is_none_or(|a| a.is_empty()),
        _ => true,
    }
}

/// The user name (`file.setAuthor`) is the identity a named exception range is checked against,
/// and the author recorded on new revisions and comments: it can't change while the document is
/// protected. Word lets the user name change under protection (it is a preference there); this
/// project doesn't, because the name says who may edit what (#1327 follow-up).
pub(crate) fn check_user_name(s: &Session) -> Result<(), CmdError> {
    if enforced(&s.doc) {
        return Err(CmdError::Disabled("file.setAuthor: stop protection before changing the user name".into()));
    }
    Ok(())
}

/// A version restore replaces the whole document, including the content the protection kept
/// read-only, so it needs the protection stopped first — and the password with it, when the
/// document has one. (`file.versions` is a pure command, so `check_before` doesn't see it; the
/// command checks this itself — see [`check_version_restore`].)
pub(crate) fn check_version_restore(s: &Session) -> Result<(), CmdError> {
    if enforced(&s.doc) {
        return Err(CmdError::Disabled("file.versions: stop protection before restoring a version".into()));
    }
    Ok(())
}

/// What decides a protected document's outright refusals: whether the current author may edit
/// any exception (`w:permStart`), and how many content controls take entries. Both walk the
/// whole document.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts {
    pub exceptions: bool,
    pub fillable: usize,
}

impl Facts {
    pub(crate) fn of(s: &Session) -> Facts {
        Facts { exceptions: s.doc.perm_ranges().iter().any(|r| editable_by(r, &s.author)), fillable: s.doc.fillable_controls() }
    }
}

/// Why protection refuses command `id` whatever its params and the selection, so the ribbon greys
/// it out ([`Session::why_disabled`], with [`Facts`] cached per document revision) and
/// [`check_before`] refuses it. Observed in Word for Mac: with Filling in forms enforced, the
/// Developer tab's control buttons and Properties are greyed out and a field's shortcut menu
/// offers no Remove Content Control. The form's fields stay as they are; only their content is
/// filled in. A document that is read only (or open to comments only) with no exception the
/// author may edit takes no other edits at all.
pub(crate) fn greyed_out(s: &Session, id: &str, facts: impl FnOnce() -> Facts) -> Option<&'static str> {
    let mode = s.doc.settings.protection.as_deref()?;
    if always_allowed(id, &Value::Null) {
        return None;
    }
    if mode == "forms" {
        match id {
            "developer.removeControl" => return Some("stop protection before removing a content control"),
            "developer.properties" => return Some("stop protection before changing a content control's properties"),
            _ if CONTROL_INSERTS.contains(&id) => return Some("stop protection before inserting a content control"),
            _ => {}
        }
    }
    let comment_ok = id.starts_with("review.") && (id.contains("Comment") || id == "review.reply");
    let tracked = matches!(id, "review.accept" | "review.reject" | "review.acceptAll" | "review.rejectAll" | "review.trackChanges");
    match mode {
        "readOnly" => (!facts().exceptions).then_some("the document is protected (read only)"),
        "forms" => (facts().fillable == 0).then_some("the document is protected (read only)"),
        "comments" if !comment_ok => (!facts().exceptions).then_some("only comments are allowed in this document"),
        "trackedChanges" if tracked => Some("changes to this document are tracked"),
        _ => None,
    }
}

/// What a protected document lets command `id` do before it runs: `Err` refuses it; `Ok(true)`
/// lets it run as long as it only changes the exceptions (checked afterwards with
/// [`outside_unchanged`]).
pub(crate) fn check_before(s: &Session, id: &str, params: &Value) -> Result<bool, CmdError> {
    let Some(mode) = s.doc.settings.protection.as_deref() else { return Ok(false) };
    if always_allowed(id, params) {
        return Ok(false);
    }
    // The user name says who may edit the document, and who the new revisions and comments
    // belong to: it can't change while the document is protected. (`file.setAuthor` is a pure
    // command, so `check_before` doesn't see it; the command checks this itself — see
    // [`check_user_name`].)
    if id == "file.setAuthor" {
        check_user_name(s)?;
    }
    // Word blocks content-control property edits until Stop Protection: changing a control's
    // lock flags would reshape what forms mode lets through. The param's presence is the
    // trigger — absent, the command keeps the current lock (as `check_formatting` inspects
    // params). In other modes, other properties (title, tag, placeholder…) stay editable; forms
    // mode refuses every property change (below).
    if id == "developer.properties" && (params.get("lockEdit").is_some() || params.get("lockDelete").is_some()) {
        return Err(CmdError::Disabled(format!("{id}: stop protection before changing a content control's locks")));
    }
    // The document's properties and its privacy options are document-wide state, and they aren't
    // tracked: a document that tracks changes would take them with no revision to show for it, and
    // the revision trail would say nothing about them. They're refused while the document is
    // protected, as the content control's properties are (#3278 follow-up).
    // The inspector's removals take content out of the document — the comments, the revisions,
    // the properties, the hidden text, the headers — so they are refused in every mode too: under
    // tracked changes they would change the document with no revision to show for it, and under
    // the other modes the document may not change at all. (An inspection without removals only
    // reads the document, and is let through by `always_allowed`.)
    if enforced(&s.doc) {
        match id {
            "file.properties" => return Err(CmdError::Disabled(format!("{id}: stop protection before changing the document's properties"))),
            "file.privacyOptions" => return Err(CmdError::Disabled(format!("{id}: stop protection before changing the privacy options"))),
            "file.inspect" => return Err(CmdError::Disabled(format!("{id}: stop protection before removing content with the inspector"))),
            _ => {}
        }
    }
    if let Some(why) = greyed_out(s, id, || Facts::of(s)) {
        return Err(CmdError::Disabled(format!("{id}: {why}")));
    }
    if s.doc.settings.protect_formatting {
        check_formatting(s, id, params)?;
    }
    // Not refused outright: read only and comments mode have an exception this author may edit,
    // forms mode has fillable controls. Comments are always allowed in comments mode.
    let comment_ok = id.starts_with("review.") && (id.contains("Comment") || id == "review.reply");
    match mode {
        "comments" if comment_ok => Ok(false),
        "readOnly" | "forms" | "comments" => Ok(true),
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
                | "table.properties"
                | "table.rowHeight"
                | "table.newStyle"
                | "table.modifyStyle"
                | "table.deleteStyle"
                | "tools.updateStyles"
                // Copies style definitions in from another file, locked flags included (#3692).
                | "styles.copyFrom"
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
/// and the formatting of every table, row and cell that holds any of that protected content, and what
/// reaches the whole document: page setup, the document defaults and the existing styles
/// (applying a style may add one), the document's settings and its metadata.
pub(crate) fn outside_unchanged(before: &Document, after: &Document, author: &str) -> bool {
    // Protection can't change mid-command (`protection_lifted` guards undo and redo).
    let mode = if Some("forms") == before.settings.protection.as_deref() { HoleMode::Controls } else { HoleMode::Perms(author.into()) };
    let (b, a) = (&before.styles, &after.styles);
    before.last_section == after.last_section
        && sections(before) == sections(after)
        && b.default_chr == a.default_chr
        && b.default_para == a.default_para
        && b.styles.iter().all(|st| a.get(&st.id) == Some(st))
        && before.settings == after.settings
        && before.core == after.core
        && before.custom_props == after.custom_props
        && comments_unchanged(before, after)
        && lists_unchanged(before, after)
        && sources_unchanged(before, after)
        && outside(before, &mode) == outside(after, &mode)
}

/// The comments' resolved flags: a comment's text is a part, and the parts are compared with the
/// rest of the document; the flag is the comment's own state.
fn comments_unchanged(before: &Document, after: &Document) -> bool {
    before.comments.iter().all(|(id, c)| after.comments.get(id).is_some_and(|a| a.resolved == c.resolved))
}

/// The list definitions: the ones the document had must be unchanged — a list's format reaches
/// every paragraph that uses it — but a new one may be added, like a new style: applying a list
/// inside an editable region adds its definition.
fn lists_unchanged(before: &Document, after: &Document) -> bool {
    before.numbering.abstracts.iter().all(|x| after.numbering.abstracts.contains(x))
        && before.numbering.nums.iter().all(|x| after.numbering.nums.contains(x))
}

/// The bibliography sources: the ones the document had must be unchanged; a new one may be added
/// (a citation inserted inside an editable region).
fn sources_unchanged(before: &Document, after: &Document) -> bool {
    before.sources.iter().all(|x| after.sources.contains(x))
}

/// The section properties paragraphs end, in order.
fn sections(doc: &Document) -> Vec<&wordcraft_doc::SectionProps> {
    doc.para_paths(StoryRef::Body).iter().filter_map(|path| doc.para(StoryRef::Body, path)?.section.as_deref()).collect()
}

/// One piece of protected content: text with its formatting, a paragraph mark's formatting, or
/// the formatting of a table, row or cell holding protected content.
#[derive(PartialEq, Debug)]
enum Piece {
    Text(String, CharProps),
    Object(Option<InlineObject>),
    Mark(Box<wordcraft_doc::ParaProps>, CharProps),
    /// Tables nested deeper than the paragraph walk reaches, compared as a whole.
    Deep(String),
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
    // `Document::para_paths` stops at the same depth; deeper nesting can't hold an editable
    // exception, so any change below it counts as a change to protected content.
    if depth > 16 {
        if !blocks.is_empty() {
            out.push(Piece::Deep(format!("{blocks:?}")));
        }
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

/// Where typing at the caret `p` goes in a document that is read only or open to comments only: a
/// caret right before the start of an exception this author may edit goes inside it, and one right
/// after its end goes back inside it. The markers take no room, so the start of a line that an
/// exception starts, and the end of a line that one ends, are those carets.
pub(crate) fn typing_pos(s: &Session, p: &Pos) -> Pos {
    if !matches!(s.doc.settings.protection.as_deref(), Some("readOnly" | "comments")) {
        return p.clone();
    }
    let Some(para) = s.doc.para_at(p) else { return p.clone() };
    if para.objects.is_empty() {
        return p.clone();
    }
    let editable: Vec<u32> = s.doc.perm_ranges().iter().filter(|r| editable_by(r, &s.author)).map(|r| r.id).collect();
    let is_start = |off: usize| matches!(para.object_at(off), Some(InlineObject::PermStart { id, .. }) if editable.contains(id));
    let is_end = |off: usize| matches!(para.object_at(off), Some(InlineObject::PermEnd { id }) if editable.contains(id));
    let w = OBJ.len_utf8();
    let mut off = p.off;
    if is_start(off) {
        while is_start(off) {
            off += w;
        }
        return Pos { off, ..p.clone() };
    }
    while off >= w && is_end(off - w) {
        off -= w;
    }
    Pos { off, ..p.clone() }
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
mod tests;

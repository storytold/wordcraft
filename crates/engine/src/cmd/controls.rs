//! Developer › Controls: content controls (see [`wordcraft_doc::control`]) — insert each kind,
//! set their properties, tick check boxes, choose list entries and dates, Design Mode.
//!
//! Typing into a control that shows its placeholder replaces the placeholder; deleting all of a
//! control's content brings the placeholder back; locked controls refuse to be deleted
//! (`sdtLocked`) or edited (`contentLocked`). Those rules live here and are called from the
//! typing and deleting commands.

use serde_json::{Value, json};
use wordcraft_doc::control::{ContentControl, ControlKind, ControlLock, ControlRange, ListItem, MAX_CONTROL_TEXT, MAX_LIST_ITEMS};
use wordcraft_doc::para::OBJ;
use wordcraft_doc::props::{CharProps, Rgb, TextColor};
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{InlineObject, Pos, StoryRef};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

const LOC: &str = "Developer › Controls";
/// The character style placeholder text is shown in.
pub const PLACEHOLDER_STYLE: &str = "PlaceholderText";

pub fn specs() -> Vec<CommandSpec> {
    const INSERT: &str = r#"{"title"?: string, "tag"?: string}"#;
    const LIST: &str = r#"{"title"?: string, "tag"?: string, "items"?: [string | {"display": string, "value"?: string}]}"#;
    vec![
        CommandSpec::new("developer.richText", "Rich Text Content Control", LOC, |s, v| insert(s, v, ControlKind::RichText)).params(INSERT),
        CommandSpec::new("developer.plainText", "Plain Text Content Control", LOC, |s, v| insert(s, v, ControlKind::Text { multi_line: false }))
            .params(INSERT),
        CommandSpec::new("developer.picture", "Picture Content Control", LOC, |s, v| insert(s, v, ControlKind::Picture)).params(INSERT),
        CommandSpec::new("developer.buildingBlock", "Building Block Gallery Content Control", LOC, |s, v| {
            insert(s, v, ControlKind::Gallery { list: true, gallery: "Quick Parts".into(), category: "General".into(), unique: false })
        })
        .params(INSERT),
        CommandSpec::new("developer.checkBox", "Check Box Content Control", LOC, |s, v| insert(s, v, ControlKind::check_box()))
            .params(r#"{"title"?: string, "tag"?: string, "checked"?: bool}"#),
        CommandSpec::new("developer.comboBox", "Combo Box Content Control", LOC, |s, v| {
            insert(s, v, ControlKind::ComboBox { items: Vec::new(), last_value: String::new() })
        })
        .params(LIST),
        CommandSpec::new("developer.dropDown", "Drop-Down List Content Control", LOC, |s, v| {
            insert(s, v, ControlKind::DropDown { items: Vec::new(), last_value: String::new() })
        })
        .params(LIST),
        CommandSpec::new("developer.datePicker", "Date Picker Content Control", LOC, |s, v| {
            insert(
                s,
                v,
                ControlKind::Date { full_date: String::new(), format: "M/d/yyyy".into(), lid: "en-US".into(), calendar: "gregorian".into(), store_as: "dateTime".into() },
            )
        })
        .params(r#"{"title"?: string, "tag"?: string, "format"?: "M/d/yyyy", "date"?: "YYYY-MM-DD"}"#),
        CommandSpec::new("developer.repeatingSection", "Repeating Section Content Control", LOC, |s, v| {
            insert(s, v, ControlKind::RepeatingSection { title: String::new(), no_insert_delete: false })
        })
        .params(INSERT),
        CommandSpec::new("developer.designMode", "Design Mode", LOC, design_mode).params(r#"{"value"?: bool}"#).pure(),
        CommandSpec::new("developer.properties", "Properties", LOC, properties).params(
            r#"{"control"?: tag or title, "title"?: string, "tag"?: string, "lockDelete"?: bool, "lockEdit"?: bool, "temporary"?: bool, "multiLine"?: bool, "placeholder"?: string, "items"?: [string | {"display", "value"}], "format"?: "M/d/yyyy", "checkedSymbol"?: char, "uncheckedSymbol"?: char}"#,
        ),
        CommandSpec::new("developer.control", "Content Control at Selection", LOC, info).params(r#"{"control"?: tag or title, "at"?: pos}"#).pure(),
        CommandSpec::new("developer.toggleCheckBox", "Toggle Check Box", LOC, toggle_check_box).params(r#"{"control"?: tag or title, "at"?: pos, "value"?: bool}"#),
        CommandSpec::new("developer.chooseItem", "Choose List Item", LOC, choose_item)
            .params(r#"{"control"?: tag or title, "at"?: pos, "index"?: number, "value"?: string, "text"?: string}"#),
        CommandSpec::new("developer.setDate", "Set Date", LOC, set_date).params(r#"{"control"?: tag or title, "at"?: pos, "date": "YYYY-MM-DD"}"#),
        CommandSpec::new("developer.removeControl", "Remove Content Control", LOC, remove_control).params(r#"{"control"?: tag or title, "at"?: pos}"#),
    ]
}

fn locked(s: &mut Session, msg: &str) -> CmdError {
    s.status = msg.to_string();
    CmdError::Failed(msg.to_string())
}

const CANT_DELETE: &str = "This content control can't be deleted.";
const CANT_EDIT: &str = "The contents of this content control can't be edited.";

/// A string param, trimmed to what a control keeps.
fn short(v: &Value, k: &str) -> Option<String> {
    p::str(v, k).map(|t| t.chars().filter(|c| !c.is_control() && *c != OBJ).take(MAX_CONTROL_TEXT).collect())
}

/// List entries from `items`: strings, or `{display, value}` objects.
fn items_param(v: &Value) -> Option<Vec<ListItem>> {
    let arr = v.get("items")?.as_array()?;
    let clean = |t: &str| -> String { t.chars().filter(|c| !c.is_control() && *c != OBJ).take(MAX_CONTROL_TEXT).collect() };
    Some(
        arr.iter()
            .take(MAX_LIST_ITEMS)
            .filter_map(|i| match i {
                Value::String(t) => Some(ListItem { display: clean(t), value: clean(t) }),
                Value::Object(_) => {
                    let display = clean(i.get("display").and_then(Value::as_str)?);
                    let value = i.get("value").and_then(Value::as_str).map(clean).unwrap_or_else(|| display.clone());
                    Some(ListItem { display, value })
                }
                _ => None,
            })
            .filter(|i| !i.display.is_empty())
            .collect(),
    )
}

/// A symbol param: one character, not a control or object character.
fn symbol_param(v: &Value, k: &str) -> Option<char> {
    let t = p::str(v, k)?;
    let mut it = t.chars();
    let c = it.next()?;
    (it.next().is_none() && !c.is_control() && c != OBJ).then_some(c)
}

/// A date `YYYY-MM-DD` (a full ISO date-time is fine too): (year, month, day), checked.
pub fn parse_date(t: &str) -> Option<(i64, u32, u32)> {
    let t = t.trim();
    let y: i64 = t.get(0..4)?.parse().ok()?;
    let m: u32 = t.get(5..7)?.parse().ok()?;
    let d: u32 = t.get(8..10)?.parse().ok()?;
    if t.get(4..5) != Some("-") || t.get(7..8) != Some("-") || !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m)
    {
        return None;
    }
    Some((y, m, d))
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a date (inverse of [`super::civil_from_days`]).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The placeholder style, added to the document when it hasn't one.
fn ensure_placeholder_style(s: &mut Session) {
    if s.doc.styles.get(PLACEHOLDER_STYLE).is_none() {
        s.doc.styles.styles.push(Style {
            id: PLACEHOLDER_STYLE.into(),
            name: "Placeholder Text".into(),
            kind: StyleKind::Character,
            based_on: Some("DefaultParagraphFont".into()),
            chr: CharProps { color: Some(TextColor::Rgb(Rgb(0x66, 0x66, 0x66))), ..Default::default() },
            priority: Some(99),
            builtin: true,
            ..Default::default()
        });
    }
}

/// Formatting for placeholder text based on `base`.
fn placeholder_props(s: &mut Session, base: &CharProps) -> CharProps {
    ensure_placeholder_style(s);
    CharProps { style: Some(PLACEHOLDER_STYLE.into()), link: None, ..base.clone() }
}

/// `base` without the placeholder style (for real content).
fn content_props(base: &CharProps) -> CharProps {
    let mut p = base.clone();
    if p.style.as_deref() == Some(PLACEHOLDER_STYLE) {
        p.style = None;
    }
    p
}

/// An id no control in the document has.
fn new_id(s: &Session) -> i64 {
    let mut used = std::collections::BTreeSet::new();
    let mut add = |bl: &wordcraft_doc::Blocks| {
        wordcraft_doc::control::walk_controls(bl, &mut |c| {
            if let Some(id) = c.id {
                used.insert(id);
            }
        })
    };
    add(&s.doc.body);
    for p in s.doc.parts.values() {
        add(&p.blocks);
    }
    let mut id = 1_000_000 + used.len() as i64;
    while used.contains(&id) {
        id += 1;
    }
    id
}

// ---------------------------------------------------------------------------------------------
// Finding the control a command is about

/// The control at `p`, also when `p` is just outside its markers (a click beside a check box).
fn control_near(s: &Session, at: &Pos) -> Option<ControlRange> {
    if let Some(r) = s.doc.control_at(at) {
        return Some(r);
    }
    let para = s.doc.para_at(at)?;
    let w = OBJ.len_utf8();
    if matches!(para.object_at(at.off), Some(InlineObject::ControlStart { .. })) {
        return s.doc.control_at(&Pos { off: at.off + w, ..at.clone() });
    }
    if at.off >= w && matches!(para.object_at(at.off - w), Some(InlineObject::ControlEnd)) {
        return s.doc.control_at(&Pos { off: at.off - w, ..at.clone() });
    }
    None
}

/// The control a command acts on: `control` (a tag or title) in the caret's story, else the one
/// at `at` (a position), else the one at the caret.
fn target(s: &Session, v: &Value) -> Result<ControlRange, CmdError> {
    if let Some(name) = p::str(v, "control") {
        let story = s.sel.focus.story;
        let stories = std::iter::once(story).chain(std::iter::once(StoryRef::Body));
        for st in stories {
            if let Some(r) =
                s.doc.control_ranges(st).into_iter().filter(|r| r.control.tag == name || r.control.title == name).min_by(|a, b| a.start.cmp(&b.start))
            {
                return Ok(r);
            }
        }
        return Err(CmdError::Failed(format!("no content control is called {name:?}")));
    }
    let at = v.get("at").and_then(super::parse_pos).unwrap_or_else(|| s.sel.focus.clone());
    control_near(s, &at).ok_or_else(|| CmdError::Disabled("the selection isn't in a content control".into()))
}

/// Replace a control's content with `text` (one paragraph's worth) in `props`; returns the end.
fn set_content(s: &mut Session, r: &ControlRange, text: &str, props: &CharProps) -> Result<Pos, CmdError> {
    let a = r.content_start();
    s.doc.delete_range(&a, &r.end)?;
    let end = s.doc.insert_text(&a, text, props)?;
    Ok(end)
}

/// The formatting of a control's content (its first character's), without placeholder styling.
fn content_base(s: &Session, r: &ControlRange) -> CharProps {
    let a = r.content_start();
    let props = s
        .doc
        .para_at(&a)
        .map(|p| if r.is_empty() { p.props_of_char(r.start.off).clone() } else { p.props_of_char(a.off).clone() })
        .unwrap_or_default();
    content_props(&props)
}

// ---------------------------------------------------------------------------------------------
// Rules for typing and deleting

/// Before text goes in at the selection: refuse what a locked or list-only control doesn't
/// take; a control showing its placeholder gets the placeholder selected (so it is replaced)
/// and stops showing it. Returns the formatting the text should get, when it changes.
pub fn prepare_edit(s: &mut Session) -> Result<Option<CharProps>, CmdError> {
    if s.sel.is_collapsed() {
        let at = s.doc.typing_pos(&s.sel.focus);
        if at != s.sel.focus {
            s.sel = Selection::caret(at);
        }
    }
    // A caret at the very edge of a control that takes no typing types beside it instead.
    for _ in 0..16 {
        if !s.sel.is_collapsed() {
            break;
        }
        let f = s.sel.focus.clone();
        let Some(r) = s.doc.control_at(&f) else { break };
        if r.control.lock.can_edit() && r.control.kind.takes_typing() {
            break;
        }
        let w = OBJ.len_utf8();
        if f == r.end {
            s.sel = Selection::caret(Pos { off: r.end.off + w, ..r.end.clone() });
        } else if f == r.content_start() {
            s.sel = Selection::caret(r.start.clone());
        } else {
            break;
        }
    }
    let (a, b) = s.sel.ordered();
    let Some(r) = s.doc.control_at(&a) else { return Ok(None) };
    if !r.control.lock.can_edit() {
        return Err(locked(s, CANT_EDIT));
    }
    if !r.control.kind.takes_typing() {
        let msg = match r.control.kind {
            ControlKind::CheckBox { .. } => "Click the check box (or use Toggle Check Box) to change it.",
            ControlKind::DropDown { .. } => "Choose an entry from the list.",
            _ => "This content control holds a picture.",
        };
        return Err(locked(s, msg));
    }
    if !r.control.showing_placeholder || !r.contains(&b) {
        return Ok(None);
    }
    let props = content_base(s, &r);
    s.sel = Selection { anchor: r.content_start(), focus: r.end.clone() };
    if let Some(c) = s.doc.control_mut(&r.start) {
        c.showing_placeholder = false;
    }
    Ok(Some(props))
}

/// Enter in a one-line plain text control does nothing.
pub fn refuses_paragraph(s: &Session) -> bool {
    s.doc.control_at(&s.sel.focus).is_some_and(|r| {
        matches!(r.control.kind, ControlKind::Text { multi_line: false } | ControlKind::CheckBox { .. } | ControlKind::DropDown { .. })
    })
}

/// Before a deletion of `a..b`: refuse deleting a control that can't be deleted, or editing
/// content that can't be edited.
pub fn check_delete(s: &mut Session, a: &Pos, b: &Pos) -> Result<(), CmdError> {
    if a.story != b.story {
        return Ok(());
    }
    let w = OBJ.len_utf8();
    let mut refusal = None;
    for r in s.doc.control_ranges(a.story) {
        let whole = (&a.path, a.off) <= (&r.start.path, r.start.off) && (&r.end.path, r.end.off + w) <= (&b.path, b.off);
        let cs = r.content_start();
        let touches = (&a.path, a.off) < (&r.end.path, r.end.off) && (&b.path, b.off) > (&cs.path, cs.off);
        if whole && !r.control.lock.can_delete() {
            refusal = Some(CANT_DELETE);
            break;
        }
        if !whole && touches && !r.control.lock.can_edit() {
            refusal = Some(CANT_EDIT);
            break;
        }
    }
    match refusal {
        Some(m) => Err(locked(s, m)),
        None => Ok(()),
    }
}

/// After the user deleted something: a control left empty shows its placeholder again.
pub fn restore_placeholder(s: &mut Session) -> Result<(), CmdError> {
    let at = s.sel.focus.clone();
    let Some(r) = s.doc.control_at(&at) else { return Ok(()) };
    if !r.is_empty() || r.control.showing_placeholder {
        return Ok(());
    }
    let text = r.control.placeholder_shown().to_string();
    if text.is_empty() {
        return Ok(());
    }
    let base = content_base(s, &r);
    let props = placeholder_props(s, &base);
    s.doc.insert_text(&r.content_start(), &text, &props)?;
    if let Some(c) = s.doc.control_mut(&r.start) {
        c.showing_placeholder = true;
    }
    s.sel = Selection::caret(r.content_start());
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Insert

fn insert(s: &mut Session, v: &Value, kind: ControlKind) -> CmdResult {
    let mut ctl = ContentControl::new(kind);
    ctl.id = Some(new_id(s));
    if let Some(t) = short(v, "title") {
        ctl.title = t;
    }
    if let Some(t) = short(v, "tag") {
        ctl.tag = t;
    }
    match &mut ctl.kind {
        ControlKind::ComboBox { items, .. } | ControlKind::DropDown { items, .. } => {
            if let Some(i) = items_param(v) {
                *items = i;
            }
        }
        ControlKind::Date { format, .. } => {
            if let Some(f) = short(v, "format").filter(|f| !f.trim().is_empty()) {
                *format = f;
            }
        }
        ControlKind::CheckBox { checked, .. } => *checked = p::bool(v, "checked").unwrap_or(false),
        _ => {}
    }
    let (a, b) = s.sel.ordered();
    if a.story != b.story || s.doc.para_at(&a).is_none() || s.doc.para_at(&b).is_none() {
        return Err(CmdError::Failed("no paragraph at the selection".into()));
    }
    if s.doc.control_at(&a).is_some_and(|r| !r.control.lock.can_edit()) {
        return Err(locked(s, CANT_EDIT));
    }
    let whole_para = a.path == b.path && a.off == 0 && s.doc.para_at(&b).is_some_and(|p| !p.is_empty() && b.off == p.len());
    let block = matches!(ctl.kind, ControlKind::RepeatingSection { .. } | ControlKind::Gallery { .. }) || a.path != b.path || whole_para;
    if block && matches!(ctl.kind, ControlKind::RichText | ControlKind::RepeatingSection { .. } | ControlKind::Gallery { .. }) {
        return insert_block(s, ctl, &a, &b);
    }
    // An inline control: around the selection (text and pictures), else at the caret with its
    // placeholder or value.
    let wraps = a != b && a.path == b.path && matches!(ctl.kind, ControlKind::RichText | ControlKind::Text { .. } | ControlKind::Picture);
    let base = s.doc.para_at(&a).map(|p| content_props(&p.props_of_char(a.off).clone())).unwrap_or_default();
    if wraps {
        s.doc.insert_object(&b, InlineObject::ControlEnd, &base)?;
        let start = a.clone();
        s.doc.insert_object(&start, InlineObject::ControlStart { control: Box::new(ctl) }, &base)?;
        let w = OBJ.len_utf8();
        s.sel = Selection { anchor: Pos { off: a.off + w, ..a.clone() }, focus: Pos { off: b.off + w, ..b.clone() } };
        return sel_result(s);
    }
    let at = delete_selection(s)?;
    let at = s.doc.typing_pos(&at);
    let (text, props) = match &mut ctl.kind {
        ControlKind::CheckBox { checked, checked_char, unchecked_char, .. } => {
            ((if *checked { *checked_char } else { *unchecked_char }).to_string(), base.clone())
        }
        ControlKind::Date { full_date, format, .. } if p::str(v, "date").is_some() => {
            let (y, m, d) = p::str(v, "date").and_then(parse_date).ok_or_else(|| CmdError::Params("date: YYYY-MM-DD".into()))?;
            *full_date = format!("{y:04}-{m:02}-{d:02}T00:00:00Z");
            (format_date_value(format, y, m, d), base.clone())
        }
        _ => {
            ctl.showing_placeholder = true;
            let t = ctl.placeholder_shown().to_string();
            (t, placeholder_props(s, &base))
        }
    };
    let after_start = s.doc.insert_object(&at, InlineObject::ControlStart { control: Box::new(ctl) }, &base)?;
    let end = s.doc.insert_text(&after_start, &text, &props)?;
    s.doc.insert_object(&end, InlineObject::ControlEnd, &base)?;
    s.sel = Selection::caret(after_start);
    s.goal_x = None;
    sel_result(s)
}

/// A block-level control around the paragraphs from `a`'s to `b`'s (same block list).
fn insert_block(s: &mut Session, mut ctl: ContentControl, a: &Pos, b: &Pos) -> CmdResult {
    let story = a.story;
    // Paragraphs in the same block list as `a`'s, up to `b`'s.
    let last = if b.path.parent() == a.path.parent() { b.path.clone() } else { a.path.clone() };
    ctl.block = true;
    let empty = a.path == last && s.doc.para(story, &a.path).is_some_and(|p| p.is_empty());
    let base = s.doc.para(story, &a.path).map(|p| content_props(&p.props_of_char(0).clone())).unwrap_or_default();
    let item = matches!(ctl.kind, ControlKind::RepeatingSection { .. }).then(|| {
        let mut i = ContentControl::new(ControlKind::RepeatingSectionItem);
        i.block = true;
        i.id = ctl.id.map(|x| x + 1);
        i
    });
    if empty && !ctl.placeholder_shown().is_empty() {
        ctl.showing_placeholder = true;
        let props = placeholder_props(s, &base);
        let text = ctl.placeholder_shown().to_string();
        s.doc.insert_text(&Pos::new(story, a.path.clone(), 0), &text, &props)?;
    }
    let end_off = s.doc.para(story, &last).map(|p| p.len()).unwrap_or(0);
    let end_props = s.doc.para(story, &last).map(|p| p.props_at(end_off).clone()).unwrap_or_default();
    let mut end = Pos::new(story, last.clone(), end_off);
    if item.is_some() {
        end = s.doc.insert_object(&end, InlineObject::ControlEnd, &end_props)?;
    }
    s.doc.insert_object(&end, InlineObject::ControlEnd, &end_props)?;
    let mut at = Pos::new(story, a.path.clone(), 0);
    at = s.doc.insert_object(&at, InlineObject::ControlStart { control: Box::new(ctl) }, &base)?;
    if let Some(i) = item {
        at = s.doc.insert_object(&at, InlineObject::ControlStart { control: Box::new(i) }, &base)?;
    }
    s.sel = Selection::caret(at);
    s.goal_x = None;
    sel_result(s)
}

// ---------------------------------------------------------------------------------------------
// Acting on a control

fn design_mode(s: &mut Session, v: &Value) -> CmdResult {
    s.view.design_mode = p::bool(v, "value").unwrap_or(!s.view.design_mode);
    Ok(json!({"designMode": s.view.design_mode}))
}

/// What a command or dialog needs to know about a control.
pub fn describe(s: &Session, r: &ControlRange) -> Value {
    let c = &r.control;
    let mut out = json!({
        "type": c.kind.name(),
        "title": c.title,
        "tag": c.tag,
        "id": c.id,
        "lockDelete": !c.lock.can_delete(),
        "lockEdit": !c.lock.can_edit(),
        "temporary": c.temporary,
        "block": c.block,
        "showingPlaceholder": c.showing_placeholder,
        "placeholder": c.placeholder_shown(),
        "text": if c.showing_placeholder { String::new() } else { s.doc.control_text(r) },
        "start": super::pos_json(&r.start),
        "end": super::pos_json(&r.end),
    });
    match &c.kind {
        ControlKind::Text { multi_line } => out["multiLine"] = json!(multi_line),
        ControlKind::CheckBox { checked, checked_char, unchecked_char, .. } => {
            out["checked"] = json!(checked);
            out["checkedSymbol"] = json!(checked_char.to_string());
            out["uncheckedSymbol"] = json!(unchecked_char.to_string());
        }
        ControlKind::ComboBox { items, last_value } | ControlKind::DropDown { items, last_value } => {
            out["items"] = json!(items.iter().map(|i| json!({"display": i.display, "value": i.value()})).collect::<Vec<_>>());
            out["value"] = json!(last_value);
        }
        ControlKind::Date { full_date, format, .. } => {
            out["date"] = json!(full_date.get(..10).unwrap_or(""));
            out["format"] = json!(format);
        }
        _ => {}
    }
    out
}

fn info(s: &mut Session, v: &Value) -> CmdResult {
    let r = target(s, v)?;
    Ok(describe(s, &r))
}

fn properties(s: &mut Session, v: &Value) -> CmdResult {
    let r = target(s, v)?;
    let mut c = r.control.clone();
    let mut content: Option<String> = None;
    if let Some(t) = short(v, "title") {
        c.title = t;
    }
    if let Some(t) = short(v, "tag") {
        c.tag = t;
    }
    let (no_delete, no_edit) = (p::bool(v, "lockDelete").unwrap_or(!c.lock.can_delete()), p::bool(v, "lockEdit").unwrap_or(!c.lock.can_edit()));
    c.lock = ControlLock::from_flags(no_delete, no_edit);
    if let Some(t) = p::bool(v, "temporary") {
        c.temporary = t;
    }
    if let Some(t) = short(v, "placeholder").filter(|t| !t.trim().is_empty()) {
        if c.showing_placeholder && t != c.placeholder_shown() {
            content = Some(t.clone());
        }
        c.placeholder_text = t;
    }
    match &mut c.kind {
        ControlKind::Text { multi_line } => {
            if let Some(m) = p::bool(v, "multiLine") {
                *multi_line = m;
            }
        }
        ControlKind::ComboBox { items, .. } | ControlKind::DropDown { items, .. } => {
            if let Some(i) = items_param(v) {
                *items = i;
            }
        }
        ControlKind::Date { format, full_date, .. } => {
            if let Some(f) = short(v, "format").filter(|f| !f.trim().is_empty()) {
                *format = f;
                // The shown date follows the new format.
                if let Some((y, m, d)) = parse_date(full_date)
                    && !c.showing_placeholder
                {
                    content = Some(format_date_value(format, y, m, d));
                }
            }
        }
        ControlKind::CheckBox { checked, checked_char, unchecked_char, .. } => {
            if let Some(ch) = symbol_param(v, "checkedSymbol") {
                *checked_char = ch;
            }
            if let Some(ch) = symbol_param(v, "uncheckedSymbol") {
                *unchecked_char = ch;
            }
            content = Some((if *checked { *checked_char } else { *unchecked_char }).to_string());
        }
        _ => {}
    }
    if let Some(text) = content {
        let base = content_base(s, &r);
        let props = if c.showing_placeholder { placeholder_props(s, &base) } else { base };
        set_content(s, &r, &text, &props)?;
    }
    if let Some(slot) = s.doc.control_mut(&r.start) {
        *slot = c;
    }
    match s.doc.control_at(&r.content_start()) {
        Some(r) => Ok(describe(s, &r)),
        None => sel_result(s),
    }
}

fn toggle_check_box(s: &mut Session, v: &Value) -> CmdResult {
    let r = target(s, v)?;
    let ControlKind::CheckBox { checked, checked_char, checked_font, unchecked_char, unchecked_font } = &r.control.kind else {
        return Err(CmdError::Disabled("the selection isn't in a check box".into()));
    };
    if !r.control.lock.can_edit() {
        return Err(locked(s, CANT_EDIT));
    }
    let now = p::bool(v, "value").unwrap_or(!checked);
    let (ch, font) = if now { (*checked_char, checked_font) } else { (*unchecked_char, unchecked_font) };
    let mut props = content_base(s, &r);
    if !font.is_empty() {
        props.font = Some(font.clone());
    }
    set_content(s, &r, &ch.to_string(), &props)?;
    if let Some(c) = s.doc.control_mut(&r.start) {
        if let ControlKind::CheckBox { checked, .. } = &mut c.kind {
            *checked = now;
        }
        c.showing_placeholder = false;
    }
    Ok(json!({"checked": now}))
}

fn choose_item(s: &mut Session, v: &Value) -> CmdResult {
    let r = target(s, v)?;
    let Some(items) = r.control.kind.items() else { return Err(CmdError::Disabled("the selection isn't in a combo box or drop-down list".into())) };
    if !r.control.lock.can_edit() {
        return Err(locked(s, CANT_EDIT));
    }
    let item = if let Some(i) = p::u64(v, "index") {
        items.get(usize::try_from(i).unwrap_or(usize::MAX))
    } else if let Some(val) = p::str(v, "value") {
        items.iter().find(|i| i.value() == val)
    } else if let Some(t) = p::str(v, "text") {
        items.iter().find(|i| i.display == t)
    } else {
        return Err(CmdError::Params("give index, value or text".into()));
    };
    let Some(item) = item.cloned() else { return Err(CmdError::Params("no such entry".into())) };
    let props = content_base(s, &r);
    let end = set_content(s, &r, &item.display, &props)?;
    if let Some(c) = s.doc.control_mut(&r.start) {
        if let ControlKind::ComboBox { last_value, .. } | ControlKind::DropDown { last_value, .. } = &mut c.kind {
            *last_value = item.value().to_string();
        }
        c.showing_placeholder = false;
    }
    s.sel = Selection::caret(after(&end));
    Ok(json!({"display": item.display, "value": item.value()}))
}

fn set_date(s: &mut Session, v: &Value) -> CmdResult {
    let r = target(s, v)?;
    let ControlKind::Date { format, .. } = &r.control.kind else { return Err(CmdError::Disabled("the selection isn't in a date picker".into())) };
    if !r.control.lock.can_edit() {
        return Err(locked(s, CANT_EDIT));
    }
    let (y, m, d) = p::str(v, "date").and_then(parse_date).ok_or_else(|| CmdError::Params("date: YYYY-MM-DD".into()))?;
    let text = format_date_value(format, y, m, d);
    let props = content_base(s, &r);
    let end = set_content(s, &r, &text, &props)?;
    if let Some(c) = s.doc.control_mut(&r.start) {
        if let ControlKind::Date { full_date, .. } = &mut c.kind {
            *full_date = format!("{y:04}-{m:02}-{d:02}T00:00:00Z");
        }
        c.showing_placeholder = false;
    }
    s.sel = Selection::caret(after(&end));
    Ok(json!({"date": format!("{y:04}-{m:02}-{d:02}"), "text": text}))
}

/// Just after the end marker at `end` (the caret leaves a control whose value was chosen).
fn after(end: &Pos) -> Pos {
    Pos { off: end.off + OBJ.len_utf8(), ..end.clone() }
}

fn remove_control(s: &mut Session, v: &Value) -> CmdResult {
    let r = target(s, v)?;
    if !r.control.lock.can_delete() {
        return Err(locked(s, CANT_DELETE));
    }
    let w = OBJ.len_utf8();
    // A placeholder goes with its control; real content stays.
    if r.control.showing_placeholder {
        s.doc.delete_range(&r.content_start(), &r.end)?;
    }
    let end = s.doc.control_at(&r.content_start()).map(|x| x.end).unwrap_or(r.end.clone());
    s.doc.para_mut(end.story, &end.path)?.delete(end.off, end.off + w)?;
    s.doc.para_mut(r.start.story, &r.start.path)?.delete(r.start.off, r.start.off + w)?;
    s.sel = Selection::caret(s.doc.clamp(&r.start));
    sel_result(s)
}

/// A date with a date picture (`M/d/yyyy`, `dddd, MMMM d, yyyy`…).
pub fn format_date_value(fmt: &str, y: i64, m: u32, d: u32) -> String {
    super::insert::format_date_parts(fmt, y, m as usize, d, (0, 0, 0), days_from_civil(y, m, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse_and_format() {
        assert_eq!(parse_date("2026-10-11"), Some((2026, 10, 11)));
        assert_eq!(parse_date("2024-02-29T00:00:00Z"), Some((2024, 2, 29)));
        assert_eq!(parse_date("2025-02-29"), None);
        assert_eq!(parse_date("20x6-1-1"), None);
        assert_eq!(parse_date("é"), None);
        assert_eq!(format_date_value("dddd, MMMM d, yyyy", 2026, 10, 11), "Sunday, October 11, 2026");
        assert_eq!(format_date_value("M/d/yyyy", 1970, 1, 1), "1/1/1970");
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(super::super::civil_from_days(days_from_civil(2000, 3, 1)), (2000, 3, 1));
    }
}

#[cfg(test)]
mod editing_tests {
    use serde_json::json;
    use wordcraft_doc::StoryRef;
    use wordcraft_doc::control::ControlKind;

    use crate::Session;

    fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
        s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
    }
    fn text(s: &Session) -> String {
        s.doc.plain_text(StoryRef::Body)
    }
    fn info(s: &mut Session) -> serde_json::Value {
        run(s, "developer.control", json!({}))
    }

    #[test]
    fn typing_replaces_the_placeholder_and_emptying_brings_it_back() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        run(&mut s, "text.insert", json!({"text": "Name: "}));
        run(&mut s, "developer.plainText", json!({"title": "Name", "tag": "name"}));
        let i = info(&mut s);
        assert_eq!((i["type"].as_str(), i["showingPlaceholder"].as_bool()), (Some("plainText"), Some(true)));
        let placeholder = i["placeholder"].as_str().unwrap().to_string();
        assert_eq!(text(&s), format!("Name: {placeholder}"));
        // Typing anywhere in the placeholder replaces all of it, in ordinary formatting.
        run(&mut s, "caret.right", json!({}));
        run(&mut s, "text.insert", json!({"text": "A"}));
        run(&mut s, "text.insert", json!({"text": "da"}));
        assert_eq!(text(&s), "Name: Ada");
        let i = info(&mut s);
        assert_eq!((i["showingPlaceholder"].as_bool(), i["text"].as_str()), (Some(false), Some("Ada")));
        let r = s.doc.control_at(&s.sel.focus).unwrap();
        let para = s.doc.para_at(&r.content_start()).unwrap();
        assert_eq!(para.props_of_char(r.content_start().off).style, None, "not placeholder-styled");
        // Enter in a one-line control does nothing.
        run(&mut s, "text.newParagraph", json!({}));
        assert_eq!(s.doc.body.len(), 1);
        // Deleting all of it shows the placeholder again; Backspace at the content's start keeps the control.
        for _ in 0..3 {
            run(&mut s, "text.backspace", json!({}));
        }
        assert_eq!(text(&s), format!("Name: {placeholder}"));
        assert_eq!(info(&mut s)["showingPlaceholder"], true);
        run(&mut s, "text.backspace", json!({}));
        assert_eq!(text(&s), format!("Name: {placeholder}"));
        assert_eq!(s.doc.control_ranges(StoryRef::Body).len(), 1);
        // Undo goes back through the edits.
        run(&mut s, "edit.undo", json!({}));
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(text(&s), "Name: A");
        // And the file keeps it.
        let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
        assert_eq!(back.control_ranges(StoryRef::Body).first().map(|r| r.control.tag.clone()), Some("name".into()));
    }

    #[test]
    fn check_boxes_toggle_and_locks_refuse() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        run(&mut s, "developer.checkBox", json!({"tag": "ok"}));
        assert_eq!(text(&s), "\u{2610}");
        // A check box takes no typing: at its edge the text goes beside it, over it it's refused.
        run(&mut s, "text.insert", json!({"text": "x"}));
        assert_eq!(text(&s), "x\u{2610}");
        run(&mut s, "edit.undo", json!({}));
        let r = s.doc.control_ranges(StoryRef::Body)[0].clone();
        s.sel = crate::Selection { anchor: r.content_start(), focus: r.end.clone() };
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        assert_eq!(run(&mut s, "developer.toggleCheckBox", json!({}))["checked"], true);
        assert_eq!(text(&s), "\u{2612}");
        assert!(matches!(s.doc.control_ranges(StoryRef::Body)[0].control.kind, ControlKind::CheckBox { checked: true, .. }));
        // Content locked: no toggling.
        run(&mut s, "developer.properties", json!({"control": "ok", "lockEdit": true}));
        assert!(s.run("developer.toggleCheckBox", &json!({"control": "ok"})).is_err());
        assert_eq!(text(&s), "\u{2612}");
        // Control locked: selecting it whole and deleting is refused, the document unchanged.
        run(&mut s, "developer.properties", json!({"control": "ok", "lockEdit": false, "lockDelete": true}));
        run(&mut s, "select.all", json!({}));
        let before = s.doc.clone();
        assert!(s.run("text.delete", &json!({})).is_err());
        assert_eq!(s.doc, before);
        assert!(s.status.contains("can't be deleted"));
        // Unlocked, it goes.
        run(&mut s, "developer.properties", json!({"control": "ok", "lockDelete": false}));
        run(&mut s, "select.all", json!({}));
        run(&mut s, "text.delete", json!({}));
        assert!(s.doc.control_ranges(StoryRef::Body).is_empty());
    }

    #[test]
    fn lists_dates_and_blocks() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        run(&mut s, "developer.dropDown", json!({"tag": "size", "items": ["Small", {"display": "Large", "value": "L"}]}));
        run(&mut s, "developer.chooseItem", json!({"control": "size", "value": "L"}));
        run(&mut s, "text.insert", json!({"text": " on "}));
        run(&mut s, "developer.datePicker", json!({"tag": "when", "format": "yyyy-MM-dd"}));
        run(&mut s, "developer.setDate", json!({"control": "when", "date": "2026-10-11"}));
        assert_eq!(text(&s), "Large on 2026-10-11");
        assert!(s.run("developer.setDate", &json!({"control": "when", "date": "2026-02-30"})).is_err());
        // A block control around two paragraphs, and a repeating section.
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": "Item"}));
        run(&mut s, "select.all", json!({}));
        run(&mut s, "developer.richText", json!({"title": "All"}));
        run(&mut s, "developer.repeatingSection", json!({}));
        let doc = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
        let kinds: Vec<_> = doc.control_ranges(StoryRef::Body).iter().map(|r| (r.control.kind.name(), r.control.block)).collect();
        for k in [("dropDown", false), ("date", false), ("richText", true), ("repeatingSection", true), ("repeatingSectionItem", true)] {
            assert!(kinds.contains(&k), "{k:?} in {kinds:?}");
        }
        assert_eq!(doc.plain_text(StoryRef::Body), s.doc.plain_text(StoryRef::Body));
    }
}

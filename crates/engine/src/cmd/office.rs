//! Office add-ins' bridge (`office.sync`): one `context.sync()` of an add-in's task pane — a batch
//! of [`Action`]s on objects reached by paths from the document — run against the open document.
//! It is one undoable step; if any action fails, the document goes back to how it was and the
//! error names the action. What an add-in may do is bounded by its manifest's permissions. Macro
//! add-ins reach the same objects (`host/sync`, [`super::developer_vba`]).
//!
//! The objects follow the Word JavaScript API's public reference: `document`, `body`,
//! `paragraphs`, `paragraph`, `range`, `font`, the selection and the document's properties, and
//! the common API's selected text and settings. Word's API has no ids, so paragraphs and ranges
//! get opaque ones here (`p:<path>`, `r:<path>:<from>-<path>:<to>`), good until the document's
//! structure changes. Writes go through WordCraft's own commands on a selection set for them
//! (`edit.pasteText`, `format.bold`, `para.style`…), then the user's selection comes back; reads
//! resolve styles as the text is drawn.

use std::collections::HashMap;

use serde_json::{Value, json};
use wordcraft_doc::props::{Align, Highlight, LineSpacing, TextColor, Underline, VertAlign};
use wordcraft_doc::{Path, Pos, StoryRef};
use wordcraft_plugin::office::{Action, MAX_ACTIONS, Permission, Step};

use crate::p;
use crate::{CmdError, CmdResult, CommandSpec, Session};

/// Most items a collection load or a search answers.
const MAX_ITEMS: usize = 10_000;

/// The `Font` properties (WordApi 1.1).
const FONT_PROPS: &[&str] =
    &["bold", "italic", "underline", "size", "name", "color", "highlightColor", "strikeThrough", "doubleStrikeThrough", "subscript", "superscript"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("office.sync", "Office Add-in", "Insert › Add-ins", sync)
            .params(r#"{"addin": id, "actions": [{"op": "load"|"set"|"call", "path": [{"p"}|{"m", "a"}], "props"?|"prop"+"value"|"method"+"args"?}]} → {"results"}; a failed sync changes nothing and fails with {"code", "message", "action"} as its message. One context.sync() of an Office add-in, as one undoable step"#)
            .pure(),
    ]
}

/// An object an action reaches.
#[derive(Clone, Debug, PartialEq)]
enum Obj {
    Root,
    Document,
    Properties,
    Body,
    /// Paragraphs of the body, or of a range.
    Paragraphs(Option<(Pos, Pos)>),
    Paragraph(Path),
    /// A range of the body: from, to (byte offsets in the paragraphs' text).
    Range(Pos, Pos),
    /// The font of a range (a paragraph's or the body's is its whole range).
    Font(Pos, Pos),
    Settings,
}

/// An Office.js error: its code and message.
struct OfficeError(&'static str, String);

type R<T> = std::result::Result<T, OfficeError>;

fn invalid(msg: impl Into<String>) -> OfficeError {
    OfficeError("InvalidArgument", msg.into())
}

fn not_found(msg: impl Into<String>) -> OfficeError {
    OfficeError("ItemNotFound", msg.into())
}

fn engine(e: CmdError) -> OfficeError {
    OfficeError("GeneralException", e.to_string())
}

/// What a sync's actions changed in the document's structure, so the ids handed out earlier in
/// the same sync keep naming the same paragraphs and text: (the action that made the edit, the
/// edit).
#[derive(Clone, Debug)]
enum Edit {
    /// A paragraph came in at this path: the paragraphs from there on in its container move down.
    InsertPara(Path),
    /// The paragraph at this path went: the ones after it move up.
    RemovePara(Path),
    /// `delta` bytes of text came in (or went, negative) at `off` in the paragraph at `path`.
    Text(Path, usize, isize),
}

thread_local! {
    /// The running sync's edits, with the index of the action that made each.
    static EDITS: std::cell::RefCell<(usize, Vec<(usize, Edit)>)> = const { std::cell::RefCell::new((0, Vec::new())) };
}

fn note(edit: Edit) {
    EDITS.with(|e| {
        let mut e = e.borrow_mut();
        let at = e.0;
        if e.1.len() < 100_000 {
            e.1.push((at, edit));
        }
    });
}

/// A path as it is after `edit`.
fn shift_path(path: &mut Path, edit: &Edit) {
    let (at, delta) = match edit {
        Edit::InsertPara(at) => (at, 1i64),
        Edit::RemovePara(at) => (at, -1i64),
        Edit::Text(..) => return,
    };
    let n = at.0.len();
    if path.0.len() < n || path.0.get(..n - 1) != at.0.get(..n - 1) {
        return;
    }
    let (Some(mine), Some(theirs)) = (path.0.get(n - 1).copied(), at.0.last().copied()) else { return };
    let moves = if delta > 0 { mine >= theirs } else { mine > theirs };
    if moves && let Some(x) = path.0.get_mut(n - 1) {
        *x = (i64::from(mine) + delta).max(0) as u32;
    }
}

/// An id handed out by action `since` as it reads after the edits the later actions made.
fn current_id(id: &str, since: usize) -> String {
    EDITS.with(|e| {
        let e = e.borrow();
        let edits: Vec<&Edit> = e.1.iter().filter(|(a, _)| *a > since).map(|(_, x)| x).collect();
        if edits.is_empty() {
            return id.to_string();
        }
        if let Some(path) = id.strip_prefix("p:").and_then(parse_path) {
            let mut path = path;
            for edit in edits {
                shift_path(&mut path, edit);
            }
            return para_id(&path);
        }
        if let Some((a, b)) = id.strip_prefix("r:").and_then(|r| r.split_once('-')) {
            let (Some(mut a), Some(mut b)) = (parse_pos(a), parse_pos(b)) else { return id.to_string() };
            for edit in edits {
                for (pos, is_end) in [(&mut a, false), (&mut b, true)] {
                    if let Edit::Text(path, off, delta) = edit {
                        // Text typed at a range's end stays outside it; at its start, it pushes it on.
                        if *path == pos.path && (pos.off > *off || (pos.off == *off && !is_end)) {
                            pos.off = (pos.off as isize + delta).max(*off as isize) as usize;
                        }
                    }
                    shift_path(&mut pos.path, edit);
                }
            }
            return range_id(&a, &b);
        }
        id.to_string()
    })
}

/// Add-in settings, per open document and add-in, until they're saved in the file.
fn settings_store() -> &'static std::sync::Mutex<HashMap<(u64, String), Value>> {
    static S: std::sync::OnceLock<std::sync::Mutex<HashMap<(u64, String), Value>>> = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

fn sync(s: &mut Session, v: &Value) -> CmdResult {
    let addin = p::req_str(v, "addin")?.to_string();
    let plugin = wordcraft_plugin::get(&addin).ok_or_else(|| CmdError::Params(format!("no add-in `{addin}` is loaded")))?;
    let office = plugin.manifest.office.clone().ok_or_else(|| CmdError::Params(format!("{addin} isn't an Office add-in")))?;
    if !plugin.enabled() {
        return Err(CmdError::Failed(format!("{addin} is turned off")));
    }
    if let Some(b) = plugin.blocked_by().first() {
        return Err(CmdError::Failed(format!("{addin} is blocked: {}", b.message)));
    }
    let actions: Vec<Action> = serde_json::from_value(v.get("actions").cloned().unwrap_or(Value::Array(Vec::new())))
        .map_err(|e| CmdError::Params(format!("`actions`: {e}")))?;
    if actions.len() > MAX_ACTIONS {
        return Err(CmdError::Params(format!("at most {MAX_ACTIONS} actions in one sync")));
    }
    // One step, or nothing at all: a failure's message is the error the task pane reads.
    let results =
        s.one_step("Office Add-in", |s| run_actions(s, &addin, office.permissions, &actions).map_err(|e| CmdError::Failed(e.to_string())))?;
    Ok(json!({"results": results}))
}

/// Runs `actions` in order for the add-in or plugin `addin`: their results, or the first failure
/// as `{code, message, action}`. The caller makes them one step (and takes them back on failure).
pub(crate) fn run_actions(s: &mut Session, addin: &str, perm: Permission, actions: &[Action]) -> std::result::Result<Vec<Value>, Value> {
    let mut results: Vec<Value> = Vec::with_capacity(actions.len());
    let mut user_sel = s.sel.clone();
    EDITS.with(|e| *e.borrow_mut() = (0, Vec::new()));
    for (i, a) in actions.iter().enumerate() {
        EDITS.with(|e| e.borrow_mut().0 = i);
        // An object made earlier in the same sync is named by that action's result.
        let a = with_results(a, &results);
        let r = run_action(s, addin, perm, &a);
        match r {
            Ok((v, keep_selection)) => {
                // `select()` makes the user's selection; anything else puts it back.
                if keep_selection {
                    user_sel = s.sel.clone();
                } else {
                    s.sel = user_sel.clone();
                    s.sel.anchor = s.doc.clamp(&s.sel.anchor);
                    s.sel.focus = s.doc.clamp(&s.sel.focus);
                }
                results.push(v);
            }
            Err(OfficeError(code, message)) => {
                s.sel = user_sel;
                s.sel.anchor = s.doc.clamp(&s.sel.anchor);
                s.sel.focus = s.doc.clamp(&s.sel.focus);
                return Err(json!({"code": code, "message": message, "action": i}));
            }
        }
    }
    Ok(results)
}

/// `a` with `{"result": i}` arguments in its path replaced by the id action `i` answered (`""`,
/// which names nothing, when it answered none).
fn with_results(a: &Action, results: &[Value]) -> Action {
    let fix = |path: &[Step]| -> Vec<Step> {
        path.iter()
            .map(|step| match step {
                Step::Method { m, a } => Step::Method {
                    m: m.clone(),
                    a: a.iter()
                        .map(|v| match v.get("result").and_then(Value::as_u64).filter(|_| v.as_object().is_some_and(|o| o.len() == 1)) {
                            Some(i) => results
                                .get(i as usize)
                                .and_then(|r| r.get("id"))
                                .and_then(Value::as_str)
                                .map(|id| json!(current_id(id, i as usize)))
                                .unwrap_or(json!("")),
                            None => v.clone(),
                        })
                        .collect(),
                },
                other => other.clone(),
            })
            .collect()
    };
    match a {
        Action::Load { path, props } => Action::Load { path: fix(path), props: props.clone() },
        Action::Set { path, prop, value } => Action::Set { path: fix(path), prop: prop.clone(), value: value.clone() },
        Action::Call { path, method, args } => Action::Call { path: fix(path), method: method.clone(), args: args.clone() },
    }
}

fn need_read(perm: Permission) -> R<()> {
    if perm.reads() { Ok(()) } else { Err(OfficeError("AccessDenied", "the add-in's manifest doesn't let it read the document".into())) }
}

fn need_write(perm: Permission) -> R<()> {
    if perm.writes() { Ok(()) } else { Err(OfficeError("AccessDenied", "the add-in's manifest doesn't let it change the document".into())) }
}

/// The action's result, and whether the selection it left is the user's new one (`select()`).
fn run_action(s: &mut Session, addin: &str, perm: Permission, a: &Action) -> R<(Value, bool)> {
    match a {
        Action::Load { path, props } => {
            let obj = resolve(s, path)?;
            if !matches!(obj, Obj::Settings) {
                need_read(perm)?;
            }
            load(s, addin, &obj, props).map(|v| (v, false))
        }
        Action::Set { path, prop, value } => {
            let obj = resolve(s, path)?;
            need_write(perm)?;
            set(s, &obj, prop, value).map(|_| (Value::Null, false))
        }
        Action::Call { path, method, args } => {
            let obj = resolve(s, path)?;
            match (&obj, method.as_str()) {
                (Obj::Settings, _) => {}
                (Obj::Document, "getSelectedData") => need_read(perm)?,
                (Obj::Document, "save") => {}
                _ => need_write(perm)?,
            }
            let keep = method == "select" || (matches!(obj, Obj::Document) && method == "setSelectedData");
            call(s, addin, &obj, method, args).map(|v| (v, keep))
        }
    }
}

// ---------------------------------------------------------------- ids and positions

fn para_id(path: &Path) -> String {
    format!("p:{path}")
}

fn parse_path(t: &str) -> Option<Path> {
    let v: Option<Vec<u32>> = t.split('/').map(|x| x.parse::<u32>().ok()).collect();
    v.filter(|v| !v.is_empty() && v.len() <= 64).map(Path)
}

fn range_id(a: &Pos, b: &Pos) -> String {
    format!("r:{}:{}-{}:{}", a.path, a.off, b.path, b.off)
}

/// A position `<path>:<off>` of a range id.
fn parse_pos(t: &str) -> Option<Pos> {
    let (path, off) = t.rsplit_once(':')?;
    Some(Pos::new(StoryRef::Body, parse_path(path)?, off.parse().ok()?))
}

/// The paragraph or range an id names (checked against the document).
fn parse_id(s: &Session, id: &str) -> R<Obj> {
    let missing = || not_found(format!("`{id}` names nothing in the document"));
    if let Some(path) = id.strip_prefix("p:") {
        let path = parse_path(path).ok_or_else(missing)?;
        s.doc.para(StoryRef::Body, &path).ok_or_else(missing)?;
        return Ok(Obj::Paragraph(path));
    }
    if let Some(r) = id.strip_prefix("r:") {
        let (a, b) = r.split_once('-').ok_or_else(missing)?;
        let (a, b) = (parse_pos(a).ok_or_else(missing)?, parse_pos(b).ok_or_else(missing)?);
        for x in [&a, &b] {
            let para = s.doc.para(StoryRef::Body, &x.path).ok_or_else(missing)?;
            if x.off > para.text.len() || !para.text.is_char_boundary(x.off) {
                return Err(missing());
            }
        }
        return Ok(Obj::Range(a.clone().min(b.clone()), a.max(b)));
    }
    Err(missing())
}

fn paths(s: &Session) -> Vec<Path> {
    s.doc.para_paths(StoryRef::Body)
}

fn para_len(s: &Session, path: &Path) -> usize {
    s.doc.para(StoryRef::Body, path).map_or(0, |p| p.text.len())
}

/// The whole of a paragraph, or of the body.
fn para_range(s: &Session, path: &Path) -> (Pos, Pos) {
    (Pos::new(StoryRef::Body, path.clone(), 0), Pos::new(StoryRef::Body, path.clone(), para_len(s, path)))
}

fn body_range(s: &Session) -> (Pos, Pos) {
    let all = paths(s);
    let first = all.first().cloned().unwrap_or_else(|| Path::top(0));
    let last = all.last().cloned().unwrap_or_else(|| Path::top(0));
    (Pos::new(StoryRef::Body, first, 0), Pos::new(StoryRef::Body, last.clone(), para_len(s, &last)))
}

/// The paragraphs a range touches.
fn paths_in(s: &Session, a: &Pos, b: &Pos) -> Vec<Path> {
    paths(s).into_iter().filter(|p| *p >= a.path && *p <= b.path).take(MAX_ITEMS).collect()
}

/// `Whole`, `Start`, `End` or `Content` of a range.
fn sub_range(a: &Pos, b: &Pos, location: &str) -> R<(Pos, Pos)> {
    match location.to_ascii_lowercase().as_str() {
        "whole" | "content" => Ok((a.clone(), b.clone())),
        "start" => Ok((a.clone(), a.clone())),
        "end" | "after" => Ok((b.clone(), b.clone())),
        "before" => Ok((a.clone(), a.clone())),
        other => Err(invalid(format!("`{other}` isn't a range location (Whole, Start, End)"))),
    }
}

fn text_of(s: &Session, a: &Pos, b: &Pos) -> String {
    let mut out = Vec::new();
    for path in paths_in(s, a, b) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let from = if path == a.path { a.off } else { 0 };
        let to = if path == b.path { b.off } else { p.text.len() };
        out.push(p.text.get(from.min(to)..to.min(p.text.len())).unwrap_or("").replace(wordcraft_doc::para::OBJ, ""));
    }
    out.join("\n")
}

fn resolve(s: &Session, path: &[Step]) -> R<Obj> {
    let mut obj = Obj::Root;
    for step in path {
        obj = match (&obj, step) {
            (Obj::Root, Step::Prop { p }) if p == "document" => Obj::Document,
            (Obj::Root, Step::Prop { p }) if p == "settings" => Obj::Settings,
            (Obj::Document, Step::Prop { p }) if p == "body" => Obj::Body,
            (Obj::Document, Step::Prop { p }) if p == "properties" => Obj::Properties,
            (Obj::Document | Obj::Body, Step::Prop { p }) if p == "paragraphs" => Obj::Paragraphs(None),
            (Obj::Document, Step::Method { m, .. }) if m == "getSelection" => {
                if s.sel.anchor.story != StoryRef::Body || s.sel.focus.story != StoryRef::Body {
                    return Err(OfficeError("GeneralException", "the selection isn't in the document's body".into()));
                }
                let (a, b) = (s.sel.anchor.clone().min(s.sel.focus.clone()), s.sel.anchor.clone().max(s.sel.focus.clone()));
                Obj::Range(a, b)
            }
            (Obj::Document, Step::Method { m, a }) if m == "getRangeById" => parse_id(s, a.first().and_then(Value::as_str).unwrap_or(""))?,
            (Obj::Body, Step::Prop { p }) if p == "font" => {
                let (a, b) = body_range(s);
                Obj::Font(a, b)
            }
            (Obj::Body, Step::Method { m, a }) if m == "getRange" => {
                let (x, y) = body_range(s);
                let (x, y) = sub_range(&x, &y, a.first().and_then(Value::as_str).unwrap_or("Whole"))?;
                Obj::Range(x, y)
            }
            (Obj::Paragraphs(within), Step::Method { m, a }) => {
                let list = match within {
                    Some((x, y)) => paths_in(s, x, y),
                    None => paths(s),
                };
                let pick = match m.as_str() {
                    "getFirst" => list.first().cloned(),
                    "getLast" => list.last().cloned(),
                    "getItemAt" => {
                        let i = a.first().and_then(Value::as_u64).ok_or_else(|| invalid("getItemAt needs an index"))?;
                        list.get(i as usize).cloned()
                    }
                    "getById" => {
                        let id = a.first().and_then(Value::as_str).unwrap_or("");
                        match parse_id(s, id)? {
                            Obj::Paragraph(p) if list.contains(&p) => Some(p),
                            _ => None,
                        }
                    }
                    other => return Err(invalid(format!("ParagraphCollection has no method `{other}()`"))),
                };
                Obj::Paragraph(pick.ok_or_else(|| not_found("there's no such paragraph"))?)
            }
            (Obj::Paragraph(path), Step::Prop { p }) if p == "font" => {
                let (a, b) = para_range(s, path);
                Obj::Font(a, b)
            }
            (Obj::Paragraph(path), Step::Method { m, a }) if m == "getRange" => {
                let (x, y) = para_range(s, path);
                let (x, y) = sub_range(&x, &y, a.first().and_then(Value::as_str).unwrap_or("Whole"))?;
                Obj::Range(x, y)
            }
            (Obj::Paragraph(path), Step::Method { m, .. }) if m == "getNext" || m == "getPrevious" => {
                let next = if m == "getNext" { s.doc.next_para(StoryRef::Body, path) } else { s.doc.prev_para(StoryRef::Body, path) };
                Obj::Paragraph(next.ok_or_else(|| not_found("there's no paragraph there"))?)
            }
            (Obj::Range(a, b), Step::Prop { p }) if p == "font" => Obj::Font(a.clone(), b.clone()),
            (Obj::Range(a, b), Step::Prop { p }) if p == "paragraphs" => Obj::Paragraphs(Some((a.clone(), b.clone()))),
            (Obj::Range(a, b), Step::Method { m, a: args }) if m == "getRange" => {
                let (x, y) = sub_range(a, b, args.first().and_then(Value::as_str).unwrap_or("Whole"))?;
                Obj::Range(x, y)
            }
            (o, step) => return Err(invalid(format!("{} has no {}", name(o), step_name(step)))),
        };
    }
    Ok(obj)
}

fn name(o: &Obj) -> &'static str {
    match o {
        Obj::Root => "the add-in's context",
        Obj::Document => "Document",
        Obj::Properties => "DocumentProperties",
        Obj::Body => "Body",
        Obj::Paragraphs(_) => "ParagraphCollection",
        Obj::Paragraph(_) => "Paragraph",
        Obj::Range(..) => "Range",
        Obj::Font(..) => "Font",
        Obj::Settings => "Settings",
    }
}

fn step_name(step: &Step) -> String {
    match step {
        Step::Prop { p } => format!("property `{p}`"),
        Step::Method { m, .. } => format!("method `{m}()`"),
    }
}

// ---------------------------------------------------------------- reading

/// The value every character of a range has for one property, or null when they differ.
fn font_props(s: &Session, a: &Pos, b: &Pos) -> Vec<wordcraft_doc::resolve::ResolvedChar> {
    let mut out = Vec::new();
    for path in paths_in(s, a, b) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let from = if path == a.path { a.off } else { 0 };
        let to = if path == b.path { b.off } else { p.text.len() };
        let style = p.props.style.as_deref();
        let mut any = false;
        for (r, c) in p.run_ranges() {
            if (r.end > from && r.start < to) || (from == to && r.start <= from && from <= r.end) {
                out.push(s.doc.styles.resolve_char(style, c));
                any = true;
            }
        }
        if !any {
            out.push(s.doc.styles.resolve_char(style, &p.mark));
        }
        if out.len() > MAX_ITEMS {
            break;
        }
    }
    out
}

fn common(values: Vec<Value>) -> Value {
    match values.split_first() {
        Some((first, rest)) if rest.iter().all(|v| v == first) => first.clone(),
        _ => Value::Null,
    }
}

fn underline_name(u: Underline) -> &'static str {
    match u {
        Underline::None => "None",
        Underline::Single => "Single",
        Underline::Words => "Word",
        Underline::Double => "Double",
        Underline::Thick => "Thick",
        Underline::Dotted => "Dotted",
        Underline::Dash => "DashLine",
        Underline::DotDash => "DotDashLine",
        Underline::DotDotDash => "TwoDotDashLine",
        Underline::Wave => "Wave",
        Underline::DoubleWave => "WaveDouble",
    }
}

fn align_name(a: Align) -> &'static str {
    match a {
        Align::Left => "Left",
        Align::Center => "Centered",
        Align::Right => "Right",
        Align::Justify => "Justified",
        Align::Distribute => "Justified",
    }
}

/// The built-in style a style id is (`Heading1`, `Normal`…), as Word's `styleBuiltIn` names it.
fn built_in(style_id: &str) -> &str {
    match style_id {
        "Normal" | "Title" | "Subtitle" | "Quote" | "IntenseQuote" | "ListParagraph" | "NoSpacing" | "TOC1" | "TOC2" | "TOC3" | "Caption" => style_id,
        s if s.starts_with("Heading") && s.len() == 8 && s.ends_with(|c: char| c.is_ascii_digit()) => style_id,
        _ => "Other",
    }
}

fn style_name(s: &Session, id: &str) -> String {
    s.doc.styles.get(id).map(|st| st.name.clone()).unwrap_or_else(|| id.to_string())
}

fn load(s: &mut Session, addin: &str, obj: &Obj, props: &[String]) -> R<Value> {
    // Settings load whole: the add-in's saved object.
    if matches!(obj, Obj::Settings) {
        let key = (s.document_id(), addin.to_string());
        let store = match settings_store().lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        return Ok(store.get(&key).cloned().unwrap_or_else(|| json!({})));
    }
    if props.len() > 256 {
        return Err(invalid("at most 256 properties in one load"));
    }
    let mut out = serde_json::Map::new();
    for prop in props {
        let v = match (obj, prop.as_str()) {
            (Obj::Properties, p) => {
                let c = &s.doc.core;
                json!(match p {
                    "title" => &c.title,
                    "subject" => &c.subject,
                    "author" => &c.creator,
                    "keywords" => &c.keywords,
                    "comments" => &c.description,
                    "category" => &c.category,
                    _ => return Err(invalid(format!("DocumentProperties has no property `{p}`"))),
                })
            }
            (Obj::Document, "name") => {
                json!(s.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Document1".into()))
            }
            (Obj::Body, "text") => {
                let (a, b) = body_range(s);
                json!(text_of(s, &a, &b))
            }
            (Obj::Body, "style") => {
                let styles: Vec<Value> = paths(s)
                    .iter()
                    .filter_map(|p| s.doc.para(StoryRef::Body, p))
                    .map(|p| json!(p.props.style.clone().unwrap_or_else(|| "Normal".into())))
                    .collect();
                match common(styles) {
                    Value::String(id) => json!(style_name(s, &id)),
                    other => other,
                }
            }
            (Obj::Paragraphs(within), "count") => json!(match within {
                Some((a, b)) => paths_in(s, a, b).len(),
                None => paths(s).len(),
            }),
            (Obj::Paragraphs(within), "items") => {
                let rest: Vec<String> = props.iter().filter(|p| *p != "items" && *p != "count").cloned().collect();
                let list = match within {
                    Some((a, b)) => paths_in(s, a, b),
                    None => paths(s),
                };
                let mut items = Vec::new();
                for path in list.into_iter().take(MAX_ITEMS) {
                    let mut item = load(s, addin, &Obj::Paragraph(path.clone()), &rest)?;
                    item["id"] = json!(para_id(&path));
                    items.push(item);
                }
                Value::Array(items)
            }
            (Obj::Paragraphs(_), _) if props.iter().any(|p| p == "items") => continue,
            (Obj::Paragraph(path), p) => {
                let para = s.doc.para(StoryRef::Body, path).ok_or_else(|| not_found("the paragraph is gone"))?;
                let style_id = para.props.style.clone().unwrap_or_else(|| "Normal".into());
                let r = s.doc.styles.resolve_para(&para.props);
                match p {
                    "id" => json!(para_id(path)),
                    "text" => json!(para.text.replace(wordcraft_doc::para::OBJ, "")),
                    "style" => json!(style_name(s, &style_id)),
                    "styleBuiltIn" => json!(built_in(&style_id)),
                    "alignment" => json!(align_name(r.align)),
                    "firstLineIndent" => json!(r.indent_first),
                    "leftIndent" => json!(r.indent_left),
                    "rightIndent" => json!(r.indent_right),
                    "spaceAfter" => json!(r.space_after),
                    "spaceBefore" => json!(r.space_before),
                    "lineSpacing" => json!(match r.line_spacing {
                        LineSpacing::Multiple(m) => m * 12.0,
                        LineSpacing::AtLeast(pt) | LineSpacing::Exactly(pt) => pt,
                    }),
                    "isListItem" => json!(para.props.numbering.is_some()),
                    _ => return Err(invalid(format!("Paragraph has no property `{p}`"))),
                }
            }
            (Obj::Range(a, b), p) => match p {
                "id" => json!(range_id(a, b)),
                "text" => json!(text_of(s, a, b)),
                "isEmpty" => json!(a == b),
                "style" => {
                    let styles: Vec<Value> = paths_in(s, a, b)
                        .iter()
                        .filter_map(|x| s.doc.para(StoryRef::Body, x))
                        .map(|x| json!(x.props.style.clone().unwrap_or_else(|| "Normal".into())))
                        .collect();
                    match common(styles) {
                        Value::String(id) => json!(style_name(s, &id)),
                        other => other,
                    }
                }
                _ => return Err(invalid(format!("Range has no property `{p}`"))),
            },
            (Obj::Font(a, b), p) => {
                let chars = font_props(s, a, b);
                let vals: Vec<Value> = chars
                    .iter()
                    .map(|c| match p {
                        "bold" => json!(c.bold),
                        "italic" => json!(c.italic),
                        "underline" => json!(underline_name(c.underline)),
                        "size" => json!(c.size),
                        "name" => json!(c.font),
                        "color" => json!(match c.color {
                            TextColor::Auto => "#000000".to_string(),
                            TextColor::Rgb(rgb) => format!("#{}", rgb.hex().to_ascii_uppercase()),
                        }),
                        "highlightColor" => c.highlight.map(|h| json!(format!("#{}", h.hex().to_ascii_uppercase()))).unwrap_or(Value::Null),
                        "strikeThrough" => json!(c.strike),
                        "doubleStrikeThrough" => json!(c.double_strike),
                        "subscript" => json!(c.vert_align == VertAlign::Subscript),
                        "superscript" => json!(c.vert_align == VertAlign::Superscript),
                        _ => Value::Bool(false),
                    })
                    .collect();
                if !FONT_PROPS.contains(&p) {
                    return Err(invalid(format!("Font has no property `{p}`")));
                }
                common(vals)
            }
            (o, p) => return Err(invalid(format!("{} has no property `{p}`", name(o)))),
        };
        out.insert(prop.clone(), v);
    }
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------- writing

fn run(s: &mut Session, id: &str, params: Value) -> R<Value> {
    // The commands run inside a sync are one undo step (see `Session::one_step`).
    s.join_next_undo();
    s.run(id, &params).map_err(engine)
}

fn select(s: &mut Session, a: &Pos, b: &Pos) -> R<()> {
    let to = |x: &Pos| serde_json::to_value(x).unwrap_or(Value::Null);
    s.run("select.range", &json!({"anchor": to(a), "focus": to(b)})).map_err(engine)?;
    Ok(())
}

/// Runs `cmd` on the text `a..b` (the selection set for it).
fn on_range(s: &mut Session, a: &Pos, b: &Pos, cmd: &str, params: Value) -> R<()> {
    select(s, a, b)?;
    run(s, cmd, params)?;
    Ok(())
}

/// Every paragraph of `a..b` selected (paragraph commands work on the paragraphs a selection touches).
fn on_paragraphs(s: &mut Session, a: &Pos, b: &Pos, cmd: &str, params: Value) -> R<()> {
    on_range(s, a, b, cmd, params)
}

fn bool_value(value: &Value, what: &str) -> R<bool> {
    value.as_bool().ok_or_else(|| invalid(format!("{what} is true or false")))
}

fn num(value: &Value, what: &str) -> R<f64> {
    value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| invalid(format!("{what} is a number")))
}

/// A colour as add-ins give it, as WordCraft's `RRGGBB`: `#RRGGBB` (or `RRGGBB`), `#RGB`,
/// `#RRGGBBAA` (text has no transparency: the alpha is dropped) or a CSS colour name (`blue`,
/// `DarkSlateGray`: any case). Office.js takes these wherever it takes a colour.
fn color(value: &Value) -> R<String> {
    let c = value.as_str().map(str::trim).unwrap_or("");
    let (hex, marked) = match c.strip_prefix('#') {
        Some(h) => (h, true),
        None => (c, false),
    };
    if hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        match (hex.len(), marked) {
            (6 | 8, _) => return Ok(hex.get(..6).unwrap_or_default().to_ascii_uppercase()),
            (3, true) => return Ok(hex.chars().flat_map(|ch| [ch, ch]).collect::<String>().to_ascii_uppercase()),
            _ => {}
        }
    }
    css_color(c).map(str::to_string).ok_or_else(|| invalid(format!("`{c}` isn't a colour like #FF0000 or a colour name like Blue")))
}

/// A CSS named colour's `RRGGBB` (case doesn't matter).
fn css_color(name: &str) -> Option<&'static str> {
    CSS_COLORS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, hex)| *hex)
}

/// A highlight colour: a name (`Yellow`, `BrightGreen`…), its `#RRGGBB`, or null for none.
fn highlight(value: &Value) -> R<String> {
    let name = match value {
        Value::Null => return Ok("none".into()),
        Value::String(t) if t.eq_ignore_ascii_case("none") || t.is_empty() => return Ok("none".into()),
        Value::String(t) => t.trim().to_string(),
        _ => return Err(invalid("highlightColor is a colour name or #RRGGBB")),
    };
    // Word's highlight names first (its `Pink` isn't CSS's), then a CSS name by its colour.
    let word_name = Highlight::ALL.iter().any(|h| highlight_label(*h).eq_ignore_ascii_case(&name));
    let hex = match css_color(&name) {
        Some(css) if !word_name => css.to_string(),
        _ => name.trim_start_matches('#').to_ascii_uppercase(),
    };
    for h in Highlight::ALL {
        let label = highlight_label(h);
        if label.eq_ignore_ascii_case(&name) || h.rgb().is_some_and(|x| x.hex().eq_ignore_ascii_case(&hex)) {
            return Ok(label);
        }
    }
    Err(invalid(format!("`{name}` isn't one of the highlight colours")))
}

fn highlight_label(h: Highlight) -> String {
    serde_json::to_value(h).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn set(s: &mut Session, obj: &Obj, prop: &str, value: &Value) -> R<()> {
    match (obj, prop) {
        (Obj::Properties, p) => {
            let key = match p {
                "title" | "subject" | "author" | "keywords" | "comments" | "category" => p,
                _ => return Err(invalid(format!("DocumentProperties.{p} can't be set"))),
            };
            let text = value.as_str().ok_or_else(|| invalid(format!("{p} is text")))?;
            run(s, "file.properties", json!({key: text}))?;
        }
        (Obj::Body, "style") => {
            let (a, b) = body_range(s);
            let style = value.as_str().ok_or_else(|| invalid("style is a style's name"))?.to_string();
            on_paragraphs(s, &a, &b, "para.style", json!({"style": style}))?;
        }
        (Obj::Paragraph(path), p) => {
            let (a, b) = para_range(s, path);
            match p {
                "style" | "styleBuiltIn" => {
                    let style = value.as_str().ok_or_else(|| invalid("style is a style's name"))?.to_string();
                    on_paragraphs(s, &a, &b, "para.style", json!({"style": style}))?;
                }
                "alignment" => {
                    let cmd = match value.as_str().map(str::to_ascii_lowercase).as_deref() {
                        Some("left") => "para.alignLeft",
                        Some("centered" | "center") => "para.alignCenter",
                        Some("right") => "para.alignRight",
                        Some("justified" | "justify") => "para.justify",
                        _ => return Err(invalid("alignment is Left, Centered, Right or Justified")),
                    };
                    on_paragraphs(s, &a, &b, cmd, json!({}))?;
                }
                "firstLineIndent" | "leftIndent" | "rightIndent" | "spaceAfter" | "spaceBefore" => {
                    let v = num(value, p)?;
                    if !(-1584.0..=1584.0).contains(&v) {
                        return Err(invalid(format!("{p} is -1584 to 1584 points")));
                    }
                    let key = match p {
                        "firstLineIndent" => "indentFirst",
                        "leftIndent" => "indentLeft",
                        "rightIndent" => "indentRight",
                        "spaceAfter" => "spaceAfter",
                        _ => "spaceBefore",
                    };
                    on_paragraphs(s, &a, &b, "para.set", json!({"props": {key: v}}))?;
                }
                "lineSpacing" => {
                    let v = num(value, p)?;
                    if !(0.0..=1584.0).contains(&v) {
                        return Err(invalid("lineSpacing is 0 to 1584 points"));
                    }
                    on_paragraphs(s, &a, &b, "para.lineSpacing", json!({"exactly": v}))?;
                }
                _ => return Err(invalid(format!("Paragraph.{p} can't be set"))),
            }
        }
        (Obj::Range(a, b), "style") => {
            let style = value.as_str().ok_or_else(|| invalid("style is a style's name"))?.to_string();
            on_paragraphs(s, a, b, "para.style", json!({"style": style}))?;
        }
        (Obj::Font(a, b), p) => {
            let (cmd, params) = match p {
                "bold" => ("format.bold", json!({"value": bool_value(value, p)?})),
                "italic" => ("format.italic", json!({"value": bool_value(value, p)?})),
                "underline" => match value.as_str().map(str::to_ascii_lowercase).as_deref() {
                    Some("none") => ("format.underline", json!({"value": false})),
                    Some(u @ ("single" | "double" | "thick" | "dotted" | "wave")) => ("format.underline", json!({"value": true, "style": u})),
                    Some("dashed" | "dash" | "dashline") => ("format.underline", json!({"value": true, "style": "dash"})),
                    Some("dotdashline" | "dotdash") => ("format.underline", json!({"value": true, "style": "dotDash"})),
                    Some("twodotdashline" | "dotdotdash") => ("format.underline", json!({"value": true, "style": "dotDotDash"})),
                    Some("word" | "words") => ("format.underline", json!({"value": true, "style": "words"})),
                    _ => {
                        return Err(invalid("underline is None, Single, Word, Double, Thick, Dotted, DashLine, DotDashLine, TwoDotDashLine or Wave"));
                    }
                },
                "size" => {
                    let v = num(value, p)?;
                    if !(1.0..=1638.0).contains(&v) {
                        return Err(invalid("size is 1 to 1638 points"));
                    }
                    ("format.size", json!({"size": v}))
                }
                "name" => {
                    ("format.font", json!({"name": value.as_str().filter(|n| !n.trim().is_empty()).ok_or_else(|| invalid("name is a font's name"))?}))
                }
                "color" => ("format.color", json!({"color": color(value)?})),
                "highlightColor" => ("format.highlight", json!({"color": highlight(value)?})),
                "strikeThrough" => ("format.strikethrough", json!({"value": bool_value(value, p)?})),
                "doubleStrikeThrough" => ("format.doubleStrikethrough", json!({"value": bool_value(value, p)?})),
                "subscript" => ("format.subscript", json!({"value": bool_value(value, p)?})),
                "superscript" => ("format.superscript", json!({"value": bool_value(value, p)?})),
                _ => return Err(invalid(format!("Font.{p} can't be set"))),
            };
            if a == b {
                return Err(invalid("there's no text to format (the range is empty)"));
            }
            on_range(s, a, b, cmd, params)?;
        }
        (o, p) => return Err(invalid(format!("{}.{p} can't be set", name(o)))),
    }
    Ok(())
}

/// Inserts `text` at `at` (replacing `at..to`), as plain text taking the formatting there; the
/// inserted text's range.
fn insert_text(s: &mut Session, at: &Pos, to: &Pos, text: &str) -> R<Value> {
    if text.len() > 16 << 20 {
        return Err(invalid("text is limited to 16 MiB"));
    }
    select(s, at, to)?;
    if at.path != to.path || text.contains('\n') || text.contains('\r') {
        // Paragraphs come and go: ids from before this edit may move in ways not tracked.
        log::debug!("office.sync: a multi-paragraph insert; earlier ids in this sync may be stale");
    } else if to.off > at.off {
        note(Edit::Text(at.path.clone(), at.off, -((to.off - at.off) as isize)));
    }
    if text.is_empty() {
        if at != to {
            run(s, "text.delete", json!({}))?;
        }
    } else {
        run(s, "edit.pasteText", json!({"text": text}))?;
        if !text.contains('\n') && !text.contains('\r') && s.sel.focus.path == at.path {
            note(Edit::Text(at.path.clone(), at.off, (s.sel.focus.off as isize - at.off as isize).max(0)));
        }
    }
    let end = s.sel.focus.clone();
    Ok(json!({"id": range_id(at, &end), "text": text}))
}

/// A new paragraph with `text` before or after `path`; its id.
fn insert_paragraph(s: &mut Session, path: &Path, text: &str, after: bool) -> R<Value> {
    if text.contains('\n') || text.contains('\r') {
        return Err(invalid("a paragraph's text has no line breaks (insert one paragraph at a time)"));
    }
    let len = para_len(s, path);
    let new = if after {
        let end = Pos::new(StoryRef::Body, path.clone(), len);
        select(s, &end, &end)?;
        run(s, "text.newParagraph", json!({}))?;
        let at = s.sel.focus.clone();
        note(Edit::InsertPara(at.path.clone()));
        if !text.is_empty() {
            run(s, "edit.pasteText", json!({"text": text}))?;
        }
        at.path
    } else {
        let start = Pos::new(StoryRef::Body, path.clone(), 0);
        select(s, &start, &start)?;
        if !text.is_empty() {
            run(s, "edit.pasteText", json!({"text": text}))?;
        }
        run(s, "text.newParagraph", json!({}))?;
        note(Edit::InsertPara(path.clone()));
        path.clone()
    };
    Ok(json!({"id": para_id(&new)}))
}

/// `insertBreak(breakType, location)` at `at`: a page or line break, or a section break (in the
/// main text only, as `layout.break` puts it).
fn insert_break(s: &mut Session, at: &Pos, kind: &str) -> R<Value> {
    let kind = match kind.to_ascii_lowercase().as_str() {
        "page" => "page",
        "line" => "textWrapping",
        "sectionnext" | "next" => "nextPage",
        "sectioncontinuous" => "continuous",
        "sectioneven" => "evenPage",
        "sectionodd" => "oddPage",
        other => return Err(invalid(format!("`{other}` isn't a break type (Page, Line, SectionNext, SectionContinuous, SectionEven, SectionOdd)"))),
    };
    select(s, at, at)?;
    run(s, "layout.break", json!({"kind": kind}))?;
    if matches!(kind, "page" | "textWrapping") && s.sel.focus.path == at.path {
        note(Edit::Text(at.path.clone(), at.off, (s.sel.focus.off as isize - at.off as isize).max(0)));
    }
    Ok(Value::Null)
}

/// Deletes `a..b`; a whole paragraph goes with its mark.
fn delete(s: &mut Session, a: &Pos, b: &Pos) -> R<()> {
    if a != b {
        on_range(s, a, b, "text.delete", json!({}))?;
    }
    Ok(())
}

fn delete_paragraph(s: &mut Session, path: &Path) -> R<()> {
    let len = para_len(s, path);
    if s.doc.next_para(StoryRef::Body, path).is_some() || s.doc.prev_para(StoryRef::Body, path).is_some() {
        note(Edit::RemovePara(path.clone()));
    }
    if let Some(next) = s.doc.next_para(StoryRef::Body, path) {
        let (a, b) = (Pos::new(StoryRef::Body, path.clone(), 0), Pos::new(StoryRef::Body, next, 0));
        return on_range(s, &a, &b, "text.delete", json!({}));
    }
    if let Some(prev) = s.doc.prev_para(StoryRef::Body, path) {
        let plen = para_len(s, &prev);
        let (a, b) = (Pos::new(StoryRef::Body, prev, plen), Pos::new(StoryRef::Body, path.clone(), len));
        return on_range(s, &a, &b, "text.delete", json!({}));
    }
    // The only paragraph: its text goes; a document always has one paragraph.
    delete(s, &Pos::new(StoryRef::Body, path.clone(), 0), &Pos::new(StoryRef::Body, path.clone(), len))
}

/// `select(mode)`: the whole range (`Select`, the default), or the caret at its `Start` or `End`.
fn select_mode(s: &mut Session, a: &Pos, b: &Pos, args: &[Value]) -> R<()> {
    match location(args, 0, "select").as_str() {
        "select" => select(s, a, b),
        "start" => select(s, a, a),
        "end" => select(s, b, b),
        other => Err(invalid(format!("select takes Select, Start or End, not `{other}`"))),
    }
}

fn location(args: &[Value], i: usize, default: &str) -> String {
    args.get(i).and_then(Value::as_str).unwrap_or(default).to_ascii_lowercase()
}

fn search(s: &Session, text: &str, opts: &Value) -> R<Value> {
    if text.is_empty() || text.len() > 255 {
        return Err(invalid("search text is 1 to 255 characters"));
    }
    let match_case = opts.get("matchCase").and_then(Value::as_bool).unwrap_or(false);
    let whole = opts.get("matchWholeWord").and_then(Value::as_bool).unwrap_or(false);
    let fold = |t: &str| if match_case { t.to_string() } else { t.to_lowercase() };
    let needle = fold(text);
    let mut items = Vec::new();
    for path in paths(s) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        // Lower-casing can change byte lengths: search char by char positions of the original.
        let hay = &p.text;
        let mut start = 0;
        while let Some(found) = hay.get(start..).and_then(|rest| {
            rest.char_indices()
                .map(|(i, _)| i)
                .find(|&i| rest.get(i..).is_some_and(|r| r.len() >= needle.len() && fold(r.get(..needle.len()).unwrap_or("")) == needle))
        }) {
            let from = start + found;
            let to = from + needle.len();
            let boundary = |i: usize| hay.get(..i).and_then(|h| h.chars().last()).is_none_or(|c| !c.is_alphanumeric());
            let after = hay.get(to..).and_then(|h| h.chars().next()).is_none_or(|c| !c.is_alphanumeric());
            if hay.is_char_boundary(to) && (!whole || (boundary(from) && after)) {
                let (a, b) = (Pos::new(StoryRef::Body, path.clone(), from), Pos::new(StoryRef::Body, path.clone(), to));
                items.push(json!({"id": range_id(&a, &b), "text": hay.get(from..to).unwrap_or("")}));
                if items.len() >= MAX_ITEMS {
                    return Ok(json!({"items": items}));
                }
            }
            start = from + hay.get(from..).and_then(|r| r.chars().next()).map_or(1, char::len_utf8);
        }
    }
    Ok(json!({"items": items}))
}

fn call(s: &mut Session, addin: &str, obj: &Obj, method: &str, args: &[Value]) -> R<Value> {
    let text_arg = || args.first().and_then(Value::as_str).map(str::to_string).ok_or_else(|| invalid(format!("{method} needs text")));
    match (obj, method) {
        (Obj::Document, "save") => Ok(Value::Null),
        (Obj::Document, "getSelectedData") => Ok(json!(s.selected_text())),
        (Obj::Document, "setSelectedData") => {
            let text = match args.first() {
                Some(Value::String(t)) => t.clone(),
                Some(Value::Array(rows)) => rows
                    .iter()
                    .map(|r| {
                        r.as_array()
                            .map(|c| c.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect::<Vec<_>>().join("\t"))
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                Some(v) => v.to_string(),
                None => String::new(),
            };
            run(s, "edit.pasteText", json!({"text": text}))?;
            Ok(Value::Null)
        }
        (Obj::Body, "insertText") => {
            let text = text_arg()?;
            let (a, b) = body_range(s);
            match location(args, 1, "end").as_str() {
                "start" => insert_text(s, &a, &a, &text),
                "end" => insert_text(s, &b, &b, &text),
                "replace" => insert_text(s, &a, &b, &text),
                other => Err(invalid(format!("Body.insertText takes Start, End or Replace, not `{other}`"))),
            }
        }
        (Obj::Body, "insertParagraph") => {
            let text = text_arg()?;
            let all = paths(s);
            match location(args, 1, "end").as_str() {
                "start" => insert_paragraph(s, &all.first().cloned().unwrap_or_else(|| Path::top(0)), &text, false),
                "end" => insert_paragraph(s, &all.last().cloned().unwrap_or_else(|| Path::top(0)), &text, true),
                other => Err(invalid(format!("Body.insertParagraph takes Start or End, not `{other}`"))),
            }
        }
        (Obj::Body, "insertBreak") => {
            let (a, b) = body_range(s);
            let kind = location(args, 0, "page");
            match location(args, 1, "end").as_str() {
                "start" => insert_break(s, &a, &kind),
                "end" => insert_break(s, &b, &kind),
                other => Err(invalid(format!("Body.insertBreak takes Start or End, not `{other}`"))),
            }
        }
        (Obj::Paragraph(path), "insertBreak") => {
            let (a, b) = para_range(s, path);
            let kind = location(args, 0, "page");
            match location(args, 1, "after").as_str() {
                "before" => insert_break(s, &a, &kind),
                "after" => insert_break(s, &b, &kind),
                other => Err(invalid(format!("Paragraph.insertBreak takes Before or After, not `{other}`"))),
            }
        }
        (Obj::Range(a, b), "insertBreak") => {
            let kind = location(args, 0, "page");
            match location(args, 1, "after").as_str() {
                "before" | "start" => insert_break(s, a, &kind),
                "after" | "end" => insert_break(s, b, &kind),
                other => Err(invalid(format!("Range.insertBreak takes Before or After, not `{other}`"))),
            }
        }
        (Obj::Body, "clear") => {
            let (a, b) = body_range(s);
            delete(s, &a, &b)?;
            Ok(Value::Null)
        }
        (Obj::Body, "search") => search(s, &text_arg()?, args.get(1).unwrap_or(&Value::Null)),
        (Obj::Paragraph(path), "insertText") => {
            let text = text_arg()?;
            let (a, b) = para_range(s, path);
            match location(args, 1, "end").as_str() {
                "start" => insert_text(s, &a, &a, &text),
                "end" => insert_text(s, &b, &b, &text),
                "replace" => insert_text(s, &a, &b, &text),
                other => Err(invalid(format!("Paragraph.insertText takes Start, End or Replace, not `{other}`"))),
            }
        }
        (Obj::Paragraph(path), "insertParagraph") => {
            let text = text_arg()?;
            match location(args, 1, "after").as_str() {
                "before" => insert_paragraph(s, path, &text, false),
                "after" => insert_paragraph(s, path, &text, true),
                other => Err(invalid(format!("Paragraph.insertParagraph takes Before or After, not `{other}`"))),
            }
        }
        (Obj::Paragraph(path), "delete") => {
            delete_paragraph(s, path)?;
            Ok(Value::Null)
        }
        (Obj::Paragraph(path), "select") => {
            let (a, b) = para_range(s, path);
            select_mode(s, &a, &b, args)?;
            Ok(Value::Null)
        }
        (Obj::Range(a, b), "insertText") => {
            let text = text_arg()?;
            match location(args, 1, "replace").as_str() {
                "start" | "before" => insert_text(s, a, a, &text),
                "end" | "after" => insert_text(s, b, b, &text),
                "replace" => insert_text(s, a, b, &text),
                other => Err(invalid(format!("Range.insertText takes Start, End, Before, After or Replace, not `{other}`"))),
            }
        }
        (Obj::Range(a, b), "insertParagraph") => {
            let text = text_arg()?;
            match location(args, 1, "after").as_str() {
                "before" => insert_paragraph(s, &a.path, &text, false),
                "after" => insert_paragraph(s, &b.path, &text, true),
                other => Err(invalid(format!("Range.insertParagraph takes Before or After, not `{other}`"))),
            }
        }
        (Obj::Range(a, b), "delete") => {
            delete(s, a, b)?;
            Ok(Value::Null)
        }
        (Obj::Range(a, b), "select") => {
            select_mode(s, a, b, args)?;
            Ok(Value::Null)
        }
        (Obj::Settings, "save") => {
            let all = args.first().cloned().filter(Value::is_object).ok_or_else(|| invalid("settings are an object"))?;
            if all.to_string().len() > 2 << 20 {
                return Err(invalid("settings are limited to 2 MB"));
            }
            let key = (s.document_id(), addin.to_string());
            let mut store = match settings_store().lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            store.insert(key, all);
            Ok(Value::Null)
        }
        (o, m) => Err(OfficeError("NotImplemented", format!("{}.{m}() isn't supported yet", name(o)))),
    }
}

/// The CSS named colours (CSS Color Module Level 4: 148 names, `grey` spellings included), as
/// `RRGGBB`.
const CSS_COLORS: &[(&str, &str)] = &[
    ("aliceblue", "F0F8FF"),
    ("antiquewhite", "FAEBD7"),
    ("aqua", "00FFFF"),
    ("aquamarine", "7FFFD4"),
    ("azure", "F0FFFF"),
    ("beige", "F5F5DC"),
    ("bisque", "FFE4C4"),
    ("black", "000000"),
    ("blanchedalmond", "FFEBCD"),
    ("blue", "0000FF"),
    ("blueviolet", "8A2BE2"),
    ("brown", "A52A2A"),
    ("burlywood", "DEB887"),
    ("cadetblue", "5F9EA0"),
    ("chartreuse", "7FFF00"),
    ("chocolate", "D2691E"),
    ("coral", "FF7F50"),
    ("cornflowerblue", "6495ED"),
    ("cornsilk", "FFF8DC"),
    ("crimson", "DC143C"),
    ("cyan", "00FFFF"),
    ("darkblue", "00008B"),
    ("darkcyan", "008B8B"),
    ("darkgoldenrod", "B8860B"),
    ("darkgray", "A9A9A9"),
    ("darkgreen", "006400"),
    ("darkgrey", "A9A9A9"),
    ("darkkhaki", "BDB76B"),
    ("darkmagenta", "8B008B"),
    ("darkolivegreen", "556B2F"),
    ("darkorange", "FF8C00"),
    ("darkorchid", "9932CC"),
    ("darkred", "8B0000"),
    ("darksalmon", "E9967A"),
    ("darkseagreen", "8FBC8F"),
    ("darkslateblue", "483D8B"),
    ("darkslategray", "2F4F4F"),
    ("darkslategrey", "2F4F4F"),
    ("darkturquoise", "00CED1"),
    ("darkviolet", "9400D3"),
    ("deeppink", "FF1493"),
    ("deepskyblue", "00BFFF"),
    ("dimgray", "696969"),
    ("dimgrey", "696969"),
    ("dodgerblue", "1E90FF"),
    ("firebrick", "B22222"),
    ("floralwhite", "FFFAF0"),
    ("forestgreen", "228B22"),
    ("fuchsia", "FF00FF"),
    ("gainsboro", "DCDCDC"),
    ("ghostwhite", "F8F8FF"),
    ("gold", "FFD700"),
    ("goldenrod", "DAA520"),
    ("gray", "808080"),
    ("green", "008000"),
    ("greenyellow", "ADFF2F"),
    ("grey", "808080"),
    ("honeydew", "F0FFF0"),
    ("hotpink", "FF69B4"),
    ("indianred", "CD5C5C"),
    ("indigo", "4B0082"),
    ("ivory", "FFFFF0"),
    ("khaki", "F0E68C"),
    ("lavender", "E6E6FA"),
    ("lavenderblush", "FFF0F5"),
    ("lawngreen", "7CFC00"),
    ("lemonchiffon", "FFFACD"),
    ("lightblue", "ADD8E6"),
    ("lightcoral", "F08080"),
    ("lightcyan", "E0FFFF"),
    ("lightgoldenrodyellow", "FAFAD2"),
    ("lightgray", "D3D3D3"),
    ("lightgreen", "90EE90"),
    ("lightgrey", "D3D3D3"),
    ("lightpink", "FFB6C1"),
    ("lightsalmon", "FFA07A"),
    ("lightseagreen", "20B2AA"),
    ("lightskyblue", "87CEFA"),
    ("lightslategray", "778899"),
    ("lightslategrey", "778899"),
    ("lightsteelblue", "B0C4DE"),
    ("lightyellow", "FFFFE0"),
    ("lime", "00FF00"),
    ("limegreen", "32CD32"),
    ("linen", "FAF0E6"),
    ("magenta", "FF00FF"),
    ("maroon", "800000"),
    ("mediumaquamarine", "66CDAA"),
    ("mediumblue", "0000CD"),
    ("mediumorchid", "BA55D3"),
    ("mediumpurple", "9370DB"),
    ("mediumseagreen", "3CB371"),
    ("mediumslateblue", "7B68EE"),
    ("mediumspringgreen", "00FA9A"),
    ("mediumturquoise", "48D1CC"),
    ("mediumvioletred", "C71585"),
    ("midnightblue", "191970"),
    ("mintcream", "F5FFFA"),
    ("mistyrose", "FFE4E1"),
    ("moccasin", "FFE4B5"),
    ("navajowhite", "FFDEAD"),
    ("navy", "000080"),
    ("oldlace", "FDF5E6"),
    ("olive", "808000"),
    ("olivedrab", "6B8E23"),
    ("orange", "FFA500"),
    ("orangered", "FF4500"),
    ("orchid", "DA70D6"),
    ("palegoldenrod", "EEE8AA"),
    ("palegreen", "98FB98"),
    ("paleturquoise", "AFEEEE"),
    ("palevioletred", "DB7093"),
    ("papayawhip", "FFEFD5"),
    ("peachpuff", "FFDAB9"),
    ("peru", "CD853F"),
    ("pink", "FFC0CB"),
    ("plum", "DDA0DD"),
    ("powderblue", "B0E0E6"),
    ("purple", "800080"),
    ("rebeccapurple", "663399"),
    ("red", "FF0000"),
    ("rosybrown", "BC8F8F"),
    ("royalblue", "4169E1"),
    ("saddlebrown", "8B4513"),
    ("salmon", "FA8072"),
    ("sandybrown", "F4A460"),
    ("seagreen", "2E8B57"),
    ("seashell", "FFF5EE"),
    ("sienna", "A0522D"),
    ("silver", "C0C0C0"),
    ("skyblue", "87CEEB"),
    ("slateblue", "6A5ACD"),
    ("slategray", "708090"),
    ("slategrey", "708090"),
    ("snow", "FFFAFA"),
    ("springgreen", "00FF7F"),
    ("steelblue", "4682B4"),
    ("tan", "D2B48C"),
    ("teal", "008080"),
    ("thistle", "D8BFD8"),
    ("tomato", "FF6347"),
    ("turquoise", "40E0D0"),
    ("violet", "EE82EE"),
    ("wheat", "F5DEB3"),
    ("white", "FFFFFF"),
    ("whitesmoke", "F5F5F5"),
    ("yellow", "FFFF00"),
    ("yellowgreen", "9ACD32"),
];

#[cfg(test)]
mod tests;

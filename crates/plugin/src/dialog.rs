//! Protocol 1.2: what a command's answer may ask of the app besides engine commands — a dialog
//! the app draws itself (`dialog`), or a local file to open (`open`).
//!
//! Plugin answers are hostile input: everything here is checked and bounded. Text over a limit is
//! cut (at a character boundary), lists over a limit are shortened, and anything malformed (a
//! wrong type, an unknown kind, a list without a command) is an error that fails the command.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

/// Most items a list dialog shows.
pub const MAX_ITEMS: usize = 10_000;
/// Longest item id, in characters.
pub const MAX_ITEM_ID: usize = 4_096;
/// Longest item label and detail, field and button label, in characters.
pub const MAX_ITEM_TEXT: usize = 256;
/// Most fields a form has.
pub const MAX_FIELDS: usize = 16;
/// Most options a `choice` field offers (a host bound; the protocol sets none).
pub const MAX_OPTIONS: usize = 1_000;
/// Longest prompt, in characters (a message's text).
pub const MAX_PROMPT: usize = 4_000;
/// Longest title, and the `open` label, in characters.
pub const MAX_TITLE: usize = 200;
/// Most extra buttons, shown left of OK.
pub const MAX_BUTTONS: usize = 4;
/// Longest field value, in characters (a host bound).
pub const MAX_VALUE: usize = 100_000;
/// Longest multi-line field value, in characters (protocol 2.2): a script of a few thousand lines.
pub const MAX_MULTILINE_VALUE: usize = 200_000;
/// A multi-line field's height in lines (protocol 2.2's `rows`): the default, the fewest and the
/// most.
pub const DEFAULT_ROWS: u8 = 8;
pub const MIN_ROWS: u8 = 2;
pub const MAX_ROWS: u8 = 30;
/// Longest path `open` accepts, in bytes (a host bound).
pub const MAX_PATH: usize = 4_096;
/// Longest command id and icon name.
const MAX_NAME: usize = 128;

/// `kind`: what the dialog shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogKind {
    /// Items to pick from (one, or several with `multi`).
    List,
    /// Fields to fill in.
    Form,
    /// Text, and buttons.
    Message,
}

/// One item of a list dialog.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub detail: String,
    /// One of the app's icon names (as for ribbon buttons); unknown names get a generic file icon.
    #[serde(default)]
    pub icon: String,
}

/// `kind` of a form field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    #[default]
    Text,
    /// A password box.
    Secret,
    Number,
    /// `value` true or false.
    Checkbox,
    /// One of `options`; `value` is an option's id.
    Choice,
}

/// One option of a `choice` field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub label: String,
}

/// One field of a form. `value` is a string for text, secret and choice fields, a number (or
/// `null`, empty) for number fields and a bool for checkboxes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub value: Json,
    #[serde(default)]
    pub kind: FieldKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Choice>,
    /// A text field edited on several lines (protocol 2.2's `"multiline": true`): Enter starts a
    /// new line instead of pressing OK.
    #[serde(default, skip_serializing_if = "is_false")]
    pub multiline: bool,
    /// A multi-line field holding code (`"code": true`): monospace, Tab indents.
    #[serde(default, skip_serializing_if = "is_false")]
    pub code: bool,
    /// A multi-line field's height in lines (`rows`, 2–30, default 8); `None` on one-line fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u8>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// An extra button: runs `command` with its own `params` (plus the selection or the fields).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DialogButton {
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub params: Json,
}

/// A dialog a plugin command asks the app to show after its step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dialog {
    pub title: String,
    pub kind: DialogKind,
    /// Text above the content (a message's text).
    #[serde(default)]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ListItem>,
    /// A list allows several items (`ids` instead of `id`).
    #[serde(default)]
    pub multi: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    /// What OK (and a double-click on an item) runs; a message may have none (OK closes).
    #[serde(default)]
    pub command: Option<String>,
    /// Passed to `command`, with the item's `id`/`ids` or the form's `fields` added.
    #[serde(default)]
    pub params: Json,
    /// OK's label; `None`: the app's own "OK".
    #[serde(default)]
    pub ok_label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<DialogButton>,
}

/// A local file a plugin command asks the app to open after its step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRequest {
    pub path: String,
    /// Where the document came from, shown as its location (at most 200 characters).
    #[serde(default)]
    pub label: Option<String>,
    /// Open read-only where the app can (WordCraft can't: it's ignored).
    #[serde(default)]
    pub read_only: bool,
}

/// At most `max` characters of `s`.
pub fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s.get(..i).unwrap_or("").to_string(),
        None => s.to_string(),
    }
}

fn text(v: &Json, key: &str, what: &str, max: usize) -> Result<Option<String>, String> {
    match v.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(s)) => Ok(Some(cut(s, max))),
        Some(_) => Err(format!("{what}: `{key}` must be text")),
    }
}

fn required(v: &Json, key: &str, what: &str, max: usize) -> Result<String, String> {
    text(v, key, what, max)?.ok_or_else(|| format!("{what}: `{key}` is missing"))
}

/// `params`: an object, or nothing (an empty object).
fn params(v: &Json, what: &str) -> Result<Json, String> {
    match v.get("params") {
        None | Some(Json::Null) => Ok(Json::Object(Map::new())),
        Some(p @ Json::Object(_)) => Ok(p.clone()),
        Some(_) => Err(format!("{what}: `params` must be an object")),
    }
}

/// A command id: letters, digits, `.`, `-`, `_`.
fn command_id(v: &Json, key: &str, what: &str) -> Result<Option<String>, String> {
    let Some(id) = text(v, key, what, usize::MAX)? else { return Ok(None) };
    if id.is_empty() || id.len() > MAX_NAME || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) {
        return Err(format!("{what}: `{key}` isn't a command id"));
    }
    Ok(Some(id))
}

fn array<'a>(v: &'a Json, key: &str, what: &str) -> Result<&'a [Json], String> {
    match v.get(key) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::Array(a)) => Ok(a),
        Some(_) => Err(format!("{what}: `{key}` must be a list")),
    }
}

fn item(v: &Json) -> Result<ListItem, String> {
    let what = "dialog item";
    if !v.is_object() {
        return Err(format!("each {what} must be an object"));
    }
    let id = required(v, "id", what, MAX_ITEM_ID)?;
    let label = required(v, "label", what, MAX_ITEM_TEXT)?;
    let detail = text(v, "detail", what, MAX_ITEM_TEXT)?.unwrap_or_default();
    let icon = text(v, "icon", what, MAX_NAME)?.unwrap_or_default();
    Ok(ListItem { id, label, detail, icon })
}

fn field(v: &Json) -> Result<Field, String> {
    let what = "dialog field";
    if !v.is_object() {
        return Err(format!("each {what} must be an object"));
    }
    let id = required(v, "id", what, MAX_ITEM_TEXT)?;
    if id.is_empty() {
        return Err(format!("{what}: `id` is empty"));
    }
    let label = text(v, "label", what, MAX_ITEM_TEXT)?.unwrap_or_default();
    let kind = match v.get("kind") {
        None | Some(Json::Null) => FieldKind::Text,
        Some(k) => serde_json::from_value(k.clone()).map_err(|_| format!("{what} `{id}`: `kind` is text, secret, number, checkbox or choice"))?,
    };
    let mut options = Vec::new();
    if kind == FieldKind::Choice {
        for o in array(v, "options", what)?.iter().take(MAX_OPTIONS) {
            if !o.is_object() {
                return Err(format!("{what} `{id}`: each option must be an object"));
            }
            let oid = required(o, "id", "choice option", MAX_ITEM_ID)?;
            let label = text(o, "label", "choice option", MAX_ITEM_TEXT)?.unwrap_or_else(|| cut(&oid, MAX_ITEM_TEXT));
            options.push(Choice { id: oid, label });
        }
        if options.is_empty() {
            return Err(format!("{what} `{id}`: a choice needs `options`"));
        }
    }
    // Protocol 2.2: multi-line and code apply to text fields; other kinds ignore them (a password
    // box stays on one line). Malformed keys are errors, as everywhere in a dialog.
    let flag = |key: &str| match v.get(key) {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{what} `{id}`: `{key}` must be true or false")),
    };
    let (wants_multi, wants_code) = (flag("multiline")?, flag("code")?);
    let rows = match v.get("rows") {
        None | Some(Json::Null) => DEFAULT_ROWS,
        Some(Json::Number(n)) => {
            n.as_f64().filter(|x| x.is_finite()).map_or(DEFAULT_ROWS, |x| x.round().clamp(f64::from(MIN_ROWS), f64::from(MAX_ROWS)) as u8)
        }
        Some(_) => return Err(format!("{what} `{id}`: `rows` must be a number")),
    };
    let multiline = kind == FieldKind::Text && (wants_multi || wants_code);
    let code = multiline && wants_code;
    let max_value = if multiline { MAX_MULTILINE_VALUE } else { MAX_VALUE };
    let raw = v.get("value").unwrap_or(&Json::Null);
    let value = match kind {
        FieldKind::Text | FieldKind::Secret => match raw {
            Json::Null => Json::String(String::new()),
            Json::String(s) => Json::String(cut(s, max_value)),
            Json::Number(n) => Json::String(n.to_string()),
            Json::Bool(b) => Json::String(b.to_string()),
            _ => return Err(format!("{what} `{id}`: `value` must be text")),
        },
        FieldKind::Number => match raw {
            Json::Null => Json::Null,
            Json::Number(_) => raw.clone(),
            Json::String(s) if s.trim().is_empty() => Json::Null,
            Json::String(s) => s
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .and_then(serde_json::Number::from_f64)
                .map(Json::Number)
                .ok_or_else(|| format!("{what} `{id}`: `value` must be a number"))?,
            _ => return Err(format!("{what} `{id}`: `value` must be a number")),
        },
        FieldKind::Checkbox => match raw {
            Json::Null => Json::Bool(false),
            Json::Bool(_) => raw.clone(),
            _ => return Err(format!("{what} `{id}`: `value` must be true or false")),
        },
        // An unknown option picks the first.
        FieldKind::Choice => {
            let want = raw.as_str().unwrap_or("");
            let pick = options.iter().find(|o| o.id == want).or(options.first()).map(|o| o.id.clone()).unwrap_or_default();
            Json::String(pick)
        }
    };
    Ok(Field { id, label, value, kind, options, multiline, code, rows: multiline.then_some(rows) })
}

fn button(v: &Json) -> Result<DialogButton, String> {
    let what = "dialog button";
    if !v.is_object() {
        return Err(format!("each {what} must be an object"));
    }
    let label = required(v, "label", what, MAX_ITEM_TEXT)?;
    let command = command_id(v, "command", what)?.ok_or_else(|| format!("{what} `{label}`: `command` is missing"))?;
    Ok(DialogButton { label, command, params: params(v, what)? })
}

/// Reads and bounds a `dialog` (see the module's notes).
pub fn parse_dialog(v: &Json) -> Result<Dialog, String> {
    let what = "dialog";
    if !v.is_object() {
        return Err("`dialog` must be an object".into());
    }
    let kind = match v.get("kind") {
        Some(Json::String(k)) => match k.as_str() {
            "list" => DialogKind::List,
            "form" => DialogKind::Form,
            "message" => DialogKind::Message,
            other => return Err(format!("dialog: unknown `kind` {:?} (list, form or message)", cut(other, 40))),
        },
        _ => return Err("dialog: `kind` (list, form or message) is missing".into()),
    };
    let title = text(v, "title", what, MAX_TITLE)?.unwrap_or_default();
    let prompt = text(v, "prompt", what, MAX_PROMPT)?.unwrap_or_default();
    let command = command_id(v, "command", what)?;
    if command.is_none() && kind != DialogKind::Message {
        return Err("dialog: a list or a form needs a `command`".into());
    }
    let ok_label = text(v, "okLabel", what, MAX_ITEM_TEXT)?.filter(|l| !l.trim().is_empty());
    let multi = match v.get("multi") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err("dialog: `multi` must be true or false".into()),
    };
    let items =
        if kind == DialogKind::List { array(v, "items", what)?.iter().take(MAX_ITEMS).map(item).collect::<Result<Vec<_>, _>>()? } else { Vec::new() };
    let fields = if kind == DialogKind::Form {
        array(v, "fields", what)?.iter().take(MAX_FIELDS).map(field).collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let mut seen = std::collections::HashSet::new();
    if let Some(f) = fields.iter().find(|f| !seen.insert(f.id.as_str())) {
        return Err(format!("dialog: field `{}` is there twice", f.id));
    }
    let buttons = array(v, "buttons", what)?.iter().take(MAX_BUTTONS).map(button).collect::<Result<Vec<_>, _>>()?;
    Ok(Dialog { title, kind, prompt, items, multi: multi && kind == DialogKind::List, fields, command, params: params(v, what)?, ok_label, buttons })
}

/// Reads and bounds an `open` request. Whether the file is there and the app opens it is the
/// app's to check.
pub fn parse_open(v: &Json) -> Result<OpenRequest, String> {
    let what = "open";
    if !v.is_object() {
        return Err("`open` must be an object".into());
    }
    let path = required(v, "path", what, usize::MAX)?;
    if path.trim().is_empty() || path.len() > MAX_PATH || path.contains('\0') {
        return Err("open: `path` must be a file's path".into());
    }
    let label = text(v, "label", what, MAX_TITLE)?.filter(|l| !l.trim().is_empty());
    let read_only = match v.get("readOnly") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err("open: `readOnly` must be true or false".into()),
    };
    Ok(OpenRequest { path, label, read_only })
}

impl Dialog {
    /// The commands the dialog runs: OK's and its buttons'.
    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.command.as_deref().into_iter().chain(self.buttons.iter().map(|b| b.command.as_str()))
    }
}

/// `params` (an object) with `extra`'s keys set.
pub fn with(params: &Json, extra: impl IntoIterator<Item = (&'static str, Json)>) -> Json {
    let mut out = params.as_object().cloned().unwrap_or_default();
    for (k, v) in extra {
        out.insert(k.to_string(), v);
    }
    Json::Object(out)
}

//! File › Options (#485): the settings agents can read and change without the window — user
//! name and initials, typing (replace selection, overtype), measurement units, formatting marks
//! and proofing. The Options page in the app sets them through the same command.

use serde_json::{Value, json};

use crate::{CmdError, CmdResult, CommandSpec, Prefs, Session, p};

/// Options panes, in the order the page lists them (the `pane` param of `file.options`).
pub const PANES: [&str; 8] = ["general", "display", "proofing", "save", "language", "accessibility", "advanced", "agents"];

/// Longest initials kept, in characters.
pub const MAX_INITIALS_CHARS: usize = 9;
/// Longest word the custom dictionary takes, in characters.
pub const MAX_WORD_CHARS: usize = 64;
/// Most words one call may add or remove.
const MAX_WORDS_PER_CALL: usize = 1000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("file.options", "Options", "File", options)
            .params(
                r#"{"pane"?: "general|display|proofing|save|language|accessibility|advanced|agents", "author"?: string, "initials"?: string, "units"?: "inches|centimeters|millimeters|points|picas", "typingReplacesSelection"?: bool, "insertKeyOvertype"?: bool, "overtype"?: bool, "marks"?: {"tabs"?: bool, "spaces"?: bool, "paragraphs"?: bool, "hidden"?: bool}, "checkSpelling"?: bool, "markGrammar"?: bool, "ignoreUppercase"?: bool, "ignoreNumbers"?: bool, "ignoreInternet"?: bool, "addWords"?: [string], "removeWords"?: [string]} — with any setting it changes them and opens nothing; without, it opens File › Options (at `pane`). Returns the settings."#,
            )
            .pure(),
        CommandSpec::new("text.overtype", "Overtype", "File › Options › Advanced", |s, v| {
            s.prefs.overtype = p::bool(v, "value").unwrap_or(!s.prefs.overtype);
            Ok(json!({"value": s.prefs.overtype}))
        })
        .params(r#"{"value"?: bool (default: toggle)}"#)
        .pure(),
    ]
}

/// The on/off settings by their `file.options` name.
fn flags() -> [(&'static str, fn(&mut Prefs) -> &mut bool); 7] {
    [
        ("typingReplacesSelection", |p| &mut p.typing_replaces_selection),
        ("insertKeyOvertype", |p| &mut p.insert_key_overtype),
        ("overtype", |p| &mut p.overtype),
        ("markGrammar", |p| &mut p.mark_grammar),
        ("ignoreUppercase", |p| &mut p.ignore_uppercase),
        ("ignoreNumbers", |p| &mut p.ignore_numbers),
        ("ignoreInternet", |p| &mut p.ignore_internet),
    ]
}

fn mark_flags() -> [(&'static str, fn(&mut wordcraft_layout::display::FormattingMarks) -> &mut bool); 4] {
    [("tabs", |m| &mut m.tabs), ("spaces", |m| &mut m.spaces), ("paragraphs", |m| &mut m.paragraphs), ("hidden", |m| &mut m.hidden)]
}

fn bool_param(v: &Value, k: &str) -> Result<Option<bool>, CmdError> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(CmdError::Params(format!("`{k}` must be true or false"))),
    }
}

/// Words for the custom dictionary: trimmed, single words, at most [`MAX_WORD_CHARS`] long.
fn words_param(v: &Value, k: &str) -> Result<Vec<String>, CmdError> {
    let Some(x) = v.get(k).filter(|x| !x.is_null()) else { return Ok(Vec::new()) };
    let bad = || CmdError::Params(format!("`{k}` must be a list of words"));
    let list = x.as_array().ok_or_else(bad)?;
    if list.len() > MAX_WORDS_PER_CALL {
        return Err(CmdError::Params(format!("`{k}` takes at most {MAX_WORDS_PER_CALL} words")));
    }
    let mut out = Vec::new();
    for w in list {
        let w = w.as_str().ok_or_else(bad)?.trim();
        if w.is_empty() || w.chars().count() > MAX_WORD_CHARS || w.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(CmdError::Params(format!("`{k}`: {w:?} isn't a single word of at most {MAX_WORD_CHARS} characters")));
        }
        out.push(w.to_string());
    }
    Ok(out)
}

fn options(s: &mut Session, v: &Value) -> CmdResult {
    let pane = match p::str(v, "pane") {
        Some(name) if PANES.contains(&name) => name,
        Some(name) => return Err(CmdError::Params(format!("unknown pane `{name}`; one of {}", PANES.join(", ")))),
        None => "general",
    };
    // Read every setting first, so a bad one changes nothing.
    let author = match v.get("author") {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) => Some(n.clone()),
        Some(_) => return Err(CmdError::Params("`author` must be a string".into())),
    };
    let initials = match v.get("initials") {
        None | Some(Value::Null) => None,
        Some(Value::String(i)) => Some(i.trim().chars().filter(|c| !c.is_control()).take(MAX_INITIALS_CHARS).collect::<String>()),
        Some(_) => return Err(CmdError::Params("`initials` must be a string".into())),
    };
    let units = match v.get("units") {
        None | Some(Value::Null) => None,
        Some(u) => Some(
            serde_json::from_value::<wordcraft_geom::Unit>(u.clone())
                .map_err(|_| CmdError::Params("`units` is one of inches, centimeters, millimeters, points, picas".into()))?,
        ),
    };
    let mut flag_values = Vec::new();
    for (k, f) in flags() {
        if let Some(b) = bool_param(v, k)? {
            flag_values.push((f, b));
        }
    }
    let mut mark_values = Vec::new();
    match v.get("marks") {
        None | Some(Value::Null) => {}
        Some(m @ Value::Object(_)) => {
            for (k, f) in mark_flags() {
                if let Some(b) = bool_param(m, k)? {
                    mark_values.push((f, b));
                }
            }
        }
        Some(_) => return Err(CmdError::Params("`marks` must be an object of true/false".into())),
    }
    let check_spelling = bool_param(v, "checkSpelling")?;
    let add = words_param(v, "addWords")?;
    let remove = words_param(v, "removeWords")?;

    let setting = author.is_some()
        || initials.is_some()
        || units.is_some()
        || !flag_values.is_empty()
        || !mark_values.is_empty()
        || check_spelling.is_some()
        || !add.is_empty()
        || !remove.is_empty();
    if let Some(name) = author {
        super::file::set_author(s, &json!({"name": name}))?;
    }
    if let Some(i) = initials {
        s.initials = i;
    }
    if let Some(u) = units {
        s.prefs.units = u;
    }
    for (f, b) in flag_values {
        *f(&mut s.prefs) = b;
    }
    for (f, b) in mark_values {
        *f(&mut s.prefs.marks) = b;
    }
    if let Some(b) = check_spelling {
        s.view.proofing = b;
    }
    for w in &add {
        wordcraft_proof::add_word(w);
    }
    for w in &remove {
        wordcraft_proof::remove_word(w);
    }
    if setting {
        // Marks, hidden text and proofing change what is laid out.
        s.relayout();
    } else {
        s.ui_requests.push(json!({"open": "options", "pane": pane}));
    }
    let mut r = current(s);
    if let Some(o) = r.as_object_mut() {
        o.insert("opened".into(), json!(!setting));
    }
    Ok(r)
}

/// The settings `file.options` reads and sets.
pub fn current(s: &Session) -> Value {
    let pr = &s.prefs;
    json!({
        "author": s.author,
        "initials": s.user_initials(),
        "units": pr.units,
        "typingReplacesSelection": pr.typing_replaces_selection,
        "insertKeyOvertype": pr.insert_key_overtype,
        "overtype": pr.overtype,
        "marks": pr.marks,
        "checkSpelling": s.view.proofing,
        "markGrammar": pr.mark_grammar,
        "ignoreUppercase": pr.ignore_uppercase,
        "ignoreNumbers": pr.ignore_numbers,
        "ignoreInternet": pr.ignore_internet,
        "dictionary": wordcraft_proof::user_dictionary(),
    })
}

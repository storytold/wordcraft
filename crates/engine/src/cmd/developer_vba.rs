//! Developer tab: VBA macros. WordCraft doesn't run VBA itself; an add-in that declares
//! `"macros": {"language": "VBA"}` does (protocol 1.1, `macros/*`), and reaches the document back
//! through the host requests answered here ([`host_call`]: `host/sync` on the same objects as
//! Office add-ins, the document's `vbaProject.bin`, messages). Running a macro is one undoable
//! step. WordCraft's own recorded command macros (View › Macros, `tools.macros`) are separate;
//! Record Macro here records the same way and has the add-in write the steps as VBA.
//!
//! Macro security (`prefs.macro_security`): `disable` runs nothing; `prompt` (the default) runs a
//! document's macros once the user enabled them for it (`developer.enableMacros`, the message
//! bar's Enable Content); `enable` runs all.
//!
//! The document's ribbon (`developer.ribbon`, `addRibbonButton`…): tabs, groups and buttons that
//! run its macros, saved with it (`customUI14.xml`) so Word shows them too. A button runs its
//! macro as Word does, with the button as the argument (`Sub Go(control As IRibbonControl)`).

use serde_json::{Value, json};
use wordcraft_doc::ribbon::{MAX_RIBBON_ITEMS, builtin_tab_id, builtin_tab_name};
use wordcraft_doc::{Ribbon, RibbonButton, RibbonGroup, RibbonTab};
use wordcraft_plugin::office::{Action, Permission};
use wordcraft_plugin::{HostError, call_macros, macro_plugins};

use crate::p;
use crate::{CmdError, CmdResult, CommandSpec, Session};

/// Where the commands live.
const LOC: &str = "Developer › Code";
/// Largest `vbaProject.bin` a plugin may write.
const MAX_PROJECT: usize = 64 << 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("developer.macros", "Macros", LOC, macros)
            .params(r#"{} → {"macros": [{"name", "module", "procedure", "plugin", "language"}], "enabled", "security"}: the document's VBA macros"#)
            .pure(),
        CommandSpec::new("developer.runMacro", "Run Macro", LOC, run_macro)
            .params(r#"{"name": "Module1.Hello", "plugin"?: id, "control"?: {"id", "tag"}} → {"ran"}: one undoable step; needs macros enabled for the document"#)
            .pure(),
        CommandSpec::new("developer.modules", "Visual Basic", LOC, modules).params(r#"{"plugin"?: id} → {"modules": [{"name", "kind", "source"}]}"#).pure(),
        CommandSpec::new("developer.saveModule", "Save Module", LOC, save_module)
            .params(r#"{"name": string, "source": string, "plugin"?: id}: adds or replaces a code module (saved with the document as .docm)"#)
            .pure(),
        CommandSpec::new("developer.deleteModule", "Remove Module", LOC, delete_module).params(r#"{"name": string, "plugin"?: id}"#).pure(),
        CommandSpec::new("developer.recordMacro", "Record Macro", LOC, record_macro)
            .params(r#"{"stop"?: bool, "module"?: "Module1"} → {"recording"} or, when stopping, {"module", "source", "steps"}: records what you do as a VBA macro"#)
            .pure(),
        CommandSpec::new("developer.ribbon", "Ribbon Buttons", LOC, ribbon)
            .params(r#"{} → {"tabs": [{"tab", "id", "builtin", "label", "groups": [{"id", "label", "buttons": [{"id", "label", "macro", "image", "large", "screentip", "tag"}]}]}], "editable"}: the document's own ribbon buttons"#)
            .pure(),
        CommandSpec::new("developer.addRibbonButton", "Add Ribbon Button", LOC, add_ribbon_button).params(
            r#"{"macro": "Module1.Go", "label"?, "tab"?: "Macros"|"Home"|"References"…, "group"?: "Macros", "image"?, "large"?: bool, "screentip"?, "tag"?} → {"id"}: a button that runs the macro, saved with the document"#,
        ),
        CommandSpec::new("developer.editRibbonButton", "Edit Ribbon Button", LOC, edit_ribbon_button)
            .params(r#"{"id", "macro"?, "label"?, "image"?, "large"?, "screentip"?, "tag"?} → {"id"}"#),
        CommandSpec::new("developer.removeRibbonButton", "Remove Ribbon Button", LOC, remove_ribbon_button).params(r#"{"id"} → {}"#),
        CommandSpec::new("developer.pressRibbonButton", "Press Ribbon Button", LOC, press_ribbon_button)
            .params(r#"{"id"} → {"ran"}: runs the button's macro, as clicking it does"#)
            .pure(),
        CommandSpec::new("developer.enableMacros", "Enable Content", LOC, enable_macros).params(r#"{"on"?: bool} → {"enabled"}: lets this document's macros run (until it closes)"#).pure(),
        CommandSpec::new("developer.macroSecurity", "Macro Security", LOC, macro_security)
            .params(r#"{"level"?: "disable"|"prompt"|"enable"} → {"level"}: which documents' macros may run"#)
            .pure(),
    ]
}

/// The plugin that runs macros: `plugin` when given, else the first one loaded.
fn macro_plugin(v: &Value) -> Result<(String, String), CmdError> {
    let all = macro_plugins();
    match p::str(v, "plugin") {
        Some(want) => all.into_iter().find(|(pid, _)| pid == want).ok_or_else(|| CmdError::Params(format!("no macro add-in `{want}` is loaded"))),
        None => all.into_iter().next().ok_or_else(|| CmdError::Failed("no add-in runs macros: install one (VBA) in Developer › Add-ins".into())),
    }
}

/// Whether the document's macros may run.
pub fn macros_allowed(s: &Session) -> bool {
    match s.prefs.macro_security.as_str() {
        "enable" => true,
        "disable" => false,
        _ => s.macros_enabled,
    }
}

/// Answers a plugin's request to the host (protocol 1.1) for the open document.
pub fn host_call(s: &mut Session, plugin: &str, method: &str, params: &Value) -> Result<Value, HostError> {
    let fail = |message: String| HostError { code: -32000, message, data: Value::Null };
    match method {
        "host/sync" => {
            let actions: Vec<Action> = serde_json::from_value(params.get("actions").cloned().unwrap_or(Value::Array(Vec::new())))
                .map_err(|e| fail(format!("`actions`: {e}")))?;
            if actions.len() > wordcraft_plugin::office::MAX_ACTIONS {
                return Err(fail("too many actions".into()));
            }
            // A plugin is a program the user installed: the document is open to it. A failed sync
            // changes nothing; what a good one changes is part of the running command's step.
            let before = (s.doc.clone(), s.sel.clone());
            match super::office::run_actions(s, plugin, Permission::ReadWriteDocument, &actions) {
                Ok(results) => Ok(json!({"results": results})),
                Err(error) => {
                    (s.doc, s.sel) = before;
                    s.touch();
                    let message = error.get("message").and_then(Value::as_str).unwrap_or("failed").to_string();
                    Err(HostError { code: -32000, message, data: error })
                }
            }
        }
        "host/vbaProject/get" => {
            let bytes = s.doc.passthrough.get(wordcraft_docx::VBA_PROJECT_PART).filter(|b| !b.is_empty()).map(|b| super::insert::base64_encode(b));
            Ok(json!({"base64": bytes}))
        }
        "host/vbaProject/set" => {
            let b64 = params.get("base64").and_then(Value::as_str).ok_or_else(|| fail("`base64` is required".into()))?;
            if b64.len() > MAX_PROJECT / 3 * 4 + 8 {
                return Err(fail("the project is larger than 64 MiB".into()));
            }
            let bytes = super::insert::base64_decode(b64).ok_or_else(|| fail("invalid base64".into()))?;
            set_project(s, bytes);
            Ok(json!({}))
        }
        "host/message" => {
            let text: String = params.get("text").and_then(Value::as_str).unwrap_or("").chars().take(4000).collect();
            s.status = text.clone();
            s.ui_requests.push(json!({"status": text}));
            // vbOK: the message doesn't wait for the user yet.
            Ok(json!({"result": 1}))
        }
        "host/log" => {
            let text: String = params.get("text").and_then(Value::as_str).unwrap_or("").chars().take(4000).collect();
            log::info!("add-in {plugin}: {text}");
            Ok(json!({}))
        }
        _ => Err(HostError::unknown()),
    }
}

/// Replaces the document's macro project. Signatures over the old one don't hold for the new one.
fn set_project(s: &mut Session, bytes: Vec<u8>) {
    let pass = &mut s.doc.passthrough;
    if let Some(manifest) = pass.get(wordcraft_docx::VBA_RELATED).map(|m| String::from_utf8_lossy(m).into_owned()) {
        let mut kept = String::new();
        for line in manifest.lines() {
            let kind = line.split('\t').next().unwrap_or("");
            if kind.to_ascii_lowercase().contains("signature") {
                if let Some(path) = line.split('\t').nth(1) {
                    pass.remove(path);
                }
            } else {
                kept.push_str(line);
                kept.push('\n');
            }
        }
        pass.insert(wordcraft_docx::VBA_RELATED.to_string(), std::sync::Arc::new(kept.into_bytes()));
    }
    pass.insert(wordcraft_docx::VBA_PROJECT_PART.to_string(), std::sync::Arc::new(bytes));
    s.touch();
}

/// Sends `method` to the macro plugin, answering its host requests, as one undo step `label`.
fn call(s: &mut Session, label: &str, plugin: &str, method: &str, params: Value) -> CmdResult {
    let plugin = plugin.to_string();
    let method = method.to_string();
    s.one_step(label, |s| {
        // The commands its host requests run aren't recorded in a macro or Repeat on their own.
        let recording = s.recording.take();
        let last = s.last_command.clone();
        let r = call_macros(&plugin, &method, params, &mut |m, p| host_call(s, &plugin, m, p));
        s.recording = recording;
        s.last_command = last;
        // The add-in's own words (a macro's `Run-time error '9': Subscript out of range`).
        r.map_err(CmdError::Failed)
    })
}

fn macros(s: &mut Session, v: &Value) -> CmdResult {
    let mut out = Vec::new();
    let plugins: Vec<(String, String)> = match p::str(v, "plugin") {
        Some(_) => vec![macro_plugin(v)?],
        None => macro_plugins(),
    };
    for (plugin, language) in plugins {
        let r = call(s, "Macros", &plugin, "macros/list", json!({}))?;
        for m in r.get("macros").and_then(Value::as_array).into_iter().flatten().take(10_000) {
            let mut m = m.clone();
            m["plugin"] = json!(plugin);
            m["language"] = json!(language);
            out.push(m);
        }
    }
    Ok(json!({"macros": out, "enabled": macros_allowed(s), "security": s.prefs.macro_security}))
}

fn run_macro(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    if name.is_empty() {
        return Err(CmdError::Params("missing `name`".into()));
    }
    if !macros_allowed(s) {
        return Err(CmdError::Disabled("macros are disabled for this document: Enable Content, or change Developer › Macro Security".into()));
    }
    let (plugin, _) = macro_plugin(v)?;
    // Protocol 1.2: the macro sees the document's file, as commands do.
    let mut params = json!({"name": name, "context": {"document": super::plugins::document_context(s)}});
    // A ribbon button's macro gets the button (`control.Id`, `control.Tag`).
    if let Some(c) = v.get("control").filter(|c| c.is_object()) {
        params["control"] = json!({"id": c.get("id").cloned().unwrap_or(json!("")), "tag": c.get("tag").cloned().unwrap_or(json!(""))});
    }
    call(s, "Run Macro", &plugin, "macros/run", params)?;
    Ok(json!({"ran": name}))
}

/// The document's ribbon: its own (editable), or the buttons of a richer one kept from the file.
pub fn current_ribbon(s: &Session) -> (Ribbon, bool) {
    if let Some(data) = wordcraft_docx::custom_ui::kept_part(&s.doc) {
        return (wordcraft_docx::custom_ui::parse_buttons(data), false);
    }
    (s.doc.ribbon.clone().unwrap_or_default(), !wordcraft_docx::custom_ui::has_kept_ribbon(&s.doc))
}

/// The WordCraft tab a ribbon tab shows on.
pub fn tab_name(t: &RibbonTab) -> String {
    match &t.builtin {
        Some(mso) => builtin_tab_name(mso).map(str::to_string).unwrap_or_else(|| mso.trim_start_matches("Tab").to_string()),
        None if !t.label.trim().is_empty() => t.label.clone(),
        None => t.id.clone(),
    }
}

fn ribbon(s: &mut Session, _v: &Value) -> CmdResult {
    let (r, editable) = current_ribbon(s);
    let tabs: Vec<Value> = r
        .tabs
        .iter()
        .map(|t| {
            let groups: Vec<Value> = t
                .groups
                .iter()
                .map(|g| {
                    let buttons: Vec<Value> = g
                        .buttons
                        .iter()
                        .map(|b| {
                            json!({"id": b.id, "label": b.label, "macro": b.on_action, "image": b.image, "large": b.large, "screentip": b.screentip, "tag": b.tag})
                        })
                        .collect();
                    json!({"id": g.id, "label": g.label, "buttons": buttons})
                })
                .collect();
            json!({"tab": tab_name(t), "id": t.id, "builtin": t.builtin, "label": t.label, "groups": groups})
        })
        .collect();
    Ok(json!({"tabs": tabs, "editable": editable}))
}

/// Text for a ribbon attribute: trimmed, at most 1024 characters (RibbonX's limit for labels).
fn text(v: &Value, key: &str) -> Option<String> {
    p::str(v, key).map(|t| t.trim().chars().take(1024).collect())
}

/// The document's editable ribbon, for a change.
fn ribbon_mut(s: &mut Session) -> Result<&mut Ribbon, CmdError> {
    if !current_ribbon(s).1 {
        return Err(CmdError::Disabled(
            "this document's ribbon came from another program with more than buttons: WordCraft runs it but doesn't change it".into(),
        ));
    }
    Ok(s.doc.ribbon.get_or_insert_with(Ribbon::default))
}

/// A macro name a button may run: `Module1.Go`, `Go`, or `Project.Module1.Go`.
fn macro_name(v: &Value) -> Result<Option<String>, CmdError> {
    let Some(m) = text(v, "macro") else { return Ok(None) };
    let ok = !m.is_empty() && m.len() <= 255 && m.chars().all(|c| c.is_alphanumeric() || "_.!'[] -".contains(c));
    if !ok {
        return Err(CmdError::Params("`macro` is a macro's name, like Module1.Hello".into()));
    }
    Ok(Some(m))
}

/// `image`: absent (no change), cleared, or a built-in picture's name.
fn image(v: &Value) -> Result<Option<Option<String>>, CmdError> {
    match v.get("image") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(t)) if t.is_empty() => Ok(Some(None)),
        Some(Value::String(t)) if t.len() <= 64 && t.chars().all(|c| c.is_ascii_alphanumeric()) => Ok(Some(Some(t.clone()))),
        Some(_) => Err(CmdError::Params("`image` is a built-in picture's name, like HappyFace".into())),
    }
}

fn add_ribbon_button(s: &mut Session, v: &Value) -> CmdResult {
    let on_action = macro_name(v)?.ok_or_else(|| CmdError::Params("missing `macro`".into()))?;
    let image = image(v)?.flatten();
    let label = text(v, "label").filter(|l| !l.is_empty()).unwrap_or_else(|| on_action.rsplit(['.', '!']).next().unwrap_or(&on_action).to_string());
    let tab = text(v, "tab").filter(|t| !t.is_empty()).unwrap_or_else(|| "Macros".into());
    let group = text(v, "group").filter(|g| !g.is_empty()).unwrap_or_else(|| "Macros".into());
    let r = ribbon_mut(s)?;
    let builtin = builtin_tab_id(&tab);
    let found = r.tabs.iter().position(|t| match builtin {
        Some(_) => t.builtin.as_deref().and_then(builtin_tab_name).is_some_and(|n| n.eq_ignore_ascii_case(&tab)),
        None => t.builtin.is_none() && tab_name(t).eq_ignore_ascii_case(&tab),
    });
    let ti = match found {
        Some(i) => i,
        None => {
            if r.tabs.len() >= MAX_RIBBON_ITEMS {
                return Err(CmdError::Params("the ribbon has too many tabs".into()));
            }
            let id = if builtin.is_some() { String::new() } else { r.fresh_id("craftTab") };
            r.tabs.push(RibbonTab {
                id,
                builtin: builtin.map(str::to_string),
                label: if builtin.is_some() { String::new() } else { tab.clone() },
                groups: vec![],
            });
            r.tabs.len() - 1
        }
    };
    let group_id = r.fresh_id("craftGroup");
    let button_id = r.fresh_id("craftButton");
    let Some(t) = r.tabs.get_mut(ti) else { return Err(CmdError::Failed("no such tab".into())) };
    let gi = match t.groups.iter().position(|g| g.label.eq_ignore_ascii_case(&group)) {
        Some(i) => i,
        None => {
            if t.groups.len() >= MAX_RIBBON_ITEMS {
                return Err(CmdError::Params("the tab has too many groups".into()));
            }
            t.groups.push(RibbonGroup { id: group_id, label: group, buttons: vec![] });
            t.groups.len() - 1
        }
    };
    let Some(g) = t.groups.get_mut(gi) else { return Err(CmdError::Failed("no such group".into())) };
    if g.buttons.len() >= MAX_RIBBON_ITEMS {
        return Err(CmdError::Params("the group has too many buttons".into()));
    }
    g.buttons.push(RibbonButton {
        id: button_id.clone(),
        label,
        on_action,
        image,
        large: v.get("large").and_then(Value::as_bool).unwrap_or(true),
        screentip: text(v, "screentip").filter(|t| !t.is_empty()),
        supertip: None,
        tag: text(v, "tag").filter(|t| !t.is_empty()),
    });
    Ok(json!({"id": button_id}))
}

fn edit_ribbon_button(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "id")?.to_string();
    let on_action = macro_name(v)?;
    let image = image(v)?;
    let r = ribbon_mut(s)?;
    let b = r.button_mut(&id).ok_or_else(|| CmdError::Params(format!("no ribbon button `{id}`")))?;
    if let Some(m) = on_action {
        b.on_action = m;
    }
    if let Some(l) = text(v, "label").filter(|l| !l.is_empty()) {
        b.label = l;
    }
    if let Some(i) = image {
        b.image = i;
    }
    if let Some(l) = v.get("large").and_then(Value::as_bool) {
        b.large = l;
    }
    if let Some(t) = text(v, "screentip") {
        b.screentip = Some(t).filter(|t| !t.is_empty());
    }
    if let Some(t) = text(v, "tag") {
        b.tag = Some(t).filter(|t| !t.is_empty());
    }
    Ok(json!({"id": id}))
}

fn remove_ribbon_button(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "id")?.to_string();
    let r = ribbon_mut(s)?;
    if !r.remove_button(&id) {
        return Err(CmdError::Params(format!("no ribbon button `{id}`")));
    }
    // A document left without buttons has no ribbon part.
    if r.is_empty() {
        s.doc.ribbon = None;
    }
    Ok(json!({}))
}

fn press_ribbon_button(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "id")?;
    let (r, _) = current_ribbon(s);
    let b = r.button(id).ok_or_else(|| CmdError::Params(format!("no ribbon button `{id}`")))?;
    // A macro named with its file (`'Letter.docm'!Module1.Go`) is this document's.
    let name = b.on_action.rsplit('!').next().unwrap_or(&b.on_action).to_string();
    run_macro(s, &json!({"name": name, "control": {"id": b.id, "tag": b.tag.clone().unwrap_or_default()}}))
}

fn modules(s: &mut Session, v: &Value) -> CmdResult {
    let (plugin, _) = macro_plugin(v)?;
    let mut r = call(s, "Visual Basic", &plugin, "macros/modules", json!({}))?;
    r["plugin"] = json!(plugin);
    Ok(r)
}

fn save_module(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    let source = p::req_str(v, "source")?.to_string();
    if name.is_empty() {
        return Err(CmdError::Params("missing `name`".into()));
    }
    if source.len() > 16 << 20 {
        return Err(CmdError::Params("a module is limited to 16 MiB".into()));
    }
    let (plugin, _) = macro_plugin(v)?;
    call(s, "Save Module", &plugin, "macros/saveModule", json!({"name": name, "source": source}))
}

fn delete_module(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    let (plugin, _) = macro_plugin(v)?;
    call(s, "Remove Module", &plugin, "macros/deleteModule", json!({"name": name}))
}

/// Record Macro: WordCraft records the commands (as View › Macros does); stopping has the add-in
/// write them as VBA at the end of the module (Module1 by default), as Word records it.
fn record_macro(s: &mut Session, v: &Value) -> CmdResult {
    let stop = v.get("stop").and_then(Value::as_bool).unwrap_or(s.recording.is_some());
    if !stop {
        macro_plugin(v)?;
        s.recording = Some((String::new(), Vec::new()));
        return Ok(json!({"recording": true}));
    }
    let Some((_, steps)) = s.recording.take() else { return Ok(json!({"recording": false})) };
    let steps: Vec<Value> = steps
        .into_iter()
        .filter(|(id, _)| !id.starts_with("developer.") && !id.starts_with("tools."))
        .map(|(id, p)| json!({"command": id, "params": p}))
        .collect();
    let (plugin, _) = macro_plugin(v)?;
    let code = call(s, "Record Macro", &plugin, "macros/record", json!({"steps": steps}))?;
    let source = code.get("source").and_then(Value::as_str).unwrap_or("").to_string();
    let module = p::str(v, "module").unwrap_or("Module1").to_string();
    let existing = call(s, "Record Macro", &plugin, "macros/modules", json!({}))?;
    let before = existing
        .get("modules")
        .and_then(Value::as_array)
        .and_then(|m| m.iter().find(|x| x.get("name").and_then(Value::as_str) == Some(module.as_str())))
        .and_then(|m| m.get("source").and_then(Value::as_str))
        .map(|t| format!("{}\r\n\r\n", t.trim_end()))
        .unwrap_or_default();
    call(s, "Record Macro", &plugin, "macros/saveModule", json!({"name": module, "source": format!("{before}{source}")}))?;
    Ok(json!({"recording": false, "module": module, "source": source, "steps": steps.len()}))
}

fn enable_macros(s: &mut Session, v: &Value) -> CmdResult {
    let on = v.get("on").and_then(Value::as_bool).unwrap_or(true);
    s.macros_enabled = on;
    Ok(json!({"enabled": on}))
}

fn macro_security(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(level) = p::str(v, "level") {
        if !matches!(level, "disable" | "prompt" | "enable") {
            return Err(CmdError::Params("`level` is disable, prompt or enable".into()));
        }
        s.prefs.macro_security = level.to_string();
    }
    Ok(json!({"level": s.prefs.macro_security}))
}

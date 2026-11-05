//! Add-ins (Insert › Add-ins, File › Options › Add-ins): load, install and unload out-of-process
//! plugins, turn them off and on, and run their commands (see `wordcraft_plugin`).
//!
//! A plugin command answers with engine commands; [`run`] executes them as one undoable step,
//! and if one fails the document goes back to how it was. While automation is confined to
//! folders (`wordcraft-cli mcp --automation-read-root …`), plugins can't be loaded, installed or
//! turned on: a plugin is a program, and would reach past the roots.
//!
//! Protocol 1.2: a command's answer may also ask for a dialog the app draws or a local file to
//! open. `plugins.run` checks them and returns them in its result (`dialog`, `open`); the app's
//! interface shows or opens them when the user ran the command, a script or an agent gets them
//! to act on. Add-ins that declare `events` hear when documents open, save and close
//! ([`notify`]), without ever holding those up.
//!
//! Manifest 1.3: an add-in may require other add-ins (with version ranges), app versions, the
//! protocol and host features. One whose requirements aren't met is blocked; `plugins.list` says
//! why, `plugins.requirements` shows the tree, `plugins.installMissing` (and `plugins.install`
//! with `withRequirements`) installs what's missing from GitHub, and installing a version outside
//! the range a turned-on add-in needs is refused unless `force`.

use serde_json::{Value, json};

use wordcraft_plugin::install::Fetched;
use wordcraft_plugin::requires::MAX_DEPTH;
use wordcraft_plugin::version::{Range, Version};

use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// Longest selection or paragraph text a plugin command receives, in characters.
const MAX_CONTEXT_CHARS: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    const LOC: &str = "File › Options › Add-ins";
    vec![
        CommandSpec::new("addins.get", "Get Add-ins", "Insert › Add-ins", |_, _| Ok(list()))
            .params("{} → the loaded add-ins: [{id, name, version, status, enabled, commands, ui…}]")
            .pure(),
        CommandSpec::new("plugins.list", "Add-ins", LOC, |_, _| Ok(list()))
            .params("{} → [{id, name, version, description, author, homepage, dir, status, state, blockedBy, requires, requiredBy, enabled, commands, ui}]. `state`: loaded, running, failed, blocked (a requirement isn't met: `blockedBy` [{id?, kind, need, have?, message}]) or disabled")
            .pure(),
        CommandSpec::new("plugins.load", "Load Add-in", LOC, load)
            .params("{path, force?} → the add-in. `path`: a folder holding plugin.json, or the manifest. Replacing an add-in with a version outside the range a turned-on add-in requires fails unless `force` (that add-in is then blocked)")
            .pure(),
        CommandSpec::new("plugins.install", "Install Add-in from GitHub", LOC, install)
            .params(
                "{url (or github), dir?, version?, withRequirements?, force?} → the add-in, with `release`, `missing` (its required and optional add-ins that aren't installed or are out of range) and, with `withRequirements`, `installed` and `failed`. Downloads the plugin in a GitHub repository (the release zip for this platform, else its source; with `version`, a range, the newest release whose tag is in it) into `dir` (default: the app's plugins folder) and loads it. `withRequirements` (default false) also installs the missing required add-ins that name `github`. A version outside the range a turned-on add-in requires fails unless `force`",
            )
            .pure(),
        CommandSpec::new("plugins.installMissing", "Install Missing Add-ins", LOC, install_missing)
            .params("{id, dir?, force?} → {id, installed: [{id, name, version, release}], failed: [{id, error}], missing, satisfied}. Installs the missing or out-of-range required add-ins of add-in `id` that name `github` (the newest release in range), and theirs in turn")
            .pure(),
        CommandSpec::new("plugins.requirements", "Add-in Requirements", LOC, requirements)
            .params("{id} → {id, satisfied, blockedBy, tree: [{id, name, need, have, state, optional, why, github, children}]}. `state` `missing` for an add-in that isn't installed")
            .pure(),
        CommandSpec::new("plugins.unload", "Unload Add-in", LOC, unload)
            .params("{id} → {unloaded, dependentsBlocked}. Until the next start. The add-ins that require it are blocked, and listed")
            .pure(),
        CommandSpec::new("plugins.setEnabled", "Turn Add-in On or Off", LOC, set_enabled)
            .params("{id, enabled} → the add-in, with `dependentsBlocked` (the add-ins that require it, now blocked). Off, it stays listed but its commands, ribbon and menu items are gone; remembered in the preferences")
            .pure(),
        CommandSpec::new("plugins.openHomepage", "Add-in Documentation", LOC, open_homepage)
            .params("{id} → {url}. Opens the add-in's web page (its manifest's `homepage`) in the browser")
            .pure(),
        CommandSpec::new("plugins.run", "Run Add-in Command", "Insert › Add-ins", run)
            .params("{plugin, command, params?} → {ran, message?, result?, open?, dialog?, collab?}. `collab.join` (protocol 2): a live session the add-in asks to start or join, for the caller to pass to `collab.join` with `plugin`. The add-in's commands run as one undoable step. `open` {path, label?, readOnly?}: a file the add-in asks to open; `dialog`: a dialog it asks to show (the app does both when the user ran the command; a script acts on them itself)")
            .pure(),
    ]
}

fn bad(msg: impl Into<String>) -> CmdError {
    CmdError::Params(msg.into())
}

/// `addins.get` and `plugins.list`.
pub fn list() -> Value {
    Value::Array(wordcraft_plugin::list().iter().map(|p| p.info()).collect())
}

/// Plugins are programs: an agent confined to folders can't start new ones.
fn check_unconfined(what: &str) -> Result<(), CmdError> {
    if crate::file_access::current().is_some() {
        return Err(CmdError::Disabled(format!("{what}: add-ins can't be loaded while automation is confined to folders")));
    }
    Ok(())
}

fn load(s: &mut Session, v: &Value) -> CmdResult {
    check_unconfined("plugins.load")?;
    let path = p::str(v, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad("`path` (string) is required"))?;
    let path = std::path::Path::new(path);
    let (manifest, _) = wordcraft_plugin::read_manifest(path).map_err(CmdError::Failed)?;
    check_conflicts(&manifest, p::bool(v, "force").unwrap_or(false))?;
    load_from(s, path)
}

/// Refuses a version outside the range a turned-on add-in requires, unless `force`.
pub fn check_conflicts(manifest: &wordcraft_plugin::Manifest, force: bool) -> Result<(), CmdError> {
    let conflicts = wordcraft_plugin::conflicts(manifest);
    if conflicts.is_empty() || force {
        return Ok(());
    }
    let what: Vec<&str> = conflicts.iter().map(|c| c.message.as_str()).collect();
    Err(CmdError::Failed(format!("{}. Pass `force: true` to install it anyway (those add-ins are then blocked)", what.join("; "))))
}

/// The plugins folder: `dir`, else the app's.
fn plugins_root(v: &Value) -> Result<std::path::PathBuf, CmdError> {
    match p::str(v, "dir").filter(|x| !x.trim().is_empty()) {
        Some(d) => Ok(std::path::PathBuf::from(d)),
        None => wordcraft_plugin::install::install_dir().ok_or_else(|| bad("there's no plugins folder: pass `dir`")),
    }
}

fn install(s: &mut Session, v: &Value) -> CmdResult {
    check_unconfined("plugins.install")?;
    let url = p::str(v, "url")
        .or_else(|| p::str(v, "github"))
        .filter(|x| !x.trim().is_empty())
        .ok_or_else(|| bad("`url` (or `github`, a string) is required"))?;
    let root = plugins_root(v)?;
    let force = p::bool(v, "force").unwrap_or(false);
    let range = match p::str(v, "version") {
        Some(r) => Some(Range::parse(r).map_err(bad)?),
        None => None,
    };
    let fetched = wordcraft_plugin::install::fetch(url, range.as_ref()).map_err(CmdError::Failed)?;
    let mut fetch = |github: &str, range: &Range| wordcraft_plugin::install::fetch(github, Some(range));
    install_fetched(s, fetched, &root, force, p::bool(v, "withRequirements").unwrap_or(false), &mut fetch)
}

/// Downloads a plugin for [`install_fetched`] and [`install_requirements`]: `(github, range)`.
pub type Fetcher<'a> = dyn FnMut(&str, &Range) -> Result<Fetched, String> + 'a;

/// Installs a downloaded plugin into `root` and loads it (`plugins.install`, after the
/// download): refused when its version is outside the range a turned-on add-in requires, unless
/// `force`. With `with_requirements`, its missing required add-ins that name `github` are
/// installed too, through `fetch`.
pub fn install_fetched(
    s: &mut Session,
    fetched: Fetched,
    root: &std::path::Path,
    force: bool,
    with_requirements: bool,
    fetch: &mut Fetcher<'_>,
) -> CmdResult {
    check_conflicts(&fetched.manifest, force)?;
    let dir = fetched.unpack(root).map_err(CmdError::Failed)?;
    let mut info = load_from(s, &dir)?;
    let plugin = fetched.manifest.id.clone();
    info["release"] = json!(fetched.release);
    if with_requirements {
        let (installed, failed) = install_requirements(s, &plugin, root, false, fetch);
        info["installed"] = Value::Array(installed);
        info["failed"] = Value::Array(failed);
        // Its own state may have changed since `load_from`.
        if let Some(p) = wordcraft_plugin::get(&plugin) {
            info["state"] = json!(p.state());
            info["blockedBy"] = json!(p.blocked_by());
            info["status"] = json!(p.status());
        }
    }
    info["missing"] = json!(wordcraft_plugin::missing(&plugin));
    Ok(info)
}

/// Installs `plugin`'s missing or out-of-range required add-ins that name `github` (the newest
/// release in range, through `fetch`), then theirs in turn, at most [`MAX_DEPTH`] deep and 64 in
/// all. Returns what was installed (`{id, name, version, release}`) and what failed (`{id, error}`).
pub fn install_requirements(s: &mut Session, plugin: &str, root: &std::path::Path, force: bool, fetch: &mut Fetcher<'_>) -> (Vec<Value>, Vec<Value>) {
    const MAX_INSTALLS: usize = 64;
    let (mut installed, mut failed) = (Vec::new(), Vec::new());
    let mut seen: Vec<String> = vec![plugin.to_string()];
    let mut queue: std::collections::VecDeque<(String, usize)> = std::collections::VecDeque::from([(plugin.to_string(), 0)]);
    while let Some((pid, depth)) = queue.pop_front() {
        for m in wordcraft_plugin::missing(&pid).into_iter().filter(|m| !m.optional) {
            if seen.contains(&m.id) || installed.len() + failed.len() >= MAX_INSTALLS {
                continue;
            }
            seen.push(m.id.clone());
            let Some(github) = m.github.clone() else { continue };
            let outcome = (|| -> Result<Value, String> {
                let range = Range::parse(&m.version)?;
                let fetched = fetch(&github, &range)?;
                let got = &fetched.manifest;
                if got.id != m.id {
                    return Err(format!("github.com/{github} holds {}, not {}", got.id, m.id));
                }
                let fits = Version::parse(&got.version).map(|v| range.matches(&v)).unwrap_or(range.is_any());
                if !fits {
                    return Err(format!("{} {} from github.com/{github} isn't in {}", m.name, got.version, range));
                }
                check_conflicts(got, force).map_err(|e| e.to_string())?;
                let dir = fetched.unpack(root)?;
                load_from(s, &dir).map_err(|e| e.to_string())?;
                Ok(json!({"id": m.id, "name": got.name, "version": got.version, "release": fetched.release}))
            })();
            match outcome {
                Ok(done) => {
                    installed.push(done);
                    if depth + 1 < MAX_DEPTH {
                        queue.push_back((m.id.clone(), depth + 1));
                    }
                }
                Err(e) => failed.push(json!({"id": m.id, "error": e})),
            }
        }
    }
    (installed, failed)
}

fn install_missing(s: &mut Session, v: &Value) -> CmdResult {
    check_unconfined("plugins.installMissing")?;
    let id = p::req_str(v, "id")?;
    if wordcraft_plugin::get(id).is_none() {
        return Err(bad(format!("no add-in `{id}` is loaded")));
    }
    let root = plugins_root(v)?;
    let mut fetch = |github: &str, range: &Range| wordcraft_plugin::install::fetch(github, Some(range));
    Ok(install_missing_with(s, id, &root, p::bool(v, "force").unwrap_or(false), &mut fetch))
}

/// `plugins.installMissing` with a given downloader (tests use one without a network).
pub fn install_missing_with(s: &mut Session, id: &str, root: &std::path::Path, force: bool, fetch: &mut Fetcher<'_>) -> Value {
    let (installed, failed) = install_requirements(s, id, root, force, fetch);
    json!({
        "id": id,
        "installed": installed,
        "failed": failed,
        "missing": wordcraft_plugin::missing(id),
        "satisfied": wordcraft_plugin::get(id).is_some_and(|p| !p.is_blocked()),
    })
}

fn requirements(_: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "id")?;
    wordcraft_plugin::requirements(id).map_err(bad)
}

/// The plugins in `ids` that are blocked now.
fn now_blocked(ids: &[String]) -> Vec<String> {
    ids.iter().filter(|d| wordcraft_plugin::get(d).is_some_and(|p| p.is_blocked())).cloned().collect()
}

/// Loads the plugin in `path` (a folder or its manifest); one the preferences list as off stays off.
pub fn load_from(s: &mut Session, path: &std::path::Path) -> CmdResult {
    let plugin = wordcraft_plugin::load(path).map_err(CmdError::Failed)?;
    if s.prefs.disabled_plugins.contains(&plugin.manifest.id) {
        let _ = wordcraft_plugin::set_enabled(&plugin.manifest.id, false);
    }
    Ok(plugin.info())
}

fn unload(_: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "id")?;
    let dependents = wordcraft_plugin::dependents(id);
    if !wordcraft_plugin::unload(id) {
        return Err(bad(format!("no add-in `{id}` is loaded")));
    }
    Ok(json!({"unloaded": id, "dependentsBlocked": now_blocked(&dependents)}))
}

fn set_enabled(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "id")?;
    let on = p::bool(v, "enabled").ok_or_else(|| bad("`enabled` (true or false) is required"))?;
    if on {
        check_unconfined("plugins.setEnabled")?;
    }
    let plugin = wordcraft_plugin::get(id).ok_or_else(|| bad(format!("no add-in `{id}` is loaded")))?;
    let dependents = if on { Vec::new() } else { wordcraft_plugin::dependents(id) };
    wordcraft_plugin::set_enabled(id, on).map_err(CmdError::Failed)?;
    s.prefs.disabled_plugins.retain(|x| x != id);
    if !on {
        s.prefs.disabled_plugins.push(id.to_string());
    }
    let mut info = plugin.info();
    info["dependentsBlocked"] = json!(now_blocked(&dependents));
    Ok(info)
}

fn open_homepage(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "id")?;
    let plugin = wordcraft_plugin::get(id).ok_or_else(|| bad(format!("no add-in `{id}` is loaded")))?;
    let url = plugin.manifest.homepage.trim();
    if url.is_empty() {
        return Err(CmdError::Failed(format!("add-in {id} has no web page")));
    }
    s.ui_requests.push(json!({"openUrl": url}));
    Ok(json!({"url": url}))
}

/// Turns off the loaded plugins the preferences list as off (at startup, after loading them).
pub fn apply_prefs(prefs: &crate::Prefs) {
    for id in &prefs.disabled_plugins {
        if wordcraft_plugin::get(id).is_some()
            && let Err(e) = wordcraft_plugin::set_enabled(id, false)
        {
            log::warn!("add-in {id}: {e}");
        }
    }
    // Say which add-ins can't start, and why.
    for (id, why) in wordcraft_plugin::refresh().blocked {
        log::warn!("add-in {id} is blocked: {}", why.iter().map(|b| b.message.as_str()).collect::<Vec<_>>().join("; "));
    }
}

/// At most `max` characters of `text`.
fn capped(text: &str, max: usize) -> (String, bool) {
    match text.char_indices().nth(max) {
        Some((i, _)) => (text.get(..i).unwrap_or("").to_string(), true),
        None => (text.to_string(), false),
    }
}

/// The document as a plugin sees it (`context.document`): its file (`path`, `null` for a new
/// document never saved) and `name`, whether it has unsaved changes (`dirty`), its title and
/// word count, and its `id` (protocol 2.2: the same in the document events).
pub fn document_context(s: &Session) -> Value {
    let name = s.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string());
    json!({
        "name": name,
        "path": s.path.as_ref().map(|p| p.display().to_string()),
        "dirty": s.dirty,
        "title": s.doc.core.title,
        "words": s.word_count(),
        "id": s.plugin_document_id(),
    })
}

/// Tells the add-ins that declared `event` about it (protocol 1.2), without waiting for them:
/// `path` is the document's file and `id` the document's ([`Session::plugin_document_id`],
/// protocol 2.2). A save's event carries its `format`, the file's extension.
pub fn notify(event: wordcraft_plugin::DocumentEvent, path: &std::path::Path, id: &str) {
    if wordcraft_plugin::events::listeners(event).is_empty() {
        return;
    }
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut params = json!({"path": path.display().to_string(), "name": name, "id": id});
    if event == wordcraft_plugin::DocumentEvent::Saved {
        params["format"] = json!(path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default());
    }
    wordcraft_plugin::send_event(event, params);
}

/// Checks a file an add-in asks to open: an absolute path to a regular file WordCraft opens.
fn check_open(path: &str) -> Result<(), CmdError> {
    let fail = |why: &str| Err(CmdError::Failed(format!("the add-in asked to open {path}, which {why}")));
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return fail("isn't an absolute path");
    }
    let lower = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !crate::io::OPEN_EXTS.iter().any(|ext| lower.strip_suffix(ext).is_some_and(|stem| stem.len() > 1 && stem.ends_with('.'))) {
        return fail("isn't a kind of file WordCraft opens");
    }
    match crate::io::is_regular_file(p) {
        Some(true) => Ok(()),
        Some(false) => fail("isn't a file"),
        None => fail("doesn't exist"),
    }
}

/// What a plugin command sees of the document: its name, the selection (its text and ends) and
/// the paragraph the caret is in. Texts are cut at 100,000 characters (`truncated`).
fn context(s: &Session) -> Value {
    let (a, b) = s.sel.ordered();
    let (sel_text, sel_cut) = capped(&s.selected_text(), MAX_CONTEXT_CHARS);
    // Pictures, fields and other objects sit in the text as one placeholder character each.
    let para = s.doc.para_at(&s.sel.focus).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).unwrap_or_default();
    let (para_text, para_cut) = capped(&para, MAX_CONTEXT_CHARS);
    json!({
        "document": document_context(s),
        "selection": {
            "text": sel_text,
            "empty": a == b,
            "truncated": sel_cut,
            "anchor": super::pos_json(&s.sel.anchor),
            "focus": super::pos_json(&s.sel.focus),
            "inTable": s.sel.focus.path.cell().is_some(),
        },
        "paragraph": {"text": para_text, "truncated": para_cut},
    })
}

/// Commands a plugin's reply can't run: managing plugins, and undo history (the reply is one
/// step of it).
fn refused(id: &str) -> bool {
    id.starts_with("plugins.")
        || id.starts_with("office.")
        || matches!(id, "edit.undo" | "edit.undoTo" | "edit.redo" | "edit.repeat" | "tools.macros" | "tools.recordMacro")
        || matches!(
            id,
            "developer.macros"
                | "developer.runMacro"
                | "developer.modules"
                | "developer.saveModule"
                | "developer.deleteModule"
                | "developer.recordMacro"
        )
}

/// `plugins.run`: one undo step, made only when the add-in changed something, so a command that
/// only answers (a dialog's step, a message) neither marks the document as changed nor adds a
/// step. Repeat and a recorded macro repeat the add-in command when it changed something.
fn run(s: &mut Session, v: &Value) -> CmdResult {
    let rev = s.rev();
    let r = s.one_step("Run Add-in Command", |s| run_step(s, v));
    if r.is_ok() && s.rev() != rev {
        s.last_command = Some(("plugins.run".to_string(), v.clone()));
        if let Some((_, steps)) = s.recording.as_mut() {
            steps.push(("plugins.run".to_string(), v.clone()));
        }
    }
    r
}

fn run_step(s: &mut Session, v: &Value) -> CmdResult {
    let plugin = p::req_str(v, "plugin")?;
    let command = p::req_str(v, "command")?;
    let params = v.get("params").cloned().filter(|x| !x.is_null()).unwrap_or_else(|| json!({}));
    // The add-in may read and write the document while it works (`host/sync`, protocol 1.1): that
    // and its answer's commands are this command's one step.
    let ctx = context(s);
    let reply = {
        let recording = s.recording.take();
        let last = s.last_command.clone();
        let plugin_id = plugin.to_string();
        let r = wordcraft_plugin::run_command_with(plugin, command, params, ctx, &mut |m, p| {
            s.join_next_undo();
            super::developer_vba::host_call(s, &plugin_id, m, p)
        });
        s.recording = recording;
        s.last_command = last;
        r.map_err(CmdError::Failed)?
    };
    // An Office add-in's command opens its task pane or calls its function: nothing runs here.
    if let Some(office) = reply.office {
        let title = wordcraft_plugin::get(plugin).map(|p| p.manifest.name.clone()).unwrap_or_default();
        s.ui_requests.push(match &office {
            wordcraft_plugin::office::OfficeCommand::ShowTaskpane { url, taskpane_id } => {
                json!({"officePane": {"plugin": plugin, "title": title, "url": url, "pane": taskpane_id}})
            }
            wordcraft_plugin::office::OfficeCommand::ExecuteFunction { function, url, control } => {
                json!({"officeFunction": {"plugin": plugin, "url": url, "function": function, "control": control}})
            }
        });
        return Ok(json!({"ran": 0, "office": office}));
    }
    if let Some((id, _)) = reply.commands.iter().find(|(id, _)| refused(id)) {
        return Err(CmdError::Failed(format!("an add-in command can't run `{id}`")));
    }
    if let Some(o) = &reply.open {
        check_open(&o.path)?;
    }
    // `run` makes this command's undo step and restores the document if it fails. The reply's
    // commands join that step, and aren't recorded in a macro or Repeat on their own (this
    // command is).
    let recording = s.recording.take();
    let last = s.last_command.clone();
    let mut failure = None;
    for (id, params) in &reply.commands {
        s.join_next_undo();
        let params = if params.is_null() { json!({}) } else { params.clone() };
        if let Err(e) = s.run(id, &params) {
            failure = Some(format!("{id}: {e}"));
            break;
        }
    }
    s.recording = recording;
    s.last_command = last;
    if let Some(why) = failure {
        return Err(CmdError::Failed(format!("{plugin} › {command} failed, nothing was changed: {why}")));
    }
    let mut out = json!({"ran": reply.commands.len()});
    if let Some(m) = reply.message {
        s.status = m.clone();
        out["message"] = json!(m);
    }
    if !reply.result.is_null() {
        out["result"] = reply.result;
    }
    // Protocol 1.2: shown or opened by the app after this step, or acted on by the caller.
    if let Some(o) = reply.open {
        out["open"] = serde_json::to_value(o).map_err(|e| CmdError::Failed(e.to_string()))?;
    }
    if let Some(d) = reply.dialog {
        out["dialog"] = serde_json::to_value(d).map_err(|e| CmdError::Failed(e.to_string()))?;
    }
    // Protocol 2: a room to join or start, for the caller (the interface runs `collab.join`).
    if let Some(mut j) = reply.collab_join {
        // The invitation a transport's `share` gives (`result.link`) travels with the room.
        if j.get("invitation").is_none()
            && let Some(link) = out.get("result").and_then(|r| r.get("link")).filter(|l| l.is_string()).cloned()
            && let Some(o) = j.as_object_mut()
        {
            o.insert("invitation".into(), link);
        }
        out["collab"] = json!({"join": j});
    }
    Ok(out)
}

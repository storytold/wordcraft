//! WordCraft's plugin host.
//!
//! A plugin is a separate program, written in any language, described by a `plugin.json`
//! manifest. WordCraft starts it on first use and talks JSON-RPC 2.0 with it over its standard
//! input and output, one message per line. A plugin contributes **commands**, run through
//! `plugins.run`: the plugin answers with engine commands, which the engine executes as one
//! undoable step. Its manifest can put those commands on the ribbon and in the right-click menu.
//!
//! The manifest and the protocol are the ones the other craft apps speak (GridCraft's plugins
//! add spreadsheet functions too), so one plugin can serve several apps: `apps` says which,
//! `uiFor` gives each app its own ribbon and menus, and `initialize` tells the plugin which app
//! started it. A plugin runs in its own process, so a crash, a hang or a bad answer from it fails
//! the command, never the document. The protocol is documented in `docs/plugins.md`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod install;
mod process;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

pub use process::{Connection, RequestError};

/// The protocol version this host speaks (sent in `initialize`).
pub const PROTOCOL_VERSION: u32 = 1;
/// This app's id in a manifest's `apps` and `uiFor`, and in `initialize`'s `host.id`.
pub const HOST_ID: &str = "wordcraft";
/// This app's name in `initialize`'s `host.name`.
pub const HOST_NAME: &str = "WordCraft";
/// The manifest's file name inside a plugin folder.
pub const MANIFEST: &str = "plugin.json";
/// Environment variable listing extra plugin folders (the platform's path-list separator).
pub const PATH_ENV: &str = "WORDCRAFT_PLUGIN_PATH";

const MAX_MANIFEST_BYTES: u64 = 1 << 20;
const MAX_DECLARATIONS: usize = 1_000;
/// Commands run on request, so they may take a while (a web lookup, a long document).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const INIT_TIMEOUT: Duration = Duration::from_secs(10);
/// A plugin that crashes or times out this many times is not started again until reloaded.
const MAX_FAILURES: u32 = 3;

// ---------------------------------------------------------------- manifest

/// `plugin.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Unique id, e.g. `org.example.citations` (letters, digits, `.`, `-`, `_`).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// The plugin's web page (documentation, support): an `https://` or `http://` URL.
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub author: String,
    /// The apps the plugin is made for (`wordcraft`, `gridcraft`, …); empty: any.
    #[serde(default)]
    pub apps: Vec<String>,
    pub run: RunSpec,
    /// Spreadsheet functions (GridCraft's). WordCraft lists them and doesn't call them.
    #[serde(default)]
    pub functions: Vec<Json>,
    #[serde(default)]
    pub commands: Vec<CommandDecl>,
    /// Ribbon groups and menu items, as written: read with [`Manifest::ui`].
    #[serde(default, rename = "ui")]
    pub ui_json: Json,
    /// Per-app ribbon groups and menu items (`{"wordcraft": {…}, "gridcraft": {…}}`): this app's
    /// entry, when there is one, is used instead of `ui`.
    #[serde(default)]
    pub ui_for: BTreeMap<String, Json>,
    /// The `ui` this app uses, read from `uiFor.wordcraft` or `ui` by [`Manifest::parse`].
    #[serde(skip)]
    ui: UiDecl,
}

/// How to start the plugin. A `command` containing a path separator is relative to the plugin
/// folder; a bare name is looked up on `PATH`. The plugin runs in its folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDecl {
    /// Unique within the plugin.
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    /// Params documentation, like the engine's commands.
    #[serde(default)]
    pub params: String,
}

/// Where the plugin's commands appear in the app.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDecl {
    /// Groups of buttons on a ribbon tab: a built-in tab (`Home`, `Review`, …) or a new one.
    #[serde(default)]
    pub ribbon: Vec<RibbonGroupDecl>,
    /// Items added to the right-click menu.
    #[serde(default)]
    pub menus: Vec<MenuItemDecl>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RibbonGroupDecl {
    /// A built-in tab's name (English, as in [`RIBBON_TABS`]) adds the group at the end of that
    /// tab; any other name makes a new tab after the built-in ones.
    pub tab: String,
    pub group: String,
    pub buttons: Vec<ButtonDecl>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ButtonDecl {
    /// One of the plugin's `commands[].id`.
    pub command: String,
    /// Default: the command's label.
    #[serde(default)]
    pub label: Option<String>,
    /// Default: the command's description.
    #[serde(default)]
    pub tooltip: Option<String>,
    /// An icon name from the app's icon set (see `docs/plugins.md`); unknown names get a default.
    #[serde(default)]
    pub icon: Option<String>,
    /// `large` (default) or `small` (stacked three high).
    #[serde(default)]
    pub size: ButtonSize,
    /// Passed to the command.
    #[serde(default)]
    pub params: Json,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ButtonSize {
    #[default]
    Large,
    Small,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuItemDecl {
    /// When the item shows: `text` (always, the default), `table` (in a table), `picture` (a
    /// picture, shape or text box selected).
    #[serde(default)]
    pub menu: MenuKind,
    pub command: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub params: Json,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MenuKind {
    #[default]
    Text,
    Table,
    Picture,
}

/// The built-in ribbon tabs a plugin can add groups to.
pub const RIBBON_TABS: &[&str] = &["Home", "Insert", "Draw", "Design", "Layout", "References", "Mailings", "Review", "View", "Developer", "Help"];
/// Longest tab, group and label text.
const MAX_LABEL: usize = 64;

fn valid_label(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= MAX_LABEL
}

fn valid_id(s: &str) -> bool {
    // Starts with a letter or digit: an id is also the folder an installed plugin goes in.
    s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.len() <= 128
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

impl Manifest {
    /// Parses and validates a manifest.
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let mut m: Manifest = serde_json::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        if !m.for_this_app() {
            let apps = m.apps.join(", ");
            return Err(format!("{MANIFEST}: plugin {} is made for {apps}, not WordCraft", m.id));
        }
        // Another app's `ui` (GridCraft's cell menus…) means nothing here: only this app's is read.
        let own = m.ui_for.get(HOST_ID).unwrap_or(&m.ui_json);
        m.ui = if own.is_null() { UiDecl::default() } else { serde_json::from_value(own.clone()).map_err(|e| format!("{MANIFEST}: ui: {e}"))? };
        m.validate()?;
        Ok(m)
    }

    /// Whether `apps` names this app (or nothing).
    pub fn for_this_app(&self) -> bool {
        self.apps.is_empty() || self.apps.iter().any(|a| a.eq_ignore_ascii_case(HOST_ID))
    }

    /// The ribbon groups and menu items this app shows.
    pub fn ui(&self) -> &UiDecl {
        &self.ui
    }

    fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err(format!("{MANIFEST}: id `{}` must be 1–128 letters, digits, `.`, `-` or `_`, starting with a letter or digit", self.id));
        }
        if self.name.trim().is_empty() {
            return Err(format!("{MANIFEST}: `name` is empty"));
        }
        let homepage = self.homepage.trim();
        if !homepage.is_empty()
            && (!(homepage.starts_with("https://") || homepage.starts_with("http://"))
                || homepage.len() > 2048
                || homepage.contains(char::is_whitespace))
        {
            return Err(format!("{MANIFEST}: `homepage` must be an http(s) URL"));
        }
        if self.run.command.trim().is_empty() {
            return Err(format!("{MANIFEST}: `run.command` is empty"));
        }
        if self.functions.len() > MAX_DECLARATIONS || self.commands.len() > MAX_DECLARATIONS || self.apps.len() > 64 {
            return Err(format!("{MANIFEST}: at most {MAX_DECLARATIONS} functions and {MAX_DECLARATIONS} commands"));
        }
        let mut seen = std::collections::HashSet::new();
        for c in &self.commands {
            if !valid_id(&c.id) {
                return Err(format!("{MANIFEST}: command id `{}` must be letters, digits, `.`, `-` or `_`", c.id));
            }
            if !seen.insert(c.id.as_str()) {
                return Err(format!("{MANIFEST}: command `{}` is declared twice", c.id));
            }
        }
        let ui = &self.ui;
        let buttons: usize = ui.ribbon.iter().map(|g| g.buttons.len()).sum();
        if ui.ribbon.len() > MAX_DECLARATIONS || buttons > MAX_DECLARATIONS || ui.menus.len() > MAX_DECLARATIONS {
            return Err(format!("{MANIFEST}: at most {MAX_DECLARATIONS} ribbon groups, buttons and menu items"));
        }
        let known = |id: &str, what: &str| {
            if seen.contains(id) { Ok(()) } else { Err(format!("{MANIFEST}: {what} runs `{id}`, which isn't one of the plugin's commands")) }
        };
        for g in &ui.ribbon {
            if !valid_label(&g.tab) || !valid_label(&g.group) {
                return Err(format!("{MANIFEST}: ribbon `tab` and `group` need 1–{MAX_LABEL} characters"));
            }
            if g.buttons.is_empty() {
                return Err(format!("{MANIFEST}: ribbon group `{}` has no buttons", g.group));
            }
            for b in &g.buttons {
                known(&b.command, &format!("a button in ribbon group `{}`", g.group))?;
                if b.label.as_deref().is_some_and(|l| !valid_label(l)) {
                    return Err(format!("{MANIFEST}: button labels need 1–{MAX_LABEL} characters"));
                }
            }
        }
        for item in &ui.menus {
            known(&item.command, "a menu item")?;
            if item.label.as_deref().is_some_and(|l| !valid_label(l)) {
                return Err(format!("{MANIFEST}: menu item labels need 1–{MAX_LABEL} characters"));
            }
        }
        Ok(())
    }

    /// The command a button or menu item runs.
    pub fn command(&self, id: &str) -> Option<&CommandDecl> {
        self.commands.iter().find(|c| c.id == id)
    }
}

// ---------------------------------------------------------------- plugins

enum State {
    /// Not started yet, or stopped after a failure (started again on the next call).
    Idle,
    Running(Connection),
    /// Failed too often; not started again until the plugin is reloaded.
    Failed(String),
}

/// A loaded plugin.
pub struct Plugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
    state: Mutex<(State, u32)>,
    /// Off: listed, but its commands and ribbon and menu items are gone.
    enabled: AtomicBool,
    /// Where it was installed from (`.craft-source.json`), for plugins installed from GitHub.
    pub source: Option<Json>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Plugin {
    fn new(manifest: Manifest, dir: PathBuf) -> Plugin {
        Plugin { manifest, state: Mutex::new((State::Idle, 0)), enabled: AtomicBool::new(true), source: install::read_source(&dir), dir }
    }

    /// Sends a request and waits for its result, starting the plugin when needed.
    fn request(&self, method: &str, params: Json, timeout: Duration) -> Result<Json, String> {
        let mut guard = lock(&self.state);
        // Checked under the lock: turning off stores the flag, then stops the process under it.
        if !self.enabled() {
            return Err(format!("plugin {} is turned off", self.manifest.id));
        }
        let (state, failures) = &mut *guard;
        if let State::Failed(why) = state {
            return Err(why.clone());
        }
        if !matches!(state, State::Running(_)) {
            let mut conn = Connection::start(&self.manifest, &self.dir)?;
            let init = json!({
                "protocolVersion": PROTOCOL_VERSION,
                "host": {"id": HOST_ID, "name": HOST_NAME, "version": env!("CARGO_PKG_VERSION")},
                "plugin": {"id": self.manifest.id, "dir": self.dir.display().to_string()},
            });
            if let Err(e) = conn.request("initialize", init, INIT_TIMEOUT) {
                *failures += 1;
                *state =
                    if *failures >= MAX_FAILURES { State::Failed(format!("stopped after {failures} failures: {}", e.message)) } else { State::Idle };
                return Err(e.message);
            }
            *state = State::Running(conn);
        }
        let State::Running(conn) = state else { return Err("plugin not running".into()) };
        match conn.request(method, params, timeout) {
            Ok(v) => Ok(v),
            Err(e) if e.fatal => {
                // The connection is gone or out of step: stop the process and start afresh next time.
                *failures += 1;
                log::warn!("plugin {}: {} (failure {failures} of {MAX_FAILURES})", self.manifest.id, e.message);
                *state =
                    if *failures >= MAX_FAILURES { State::Failed(format!("stopped after {failures} failures: {}", e.message)) } else { State::Idle };
                Err(e.message)
            }
            Err(e) => Err(e.message),
        }
    }

    /// False once turned off with [`set_enabled`].
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// `off`, `ready` (not started), `running`, or `failed: …`.
    pub fn status(&self) -> String {
        if !self.enabled() {
            return "off".into();
        }
        match &lock(&self.state).0 {
            State::Idle => "ready".into(),
            State::Running(_) => "running".into(),
            State::Failed(why) => format!("failed: {why}"),
        }
    }

    /// The plugin as `plugins.list` reports it.
    pub fn info(&self) -> Json {
        let m = &self.manifest;
        json!({
            "id": m.id,
            "name": m.name,
            "version": m.version,
            "description": m.description,
            "homepage": m.homepage,
            "author": m.author,
            "apps": m.apps,
            "enabled": self.enabled(),
            "dir": self.dir.display().to_string(),
            "source": self.source.clone().unwrap_or(Json::Null),
            "status": self.status(),
            "commands": m.commands.iter().map(|c| json!({
                "id": c.id, "label": c.label, "description": c.description, "params": c.params,
            })).collect::<Vec<_>>(),
            "ui": m.ui(),
        })
    }

    fn stop(&self) {
        let mut guard = lock(&self.state);
        if let State::Running(conn) = std::mem::replace(&mut guard.0, State::Idle) {
            conn.shutdown();
        }
    }
}

// ---------------------------------------------------------------- the registry

/// Plugins are application-wide, like Word's add-ins: one set per process.
fn loaded() -> &'static Mutex<Vec<Arc<Plugin>>> {
    static LOADED: OnceLock<Mutex<Vec<Arc<Plugin>>>> = OnceLock::new();
    LOADED.get_or_init(|| Mutex::new(Vec::new()))
}

/// The plugin with this id.
pub fn get(id: &str) -> Option<Arc<Plugin>> {
    lock(loaded()).iter().find(|p| p.manifest.id == id).cloned()
}

/// Every loaded plugin, in load order.
pub fn list() -> Vec<Arc<Plugin>> {
    lock(loaded()).clone()
}

/// Loads the plugin in `path` (a folder holding `plugin.json`, or the manifest itself). A plugin
/// with the same id is unloaded first. The process starts on first use.
pub fn load(path: &Path) -> Result<Arc<Plugin>, String> {
    let (dir, file) = if path.is_dir() {
        (path.to_path_buf(), path.join(MANIFEST))
    } else {
        (path.parent().unwrap_or(Path::new(".")).to_path_buf(), path.to_path_buf())
    };
    let len = std::fs::metadata(&file).map_err(|e| format!("{}: {e}", file.display()))?.len();
    if len > MAX_MANIFEST_BYTES {
        return Err(format!("{}: manifest larger than 1 MiB", file.display()));
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let manifest = Manifest::parse(&text)?;
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    Ok(add(manifest, dir))
}

/// Adds a parsed manifest, replacing a plugin with the same id (tests and [`load`]).
pub fn add(manifest: Manifest, dir: PathBuf) -> Arc<Plugin> {
    unload(&manifest.id);
    let plugin = Arc::new(Plugin::new(manifest, dir));
    lock(loaded()).push(plugin.clone());
    log::info!("plugin {} {} loaded from {}", plugin.manifest.id, plugin.manifest.version, plugin.dir.display());
    plugin
}

/// Turns a loaded plugin off or back on. Off, it stays listed, but its commands are refused, its
/// ribbon and menu items are gone and its process is stopped. Returns whether anything changed.
pub fn set_enabled(id: &str, on: bool) -> Result<bool, String> {
    let plugin = get(id).ok_or_else(|| format!("no plugin `{id}` is loaded"))?;
    if plugin.enabled() == on {
        return Ok(false);
    }
    if on {
        // A plugin that failed too often gets a fresh start.
        *lock(&plugin.state) = (State::Idle, 0);
        plugin.enabled.store(true, Ordering::Relaxed);
    } else {
        plugin.enabled.store(false, Ordering::Relaxed);
        plugin.stop();
    }
    log::info!("plugin {id} turned {}", if on { "on" } else { "off" });
    Ok(true)
}

/// Unloads a plugin and asks its process to exit. True when it was loaded.
pub fn unload(id: &str) -> bool {
    let plugin = {
        let mut all = lock(loaded());
        let Some(i) = all.iter().position(|p| p.manifest.id == id) else { return false };
        all.remove(i)
    };
    plugin.stop();
    true
}

/// Plugin folders to load at startup: each folder in [`PATH_ENV`], then `extra`. A folder holding
/// `plugin.json` is one plugin; otherwise each of its subfolders holding one is.
pub fn discover(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = std::env::var_os(PATH_ENV).map(|v| std::env::split_paths(&v).collect()).unwrap_or_default();
    roots.extend(extra.iter().cloned());
    let mut out = Vec::new();
    for root in roots {
        if root.join(MANIFEST).is_file() {
            out.push(root);
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join(MANIFEST).is_file()).collect();
        dirs.sort();
        out.extend(dirs);
    }
    out
}

/// Loads every plugin [`discover`] finds; failures (a plugin made only for other apps among
/// them) are returned and logged, not fatal.
pub fn autoload(extra: &[PathBuf]) -> Vec<(PathBuf, Result<String, String>)> {
    discover(extra)
        .into_iter()
        .map(|dir| {
            let r = load(&dir).map(|p| p.manifest.id.clone());
            if let Err(e) = &r {
                log::warn!("plugin {}: {e}", dir.display());
            }
            (dir, r)
        })
        .collect()
}

// ---------------------------------------------------------------- commands

/// What a plugin command asks the engine to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandReply {
    /// Engine commands to run, in order: `(id, params)`.
    pub commands: Vec<(String, Json)>,
    /// A message to show the user.
    pub message: Option<String>,
    /// Anything else the plugin returned (handed back to the caller).
    pub result: Json,
}

/// Most engine commands one plugin command may run.
pub const MAX_REPLY_COMMANDS: usize = 10_000;

/// Runs one of a plugin's commands. `context` describes the document (selection, paragraph).
pub fn run_command(plugin_id: &str, command: &str, params: Json, context: Json) -> Result<CommandReply, String> {
    let plugin = get(plugin_id).ok_or_else(|| format!("no plugin `{plugin_id}` is loaded"))?;
    if !plugin.enabled() {
        return Err(format!("plugin {plugin_id} is turned off (File › Options › Add-ins)"));
    }
    if !plugin.manifest.commands.iter().any(|c| c.id == command) {
        return Err(format!("plugin {plugin_id} has no command `{command}`"));
    }
    let r = plugin.request("commands/run", json!({"command": command, "params": params, "context": context}), COMMAND_TIMEOUT)?;
    parse_reply(r)
}

fn parse_reply(r: Json) -> Result<CommandReply, String> {
    let mut reply = CommandReply {
        message: r.get("message").and_then(Json::as_str).map(str::to_string),
        result: r.get("result").cloned().unwrap_or(Json::Null),
        ..Default::default()
    };
    if let Some(list) = r.get("commands") {
        let list = list.as_array().ok_or("`commands` must be an array")?;
        if list.len() > MAX_REPLY_COMMANDS {
            return Err(format!("a plugin command may run at most {MAX_REPLY_COMMANDS} commands"));
        }
        for c in list {
            let id = c.get("command").and_then(Json::as_str).ok_or("each entry of `commands` needs a `command` id")?;
            reply.commands.push((id.to_string(), c.get("params").cloned().unwrap_or(Json::Null)));
        }
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;

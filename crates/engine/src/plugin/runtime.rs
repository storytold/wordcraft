//! The session's plug-ins.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use craft_plugin::{Fetch, Manifest, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::Pending;

/// An installed plug-in.
#[derive(Clone, Debug)]
pub struct InstalledPlugin {
    pub manifest: Manifest,
    /// The SHA-256 of its bundle (lower-case hex).
    pub sha256: String,
    /// Where it came from.
    pub source: Source,
    /// Kept in the user's `Plug-ins` folder; otherwise for this session only (the CLI, MCP).
    pub stored: bool,
    pub(crate) wasm: Arc<Vec<u8>>,
}

impl InstalledPlugin {
    /// What `plugin.list` says about it.
    pub fn describe(&self) -> Value {
        let mut v = serde_json::to_value(&self.manifest).unwrap_or_else(|_| json!({"id": self.manifest.id}));
        v["kind"] = json!(self.manifest.kind);
        v["sha256"] = json!(self.sha256);
        v["source"] = serde_json::to_value(&self.source).unwrap_or(Value::Null);
        v["stored"] = json!(self.stored);
        v
    }
}

/// Each plug-in's own settings and the catalogs added: `<settings folder>/plugin-settings.json`
/// beside the `Plug-ins` folder, when the session keeps plug-ins there.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginPrefs {
    /// Each plug-in's settings, by plug-in id (`plugin::MAX_SETTINGS_BYTES` each).
    pub settings: BTreeMap<String, Map<String, Value>>,
    /// Plug-in catalogs added with `plugin.catalog.add` (`index.json` URLs).
    pub catalogs: Vec<String>,
}

/// The settings file's name in the settings folder.
#[cfg(not(target_arch = "wasm32"))]
pub const PREFS_FILE: &str = "plugin-settings.json";
/// Largest settings file read: every plug-in's settings at their limit would be far more, but a
/// file this size is already a damaged one.
#[cfg(not(target_arch = "wasm32"))]
const MAX_PREFS_BYTES: u64 = 8 << 20;

/// The session's plug-ins: the installed ones, where they are kept, their settings and catalogs.
/// The desktop app opens the store ([`Runtime::open_store`]) and sets the HTTPS client
/// ([`Runtime::set_fetch`]); the CLI and MCP keep plug-ins for the session only.
#[derive(Default)]
pub struct Runtime {
    /// The user's `Plug-ins` folder.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) store: Option<craft_plugin::store::Store>,
    /// Where [`PluginPrefs`] are kept (with the store).
    #[cfg(not(target_arch = "wasm32"))]
    prefs_file: Option<std::path::PathBuf>,
    plugins: Vec<InstalledPlugin>,
    /// Plug-in folders that couldn't be read, and plug-ins this version can't run.
    problems: Vec<String>,
    /// HTTPS for catalogs and GitHub releases. Without one, only files and `file://` catalogs
    /// install.
    pub(crate) fetch: Mutex<Option<Box<dyn Fetch + Send>>>,
    pub(crate) pending: Option<Pending>,
    /// The plug-ins' settings and catalogs.
    pub prefs: PluginPrefs,
    /// The last run (plug-in id, command id, params).
    pub last: Option<(String, String, Value)>,
}

impl Runtime {
    /// Keep plug-ins in `<settings_dir>/Plug-ins` (and their settings in [`PREFS_FILE`] beside
    /// it), and read the ones installed.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_store(&mut self, settings_dir: &std::path::Path) {
        let store = craft_plugin::store::Store::new(craft_plugin::store::plugins_dir(settings_dir));
        let listing = store.list_for(&super::APP);
        self.plugins.retain(|p| !p.stored);
        self.problems = listing.problems.into_iter().map(|p| format!("{}: {}", p.dir.display(), p.error)).collect();
        // The store keeps whatever was installed; only plug-ins for this WordCraft are offered
        self.problems.extend(listing.not_runnable.into_iter().map(|(i, why)| format!("{}: {why}", i.manifest.name)));
        for (i, _) in listing.runnable {
            let plugin = InstalledPlugin {
                manifest: i.manifest,
                sha256: i.record.sha256,
                source: i.record.source,
                stored: true,
                wasm: Arc::new(i.bundle.wasm),
            };
            self.keep(plugin);
        }
        self.store = Some(store);
        let file = settings_dir.join(PREFS_FILE);
        if file.exists() {
            match crate::io::read_regular_file_within(&file, MAX_PREFS_BYTES).map(|bytes| serde_json::from_slice::<PluginPrefs>(&bytes)) {
                Some(Ok(prefs)) => self.prefs = prefs,
                Some(Err(e)) => self.problems.push(format!("{}: {e}", file.display())),
                None => self.problems.push(format!("{}: not a readable settings file", file.display())),
            }
        }
        self.prefs.catalogs.truncate(super::MAX_CATALOGS);
        self.prefs_file = Some(file);
    }

    /// Use `fetch` for catalogs and GitHub releases (the desktop app's own HTTPS client).
    pub fn set_fetch(&mut self, fetch: Option<Box<dyn Fetch + Send>>) {
        *self.fetch.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner) = fetch;
    }

    /// Write [`Runtime::prefs`] to the settings folder (when the store is open). A failure is
    /// logged: the settings stay in effect for the session.
    pub fn save_prefs(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(file) = &self.prefs_file {
            let written = serde_json::to_vec_pretty(&self.prefs).map_err(|e| e.to_string()).and_then(|bytes| write_atomic(file, &bytes));
            if let Err(e) = written {
                log::warn!("plug-in settings: {e}");
            }
        }
    }

    /// The installed plug-ins.
    pub fn list(&self) -> &[InstalledPlugin] {
        &self.plugins
    }

    /// Why plug-in folders couldn't be used.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Plug-in `id` (ignoring case, as the store does).
    pub fn get(&self, id: &str) -> Option<&InstalledPlugin> {
        self.plugins.iter().find(|p| p.manifest.id.eq_ignore_ascii_case(id))
    }

    /// Whether plug-ins are kept in the user's folder (else for this session only).
    pub fn has_store(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.store.is_some();
        #[cfg(target_arch = "wasm32")]
        false
    }

    /// Put `plugin` in the list, in place of one with the same id.
    pub(crate) fn keep(&mut self, plugin: InstalledPlugin) -> InstalledPlugin {
        match self.plugins.iter_mut().find(|p| p.manifest.id.eq_ignore_ascii_case(&plugin.manifest.id)) {
            Some(slot) => *slot = plugin.clone(),
            None => self.plugins.push(plugin.clone()),
        }
        plugin
    }

    /// Drop plug-in `id` from the list, with its settings.
    pub(crate) fn forget(&mut self, id: &str) {
        self.plugins.retain(|p| !p.manifest.id.eq_ignore_ascii_case(id));
        self.prefs.settings.retain(|k, _| !k.eq_ignore_ascii_case(id));
        if self.last.as_ref().is_some_and(|(last, ..)| last.eq_ignore_ascii_case(id)) {
            self.last = None;
        }
    }
}

/// Write `bytes` next to `path` and rename it into place.
#[cfg(not(target_arch = "wasm32"))]
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let part = path.with_extension("json.part");
    std::fs::write(&part, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    std::fs::rename(&part, path).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("could not write {}: {e}", path.display())
    })
}

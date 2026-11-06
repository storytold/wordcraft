//! Custom keyboard shortcuts (Tools › Customize Keyboard, `tools.customizeKeyboard`): keys the
//! user assigned to commands and built-in keys the user removed. The user's map is consulted
//! before the registry's built-in `shortcut`s, so an assignment overrides a built-in key and a
//! removed built-in key does nothing.
//!
//! Keys are written like the registry's shortcuts (`Mod+Shift+K`, `Alt+F7`, `F2`; `Mod` is Ctrl,
//! or ⌘ on macOS) and compared in one canonical form ([`normalize`]). The map is saved with the
//! front end's preferences; reading a saved map never fails: entries that aren't a valid key or
//! command are dropped.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CommandSpec, Registry};

/// At most this many saved entries are read (a hand-edited settings file can't balloon memory).
const MAX_ENTRIES: usize = 2000;
/// Longest key text accepted.
const MAX_KEY_LEN: usize = 48;

/// Named (non-character) keys, in their canonical spelling.
const NAMED: &[&str] =
    &["Left", "Right", "Up", "Down", "Home", "End", "PageUp", "PageDown", "Enter", "Tab", "Backspace", "Delete", "Escape", "Space", "Insert"];

/// Custom keyboard shortcuts. `assigned` maps a key to the command it runs; `removed` holds
/// built-in keys that were taken away (removed, or reassigned to another command).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KeyMap {
    pub assigned: BTreeMap<String, String>,
    pub removed: BTreeSet<String>,
    /// A localized built-in key removed only for that interface locale. Unrelated English
    /// bindings are unaffected (pt-BR Ctrl+N is Bold; English Ctrl+N is New).
    #[serde(rename = "removedLocalized", skip_serializing_if = "BTreeMap::is_empty")]
    pub removed_localized: BTreeMap<String, BTreeSet<String>>,
}

/// Word for Windows Brazilian Portuguese defaults that differ from the English layout.
/// Used by both the UI and the locale-aware keyboard customization command.
pub const PT_BR_WORD_KEYS: &[(&str, &str)] = &[
    ("Mod+N", "format.bold"),
    ("Mod+S", "format.underline"),
    ("Mod+B", "file.save"),
    ("Mod+O", "file.new"),
    ("Mod+A", "file.open"),
    ("Mod+T", "select.all"),
    ("Mod+Q", "para.alignLeft"),
    ("Mod+L", "edit.find"),
];

/// A key in canonical form: modifiers in the order `Mod+Ctrl+Alt+Shift+`, then the key (a single
/// character upper-cased, a named key, or `F1`–`F24`). `None` when it isn't a key.
pub fn normalize(key: &str) -> Option<String> {
    let key = key.trim();
    if key.is_empty() || key.len() > MAX_KEY_LEN {
        return None;
    }
    // `Mod++` is the plus key with Mod.
    let (mods, name) = match key.strip_suffix("++") {
        Some(m) => (m, "+"),
        None => match key.rsplit_once('+') {
            Some((m, n)) => (m, n),
            None => ("", key),
        },
    };
    let (mut cmd, mut ctrl, mut alt, mut shift) = (false, false, false, false);
    for m in mods.split('+').filter(|m| !m.is_empty()) {
        match m.to_ascii_lowercase().as_str() {
            "mod" => cmd = true,
            "ctrl" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            _ => return None,
        }
    }
    let name = canonical_name(name)?;
    let mut out = String::new();
    for (on, m) in [(cmd, "Mod+"), (ctrl, "Ctrl+"), (alt, "Alt+"), (shift, "Shift+")] {
        if on {
            out.push_str(m);
        }
    }
    out.push_str(&name);
    Some(out)
}

fn canonical_name(name: &str) -> Option<String> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return (!c.is_whitespace() && !c.is_control()).then(|| c.to_uppercase().collect());
    }
    if let Some(n) = NAMED.iter().find(|n| n.eq_ignore_ascii_case(name)) {
        return Some((*n).to_string());
    }
    let f = name.strip_prefix('F').or_else(|| name.strip_prefix('f'))?;
    let n: u8 = f.parse().ok()?;
    (1..=24).contains(&n).then(|| format!("F{n}"))
}

/// A command's built-in keys, normalized.
fn builtin_keys(spec: &CommandSpec) -> impl Iterator<Item = String> + '_ {
    spec.shortcut.split(" / ").filter_map(normalize)
}

/// The command a key runs without customization (the first command listing it).
fn builtin<'a>(reg: &'a Registry, key: &str) -> Option<&'a CommandSpec> {
    reg.all().iter().find(|s| !s.shortcut.is_empty() && builtin_keys(s).any(|k| k == key))
}

impl KeyMap {
    /// Read a saved map, keeping only well-formed entries (never fails).
    pub fn from_value(v: &Value) -> KeyMap {
        let mut map = KeyMap::default();
        if let Some(a) = v.get("assigned").and_then(Value::as_object) {
            for (k, id) in a.iter().take(MAX_ENTRIES) {
                if let (Some(k), Some(id)) = (normalize(k), id.as_str().filter(|id| !id.is_empty() && id.len() <= 128)) {
                    map.assigned.insert(k, id.to_string());
                }
            }
        }
        if let Some(r) = v.get("removed").and_then(Value::as_array) {
            map.removed.extend(r.iter().take(MAX_ENTRIES).filter_map(Value::as_str).filter_map(normalize));
        }
        if let Some(keys) = v.get("removedLocalized").and_then(Value::as_object)
            && let Some(br) = keys.get("pt-br").and_then(Value::as_array)
        {
            let removed: BTreeSet<_> = br
                .iter()
                .take(MAX_ENTRIES)
                .filter_map(Value::as_str)
                .filter_map(normalize)
                .filter(|key| PT_BR_WORD_KEYS.iter().any(|(k, _)| *k == key))
                .collect();
            if !removed.is_empty() {
                map.removed_localized.insert("pt-br".to_string(), removed);
            }
        }
        map
    }

    /// Whether nothing is customized.
    pub fn is_empty(&self) -> bool {
        self.assigned.is_empty() && self.removed.is_empty() && self.removed_localized.values().all(BTreeSet::is_empty)
    }

    /// Drop entries for commands that don't exist and removals of keys that aren't built in.
    pub fn retain_known(&mut self, reg: &Registry) {
        self.assigned.retain(|_, id| reg.get(id).is_some());
        self.removed.retain(|k| builtin(reg, k).is_some());
        self.removed_localized.retain(|locale, keys| {
            if locale != "pt-br" { return false; }
            keys.retain(|key| PT_BR_WORD_KEYS.iter().any(|(k, id)| *k == key && reg.get(id).is_some()));
            !keys.is_empty()
        });
    }

    /// The command a key runs: the user's assignment, else the built-in one unless removed.
    pub fn resolve<'a>(&self, reg: &'a Registry, key: &str) -> Option<&'a CommandSpec> {
        let key = normalize(key)?;
        if let Some(spec) = self.assigned.get(&key).and_then(|id| reg.get(id)) {
            return Some(spec);
        }
        if self.removed.contains(&key) {
            return None;
        }
        builtin(reg, &key)
    }

    /// The keys that run a command now: its built-in keys still in effect, then the user's.
    pub fn keys_for(&self, reg: &Registry, id: &str) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        if let Some(spec) = reg.get(id) {
            for k in builtin_keys(spec) {
                // Uncustomized (the usual case, every frame for tooltips): no registry scan.
                if !keys.contains(&k) && (self.is_empty() || self.resolve(reg, &k).is_some_and(|s| s.id == id)) {
                    keys.push(k);
                }
            }
        }
        for (k, owner) in &self.assigned {
            if owner == id && !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        keys
    }

    /// Assign `key` to command `id`; the key leaves whatever command had it. Returns that command.
    pub fn assign(&mut self, reg: &Registry, id: &str, key: &str) -> Result<Option<String>, String> {
        if reg.get(id).is_none() {
            return Err(format!("unknown command `{id}`"));
        }
        let key = normalize(key).ok_or_else(|| format!("`{key}` isn't a key (e.g. Mod+Shift+K, Alt+F7, F2)"))?;
        let previous = self.resolve(reg, &key).map(|s| s.id.to_string()).filter(|p| p != id);
        match builtin(reg, &key) {
            // Back to its own built-in key: no custom entry needed.
            Some(b) if b.id == id => {
                self.assigned.remove(&key);
                self.removed.remove(&key);
            }
            // Taken from a built-in command: it loses the key for good.
            Some(_) => {
                self.assigned.insert(key.clone(), id.to_string());
                self.removed.insert(key);
            }
            None => {
                self.assigned.insert(key, id.to_string());
            }
        }
        Ok(previous)
    }

    /// Take `key` away from command `id`. Returns whether it had the key.
    pub fn remove(&mut self, reg: &Registry, id: &str, key: &str) -> Result<bool, String> {
        let key = normalize(key).ok_or_else(|| format!("`{key}` isn't a key"))?;
        if self.resolve(reg, &key).is_none_or(|s| s.id != id) {
            return Ok(false);
        }
        self.assigned.remove(&key);
        if builtin(reg, &key).is_some() {
            self.removed.insert(key);
        }
        Ok(true)
    }

    /// Remove a locale-only built-in without disabling the English command on the same key.
    /// For unlocalized keys, continue to use the standard global removal path.
    pub fn remove_for_locale(&mut self, reg: &Registry, locale: &str, id: &str, key: &str) -> Result<bool, String> {
        let key = normalize(key).ok_or_else(|| format!("`{key}` isn't a key"))?;
        let Some((_, owner)) = (if locale == "pt-br" { PT_BR_WORD_KEYS } else { &[] })
            .iter()
            .find(|(k, _)| *k == key)
        else {
            return self.remove(reg, id, &key);
        };
        // A custom assignment may own this key instead of the locale's built-in command.
        if let Some(assigned) = self.assigned.get(&key) {
            if assigned != id { return Ok(false); }
            self.assigned.remove(&key);
            // Removing another custom command must not also remove the
            // localized built-in that happens to use the same key.
            if *owner != id { return Ok(true); }
        } else if *owner != id || self.removed.contains(&key) {
            return Ok(false);
        }
        let removed = self.removed_localized.entry("pt-br".into()).or_default().insert(key);
        Ok(removed)
    }

    /// Back to the built-in keys only.
    pub fn reset(&mut self) {
        *self = KeyMap::default();
    }
}

impl<'de> Deserialize<'de> for KeyMap {
    /// Lenient: any JSON reads (junk entries dropped), so a damaged map never loses the other
    /// preferences saved next to it.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(KeyMap::from_value(&Value::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn keys_normalize_to_one_spelling() {
        assert_eq!(normalize("mod+shift+alt+v").as_deref(), Some("Mod+Alt+Shift+V"));
        assert_eq!(normalize("Mod+=").as_deref(), Some("Mod+="));
        assert_eq!(normalize("Mod++").as_deref(), Some("Mod++"));
        assert_eq!(normalize("f12").as_deref(), Some("F12"));
        assert_eq!(normalize("Ctrl+pagedown").as_deref(), Some("Ctrl+PageDown"));
        for junk in ["", "Mod+", "Hyper+K", "F99", "Mod+Shift+NotAKey", "Mod+\u{7}", &"x".repeat(100)] {
            assert_eq!(normalize(junk), None, "{junk:?}");
        }
    }

    /// #368: an assignment overrides a built-in key, a removed built-in key runs nothing, and
    /// Reset All brings the built-ins back.
    #[test]
    fn user_keys_override_and_remove_built_ins() {
        let reg = crate::cmd::registry();
        let mut m = KeyMap::default();
        assert_eq!(m.resolve(&reg, "Mod+B").map(|s| s.id), Some("format.bold"));
        // Mod+B to Italic: Bold loses it.
        assert_eq!(m.assign(&reg, "format.italic", "mod+b").unwrap().as_deref(), Some("format.bold"));
        assert_eq!(m.resolve(&reg, "Mod+B").map(|s| s.id), Some("format.italic"));
        assert!(m.keys_for(&reg, "format.bold").is_empty());
        assert_eq!(m.keys_for(&reg, "format.italic"), ["Mod+I", "Mod+B"]);
        // Removing it from Italic leaves Mod+B doing nothing (Bold doesn't get it back).
        assert!(m.remove(&reg, "format.italic", "Mod+B").unwrap());
        assert!(m.resolve(&reg, "Mod+B").is_none());
        // Removing a built-in key disables it; a new key works.
        assert!(m.remove(&reg, "format.underline", "Mod+U").unwrap());
        assert!(m.resolve(&reg, "Mod+U").is_none());
        assert_eq!(m.assign(&reg, "format.underline", "Mod+Alt+Shift+U").unwrap(), None);
        assert_eq!(m.resolve(&reg, "Mod+Shift+Alt+U").map(|s| s.id), Some("format.underline"));
        assert!(m.assign(&reg, "no.such.command", "Mod+Q").is_err());
        assert!(m.assign(&reg, "format.bold", "Hyper+Q").is_err());
        m.reset();
        assert_eq!(m.resolve(&reg, "Mod+B").map(|s| s.id), Some("format.bold"));
        assert_eq!(m.resolve(&reg, "Mod+U").map(|s| s.id), Some("format.underline"));

        // The command does the same for agents, without opening the dialog.
        let mut s = crate::Session::new(wordcraft_doc::Document::new());
        let r = s.run("tools.customizeKeyboard", &json!({"assign": {"command": "format.italic", "key": "Mod+B"}, "key": "Mod+B"})).unwrap();
        assert_eq!((r["replaced"].as_str(), r["assignedTo"].as_str()), (Some("format.bold"), Some("format.italic")));
        assert!(s.ui_requests.is_empty(), "programmatic calls never open dialogs");
        s.run("tools.customizeKeyboard", &json!({})).unwrap();
        assert_eq!(s.ui_requests.last(), Some(&json!({"open": "customizeKeyboard"})));
    }

    #[test]
    fn saved_maps_round_trip_and_junk_is_ignored() {
        let reg = crate::cmd::registry();
        let mut m = KeyMap::default();
        m.assign(&reg, "format.italic", "Mod+B").unwrap();
        let back: KeyMap = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
        let junk = json!({"assigned": {"Hyper+X": "format.bold", "Mod+Q": 7, "Mod+J": "no.such", "Mod+Alt+K": "format.bold"}, "removed": ["Mod+U", 3, "F99", "Mod+Shift+Q"]});
        let mut j: KeyMap = serde_json::from_value(junk).unwrap();
        j.retain_known(&reg);
        assert_eq!(j.assigned.len(), 1, "{j:?}");
        assert_eq!(j.resolve(&reg, "Mod+Alt+K").map(|s| s.id), Some("format.bold"));
        assert_eq!(j.removed.iter().collect::<Vec<_>>(), ["Mod+U"]);
        for v in [json!(null), json!(5), json!("x"), json!({"assigned": [1], "removed": {"a": 1}})] {
            assert!(serde_json::from_value::<KeyMap>(v).unwrap().is_empty());
        }
    }
}

//! Custom keyboard shortcuts (Tools › Customize Keyboard, `tools.customizeKeyboard`). The user's
//! changes are craft-keymap [`Overrides`]: a command's entry is the full list of keys that
//! replaces its registry `shortcut`s, so an assignment that takes a built-in key gives the losing
//! command an entry of its own.
//!
//! The registry writes keys as `Mod+Shift+K`, `Alt+F7` or `F2` (`Mod` is Ctrl, or ⌘ on macOS); the
//! keymap reads that spelling. The overrides are saved with the front end's preferences.

use std::collections::BTreeMap;

use craft_keymap::{Binding, Chord, CommandDef, CommandRef, Keymap, Overrides, Platform, Scope, Scopes};
use serde_json::Value;

use crate::{CommandSpec, Registry};

/// At most this many entries of an old saved map are read (a hand-edited settings file can't
/// balloon memory).
const MAX_ENTRIES: usize = 2000;
/// Longest key text an old saved map may hold.
const MAX_KEY_LEN: usize = 48;
/// Longest command id an old saved map may hold.
const MAX_ID_LEN: usize = 128;

/// A command's built-in keys as the registry writes them.
fn pieces(spec: &CommandSpec) -> impl Iterator<Item = &'static str> {
    spec.shortcut.split(" / ").filter(|piece| !piece.is_empty())
}

/// The keymap of every engine command with its registry keys, under `overrides`. Off macOS,
/// Alt+Q is the interface's command search key, so it never reaches an engine command.
pub(crate) fn registry_keymap(reg: &Registry, overrides: &Overrides) -> Keymap {
    let catalog: Vec<CommandDef> = reg
        .all()
        .iter()
        .map(|spec| {
            pieces(spec)
                .filter_map(|piece| Binding::parse(piece).ok())
                .fold(CommandDef::new(CommandRef::new(spec.id), spec.label, Scope::app()), CommandDef::with_default)
        })
        .collect();
    let platform = Platform::current();
    let reserved: Vec<Chord> = if platform == Platform::Mac { Vec::new() } else { Chord::parse("Alt+Q").ok().into_iter().collect() };
    Keymap::new(catalog, Scopes::new(), platform).with_reserved(reserved).with_overrides(overrides.clone())
}

/// A key of an old saved map: its canonical text and the chord it is on `platform`.
fn legacy_chord(text: &str, platform: Platform) -> Option<(String, Chord)> {
    if text.len() > MAX_KEY_LEN {
        return None;
    }
    let chord = Chord::parse(text.trim()).ok()?;
    Some((chord.portable(), chord.effective(platform)))
}

/// A command's built-in keys: the registry's text and the chord on `platform`.
fn default_chords(spec: &CommandSpec, platform: Platform) -> Vec<(String, Chord)> {
    pieces(spec).filter_map(|piece| Some((piece.to_string(), Chord::parse(piece).ok()?.effective(platform)))).collect()
}

/// The command that has `chord` among its built-in keys (the first in the registry).
fn default_holder(reg: &Registry, chord: Chord, platform: Platform) -> Option<&CommandSpec> {
    reg.all().iter().find(|spec| default_chords(spec, platform).iter().any(|(_, c)| *c == chord))
}

/// The key list being built for `spec`, starting from its built-in keys.
fn entry<'a>(lists: &'a mut BTreeMap<&'static str, Vec<(String, Chord)>>, spec: &CommandSpec, platform: Platform) -> &'a mut Vec<(String, Chord)> {
    lists.entry(spec.id).or_insert_with(|| default_chords(spec, platform))
}

/// The overrides an old saved `{"assigned": {key: command}, "removed": [key]}` map stands for: an
/// assigned key joins its command's keys and leaves the command that had it built in; a removed
/// built-in key leaves its command. Only commands whose keys differ from their built-in ones get
/// an entry. Entries that aren't a key or a known command are dropped, and any JSON value reads.
pub fn overrides_from_legacy(reg: &Registry, saved: &Value) -> Overrides {
    let platform = Platform::current();
    let mut lists: BTreeMap<&'static str, Vec<(String, Chord)>> = BTreeMap::new();
    // Removals first: a key both removed and assigned (taken from a built-in command) ends with
    // the command it was assigned to.
    if let Some(removed) = saved.get("removed").and_then(Value::as_array) {
        for (_, chord) in removed.iter().take(MAX_ENTRIES).filter_map(Value::as_str).filter_map(|t| legacy_chord(t, platform)) {
            if let Some(spec) = default_holder(reg, chord, platform) {
                entry(&mut lists, spec, platform).retain(|(_, c)| *c != chord);
            }
        }
    }
    if let Some(assigned) = saved.get("assigned").and_then(Value::as_object) {
        for (key, id) in assigned.iter().take(MAX_ENTRIES) {
            let Some((text, chord)) = legacy_chord(key, platform) else { continue };
            let Some(spec) = id.as_str().filter(|id| !id.is_empty() && id.len() <= MAX_ID_LEN).and_then(|id| reg.get(id)) else { continue };
            if let Some(holder) = default_holder(reg, chord, platform).filter(|h| h.id != spec.id) {
                entry(&mut lists, holder, platform).retain(|(_, c)| *c != chord);
            }
            let list = entry(&mut lists, spec, platform);
            if !list.iter().any(|(_, c)| *c == chord) {
                list.push((text, chord));
            }
        }
    }
    let mut overrides = Overrides::new();
    for (id, list) in lists {
        let Some(spec) = reg.get(id) else { continue };
        let defaults = default_chords(spec, platform);
        let unchanged = list.len() == defaults.len() && list.iter().zip(&defaults).all(|((_, a), (_, b))| a == b);
        if !unchanged {
            let _ = overrides.insert(CommandRef::new(id).key(), list.into_iter().map(|(text, _)| text).collect());
        }
    }
    overrides
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The keys a command has, through the command.
    fn keys(s: &mut crate::Session, id: &str) -> Vec<String> {
        let r = s.run("tools.customizeKeyboard", &json!({"command": id})).unwrap();
        r["keys"].as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect()
    }

    /// The command a key runs, through the command.
    fn holder(s: &mut crate::Session, key: &str) -> Option<String> {
        let r = s.run("tools.customizeKeyboard", &json!({"key": key})).unwrap();
        r["assignedTo"].as_str().map(str::to_string)
    }

    /// #368: an assignment takes a built-in key from its command, a removed built-in key runs
    /// nothing, and Reset All brings the built-ins back; agents do it without the dialog.
    #[test]
    fn user_keys_override_and_remove_built_ins() {
        let mut s = crate::Session::new(wordcraft_doc::Document::new());
        let bold = keys(&mut s, "format.bold");
        let italic = keys(&mut s, "format.italic");
        let underline = keys(&mut s, "format.underline");
        assert!(!bold.is_empty() && !italic.is_empty() && !underline.is_empty());

        // Mod+B to Italic: Bold loses it.
        let r = s.run("tools.customizeKeyboard", &json!({"assign": {"command": "format.italic", "key": "Mod+B"}, "key": "Mod+B"})).unwrap();
        assert_eq!((r["replaced"].as_str(), r["assignedTo"].as_str()), (Some("format.bold"), Some("format.italic")));
        assert!(s.ui_requests.is_empty(), "programmatic calls never open dialogs");
        assert!(keys(&mut s, "format.bold").is_empty());
        let now = keys(&mut s, "format.italic");
        assert_eq!(now.len(), italic.len() + 1);
        assert!(now.starts_with(&italic), "{now:?}");
        assert!(!s.keymap().is_empty());
        assert_eq!(r["custom"], serde_json::to_value(s.keymap()).unwrap());

        // Removing it from Italic leaves Mod+B doing nothing; Bold doesn't get it back.
        let r = s.run("tools.customizeKeyboard", &json!({"remove": {"command": "format.italic", "key": "Mod+B"}})).unwrap();
        assert_eq!(r["removed"], json!(true));
        assert_eq!(holder(&mut s, "Mod+B"), None);
        assert!(keys(&mut s, "format.bold").is_empty());

        // Removing a built-in key disables it; a new key works.
        let first = underline[0].clone();
        s.run("tools.customizeKeyboard", &json!({"remove": {"command": "format.underline", "key": first}})).unwrap();
        assert_eq!(holder(&mut s, &first), None);
        s.run("tools.customizeKeyboard", &json!({"assign": {"command": "format.underline", "key": "Mod+Alt+Shift+U"}})).unwrap();
        assert_eq!(holder(&mut s, "Mod+Shift+Alt+U").as_deref(), Some("format.underline"));

        // Mistakes are errors.
        assert!(s.run("tools.customizeKeyboard", &json!({"assign": {"command": "no.such.command", "key": "Mod+Q"}})).is_err());
        assert!(s.run("tools.customizeKeyboard", &json!({"assign": {"command": "format.bold", "key": "Hyper+Q"}})).is_err());
        if !cfg!(target_os = "macos") {
            assert!(
                s.run("tools.customizeKeyboard", &json!({"assign": {"command": "format.bold", "key": "Alt+Q"}})).is_err(),
                "Alt+Q is the command search"
            );
        }

        s.run("tools.customizeKeyboard", &json!({"reset": true})).unwrap();
        assert!(s.keymap().is_empty());
        assert_eq!(keys(&mut s, "format.bold"), bold);
        assert_eq!(keys(&mut s, "format.italic"), italic);
        assert_eq!(keys(&mut s, "format.underline"), underline);
        assert!(s.ui_requests.is_empty());

        s.run("tools.customizeKeyboard", &json!({})).unwrap();
        assert_eq!(s.ui_requests.last(), Some(&json!({"open": "customizeKeyboard"})));
    }

    /// The old saved map reads as the overrides it stands for; its junk is dropped.
    #[test]
    fn legacy_saved_maps_import_as_overrides() {
        let reg = crate::cmd::registry();
        let chord = |text: &str| Chord::parse(text).unwrap();
        let saved = json!({
            "assigned": {"Mod+B": "format.italic", "Hyper+X": "format.bold", "Mod+Q": 7, "Mod+J": "no.such", "Mod+I": "format.italic"},
            "removed": ["Mod+U", 3, "F99"]
        });
        let o = overrides_from_legacy(&reg, &saved);
        let mut ids: Vec<&str> = o.iter().map(|(k, _)| k).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["format.bold", "format.italic", "format.underline"], "{o:?}");

        let keymap = registry_keymap(&reg, &o);
        let id = |c: &str| keymap.holder(&chord(c)).map(|c| c.id);
        assert_eq!(id("Mod+B").as_deref(), Some("format.italic"), "the assignment wins");
        assert_eq!(id("Mod+I").as_deref(), Some("format.italic"), "Italic keeps its own key");
        assert_eq!(id("Mod+U"), None, "the removed built-in key runs nothing");
        let platform = keymap.platform();
        let bold = keymap.bindings(&CommandRef::new("format.bold"));
        assert!(!bold.iter().any(|b| b.trigger.chord.effective(platform) == chord("Mod+B").effective(platform)), "{bold:?}");
        assert!(keymap.problems().is_empty(), "{:?}", keymap.problems());

        // An assignment of a command's own built-in key changes nothing.
        assert!(overrides_from_legacy(&reg, &json!({"assigned": {"Mod+I": "format.italic"}})).is_empty());
        for v in [json!(null), json!(5), json!("x"), json!({"assigned": [1], "removed": {"a": 1}})] {
            assert!(overrides_from_legacy(&reg, &v).is_empty(), "{v}");
        }
    }
}

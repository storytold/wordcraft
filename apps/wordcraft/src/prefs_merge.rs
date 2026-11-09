//! Several WordCraft processes share one `ui.json` (opening a file from the file manager starts
//! another). Each reads it at start-up and writes it at exit (and when a change must survive a
//! crash, like taking a document off the recent list), so without a merge the process that writes
//! last writes back what it loaded and loses the others' changes (#3773).
//!
//! The policy: when saving, read `ui.json` again as this version reads it ([`as_read`]: a setting
//! of the wrong type reads as its default, and keys it doesn't know or reads only to migrate are
//! left out), and write it with only what this process changed since it last loaded or saved its
//! preferences (`base`). Every other setting keeps what the file holds now, including the absence
//! of one another process removed (a shortcut, a task pane's place).
//!
//! - A setting both changed takes this process's value.
//! - Groups of settings (editing options, AutoCorrect's switches, keyboard shortcuts, the ribbon,
//!   task pane places) merge entry by entry ([`rule`]). Any other object is one setting: the
//!   window's size and place, or one task pane's place, are never mixed from two processes.
//! - Lists of user entries (AutoCorrect entries by the text they replace, its exceptions, removed
//!   built-in shortcuts, ribbon tabs by name, Quick Access Toolbar commands, caption labels, pens,
//!   custom dictionary words) merge entry by entry too ([`merge_entries`]): entries either process
//!   added are kept, entries either process removed stay removed, and an entry both changed takes
//!   this process's version. A ribbon or toolbar left as it comes counts as its default entries.
//! - Recent lists (documents, symbols, colours, fonts) take the entries this process added or used
//!   again first, then the file's, without duplicates, without the entries this process removed,
//!   and within each list's limit ([`merge_recent`]).
//! - Any other list (the task panes' order, a floating pane's rectangle) is one setting.
//!
//! What this process changed is found by comparing with `base`, not by recording each change: a
//! setting changed and then changed back counts as untouched, so the file's value stays, and a
//! recent entry used again without moving (it was already first) counts as not used.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};
use wordcraft_engine::ribbon::{DEFAULT_QAT, RibbonLayout};
use wordcraft_ui_egui::{MAX_RECENT, MAX_RECENT_FONTS, UiState, colors, dialogs_insert};

/// Whether two entries of a recent list are the same entry.
type Same = fn(&Value, &Value) -> bool;

/// What tells an entry of a list of user entries from the others. Entries with the same key are
/// told apart by their order ([`ids`]).
type Key = fn(&Value) -> String;

/// How a setting this process changed merges with the file's.
#[derive(Clone, Copy)]
enum Rule {
    /// A group of settings: each entry on its own.
    Group,
    /// A recent list, most recent first: how many entries it keeps, and when two are the same.
    Recent(usize, Same),
    /// A list of user entries: each entry on its own.
    Entries(Key),
    /// One setting: this process's value.
    Whole,
}

/// The rule for the setting at `path` (`ui.json`'s keys from the top).
fn rule(path: &[&str]) -> Rule {
    match path {
        [] | ["editing" | "keyboard" | "ribbon" | "panePlaces"] | ["editing", "markup" | "marks" | "autocorrect"] | ["keyboard", "assigned"] => {
            Rule::Group
        }
        ["recent"] => Rule::Recent(MAX_RECENT, same_entry),
        ["recentSymbols"] => Rule::Recent(dialogs_insert::RECENT_SYMBOLS, same_entry),
        ["recentColors"] => Rule::Recent(colors::RECENT_MAX, same_entry),
        ["recentFonts"] => Rule::Recent(MAX_RECENT_FONTS, same_font),
        ["editing", "autocorrect", "entries"] => Rule::Entries(replaced_text),
        ["ribbon", "tabs"] => Rule::Entries(tab_name),
        ["editing", "autocorrect", "removed" | "exceptions" | "exceptionsRemoved" | "initialCapsExceptions"]
        | ["keyboard", "removed"]
        | ["ribbon", "qat"]
        | ["captionLabels" | "inkPens" | "dictionary"] => Rule::Entries(whole_entry),
        _ => Rule::Whole,
    }
}

/// What a setting saved only when it isn't its default stands for when it is missing: the
/// default ribbon tabs and Quick Access Toolbar, so that two processes customizing them from the
/// default merge entry by entry.
fn when_missing(path: &[&str]) -> Option<Value> {
    match path {
        ["ribbon", "tabs"] => serde_json::to_value(RibbonLayout::default().tabs()).ok(),
        ["ribbon", "qat"] => serde_json::to_value(DEFAULT_QAT).ok(),
        _ => None,
    }
}

/// Entries saved alike (colours are saved normalised, `colors::push_recent`).
fn same_entry(a: &Value, b: &Value) -> bool {
    a == b
}

/// Font names ignoring case, as `push_recent_font` compares them.
fn same_font(a: &Value, b: &Value) -> bool {
    match (a.as_str(), b.as_str()) {
        (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
        _ => a == b,
    }
}

/// The entry as saved.
fn whole_entry(v: &Value) -> String {
    v.to_string()
}

/// An AutoCorrect entry `[from, to]` by the text it replaces.
fn replaced_text(v: &Value) -> String {
    v.get(0).unwrap_or(v).to_string()
}

/// A ribbon tab by its name, ignoring case as the ribbon does.
fn tab_name(v: &Value) -> String {
    v.get("name").and_then(Value::as_str).map_or_else(|| v.to_string(), str::to_lowercase)
}

/// `ui.json`'s bytes as this version reads them, saved again: `None` when they aren't
/// preferences at all.
pub fn as_read(bytes: &[u8]) -> Option<Value> {
    serde_json::to_value(UiState::from_json(bytes)?).ok()
}

/// The preferences to write: `ours` (this process's, now), with every value it left as it was in
/// `base` (what it last loaded or saved) taken from `disk` (what `ui.json` holds now, [`as_read`]).
pub fn merge(base: &Value, ours: Value, disk: &Value) -> Value {
    let fallback = ours.clone();
    merge_setting(&mut Vec::new(), Some(base), Some(ours), Some(disk)).unwrap_or(fallback)
}

/// One setting, at `path`. `None` for a setting left out (this process removed it, or it left it
/// alone and another process removed it).
fn merge_setting(path: &mut Vec<String>, base: Option<&Value>, ours: Option<Value>, disk: Option<&Value>) -> Option<Value> {
    if base == ours.as_ref() {
        // Unchanged here: whatever the file holds now.
        return disk.cloned();
    }
    let ours = ours?;
    let keys: Vec<&str> = path.iter().map(String::as_str).collect();
    Some(match (rule(&keys), ours, base, disk) {
        (Rule::Group, Value::Object(ours), Some(Value::Object(base)), Some(Value::Object(disk))) => {
            Value::Object(merge_group(path, base, ours, disk))
        }
        (Rule::Recent(cap, same), Value::Array(ours), Some(Value::Array(base)), Some(Value::Array(disk))) => {
            Value::Array(merge_recent(base, ours, disk, cap, same))
        }
        (Rule::Entries(key), Value::Array(ours), Some(Value::Array(base)), Some(Value::Array(disk))) => {
            Value::Array(merge_entries(base, &ours, disk, key))
        }
        (_, ours, ..) => ours,
    })
}

/// A group of settings, entry by entry: this process's entries in their order, then those only
/// the file or `base` has.
fn merge_group(path: &mut Vec<String>, base: &Map<String, Value>, mut ours: Map<String, Value>, disk: &Map<String, Value>) -> Map<String, Value> {
    let mut names: Vec<String> = ours.keys().cloned().collect();
    let mut seen: HashSet<String> = names.iter().cloned().collect();
    for k in disk.keys().chain(base.keys()) {
        if seen.insert(k.clone()) {
            names.push(k.clone());
        }
    }
    let mut merged = Map::new();
    for k in names {
        path.push(k.clone());
        let keys: Vec<&str> = path.iter().map(String::as_str).collect();
        let missing = when_missing(&keys);
        let base_v = base.get(&k).cloned().or_else(|| missing.clone());
        let disk_v = disk.get(&k).cloned().or_else(|| missing.clone());
        let ours_v = ours.remove(&k).or_else(|| missing.clone());
        let v = merge_setting(path, base_v.as_ref(), ours_v, disk_v.as_ref());
        path.pop();
        // A setting saved only when it isn't its default is left out when it is.
        if let Some(v) = v.filter(|v| Some(v) != missing.as_ref()) {
            merged.insert(k, v);
        }
    }
    merged
}

/// A list entry: its key, and how many entries before it have the same key.
type Id = (String, usize);

/// The entries of `list` told apart by `key`, in order.
fn ids(list: &[Value], key: Key) -> Vec<Id> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    list.iter()
        .map(|v| {
            let k = key(v);
            let n = seen.entry(k.clone()).or_default();
            *n += 1;
            (k, *n - 1)
        })
        .collect()
}

/// A list of user entries, entry by entry. An entry either process added is kept, one either
/// removed stays removed, and one this process changed takes its version (else the file's). The
/// order is this process's if it reordered the entries it loaded, else the file's; an entry only
/// the other list has goes after the entry it follows there.
fn merge_entries(base: &[Value], ours: &[Value], disk: &[Value], key: Key) -> Vec<Value> {
    let (base_ids, ours_ids, disk_ids) = (ids(base, key), ids(ours, key), ids(disk, key));
    let index = |ids: &[Id], list: &[Value]| -> HashMap<Id, Value> { ids.iter().cloned().zip(list.iter().cloned()).collect() };
    let (b, o, d) = (index(&base_ids, base), index(&ours_ids, ours), index(&disk_ids, disk));
    let value = |id: &Id| -> Option<&Value> {
        match (b.get(id), o.get(id), d.get(id)) {
            // Left alone here: the file's, or gone if the other process removed it.
            (Some(b), Some(o), d) if b == o => d,
            // Added or changed here.
            (_, Some(o), _) => Some(o),
            // Removed here.
            (Some(_), None, _) => None,
            // Added there.
            (None, None, d) => d,
        }
    };
    let common = |ids: &[Id], other: &HashMap<Id, Value>| -> Vec<Id> { ids.iter().filter(|id| other.contains_key(*id)).cloned().collect() };
    let reordered = common(&ours_ids, &b) != common(&base_ids, &o);
    let (first, second) = if reordered { (&ours_ids, &disk_ids) } else { (&disk_ids, &ours_ids) };
    let in_first: HashSet<&Id> = first.iter().collect();
    let kept: HashSet<&Id> = first.iter().filter(|id| value(id).is_some()).collect();
    // Entries only `second` has, by the kept entry they follow there (`None`: at the start).
    let mut after: HashMap<Option<&Id>, Vec<&Id>> = HashMap::new();
    let mut anchor = None;
    for id in second {
        if kept.contains(id) {
            anchor = Some(id);
        } else if !in_first.contains(id) && value(id).is_some() {
            after.entry(anchor).or_default().push(id);
        }
    }
    let mut merged = Vec::new();
    let mut emit = |ids: &[&Id]| merged.extend(ids.iter().filter_map(|id| value(id).cloned()));
    emit(after.get(&None).map_or(&[], Vec::as_slice));
    for id in first.iter().filter(|id| kept.contains(id)) {
        emit(&[id]);
        emit(after.get(&Some(id)).map_or(&[], Vec::as_slice));
    }
    merged
}

/// The entries this process put at the front of `ours` ([`touched_len`]), then `disk`'s, leaving
/// out what this process removed (in `base` but no longer in `ours`), without duplicates and at
/// most `cap` entries. Entries are told apart by `same`.
fn merge_recent(base: &[Value], ours: Vec<Value>, disk: &[Value], cap: usize, same: Same) -> Vec<Value> {
    let has = |list: &[Value], v: &Value| list.iter().any(|x| same(x, v));
    let removed = |v: &Value| has(base, v) && !has(&ours, v);
    let touched = &ours[..touched_len(base, &ours, same)];
    let mut merged: Vec<Value> = Vec::new();
    for v in touched.iter().cloned().chain(disk.iter().filter(|v| !removed(v)).cloned()) {
        if merged.len() == cap {
            break;
        }
        if !has(&merged, &v) {
            merged.push(v);
        }
    }
    merged
}

/// How many of `ours`'s first entries this process added or used again. Each use moves an entry
/// to the front of a recent list, so what it left alone is a tail of `ours` in `base`'s order: the
/// longest such tail is untouched, and everything before it (new entries, and entries that moved
/// ahead of one loaded before them) was touched. A use that leaves the order as it was loaded
/// (the first entry used again) can't be told from no use, and merges as none.
fn touched_len(base: &[Value], ours: &[Value], same: Same) -> usize {
    let mut end = base.len();
    for (i, v) in ours.iter().enumerate().rev() {
        match base[..end].iter().rposition(|b| same(b, v)) {
            Some(at) => end = at,
            None => return i + 1,
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wordcraft_ui_egui::UiState;

    use super::{as_read, merge};

    /// The preferences a process holds after loading a `ui.json` with `settings` (every other
    /// setting at its default), as it saves them.
    fn prefs(settings: Value) -> Value {
        as_read(&serde_json::to_vec(&settings).unwrap()).unwrap()
    }

    /// What the next process loads from the merged preferences.
    fn reloaded(merged: &Value) -> UiState {
        UiState::from_json(&serde_json::to_vec(merged).unwrap()).unwrap()
    }

    /// `ui.json` after B and then A, which both loaded `loaded`, save `b` and `a`.
    fn b_then_a(loaded: &Value, b: Value, a: Value) -> Value {
        let after_b = merge(loaded, b, loaded);
        merge(loaded, a, &after_b)
    }

    #[test]
    fn two_processes_keep_each_others_changes() {
        // #3773: A and B both start from the same file.
        let loaded =
            json!({"theme": "light", "author": "Ann", "recent": ["a.docx", "b.docx"], "editing": {"smartQuotes": true, "autoCapitalize": true}});
        // B changes the theme and opens c.docx, then exits first.
        let b = json!({"theme": "dark", "author": "Ann", "recent": ["c.docx", "a.docx", "b.docx"], "editing": {"smartQuotes": false, "autoCapitalize": true}});
        // A changes the author name and opens d.docx, then exits.
        let a = json!({"theme": "light", "author": "Bo", "recent": ["d.docx", "a.docx", "b.docx"], "editing": {"smartQuotes": true, "autoCapitalize": false}});
        assert_eq!(
            b_then_a(&loaded, b, a),
            json!({"theme": "dark", "author": "Bo", "recent": ["d.docx", "c.docx", "a.docx", "b.docx"], "editing": {"smartQuotes": false, "autoCapitalize": false}})
        );
    }

    #[test]
    fn a_process_that_changed_nothing_writes_the_file_back_as_it_is() {
        let loaded = json!({"theme": "light", "recent": ["a.docx"]});
        let disk = json!({"theme": "dark", "recent": ["b.docx", "a.docx"]});
        assert_eq!(merge(&loaded, loaded.clone(), &disk), disk);
    }

    #[test]
    fn this_process_wins_a_setting_both_changed() {
        let loaded = json!({"theme": "light"});
        assert_eq!(merge(&loaded, json!({"theme": "system"}), &json!({"theme": "dark"})), json!({"theme": "system"}));
    }

    #[test]
    fn a_setting_changed_and_changed_back_counts_as_untouched() {
        // Changes are found by comparing with what was loaded: A switched to dark and back.
        let loaded = json!({"theme": "light"});
        assert_eq!(merge(&loaded, json!({"theme": "light"}), &json!({"theme": "dark"})), json!({"theme": "dark"}));
    }

    #[test]
    fn a_recent_entry_removed_here_stays_removed() {
        let loaded = json!({"recent": ["a.docx", "b.docx"]});
        let ours = json!({"recent": ["b.docx"]});
        let disk = json!({"recent": ["c.docx", "a.docx", "b.docx"]});
        assert_eq!(merge(&loaded, ours, &disk), json!({"recent": ["c.docx", "b.docx"]}));
        // Clear Recent clears what the other process had added too only if it was loaded here.
        assert_eq!(merge(&loaded, json!({"recent": []}), &json!({"recent": ["c.docx", "a.docx", "b.docx"]})), json!({"recent": ["c.docx"]}));
    }

    #[test]
    fn a_recent_entry_used_again_here_moves_to_the_front() {
        // Applying a font already in Recently Used Fonts moves it to the front without adding one.
        let loaded = json!({"recentFonts": ["A", "B"]});
        assert_eq!(merge(&loaded, json!({"recentFonts": ["B", "A"]}), &loaded), json!({"recentFonts": ["B", "A"]}));
        // Here C was used again, D opened and A removed; the other process used B again and added E.
        let loaded = json!({"recent": ["a.docx", "b.docx", "c.docx"]});
        let ours = json!({"recent": ["d.docx", "c.docx", "b.docx"]});
        let disk = json!({"recent": ["e.docx", "b.docx", "a.docx", "c.docx"]});
        assert_eq!(merge(&loaded, ours, &disk), json!({"recent": ["d.docx", "c.docx", "e.docx", "b.docx"]}));
        // Entries this process only kept, in the order it loaded them, don't override the file's order.
        let loaded = json!({"recentSymbols": ["x", "y", "z"]});
        let ours = json!({"recentSymbols": ["w", "x", "z"]});
        let disk = json!({"recentSymbols": ["z", "x", "y"]});
        assert_eq!(merge(&loaded, ours, &disk), json!({"recentSymbols": ["w", "z", "x"]}));
    }

    #[test]
    fn recent_fonts_are_one_entry_whatever_their_case() {
        // Recently Used Fonts ignores case (`push_recent_font`): Arial applied here as `arial` and
        // there as `ARIAL` is one entry, and doesn't push a distinct font out of the ten.
        let loaded = json!({"recentFonts": ["A", "B", "C", "D", "E", "F", "G", "H", "I", "Arial"]});
        let ours = json!({"recentFonts": ["arial", "A", "B", "C", "D", "E", "F", "G", "H", "I"]});
        let disk = json!({"recentFonts": ["ARIAL", "A", "B", "C", "D", "E", "F", "G", "H", "I"]});
        assert_eq!(merge(&loaded, ours, &disk), json!({"recentFonts": ["arial", "A", "B", "C", "D", "E", "F", "G", "H", "I"]}));
        // Other lists compare their entries as saved.
        let loaded = json!({"recent": ["a.docx"]});
        assert_eq!(merge(&loaded, json!({"recent": ["A.docx", "a.docx"]}), &loaded), json!({"recent": ["A.docx", "a.docx"]}));
    }

    #[test]
    fn merged_recent_lists_keep_their_limit() {
        // Recently Used Fonts keeps ten.
        let loaded = json!({"recentFonts": ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"]});
        let ours = json!({"recentFonts": ["X", "A", "B", "C", "D", "E", "F", "G", "H", "I"]});
        let disk = json!({"recentFonts": ["Y", "A", "B", "C", "D", "E", "F", "G", "H", "I"]});
        assert_eq!(merge(&loaded, ours, &disk), json!({"recentFonts": ["X", "Y", "A", "B", "C", "D", "E", "F", "G", "H"]}));
    }

    #[test]
    fn a_bad_recent_entry_in_the_file_is_not_carried_forward() {
        // A non-string entry makes the whole list read as empty (`options::lenient` falls back per
        // setting). Merging it in would keep Recent empty in every later session; instead the
        // list reads as this version reads it, and the next save heals it.
        let loaded = prefs(json!({"recent": []}));
        let ours = prefs(json!({"recent": ["x.docx"]}));
        let disk = as_read(br#"{"recent": [{"path": "y.docx"}, "z.docx"]}"#).unwrap();
        let merged = merge(&loaded, ours, &disk);
        assert_eq!(merged["recent"], json!(["x.docx"]));
        assert_eq!(loaded_recent(&merged), ["x.docx"]);
        // Same for recent symbols.
        let ours = prefs(json!({"recentSymbols": [{"ch": "§", "font": "Arial"}]}));
        let disk = as_read(r#"{"recentSymbols": [{"ch": "¶", "font": "Arial"}, 7]}"#.as_bytes()).unwrap();
        assert_eq!(reloaded(&merge(&prefs(json!({})), ours, &disk)).recent_symbols.len(), 1);
    }

    /// The recent documents the next process loads.
    fn loaded_recent(merged: &Value) -> Vec<String> {
        reloaded(merged).recent
    }

    #[test]
    fn only_keys_this_version_writes_are_kept() {
        // A legacy key read once to migrate, and junk, are not written back, even from the file.
        let loaded = prefs(json!({"theme": "light"}));
        let disk = as_read(br#"{"theme": "light", "dark": true, "interfaceSize": 120, "junk": 1}"#).unwrap();
        let merged = merge(&loaded, loaded.clone(), &disk);
        for gone in ["dark", "interfaceSize", "junk"] {
            assert!(merged.get(gone).is_none(), "{gone}");
        }
        assert_eq!(merged["theme"], "light");
    }

    #[test]
    fn a_file_another_process_damaged_gives_way_to_ours() {
        let loaded = json!({"theme": "light", "recent": ["a.docx"]});
        let ours = json!({"theme": "dark", "recent": ["a.docx"]});
        // Not preferences at all: everything comes from this process.
        assert_eq!(as_read(b"\"garbage\""), None);
        assert_eq!(merge(&loaded, ours.clone(), &json!("garbage")), ours);
        // A list replaced by something else keeps ours.
        assert_eq!(merge(&loaded, json!({"recent": ["b.docx", "a.docx"]}), &json!({"recent": 3})), json!({"recent": ["b.docx", "a.docx"]}));
    }

    #[test]
    fn autocorrect_entries_and_exceptions_from_both_processes_are_kept() {
        let loaded = prefs(json!({"editing": {"autocorrect": {"entries": [["teh", "the"]], "exceptions": ["approx."]}}}));
        // B adds an entry and an exception; A adds another entry, changes `teh`'s replacement and
        // removes `approx.`. Each added entry follows the entry it was added after.
        let b = prefs(json!({"editing": {"autocorrect": {"entries": [["teh", "the"], ["teh2", "the"]], "exceptions": ["approx.", "fig."]}}}));
        let a = prefs(json!({"editing": {"autocorrect": {"entries": [["teh", "THE"], ["adn", "and"]], "exceptions": []}}}));
        let ac = reloaded(&b_then_a(&loaded, b, a)).editing.autocorrect;
        let entries: Vec<(&str, &str)> = ac.entries.iter().map(|(f, t)| (f.as_str(), t.as_str())).collect();
        assert_eq!(entries, [("teh", "THE"), ("adn", "and"), ("teh2", "the")]);
        assert_eq!(ac.exceptions, ["fig."]);
    }

    #[test]
    fn keyboard_customisations_merge_key_by_key() {
        let loaded = prefs(json!({"keyboard": {"assigned": {"Mod+Shift+K": "format.bold"}, "removed": []}}));
        // B takes Ctrl+B off Bold and deletes its own Mod+Shift+K; A takes Ctrl+I off Italic and
        // assigns Mod+Shift+J.
        let b = prefs(json!({"keyboard": {"assigned": {}, "removed": ["Mod+B"]}}));
        let a = prefs(json!({"keyboard": {"assigned": {"Mod+Shift+K": "format.bold", "Mod+Shift+J": "format.italic"}, "removed": ["Mod+I"]}}));
        let keys = reloaded(&b_then_a(&loaded, b, a)).keyboard;
        assert_eq!(keys.assigned.into_iter().collect::<Vec<_>>(), [("Mod+Shift+J".to_string(), "format.italic".to_string())]);
        assert_eq!(keys.removed.into_iter().collect::<Vec<_>>(), ["Mod+B", "Mod+I"]);
    }

    #[test]
    fn ribbon_changes_from_both_processes_are_kept() {
        // Both start from the default ribbon. B hides Draw; A adds a custom tab after Home.
        let loaded = prefs(json!({}));
        let b = prefs(json!({"ribbon": {"tabs": hide(default_tabs(), "Draw")}}));
        let mut a_tabs = default_tabs();
        a_tabs.insert(1, json!({"name": "Mine", "custom": true}));
        let a = prefs(json!({"ribbon": {"tabs": a_tabs}}));
        let mut want = hide(default_tabs(), "Draw");
        want.insert(1, json!({"name": "Mine", "custom": true}));
        assert_eq!(serde_json::to_value(loaded_ribbon(&b_then_a(&loaded, b, a)).tabs).unwrap(), Value::Array(want));
    }

    #[test]
    fn a_toolbar_reset_by_one_process_stays_reset() {
        // B resets its custom Quick Access Toolbar to the default; A, which left it alone, adds a
        // custom tab. The toolbar stays the default.
        let loaded = prefs(json!({"ribbon": {"qat": ["file.save", "format.bold"]}}));
        let b = prefs(json!({}));
        let mut a_tabs = default_tabs();
        a_tabs.push(json!({"name": "Mine", "custom": true}));
        let a = prefs(json!({"ribbon": {"tabs": a_tabs, "qat": ["file.save", "format.bold"]}}));
        let merged = b_then_a(&loaded, b, a);
        assert!(merged["ribbon"].get("qat").is_none(), "{}", merged["ribbon"]);
        let ribbon = loaded_ribbon(&merged);
        assert_eq!(ribbon.qat, None);
        assert!(ribbon.tabs.iter().any(|t| t.name == "Mine"));
        // Two processes customizing the default toolbar: both commands, Save removed by A gone.
        let loaded = prefs(json!({}));
        let b = prefs(json!({"ribbon": {"qat": ["file.save", "edit.undo", "edit.redo", "format.bold"]}}));
        let a = prefs(json!({"ribbon": {"qat": ["edit.undo", "edit.redo", "format.italic"]}}));
        assert_eq!(loaded_ribbon(&b_then_a(&loaded, b, a)).qat.unwrap(), ["edit.undo", "edit.redo", "format.italic", "format.bold"]);
    }

    /// The default ribbon tabs, as saved once the ribbon is customized.
    fn default_tabs() -> Vec<Value> {
        serde_json::from_value(serde_json::to_value(wordcraft_engine::RibbonLayout::default().tabs()).unwrap()).unwrap()
    }

    /// `tabs` with the tab `name` hidden.
    fn hide(mut tabs: Vec<Value>, name: &str) -> Vec<Value> {
        for t in &mut tabs {
            if t["name"] == name {
                t["hidden"] = json!(true);
            }
        }
        tabs
    }

    /// The ribbon the next process loads.
    fn loaded_ribbon(merged: &Value) -> wordcraft_engine::RibbonLayout {
        reloaded(merged).ribbon
    }

    #[test]
    fn a_pane_order_changed_by_the_other_process_is_kept() {
        // `paneOrder` is saved only when it isn't the default. B reorders; A leaves it alone.
        let loaded = prefs(json!({}));
        let order = json!(["styles", "navigation"]);
        let b = prefs(json!({"paneOrder": order}));
        assert_eq!(b_then_a(&loaded, b, loaded.clone())["paneOrder"], order);
        // B goes back to the default order; A, which loaded B's order and left it, doesn't bring
        // it back.
        let loaded = prefs(json!({"paneOrder": order}));
        let merged = b_then_a(&loaded, prefs(json!({})), loaded.clone());
        assert!(merged.get("paneOrder").is_none(), "{merged}");
    }

    #[test]
    fn the_window_and_a_pane_place_are_one_setting_each() {
        // A moved its window and B resized its own: the window is A's, not A's place with B's size.
        let window = |x: f32, width: f32| json!({"window": {"position": [x, 10.0], "width": width, "height": 600.0, "maximized": false}});
        let loaded = prefs(window(0.0, 800.0));
        let merged = b_then_a(&loaded, prefs(window(0.0, 1000.0)), prefs(window(300.0, 800.0)));
        assert_eq!(merged["window"], prefs(window(300.0, 800.0))["window"]);
    }

    #[test]
    fn pens_labels_and_dictionary_words_from_both_processes_are_kept() {
        let pen = |color: &str| json!({"kind": "pen", "color": color, "width": 1.0});
        let loaded = prefs(json!({"inkPens": [pen("FF0000"), pen("FF0000")], "captionLabels": ["Plate"], "dictionary": ["foo", "bar"]}));
        // B adds a third red pen, a label and a word; A adds a blue pen, removes `bar` and adds a
        // label of its own. Identical pens stay apart, by their order.
        let b = prefs(
            json!({"inkPens": [pen("FF0000"), pen("FF0000"), pen("FF0000")], "captionLabels": ["Plate", "Chart"], "dictionary": ["foo", "bar", "baz"]}),
        );
        let a = prefs(json!({"inkPens": [pen("FF0000"), pen("FF0000"), pen("0000FF")], "captionLabels": ["Plate", "Map"], "dictionary": ["foo"]}));
        let merged = b_then_a(&loaded, b, a);
        let ui = reloaded(&merged);
        let colors: Vec<String> = ui.ink_pens.iter().map(|p| serde_json::to_value(p).unwrap()["color"].as_str().unwrap().to_string()).collect();
        assert_eq!(colors, ["FF0000", "FF0000", "0000FF", "FF0000"]);
        assert_eq!(ui.caption_labels, ["Plate", "Map", "Chart"]);
        assert_eq!(merged["dictionary"], json!(["foo", "baz"]));
    }
}

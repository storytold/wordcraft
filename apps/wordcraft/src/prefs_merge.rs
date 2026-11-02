//! Several WordCraft processes share one `ui.json` (opening a file from the file manager starts
//! another). Each reads it at start-up and writes it at exit, so without a merge the process that
//! exits last writes back what it loaded and loses the others' changes (#3773).
//!
//! The policy: at exit, re-read `ui.json` and write it with only what this process changed since
//! it loaded its preferences. Every other setting keeps what the file holds now. Recent lists
//! (documents, symbols, colours, fonts) are merged instead: this process's new entries first, then
//! the file's, without duplicates and without the entries this process removed.

use serde_json::Value;
use wordcraft_ui_egui::{MAX_RECENT, MAX_RECENT_FONTS, colors, dialogs_insert};

/// The `ui.json` lists, most recent first, that take entries from both processes, and how many
/// entries each keeps.
const RECENT_LISTS: [(&str, usize); 4] = [
    ("recent", MAX_RECENT),
    ("recentSymbols", dialogs_insert::RECENT_SYMBOLS),
    ("recentColors", colors::RECENT_MAX),
    ("recentFonts", MAX_RECENT_FONTS),
];

/// The preferences to write: `ours` (this process's, now), with every value it left as it was in
/// `base` (what it loaded) taken from `disk` (what `ui.json` holds now). Only the settings `ours`
/// has are written, as before: one this version no longer writes (read once to migrate) is dropped.
pub fn merge(base: &Value, ours: Value, disk: &Value) -> Value {
    match ours {
        Value::Object(ours) => Value::Object(
            ours.into_iter()
                .map(|(k, v)| {
                    let cap = RECENT_LISTS.iter().find(|(name, _)| *name == k).map(|(_, cap)| *cap);
                    let merged = merge_value(base.get(&k), v, disk.get(&k), cap);
                    (k, merged)
                })
                .collect(),
        ),
        ours => ours,
    }
}

/// One setting. `recent_cap` is the limit of the recent list it is, if it is one.
fn merge_value(base: Option<&Value>, ours: Value, disk: Option<&Value>, recent_cap: Option<usize>) -> Value {
    if base == Some(&ours) {
        // Unchanged here: whatever the file holds now, if it has this setting.
        return disk.cloned().unwrap_or(ours);
    }
    match (ours, base, disk, recent_cap) {
        // A group of settings (editing options, keyboard customisations): each entry on its own.
        // Entries the other process added are kept, entries this one removed stay removed.
        (Value::Object(mut ours), Some(Value::Object(base)), Some(Value::Object(disk)), _) => {
            for (k, v) in disk {
                if !base.contains_key(k) && !ours.contains_key(k) {
                    ours.insert(k.clone(), v.clone());
                }
            }
            Value::Object(
                ours.into_iter()
                    .map(|(k, v)| {
                        let merged = merge_value(base.get(&k), v, disk.get(&k), None);
                        (k, merged)
                    })
                    .collect(),
            )
        }
        (Value::Array(ours), Some(Value::Array(base)), Some(Value::Array(disk)), Some(cap)) => Value::Array(merge_recent(base, ours, disk, cap)),
        (ours, ..) => ours,
    }
}

/// `ours`'s entries that `base` didn't have, then `disk`'s, leaving out what this process removed
/// (in `base` but no longer in `ours`), without duplicates and at most `cap` entries.
fn merge_recent(base: &[Value], ours: Vec<Value>, disk: &[Value], cap: usize) -> Vec<Value> {
    let removed = |v: &Value| base.contains(v) && !ours.contains(v);
    let added: Vec<Value> = ours.iter().filter(|v| !base.contains(v)).cloned().collect();
    let mut merged: Vec<Value> = Vec::new();
    for v in added.into_iter().chain(disk.iter().filter(|v| !removed(v)).cloned()) {
        if merged.len() == cap {
            break;
        }
        if !merged.contains(&v) {
            merged.push(v);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::merge;

    #[test]
    fn two_processes_keep_each_others_changes() {
        // #3773: A and B both start from the same file.
        let loaded =
            json!({"theme": "light", "author": "Ann", "recent": ["a.docx", "b.docx"], "editing": {"smartQuotes": true, "autoCapitalize": true}});
        // B changes the theme and opens c.docx, then exits first.
        let b = json!({"theme": "dark", "author": "Ann", "recent": ["c.docx", "a.docx", "b.docx"], "editing": {"smartQuotes": false, "autoCapitalize": true}});
        let after_b = merge(&loaded, b, &loaded);
        // A changes the author name and opens d.docx, then exits.
        let a = json!({"theme": "light", "author": "Bo", "recent": ["d.docx", "a.docx", "b.docx"], "editing": {"smartQuotes": true, "autoCapitalize": false}});
        let after_a = merge(&loaded, a, &after_b);
        assert_eq!(
            after_a,
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
    fn a_recent_entry_removed_here_stays_removed() {
        let loaded = json!({"recent": ["a.docx", "b.docx"]});
        let ours = json!({"recent": ["b.docx"]});
        let disk = json!({"recent": ["c.docx", "a.docx", "b.docx"]});
        assert_eq!(merge(&loaded, ours, &disk), json!({"recent": ["c.docx", "b.docx"]}));
        // Clear Recent clears what the other process had added too only if it was loaded here.
        assert_eq!(merge(&loaded, json!({"recent": []}), &json!({"recent": ["c.docx", "a.docx", "b.docx"]})), json!({"recent": ["c.docx"]}));
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
    fn entries_of_a_group_merge_one_by_one() {
        // B added a shortcut for Bold; A changed Italic's and removed Underline's.
        let loaded = json!({"keyboard": {"format.italic": "Mod+I", "format.underline": "Mod+U"}});
        let disk = json!({"keyboard": {"format.italic": "Mod+I", "format.underline": "Mod+U", "format.bold": "Mod+B"}});
        let ours = json!({"keyboard": {"format.italic": "Mod+Shift+I"}});
        assert_eq!(merge(&loaded, ours, &disk), json!({"keyboard": {"format.italic": "Mod+Shift+I", "format.bold": "Mod+B"}}));
    }

    #[test]
    fn only_keys_this_version_writes_are_kept() {
        // A legacy key read once to migrate, and junk, are not written back.
        let loaded = json!({"theme": "light"});
        let disk = json!({"theme": "light", "dark": true, "junk": 1});
        assert_eq!(merge(&loaded, json!({"theme": "light"}), &disk), json!({"theme": "light"}));
    }

    #[test]
    fn a_file_another_process_damaged_gives_way_to_ours() {
        let loaded = json!({"theme": "light", "recent": ["a.docx"]});
        let ours = json!({"theme": "dark", "recent": ["a.docx"]});
        // Not an object: everything comes from this process.
        assert_eq!(merge(&loaded, ours.clone(), &json!("garbage")), ours);
        // A list replaced by something else keeps ours.
        assert_eq!(merge(&loaded, json!({"recent": ["b.docx", "a.docx"]}), &json!({"recent": 3})), json!({"recent": ["b.docx", "a.docx"]}));
    }
}

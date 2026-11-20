//! Keyboard shortcuts for styles (#3910): Format › Shortcut key… in the style dialogs and the
//! Styles category of Customize Keyboard bind a key to a paragraph or character style, as in
//! Word. The binding lives in the same [`crate::KeyMap`] as command keys (saved with the
//! preferences); pressing the key applies the style through `styles.apply`.

use serde_json::{Value, json};

use wordcraft_doc::Pos;

use crate::Session;
use crate::keymap::{KeyMap, KeyTarget};

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A free key bound to a style resolves to the style, shows in the style's keys and runs no command.
#[test]
fn a_free_key_bound_to_a_style_applies_it() {
    let reg = crate::cmd::registry();
    let mut m = KeyMap::default();
    assert_eq!(m.resolve_target(&reg, "Mod+Alt+Shift+H"), None);
    assert_eq!(m.assign_style(&reg, "Heading 2", "mod+shift+alt+h").unwrap(), None);
    assert_eq!(m.resolve_target(&reg, "Mod+Alt+Shift+H"), Some(KeyTarget::Style("Heading 2".into())));
    assert!(m.resolve(&reg, "Mod+Alt+Shift+H").is_none(), "a style key runs no command");
    assert_eq!(m.keys_for_style("Heading 2"), ["Mod+Alt+Shift+H"]);
    assert!(m.keys_for_style("Heading 3").is_empty());
    assert!(!m.is_empty());
    // Command keys still resolve as before.
    assert_eq!(m.resolve_target(&reg, "Mod+B"), Some(KeyTarget::Command("format.bold".into())));
    // Not a key / empty style name.
    assert!(m.assign_style(&reg, "Heading 2", "Hyper+Q").is_err());
    assert!(m.assign_style(&reg, "  ", "Mod+Alt+Shift+J").is_err());
}

/// Taking a built-in key for a style: the command loses it (like reassigning it to another
/// command), and the previous owner is reported. Assigning the key back to a command, or to
/// another style, takes it from the style.
#[test]
fn a_style_key_takes_the_key_from_its_previous_owner() {
    let reg = crate::cmd::registry();
    let mut m = KeyMap::default();
    assert_eq!(m.assign_style(&reg, "Strong", "Mod+B").unwrap(), Some(KeyTarget::Command("format.bold".into())));
    assert_eq!(m.resolve_target(&reg, "Mod+B"), Some(KeyTarget::Style("Strong".into())));
    assert!(m.resolve(&reg, "Mod+B").is_none());
    assert!(m.keys_for(&reg, "format.bold").is_empty());
    // Another style takes it.
    assert_eq!(m.assign_style(&reg, "Emphasis", "Mod+B").unwrap(), Some(KeyTarget::Style("Strong".into())));
    assert!(m.keys_for_style("Strong").is_empty());
    assert_eq!(m.keys_for_style("Emphasis"), ["Mod+B"]);
    // A command takes it back: the style loses it.
    assert_eq!(m.assign(&reg, "format.bold", "Mod+B").unwrap(), None);
    assert!(m.keys_for_style("Emphasis").is_empty());
    assert_eq!(m.resolve_target(&reg, "Mod+B"), Some(KeyTarget::Command("format.bold".into())));
    // Removing a style key: a built-in key the style had taken goes back to its command.
    m.assign_style(&reg, "Strong", "Mod+U").unwrap();
    assert!(m.remove_style(&reg, "Strong", "Mod+U").unwrap());
    assert!(!m.remove_style(&reg, "Strong", "Mod+U").unwrap(), "already gone");
    assert_eq!(m.resolve_target(&reg, "Mod+U"), Some(KeyTarget::Command("format.underline".into())));
    m.reset();
    assert_eq!(m.resolve_target(&reg, "Mod+U"), Some(KeyTarget::Command("format.underline".into())));
    assert!(m.is_empty());
}

/// #3910 review: removing a style's key returns a built-in key to its command, as Word's Remove
/// returns a key to its default setting. Mod+B given to Heading 2 and then removed is Bold again.
#[test]
fn removing_a_style_key_gives_the_built_in_back() {
    let reg = crate::cmd::registry();
    let mut m = KeyMap::default();
    m.assign_style(&reg, "Heading 2", "Mod+B").unwrap();
    assert!(!m.keys_for(&reg, "format.bold").iter().any(|k| k == "Mod+B"));
    assert!(m.remove_style(&reg, "Heading 2", "Mod+B").unwrap());
    assert_eq!(m.resolve(&reg, "Mod+B").map(|c| c.id), Some("format.bold"));
    assert!(m.keys_for(&reg, "format.bold").iter().any(|k| k == "Mod+B"));
    assert!(m.is_empty(), "nothing customized is left: {m:?}");

    // The same through the command (the dialog's Remove button).
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "tools.customizeKeyboard", json!({"assign": {"style": "Heading 2", "key": "Mod+B"}}));
    let r = run(&mut s, "tools.customizeKeyboard", json!({"remove": {"style": "Heading 2", "key": "Mod+B"}, "key": "Mod+B"}));
    assert_eq!((r["removed"].as_bool(), r["assignedTo"].as_str()), (Some(true), Some("format.bold")));
    run(&mut s, "text.insert", json!({"text": "Bold"}));
    run(&mut s, "select.all", json!({}));
    s.press_key("Mod+B").expect("bound").unwrap();
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(0).bold, Some(true), "Mod+B is Bold again");
}

/// A style key counts as bound: callers that only fall back to a built-in behaviour for a free
/// key (F6 cycling panes, Mod+Shift+0 resetting the interface size) leave it alone.
#[test]
fn a_style_key_is_bound() {
    let reg = crate::cmd::registry();
    let mut m = KeyMap::default();
    assert!(!m.is_bound(&reg, "Mod+Alt+Shift+H"));
    m.assign_style(&reg, "Heading 2", "F6").unwrap();
    m.assign_style(&reg, "Heading 3", "Mod+Shift+0").unwrap();
    for key in ["F6", "Mod+Shift+0"] {
        assert!(m.is_bound(&reg, key), "{key}");
        assert!(m.resolve(&reg, key).is_none(), "{key} runs no command");
    }
    assert!(m.is_bound(&reg, "Mod+B"), "command keys are bound too");
}

/// #3910 review: a style is matched by name or id. A key bound to `Heading2` (its id) shows for
/// "Heading 2" and is removed by either spelling; the command saves the name, as the dialog does.
#[test]
fn style_keys_match_by_name_or_id() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    let r = run(&mut s, "tools.customizeKeyboard", json!({"assign": {"style": "Heading2", "key": "Mod+Alt+Shift+H"}}));
    assert_eq!(r["custom"]["styles"], json!({"Mod+Alt+Shift+H": "Heading 2"}), "saved by name");
    for style in ["Heading 2", "Heading2", "heading 2"] {
        assert_eq!(run(&mut s, "tools.customizeKeyboard", json!({"style": style}))["keys"], json!(["Mod+Alt+Shift+H"]), "{style}");
    }
    // Rebinding the same style under its other spelling replaces nothing.
    let r = run(&mut s, "tools.customizeKeyboard", json!({"assign": {"style": "Heading 2", "key": "Mod+Alt+Shift+H"}}));
    assert_eq!(r["replaced"], Value::Null);
    // A map saved with the id (an older settings file, say) reads the same.
    s.keymap = KeyMap::from_value(&json!({"styles": {"Mod+Alt+Shift+J": "Heading2"}}));
    assert_eq!(s.keymap.keys_for_style_in(&s.doc.styles, "Heading 2"), ["Mod+Alt+Shift+J"]);
    let r = run(&mut s, "tools.customizeKeyboard", json!({"remove": {"style": "Heading 2", "key": "Mod+Alt+Shift+J"}}));
    assert_eq!(r["removed"], json!(true));
    assert!(s.keymap.is_empty());
    // Different styles never match, and a name no document defines matches only itself.
    s.keymap = KeyMap::from_value(&json!({"styles": {"Mod+Alt+Shift+J": "Heading 3", "Mod+Alt+Shift+K": "Gone"}}));
    assert!(s.keymap.keys_for_style_in(&s.doc.styles, "Heading 2").is_empty());
    assert_eq!(s.keymap.keys_for_style_in(&s.doc.styles, "Gone"), ["Mod+Alt+Shift+K"]);
}

/// Style keys are saved next to command keys and read back; junk entries are dropped.
#[test]
fn style_keys_round_trip_and_junk_is_ignored() {
    let reg = crate::cmd::registry();
    let mut m = KeyMap::default();
    m.assign_style(&reg, "Heading 1", "Mod+Alt+Shift+1").unwrap();
    m.assign_style(&reg, "Strong", "Mod+B").unwrap();
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(v["styles"], json!({"Mod+Alt+Shift+1": "Heading 1", "Mod+B": "Strong"}));
    let back: KeyMap = serde_json::from_value(v).unwrap();
    assert_eq!(back, m);
    // Maps saved before style keys existed still read.
    let old: KeyMap = serde_json::from_value(json!({"assigned": {"Mod+Alt+K": "format.bold"}, "removed": []})).unwrap();
    assert!(old.keys_for_style("Heading 1").is_empty());
    let junk = json!({"styles": {"Hyper+X": "Heading 1", "Mod+Q": 7, "Mod+J": "", "Mod+Alt+Shift+K": "Title"}});
    let mut j: KeyMap = serde_json::from_value(junk).unwrap();
    j.retain_known(&reg);
    assert_eq!(j.resolve_target(&reg, "Mod+Alt+Shift+K"), Some(KeyTarget::Style("Title".into())), "styles aren't checked against a registry");
    assert_eq!(j.keys_for_style("Heading 1").len() + j.keys_for_style("").len(), 0);
}

/// `tools.customizeKeyboard` does the same for agents: assign, query and remove style keys,
/// without opening the dialog; `key` reports a style owner as `assignedToStyle`.
#[test]
fn customize_keyboard_binds_styles_for_agents() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    let r = run(&mut s, "tools.customizeKeyboard", json!({"assign": {"style": "Heading 2", "key": "Mod+Alt+Shift+H"}}));
    assert_eq!(r["replaced"], Value::Null);
    let r = run(&mut s, "tools.customizeKeyboard", json!({"style": "Heading 2"}));
    assert_eq!(r["keys"], json!(["Mod+Alt+Shift+H"]));
    let r = run(&mut s, "tools.customizeKeyboard", json!({"key": "mod+alt+shift+h"}));
    assert_eq!((r["assignedTo"].clone(), r["assignedToStyle"].as_str()), (Value::Null, Some("Heading 2")));
    let r = run(&mut s, "tools.customizeKeyboard", json!({"assign": {"style": "Strong", "key": "Mod+B"}}));
    assert_eq!(r["replaced"], json!({"command": "format.bold"}));
    let r = run(&mut s, "tools.customizeKeyboard", json!({"remove": {"style": "Heading 2", "key": "Mod+Alt+Shift+H"}}));
    assert_eq!(r["removed"], json!(true));
    assert!(s.ui_requests.is_empty(), "programmatic calls never open dialogs");
    // `assign`/`remove` need exactly one of `command` / `style`.
    assert!(s.run("tools.customizeKeyboard", &json!({"assign": {"key": "Mod+B"}})).is_err());
    assert!(s.run("tools.customizeKeyboard", &json!({"assign": {"command": "format.bold", "style": "Strong", "key": "Mod+B"}})).is_err());
}

/// Pressing a style's key applies the style to the selection's paragraphs (Session::press_key
/// is the engine half of the UI's keyboard dispatch: style keys run `styles.apply`).
#[test]
fn pressing_a_style_key_applies_the_style() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Chapter"}));
    run(&mut s, "tools.customizeKeyboard", json!({"assign": {"style": "Heading 2", "key": "Mod+Alt+Shift+H"}}));
    s.press_key("Mod+Alt+Shift+H").expect("bound").unwrap();
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.style.as_deref(), Some("Heading2"));
    // A key bound to a command still runs the command; an unbound key reports None.
    assert!(s.press_key("Mod+Alt+Shift+F11").is_none());
}

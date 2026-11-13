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
    // Removing a style key: a built-in key the style had taken stays removed (nothing runs).
    m.assign_style(&reg, "Strong", "Mod+U").unwrap();
    assert!(m.remove_style(&reg, "Strong", "Mod+U").unwrap());
    assert!(!m.remove_style(&reg, "Strong", "Mod+U").unwrap(), "already gone");
    assert_eq!(m.resolve_target(&reg, "Mod+U"), None);
    m.reset();
    assert_eq!(m.resolve_target(&reg, "Mod+U"), Some(KeyTarget::Command("format.underline".into())));
    assert!(m.is_empty());
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

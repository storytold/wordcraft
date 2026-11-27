//! Keyboard shortcuts for styles (#3910): Format › Shortcut key… in the style dialogs and the
//! Styles category of Customize Keyboard bind a key to a paragraph or character style, as in
//! Word. A style key is `styles.apply` with the style's id, kept in the session's keymap
//! overrides next to command keys (saved with the preferences); pressing it applies the style.

use serde_json::{Value, json};

use wordcraft_doc::Pos;

use crate::Session;
use crate::keymap::{style_command, style_defs, style_of};

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn keyboard(s: &mut Session, v: Value) -> Value {
    run(s, "tools.customizeKeyboard", v)
}

/// A key's canonical text, as `keys` lists it (`Mod+B` is `Cmd+B`).
fn canon(key: &str) -> String {
    craft_keymap::Chord::parse(key).unwrap().portable()
}

fn session_with(text: &str) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": text}));
    s
}

/// A free key given to a style applies it, shows in the style's keys and is reported as the
/// style's; command keys work as before.
#[test]
fn a_free_key_bound_to_a_style_applies_it() {
    let mut s = session_with("Chapter");
    assert_eq!(keyboard(&mut s, json!({"key": "Mod+Alt+Shift+H"}))["assignedToStyle"], Value::Null);
    let r = keyboard(&mut s, json!({"assign": {"style": "Heading 2", "key": "mod+shift+alt+h"}}));
    assert_eq!((r["replaced"].clone(), r["replacedStyle"].clone()), (Value::Null, Value::Null));
    let r = keyboard(&mut s, json!({"key": "Mod+Alt+Shift+H"}));
    assert_eq!((r["assignedTo"].clone(), r["assignedToStyle"].as_str()), (Value::Null, Some("Heading 2")));
    assert_eq!(keyboard(&mut s, json!({"style": "Heading 2"}))["keys"], json!([canon("Mod+Alt+Shift+H")]));
    assert_eq!(keyboard(&mut s, json!({"style": "Heading 3"}))["keys"], json!([]));
    assert_eq!(keyboard(&mut s, json!({"key": "Mod+B"}))["assignedTo"], json!("format.bold"));
    assert!(s.ui_requests.is_empty(), "programmatic calls never open dialogs");

    s.press_key("Mod+Alt+Shift+H").expect("bound").unwrap();
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.style.as_deref(), Some("Heading2"));
    assert!(s.press_key("Mod+Alt+Shift+F11").is_none(), "an unbound key does nothing");
}

/// Taking a built-in key for a style: the command loses it and is reported; another style or a
/// command can take it back.
#[test]
fn a_style_key_takes_the_key_from_its_previous_owner() {
    let mut s = session_with("x");
    let r = keyboard(&mut s, json!({"assign": {"style": "Strong", "key": "Mod+B"}}));
    assert_eq!(r["replaced"], json!("format.bold"));
    assert_eq!(keyboard(&mut s, json!({"command": "format.bold"}))["keys"], json!([]));
    let r = keyboard(&mut s, json!({"assign": {"style": "Emphasis", "key": "Mod+B"}}));
    assert_eq!((r["replaced"].clone(), r["replacedStyle"].as_str()), (Value::Null, Some("Strong")));
    assert_eq!(keyboard(&mut s, json!({"style": "Strong"}))["keys"], json!([]));
    assert_eq!(keyboard(&mut s, json!({"style": "Emphasis"}))["keys"], json!([canon("Mod+B")]));
    let r = keyboard(&mut s, json!({"assign": {"command": "format.bold", "key": "Mod+B"}}));
    assert_eq!(r["replacedStyle"], json!("Emphasis"));
    assert_eq!(keyboard(&mut s, json!({"style": "Emphasis"}))["keys"], json!([]));
    assert_eq!(keyboard(&mut s, json!({"key": "Mod+B"}))["assignedTo"], json!("format.bold"));
}

/// #3910 review: removing a style's key gives a built-in key back to its command, as Word's
/// Remove returns a key to its default; nothing customized is left.
#[test]
fn removing_a_style_key_gives_the_built_in_back() {
    let mut s = session_with("Bold");
    keyboard(&mut s, json!({"assign": {"style": "Heading 2", "key": "Mod+B"}}));
    let r = keyboard(&mut s, json!({"remove": {"style": "Heading 2", "key": "Mod+B"}, "key": "Mod+B"}));
    assert_eq!((r["removed"].as_bool(), r["assignedTo"].as_str()), (Some(true), Some("format.bold")));
    assert!(s.keymap().is_empty(), "{:?}", s.keymap());
    run(&mut s, "select.all", json!({}));
    s.press_key("Mod+B").expect("bound").unwrap();
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(0).bold, Some(true), "Mod+B is Bold again");
}

/// #3910 review: a style is matched by name or id in any case, and the key is kept by id, so
/// every spelling finds and removes it.
#[test]
fn style_keys_match_by_name_or_id() {
    let mut s = session_with("x");
    keyboard(&mut s, json!({"assign": {"style": "Heading2", "key": "Mod+Alt+Shift+H"}}));
    for style in ["Heading 2", "Heading2", "heading 2"] {
        assert_eq!(keyboard(&mut s, json!({"style": style}))["keys"], json!([canon("Mod+Alt+Shift+H")]), "{style}");
    }
    let r = keyboard(&mut s, json!({"assign": {"style": "heading 2", "key": "Mod+Alt+Shift+H"}}));
    assert_eq!((r["replaced"].clone(), r["replacedStyle"].clone()), (Value::Null, Value::Null), "same style");
    let key = style_command("Heading2").key();
    assert!(s.keymap().get(&key).is_some(), "kept by id: {:?}", s.keymap());
    let r = keyboard(&mut s, json!({"remove": {"style": "Heading 2", "key": "Mod+Alt+Shift+H"}}));
    assert_eq!(r["removed"], json!(true));
    assert!(s.keymap().is_empty());
}

/// Style keys are saved with the other keyboard overrides and read back; an entry whose
/// parameters aren't a style is not a style key.
#[test]
fn style_keys_round_trip_with_the_overrides() {
    let mut s = session_with("x");
    keyboard(&mut s, json!({"assign": {"style": "Heading 1", "key": "Mod+Alt+Shift+1"}}));
    keyboard(&mut s, json!({"assign": {"style": "Strong", "key": "Mod+B"}}));
    let saved = serde_json::to_value(s.keymap()).unwrap();
    let back: craft_keymap::Overrides = serde_json::from_value(saved).unwrap();
    assert_eq!(&back, s.keymap());
    let styles: Vec<String> = style_defs(&back).filter_map(|d| style_of(&d.command).map(str::to_string)).collect();
    assert_eq!(styles, ["Heading1", "Strong"]);

    for params in [json!({"style": ""}), json!({"style": 7}), json!({"name": "Strong"})] {
        assert_eq!(style_of(&craft_keymap::CommandRef::with_params("styles.apply", params.clone())), None, "{params}");
    }
    assert_eq!(style_of(&craft_keymap::CommandRef::with_params("para.style", json!({"style": "Strong"}))), None);
}

/// Mistakes are errors: no target, both targets, a style the document doesn't have.
#[test]
fn style_key_mistakes_are_errors() {
    let mut s = session_with("x");
    for v in [
        json!({"assign": {"key": "Mod+Alt+Shift+H"}}),
        json!({"assign": {"command": "format.bold", "style": "Strong", "key": "Mod+Alt+Shift+H"}}),
        json!({"assign": {"style": "No Such Style", "key": "Mod+Alt+Shift+H"}}),
        json!({"assign": {"style": "Strong", "key": "Hyper+Q"}}),
        json!({"style": "No Such Style"}),
    ] {
        assert!(s.run("tools.customizeKeyboard", &v).is_err(), "{v}");
    }
    assert!(s.keymap().is_empty());
}

/// The parts of a call apply in order: a reset before an assign to an unknown style stays done,
/// and only the assign is an error.
#[test]
fn reset_then_unknown_style_keeps_the_reset() {
    let mut s = session_with("x");
    keyboard(&mut s, json!({"assign": {"style": "Strong", "key": "Mod+B"}}));
    let v = json!({"reset": true, "assign": {"style": "No Such Style", "key": "Mod+Alt+Shift+H"}});
    assert!(s.run("tools.customizeKeyboard", &v).is_err());
    assert!(s.keymap().is_empty(), "the reset ran: {:?}", s.keymap());
    assert_eq!(keyboard(&mut s, json!({"key": "Mod+B"}))["assignedTo"], json!("format.bold"));
}

/// A style key saved under a style id this document doesn't have can still be removed by that id;
/// an id with no saved key is still an error.
#[test]
fn removing_a_key_of_an_absent_style() {
    let mut s = session_with("x");
    keyboard(&mut s, json!({"assign": {"style": "Heading 2", "key": "Mod+Alt+Shift+H"}}));
    let saved = serde_json::to_string(s.keymap()).unwrap().replace("Heading2", "OtherDocStyle");
    assert!(saved.contains("OtherDocStyle"), "{saved}");
    let mut t = session_with("y");
    *t.keymap_mut() = serde_json::from_str(&saved).unwrap();
    let bad = json!({"remove": {"style": "NeverSaved", "key": "Mod+Alt+Shift+H"}});
    assert!(t.run("tools.customizeKeyboard", &bad).is_err());
    let r = keyboard(&mut t, json!({"remove": {"style": "OtherDocStyle", "key": "Mod+Alt+Shift+H"}}));
    assert_eq!(r["removed"], json!(true));
    assert!(t.keymap().is_empty(), "{:?}", t.keymap());
}

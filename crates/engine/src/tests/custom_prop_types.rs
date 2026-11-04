//! Typed custom document properties (`file.properties` with `"type"`): Word's Custom tab offers
//! Text, Date, Number and Yes or no, stored as `vt:lpwstr`, `vt:filetime`, `vt:i4`/`vt:r8` and
//! `vt:bool` (ECMA-376 Part 1 §22.3, §22.4).

use serde_json::{Value, json};
use wordcraft_doc::Document;

use crate::Session;

fn s() -> Session {
    Session::new(Document::from_text("Body"))
}

/// The stored (kind, value) of the property `name`.
fn prop(s: &Session, name: &str) -> Option<(String, String)> {
    s.doc.custom_props.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map(|p| (p.kind.clone(), p.value.clone()))
}

fn set(s: &mut Session, name: &str, ty: &str, value: &str) -> Result<Value, String> {
    s.run("file.properties", &json!({"custom": { (name): {"value": value, "type": ty} }})).map_err(|e| e.to_string())
}

#[test]
fn each_type_is_stored_as_its_variant() {
    let cases: &[(&str, &str, &str, &str)] = &[
        ("text", "  draft ", "lpwstr", "  draft "),
        ("number", "42", "i4", "42"),
        ("number", " -7 ", "i4", "-7"),
        ("number", "1e3", "i4", "1000"),
        ("number", "3.5", "r8", "3.5"),
        ("number", "-0.25", "r8", "-0.25"),
        ("number", "3000000000", "r8", "3000000000"),
        ("date", "2026-11-30", "filetime", "2026-11-30T00:00:00Z"),
        ("date", "2024-02-29", "filetime", "2024-02-29T00:00:00Z"),
        ("date", "2026-11-30T14:05:00Z", "filetime", "2026-11-30T14:05:00Z"),
        ("yesno", "true", "bool", "true"),
        ("yesno", "No", "bool", "false"),
        ("yesno", "YES", "bool", "true"),
    ];
    for &(ty, input, kind, value) in cases {
        let mut s = s();
        let r = set(&mut s, "P", ty, input).unwrap_or_else(|e| panic!("{ty} {input:?}: {e}"));
        assert_eq!(prop(&s, "P"), Some((kind.to_string(), value.to_string())), "{ty} {input:?}");
        let listed = r["custom"].as_array().cloned().unwrap_or_default();
        assert!(listed.iter().any(|p| p["name"] == "P" && p["kind"] == kind && p["value"] == value), "{ty} {input:?}: result lists it");
    }
}

#[test]
fn invalid_values_are_refused_and_not_stored() {
    let cases: &[(&str, &str)] = &[
        ("number", "abc"),
        ("number", "NaN"),
        ("number", "inf"),
        ("number", "1,5"),
        ("number", ""),
        ("date", "2026-02-30"),
        ("date", "2025-02-29"),
        ("date", "2026-13-01"),
        ("date", "01/11/2026"),
        ("yesno", "maybe"),
        ("colour", "red"),
    ];
    for &(ty, input) in cases {
        let mut s = s();
        assert!(set(&mut s, "P", ty, input).is_err(), "{ty} {input:?} is refused");
        assert_eq!(prop(&s, "P"), None, "{ty} {input:?} is not stored");
    }
}

#[test]
fn retyping_replaces_in_place_and_untyped_forms_stay_text() {
    let mut s = s();
    s.run("file.properties", &json!({"custom": {"A": "x", "Pages": "twelve", "Z": "y"}})).unwrap();
    set(&mut s, "pages", "number", "12").unwrap();
    let names: Vec<&str> = s.doc.custom_props.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["A", "Pages", "Z"], "kept its place and spelling");
    assert_eq!(prop(&s, "Pages"), Some(("i4".into(), "12".into())));
    // A plain string, or an object without a type, is text again.
    s.run("file.properties", &json!({"custom": {"Pages": "12 pages"}})).unwrap();
    assert_eq!(prop(&s, "Pages"), Some(("lpwstr".into(), "12 pages".into())));
    s.run("file.properties", &json!({"custom": {"Z": {"value": "1"}}})).unwrap();
    assert_eq!(prop(&s, "Z"), Some(("lpwstr".into(), "1".into())));
}

#[test]
fn a_typed_set_is_one_undo_step() {
    let mut s = s();
    s.run("file.properties", &json!({"custom": {"Reviewed": "pending"}})).unwrap();
    s.run("file.properties", &json!({"custom": {"Reviewed": {"value": "yes", "type": "yesno"}, "Due": {"value": "2026-11-30", "type": "date"}}}))
        .unwrap();
    assert_eq!(prop(&s, "Reviewed"), Some(("bool".into(), "true".into())));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(prop(&s, "Reviewed"), Some(("lpwstr".into(), "pending".into())));
    assert_eq!(prop(&s, "Due"), None);
}

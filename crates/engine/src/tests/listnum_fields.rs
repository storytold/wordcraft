//! LISTNUM fields (ECMA-376 §17.16.5.32) number like a Word multilevel list on F9: one set of level
//! counters for the whole body, the level from `\l` or from the heading before the field. Expected
//! results read from Microsoft 365 Word's PDF of a probe document. Alt+Ctrl+L inserts a LISTNUM field
//! and Alt+Ctrl+Y repeats Find (Windows and Linux).

use serde_json::{Value, json};
use wordcraft_doc::{InlineObject, StoryRef};

use crate::{Session, cmd};

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Every body field's code and result, in document order.
fn fields(s: &Session) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        for o in &s.doc.para(StoryRef::Body, &path).unwrap().objects {
            if let InlineObject::Field { instr, result, .. } = o {
                out.push((instr.trim().to_string(), result.clone()));
            }
        }
    }
    out
}

fn id(k: &str) -> Option<&'static str> {
    cmd::registry().by_shortcut(k).map(|c| c.id)
}

/// (paragraph text, style, LISTNUM code or "" for none, Word's result)
const PROBE: [(&str, &str, &str, &str); 13] = [
    ("R01 ", "", "LISTNUM", "1)"),
    ("R02 ", "", "LISTNUM", "2)"),
    ("R03 ", "", r"LISTNUM \l 3", "i)"),
    ("R04 ", "", r"LISTNUM \l 2", "b)"),
    ("R05 ", "", "LISTNUM LegalDefault", "3."),
    ("Head one", "Heading 1", "", ""),
    ("R06 ", "", "LISTNUM", "a)"),
    ("Head two", "Heading 2", "", ""),
    ("R07 ", "", "LISTNUM", "i)"),
    ("R08 heading ", "Heading 1", "LISTNUM", "4)"),
    ("R09 ", "", r"LISTNUM \l 1", "5)"),
    ("Head three", "Heading 3", "", ""),
    ("R10 ", "", "LISTNUM", "(1)"),
];

fn probe() -> Session {
    let text: Vec<&str> = PROBE.iter().map(|r| r.0.trim_end()).collect();
    let mut s = Session::new(wordcraft_doc::Document::from_text(&text.join("\n")));
    for (t, style, instr, _) in PROBE {
        s.run("select.text", &json!({"text": t.trim_end()})).unwrap();
        if !style.is_empty() {
            run(&mut s, "styles.apply", json!({"style": style}));
        }
        if !instr.is_empty() {
            run(&mut s, "caret.end", json!({}));
            run(&mut s, "insert.field", json!({"instr": instr}));
        }
    }
    s
}

#[test]
fn f9_numbers_listnum_fields_as_word_does() {
    let mut s = probe();
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.updateFields", json!({"all": true}));
    let want: Vec<(String, String)> = PROBE.iter().filter(|r| !r.2.is_empty()).map(|r| (r.2.to_string(), r.3.to_string())).collect();
    assert_eq!(fields(&s), want);
}

#[test]
fn a_locked_listnum_keeps_its_result_and_does_not_count() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("A\nB\nC"));
    for t in ["A", "B", "C"] {
        s.run("select.text", &json!({"text": t})).unwrap();
        run(&mut s, "caret.end", json!({}));
        run(&mut s, "insert.field", json!({"instr": "LISTNUM"}));
    }
    // Lock the middle one with a stale result.
    let path = s.doc.para_paths(StoryRef::Body)[1].clone();
    let p = s.doc.para_mut(StoryRef::Body, &path).unwrap();
    for o in &mut p.objects {
        if let InlineObject::Field { locked, result, .. } = o {
            *locked = true;
            *result = "OLD".into();
        }
    }
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.updateFields", json!({"all": true}));
    let got: Vec<String> = fields(&s).into_iter().map(|f| f.1).collect();
    assert_eq!(got, ["1)", "OLD", "2)"]);
}

#[test]
fn insert_list_num_inserts_a_numbered_field_in_one_undo_step() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("First\nSecond"));
    s.run("select.text", &json!({"text": "First"})).unwrap();
    run(&mut s, "caret.end", json!({}));
    run(&mut s, "insert.listNum", json!({}));
    s.run("select.text", &json!({"text": "Second"})).unwrap();
    run(&mut s, "caret.end", json!({}));
    let depth = s.undo_depth();
    run(&mut s, "insert.listNum", json!({}));
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    let got: Vec<(String, String)> = fields(&s);
    assert_eq!(got, [("LISTNUM".to_string(), "1)".to_string()), ("LISTNUM".to_string(), "2)".to_string())]);
}

#[test]
fn listnum_and_repeat_find_keys() {
    if cfg!(target_os = "macos") {
        assert_ne!(id("Mod+Alt+L"), Some("insert.listNum"));
        assert_ne!(id("Mod+Alt+Y"), Some("edit.findNext"));
    } else {
        assert_eq!(id("Mod+Alt+L"), Some("insert.listNum"));
        assert_eq!(id("Mod+Alt+Y"), Some("edit.findNext"));
    }
}

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

// PR #5881 review: a LISTNUM in a tracked deletion, undo, macOS keys, name casing, output path.

/// Three paragraphs ending in LISTNUM; the middle field is a tracked deletion with result "OLD".
fn with_deleted_middle() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::from_text("A\nB\nC"));
    for t in ["A", "B", "C"] {
        s.run("select.text", &json!({"text": t})).unwrap();
        run(&mut s, "caret.end", json!({}));
        run(&mut s, "insert.field", json!({"instr": "LISTNUM"}));
    }
    let path = s.doc.para_paths(StoryRef::Body)[1].clone();
    let p = s.doc.para_mut(StoryRef::Body, &path).unwrap();
    for o in &mut p.objects {
        if let InlineObject::Field { result, .. } = o {
            *result = "OLD".into();
        }
    }
    p.touch();
    let off = s.doc.para(StoryRef::Body, &path).unwrap().object_offsets()[0];
    let at = |off| wordcraft_doc::Pos { story: StoryRef::Body, path: path.clone(), off };
    s.doc.format_range(&at(off), &at(off + '\u{FFFC}'.len_utf8()), &|c| c.del = Some(1)).unwrap();
    s
}

/// Upstream's #5594 rule for SEQ, applied to LISTNUM: a field in a tracked deletion keeps its result
/// and the fields after it are numbered as if it weren't there, on F9 and in output.
#[test]
fn a_listnum_in_a_tracked_deletion_keeps_its_result_and_does_not_count() {
    let mut s = with_deleted_middle();
    let out = crate::cmd::references::output_fields(&s.doc).expect("stale results change");
    let shown: Vec<String> = out
        .para_paths(StoryRef::Body)
        .iter()
        .flat_map(|p| out.para(StoryRef::Body, p).unwrap().objects.clone())
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(shown, ["1)", "OLD", "2)"], "output");
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.updateFields", json!({"all": true}));
    let got: Vec<String> = fields(&s).into_iter().map(|f| f.1).collect();
    assert_eq!(got, ["1)", "OLD", "2)"], "F9");
}

/// Each Insert LISTNUM is one undo step: undoing the second leaves the first, undoing again leaves none.
#[test]
fn insert_list_num_undoes_in_one_step_each() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("First\nSecond"));
    for t in ["First", "Second"] {
        s.run("select.text", &json!({"text": t})).unwrap();
        run(&mut s, "caret.end", json!({}));
        run(&mut s, "insert.listNum", json!({}));
    }
    assert_eq!(fields(&s).len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fields(&s), [("LISTNUM".to_string(), "1)".to_string())]);
    run(&mut s, "edit.undo", json!({}));
    assert!(fields(&s).is_empty());
    assert_eq!(s.doc.plain_text(StoryRef::Body).lines().collect::<Vec<_>>(), ["First", "Second"]);
}

/// On macOS ⌘⌥L stays Word for Mac's Spelling & Grammar and ⌘⌥Y is unbound.
#[cfg(target_os = "macos")]
#[test]
fn listnum_keys_are_not_bound_on_macos() {
    assert_eq!(id("Mod+Alt+L"), Some("review.spelling"));
    assert_eq!(id("Mod+Alt+Y"), None);
}

/// The field name is case-insensitive: `listnum` and `ListNum` count with `LISTNUM`.
#[test]
fn listnum_field_names_are_case_insensitive() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("A\nB\nC"));
    for (t, instr) in [("A", "LISTNUM"), ("B", "listnum"), ("C", r"ListNum \l 2")] {
        s.run("select.text", &json!({"text": t})).unwrap();
        run(&mut s, "caret.end", json!({}));
        run(&mut s, "insert.field", json!({"instr": instr}));
    }
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.updateFields", json!({"all": true}));
    let got: Vec<String> = fields(&s).into_iter().map(|f| f.1).collect();
    assert_eq!(got, ["1)", "2)", "a)"]);
}

/// Printing or exporting the Word probe gives Word's results too (LegalDefault, OutlineDefault,
/// `\s`, levels from headings), without changing the stored ones.
#[test]
fn output_numbers_the_probe_as_word_does() {
    let s = probe();
    // `None` when every stored result is already current: output then lays out the document as it is.
    let out = crate::cmd::references::output_fields(&s.doc).unwrap_or_else(|| s.doc.clone());
    let mut shown = Vec::new();
    for path in out.para_paths(StoryRef::Body) {
        for o in &out.para(StoryRef::Body, &path).unwrap().objects {
            if let InlineObject::Field { instr, result, .. } = o {
                shown.push((instr.trim().to_string(), result.clone()));
            }
        }
    }
    let want: Vec<(String, String)> = PROBE.iter().filter(|r| !r.2.is_empty()).map(|r| (r.2.to_string(), r.3.to_string())).collect();
    assert_eq!(shown, want);
}

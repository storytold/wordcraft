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

/// Locking doesn't freeze a LISTNUM: Word renumbers a locked one and counts it (measured in
/// Microsoft 365 Word: with the middle of three fields locked and a stale result, the update gives
/// 1) 2) 3)).
#[test]
fn a_locked_listnum_still_renumbers_and_counts() {
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
    assert_eq!(got, ["1)", "2)", "3)"]);
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

/// A body paragraph anchoring a text box holding `box_text`; the caret at the end of the box's text.
fn text_box_session(body: &str, box_text: &str) -> (Session, u32) {
    let mut s = Session::new(wordcraft_doc::Document::from_text(body));
    s.run("select.text", &json!({"text": body})).unwrap();
    run(&mut s, "caret.end", json!({}));
    let id = run(&mut s, "insert.textBox", json!({"text": box_text}))["story"].as_u64().unwrap() as u32;
    let end = box_text.len();
    s.sel = crate::session::Selection::caret(wordcraft_doc::Pos::new(StoryRef::Part(id), wordcraft_doc::Path::top(0), end));
    (s, id)
}

/// The text box's field codes and results, in order.
fn box_fields(s: &Session, id: u32) -> Vec<(String, String)> {
    let story = StoryRef::Part(id);
    let mut out = Vec::new();
    for path in s.doc.para_paths(story) {
        for o in &s.doc.para(story, &path).unwrap().objects {
            if let InlineObject::Field { instr, result, .. } = o {
                out.push((instr.trim().to_string(), result.clone()));
            }
        }
    }
    out
}

/// #5881 review: Alt+Ctrl+L in a text box inserts a numbered field, not an empty one.
#[test]
fn insert_list_num_in_a_text_box_is_numbered() {
    let (mut s, id) = text_box_session("Anchor", "Boxed");
    run(&mut s, "insert.listNum", json!({}));
    assert_eq!(box_fields(&s, id), [("LISTNUM".to_string(), "1)".to_string())]);
}

/// #5881 review: F9 with the caret in a text box numbers its LISTNUM fields.
#[test]
fn f9_in_a_text_box_numbers_listnum() {
    let (mut s, id) = text_box_session("Anchor", "Boxed");
    run(&mut s, "insert.field", json!({"instr": "LISTNUM", "result": ""}));
    // Clear whatever the insert stored, then update from inside the box.
    for path in s.doc.para_paths(StoryRef::Part(id)) {
        for o in &mut s.doc.para_mut(StoryRef::Part(id), &path).unwrap().objects {
            if let InlineObject::Field { result, .. } = o {
                result.clear();
            }
        }
    }
    let end = "Boxed".len() + 1;
    s.sel = crate::session::Selection::caret(wordcraft_doc::Pos::new(StoryRef::Part(id), wordcraft_doc::Path::top(0), end));
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(box_fields(&s, id), [("LISTNUM".to_string(), "1)".to_string())]);
}

/// A LISTNUM in the body and one in a text box anchored after it share one numbering.
#[test]
fn listnum_in_body_and_text_box_count_together() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("First\nAnchor"));
    s.run("select.text", &json!({"text": "First"})).unwrap();
    run(&mut s, "caret.end", json!({}));
    run(&mut s, "insert.listNum", json!({}));
    s.run("select.text", &json!({"text": "Anchor"})).unwrap();
    run(&mut s, "caret.end", json!({}));
    let id = run(&mut s, "insert.textBox", json!({"text": "Boxed"}))["story"].as_u64().unwrap() as u32;
    s.sel = crate::session::Selection::caret(wordcraft_doc::Pos::new(StoryRef::Part(id), wordcraft_doc::Path::top(0), "Boxed".len()));
    run(&mut s, "insert.listNum", json!({}));
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(fields(&s), [("LISTNUM".to_string(), "1)".to_string())]);
    assert_eq!(box_fields(&s, id), [("LISTNUM".to_string(), "2)".to_string())]);
}

/// Fields inserted one per paragraph, then F9: each field's result.
fn numbered(rows: &[&str]) -> Vec<String> {
    let names: Vec<String> = (1..=rows.len()).map(|i| format!("P{i}")).collect();
    let mut s = Session::new(wordcraft_doc::Document::from_text(&names.join("\n")));
    for (name, instr) in names.iter().zip(rows) {
        s.run("select.text", &json!({"text": name})).unwrap();
        run(&mut s, "caret.end", json!({}));
        run(&mut s, "insert.field", json!({"instr": instr}));
    }
    run(&mut s, "references.updateFields", json!({}));
    fields(&s).into_iter().map(|(_, r)| r).collect()
}

/// #5881 review: an ancestor level started at 0 by `\s 0` stays 0 under deeper LISTNUMs; a level
/// never started (or reset by a shallower field) shows 1. Measured in Microsoft 365 Word 16.0.20527.
#[test]
fn s0_ancestor_stays_zero_as_word_shows() {
    assert_eq!(
        numbered(&[r"LISTNUM LegalDefault \l 1 \s 0", r"LISTNUM LegalDefault \l 2", r"LISTNUM LegalDefault \l 2", r"LISTNUM LegalDefault \l 1"]),
        ["0.", "0.1.", "0.2.", "1."]
    );
    assert_eq!(
        numbered(&[
            r"LISTNUM LegalDefault \l 2",
            r"LISTNUM LegalDefault \l 3",
            r"LISTNUM LegalDefault \l 1 \s 0",
            r"LISTNUM LegalDefault \l 3",
            r"LISTNUM \l 1 \s 0",
            r"LISTNUM \l 2",
        ]),
        ["1.1.", "1.1.1.", "0.", "0.1.1.", "0)", "a)"]
    );
}

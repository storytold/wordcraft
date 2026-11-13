//! SYMBOL fields (ECMA-376 §17.16.5.66) show the character their code gives, as Microsoft 365 Word
//! prints them, even when the stored result is stale; F9 stores that character; a missing code, a
//! code that is not a number and a code above 255 without `\u` give `###`. Expected results read
//! from Word's PDF of a probe document whose fields were all stored with the result "OLD".

use serde_json::{Value, json};
use wordcraft_doc::{CharProps, InlineObject, StoryRef};

use crate::Session;

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The character a symbol font draws for `code` (what the layout already shows for a SYMBOL field
/// without a stored result).
fn in_font(font: &str, code: u32) -> String {
    let c = char::from_u32(code).unwrap();
    wordcraft_doc::symbol_fonts::symbol_char(font, c).or_else(|| char::from_u32(0xF000 + code)).unwrap().to_string()
}

/// (field code, Word's result)
fn probe() -> Vec<(&'static str, String)> {
    vec![
        ("SYMBOL 169", "©".into()),
        ("SYMBOL 0xA9", "©".into()),
        (r#"SYMBOL 169 \f "Symbol""#, in_font("Symbol", 169)),
        (r"SYMBOL 65 \f Wingdings", in_font("Wingdings", 65)),
        (r"SYMBOL 8364 \u", "€".into()),
        (r"SYMBOL 0x20AC \u", "€".into()),
        (r"SYMBOL 169 \s 24", "©".into()),
        (r#"SYMBOL 169 \f "Arial" \s 12 \h"#, "©".into()),
        ("SYMBOL 300", "###".into()),
        ("SYMBOL 65", "A".into()),
        (r"SYMBOL 65 \a", "A".into()),
        (r#"SYMBOL 0x41 \u \f "Courier New""#, "A".into()),
        ("SYMBOL abc", "###".into()),
        ("SYMBOL", "###".into()),
    ]
}

/// One paragraph `Snn` per probe row, each ending in its field with the stale result "OLD"
/// (`locked` for the rows listed). Built before the session exists, as a file opens.
fn session(locked_rows: &[usize]) -> Session {
    let rows = probe();
    let text: Vec<String> = (0..rows.len()).map(|i| format!("S{:02}", i + 1)).collect();
    let mut doc = wordcraft_doc::Document::from_text(&text.join("\n"));
    for (i, path) in doc.para_paths(StoryRef::Body).into_iter().enumerate() {
        let p = doc.para_mut(StoryRef::Body, &path).unwrap();
        let field = InlineObject::Field {
            instr: rows[i].0.to_string(),
            result: "OLD".into(),
            locked: locked_rows.contains(&i),
            dirty: false,
            code: false,
            form: String::new(),
        };
        p.insert_object(p.len(), field, &CharProps::default()).unwrap();
    }
    Session::new(doc)
}

/// Every body field's stored result, in document order.
fn results(s: &Session) -> Vec<String> {
    let mut out = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        for o in &s.doc.para(StoryRef::Body, &path).unwrap().objects {
            if let InlineObject::Field { result, .. } = o {
                out.push(result.clone());
            }
        }
    }
    out
}

/// What each laid-out paragraph on page 1 shows for its fields (one field per paragraph here).
fn page_lines(s: &mut Session) -> Vec<String> {
    let l = s.layout();
    l.pages[0]
        .items
        .iter()
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Lines { para, .. } => Some(para.shown.iter().map(|x| x.1.clone()).collect::<String>()),
            _ => None,
        })
        .collect()
}

#[test]
fn f9_stores_the_character_word_shows() {
    let mut s = session(&[]);
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.updateFields", json!({"all": true}));
    let want: Vec<String> = probe().into_iter().map(|r| r.1).collect();
    assert_eq!(results(&s), want);
}

#[test]
fn a_stale_result_is_not_shown() {
    let mut s = session(&[]);
    let shown = page_lines(&mut s);
    let want: Vec<String> = probe().into_iter().map(|r| r.1).collect();
    assert_eq!(shown, want, "SYMBOL fields show their character, not the stored text");
    assert_eq!(results(&s), vec!["OLD".to_string(); want.len()], "showing does not change the stored results");
}

#[test]
fn a_locked_field_keeps_its_stored_result() {
    let mut s = session(&[0, 8]);
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.updateFields", json!({"all": true}));
    let got = results(&s);
    assert_eq!(got[0], "OLD");
    assert_eq!(got[8], "OLD");
    assert_eq!(got[1], "©");
    let shown = page_lines(&mut s);
    assert_eq!((shown[0].as_str(), shown[1].as_str(), shown[8].as_str()), ("OLD", "©", "OLD"), "{shown:?}");
}

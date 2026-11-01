//! CJK typing and selection regressions: preservation, grapheme-safe caret, mixed Latin/CJK,
//! supplementary-plane characters. See `docs/cjk-typography-audit.md`.

use serde_json::json;
use wordcraft_doc::Pos;

use crate::Session;

fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn run(s: &mut Session, id: &str, v: serde_json::Value) {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// Type one `text.insert` per Unicode scalar. This does not exercise the IME composition/commit path.
fn typ(s: &mut Session, keys: &str) {
    for c in keys.chars() {
        match c {
            '\n' => run(s, "text.newParagraph", json!({})),
            '\u{8}' => run(s, "text.backspace", json!({})),
            c => run(s, "text.insert", json!({"text": c.to_string()})),
        }
    }
}

fn typed(keys: &str) -> Session {
    let mut s = s();
    typ(&mut s, keys);
    s
}

fn body_text(s: &Session) -> String {
    s.doc.body.iter().find_map(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default()
}

/// Ordinary Japanese survives insert.
#[test]
fn japanese_text_is_preserved_when_typed() {
    let ja = "春はあけぼの。";
    assert_eq!(body_text(&typed(ja)), ja);
}

/// Ordinary Chinese survives insert.
#[test]
fn chinese_text_is_preserved_when_typed() {
    let zh = "学而时习之，不亦说乎？";
    let s = typed(zh);
    assert_eq!(body_text(&s), zh);
}

/// Caret left/right and Backspace move by one ideograph/kana (UAX #29 grapheme).
#[test]
fn japanese_caret_and_backspace_step_by_grapheme() {
    let ja = "日本語";
    let mut s = typed(ja);
    assert_eq!(s.sel.focus.off, ja.len());
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, "日本".len());
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, "日".len());
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, 0);
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus.off, "日".len());
    // Backspace from the end removes one character at a time.
    s.sel = crate::Selection::caret(Pos::body(0, ja.len()));
    typ(&mut s, "\u{8}");
    assert_eq!(body_text(&s), "日本");
    typ(&mut s, "\u{8}");
    assert_eq!(body_text(&s), "日");
}

/// Selection across mixed Latin/CJK covers exact byte ranges of whole characters.
#[test]
fn mixed_latin_cjk_selection_covers_whole_characters() {
    let t = "Hi世界";
    let mut s = typed(t);
    // Select 「世界」 only: caret at end, shift-left twice.
    run(&mut s, "caret.left", json!({"extend": true}));
    run(&mut s, "caret.left", json!({"extend": true}));
    let (a, b) = s.sel.ordered();
    assert_eq!(a.off, "Hi".len());
    assert_eq!(b.off, t.len());
    assert_eq!(&t[a.off..b.off], "世界");
}

/// Supplementary Ideographic Plane character types, steps, and deletes as one cluster.
#[test]
fn supplementary_plane_ideograph_types_and_deletes_as_one_character() {
    let sip = '\u{20000}';
    let mut s = typed(&format!("漢{sip}"));
    assert_eq!(body_text(&s), format!("漢{sip}"));
    assert_eq!(s.sel.focus.off, format!("漢{sip}").len());
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, "漢".len(), "left skips the whole SIP scalar");
    run(&mut s, "caret.right", json!({}));
    // Backspace from the end removes the SIP first, then 漢.
    typ(&mut s, "\u{8}");
    assert_eq!(body_text(&s), "漢");
    typ(&mut s, "\u{8}");
    assert!(body_text(&s).is_empty());
}

/// IVS (base + variation selector) is one caret/Backspace unit (UAX #29 Extend).
/// Variation-glyph selection is not verified here; this only pins text editing behaviour.
#[test]
fn ideographic_variation_sequence_edits_as_one_grapheme() {
    let ivs = "\u{8FBA}\u{E0100}";
    let mut s = typed(&format!("東{ivs}"));
    assert_eq!(body_text(&s), format!("東{ivs}"));
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, "東".len(), "left skips the whole IVS");
    s.sel = crate::Selection::caret(Pos::body(0, format!("東{ivs}").len()));
    typ(&mut s, "\u{8}");
    assert_eq!(body_text(&s), "東", "Backspace removes base+VS together");
}

/// Left/Right step over combining-mark, halfwidth-mark, emoji-ZWJ and decomposed-Hangul sequences
/// as one unit (UAX #29). Backspace/Delete are not asserted: Word's behaviour needs observing.
#[test]
fn caret_steps_over_multi_scalar_sequences_as_one_unit() {
    let units = ["あ", "か\u{3099}", "\u{FF8A}\u{FF9E}", "\u{1F468}\u{200D}\u{1F469}", "\u{1112}\u{1161}\u{11AB}", "い"];
    let text = units.concat();
    let mut s = typed(&text);
    assert_eq!(body_text(&s), text);
    let mut off = text.len();
    for unit in units.iter().rev() {
        run(&mut s, "caret.left", json!({}));
        off -= unit.len();
        assert_eq!(s.sel.focus.off, off, "left over {unit:?}");
    }
    for unit in units {
        run(&mut s, "caret.right", json!({}));
        off += unit.len();
        assert_eq!(s.sel.focus.off, off, "right over {unit:?}");
    }
}

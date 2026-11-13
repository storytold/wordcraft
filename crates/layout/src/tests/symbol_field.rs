//! `SYMBOL` fields stored without a result (ECMA-376 Part 1 §17.16.5.68) show their character.

use crate::fields::{FieldCtx, field_text};

fn shown(instr: &str) -> String {
    field_text(instr, "", &FieldCtx::default()).0
}

#[test]
fn a_symbol_font_code_shows_its_unicode_character() {
    // Word's PDF of a real footer, `Copyright { SYMBOL 211 \f "Symbol" } Microsoft`, shows ©.
    let cases = [
        (r#"SYMBOL 211 \f "Symbol""#, "\u{a9}"),
        (r#"SYMBOL 227 \f "Symbol" \s 10"#, "\u{a9}"),
        (r#"SYMBOL 210 \f "Symbol""#, "\u{ae}"),
        (r#"SYMBOL 212 \f "Symbol""#, "\u{2122}"),
        (r#"SYMBOL 97 \f Symbol"#, "\u{3b1}"),
        (r#"SYMBOL 0x41 \f "Symbol" \h"#, "\u{391}"),
        (r#" SYMBOL 179 \f "Symbol" "#, "\u{2265}"),
    ];
    for (instr, want) in cases {
        assert_eq!(shown(instr), want, "{instr}");
    }
}

#[test]
fn other_codes_are_ansi_or_unicode() {
    let cases = [
        ("SYMBOL 169", "\u{a9}"),
        (r"SYMBOL 128 \a", "\u{20ac}"),
        ("SYMBOL 153", "\u{2122}"),
        (r"SYMBOL 8364 \u", "\u{20ac}"),
        (r"SYMBOL 0x20AC \u", "\u{20ac}"),
        (r#"SYMBOL 65 \f "Times New Roman""#, "A"),
        ("SYMBOL 129", ""),
        ("SYMBOL 0", ""),
        ("SYMBOL 300", "###"),
        (r"SYMBOL 300 \u", "Ĭ"),
        (r"SYMBOL \f Symbol", "###"),
        ("SYMBOL", "###"),
        ("SYMBOL abc", "###"),
    ];
    for (instr, want) in cases {
        assert_eq!(shown(instr), want, "{instr}");
    }
}

#[test]
fn a_stale_result_is_recomputed_and_the_field_is_not_page_dependent() {
    let ctx = FieldCtx::default();
    assert_eq!(field_text(r#"SYMBOL 211 \f "Symbol""#, "x", &ctx), ("\u{a9}".into(), false));
    assert_eq!(field_text(r#"SYMBOL 211 \f "Symbol""#, "", &ctx), ("\u{a9}".into(), false));
}

// Review of #5557: private-use codes, Symbol 0xAD, unmapped codes, control codes, `\j` Shift-JIS,
// and no second symbol-font mapping of the shown character.

fn check(cases: &[(&str, &str)]) {
    for (instr, want) in cases {
        assert_eq!(shown(instr), *want, "{instr}");
    }
}

#[test]
fn private_use_codes_in_a_symbol_font() {
    check(&[
        (r#"SYMBOL 61623 \f "Symbol" \s 10 \h"#, "\u{2022}"),
        (r#"SYMBOL 0xF0B7 \f "Symbol""#, "\u{2022}"),
        (r#"SYMBOL 0xF0A7 \f "Wingdings""#, "\u{25AA}"),
        ("SYMBOL 61623", "###"),
        (r"SYMBOL 61623 \u", "\u{F0B7}"),
    ]);
}

#[test]
fn symbol_0xad_is_the_up_arrow_and_unmapped_codes_show_nothing() {
    check(&[
        (r#"SYMBOL 173 \f "Symbol""#, "\u{2191}"),
        (r#"SYMBOL 0xAD \f "Symbol""#, "\u{2191}"),
        (r#"SYMBOL 0xF0AD \f "Symbol""#, "\u{2191}"),
        (r#"SYMBOL 128 \f "Symbol""#, ""),
        (r#"SYMBOL 0xF080 \f "Symbol""#, ""),
    ]);
}

#[test]
fn control_codes_show_nothing_but_tab() {
    check(&[("SYMBOL 13", ""), ("SYMBOL 10", ""), ("SYMBOL 31", ""), (r"SYMBOL 13 \u", ""), (r#"SYMBOL 0x0D \f "Symbol""#, ""), ("SYMBOL 9", "\t")]);
}

#[test]
fn j_reads_shift_jis() {
    check(&[
        (r"SYMBOL 0x82A0 \j", "\u{3042}"),
        (r"SYMBOL 33440 \j", "\u{3042}"),
        (r"SYMBOL 0x889F \j", "\u{4E9C}"),
        (r"SYMBOL 0x8140 \j", "\u{3000}"),
        (r"SYMBOL 0xB1 \j", "\u{FF71}"),
        (r"SYMBOL 65 \j", "A"),
        (r"SYMBOL 0x82 \j", ""),
        (r"SYMBOL 0x1000000 \j", ""),
    ]);
}

/// The text drawn on the first page when a SYMBOL field with `instr` sits between "a" and "b" in
/// a run set in `font`.
fn drawn_with_field_in(instr: &str, font: &str) -> String {
    use super::*;
    let mut d = Document::from_text("ab");
    let props = wordcraft_doc::props::CharProps { font: Some(font.into()), ..Default::default() };
    let field = InlineObject::Field { instr: instr.into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() };
    d.insert_object(&Pos::body(0, 1), field, &props).unwrap();
    let end = Pos::body(0, 1 + wordcraft_doc::para::OBJ.len_utf8());
    d.format_range(&Pos::body(0, 1), &end, &|c| c.font = Some(font.into())).unwrap();
    let l = lay(&d);
    let items = display::page_display(&d, &l.pages[0], &display::DisplayOptions::default());
    items.iter().filter_map(|i| if let display::Draw::Glyphs { text, .. } = i { Some(text.as_str()) } else { None }).collect()
}

#[test]
fn a_symbol_field_is_not_mapped_again_by_its_runs_symbol_font() {
    // © (U+00A9) from `\f "Symbol"` is final: a run set in a symbol font must not map it as its
    // own code 0xA9 (Symbol 0xA9 is ♥), whether the font is installed or substituted.
    for font in ["Symbol", "Wingdings"] {
        let text = drawn_with_field_in(r#"SYMBOL 211 \f "Symbol""#, font);
        assert!(text.contains('\u{a9}') && !text.contains('\u{2665}'), "{font}: {text:?}");
    }
    let text = drawn_with_field_in(r#"SYMBOL 211 \f "Symbol""#, "Times New Roman");
    assert!(text.contains('\u{a9}'), "{text:?}");
}

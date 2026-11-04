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
        ("SYMBOL", ""),
        ("SYMBOL abc", ""),
    ];
    for (instr, want) in cases {
        assert_eq!(shown(instr), want, "{instr}");
    }
}

#[test]
fn a_stored_result_is_kept_and_the_field_is_not_page_dependent() {
    let ctx = FieldCtx::default();
    assert_eq!(field_text(r#"SYMBOL 211 \f "Symbol""#, "x", &ctx), ("x".into(), false));
    assert_eq!(field_text(r#"SYMBOL 211 \f "Symbol""#, "", &ctx), ("\u{a9}".into(), false));
}

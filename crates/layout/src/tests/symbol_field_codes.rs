//! `SYMBOL` field codes (ISO/IEC 29500-1 §17.16.5.61): signed 16-bit codes from older converted
//! files, and Word's own limits on `\u` (Basic Multilingual Plane only) and `\j` (0x5C and 0x7E).

use crate::fields::symbol_text;

fn check(cases: &[(&str, &str)]) {
    for (instr, want) in cases {
        assert_eq!(symbol_text(instr), *want, "{instr}");
    }
}

#[test]
fn negative_codes_read_as_16_bit() {
    check(&[(r#"SYMBOL -3913 \f "Symbol""#, "\u{2022}"), (r#"SYMBOL -70000 \f "Symbol""#, "###")]);
    assert_eq!(symbol_text(r#"SYMBOL -4004 \f "Wingdings""#), symbol_text(r#"SYMBOL 61532 \f "Wingdings""#));
}

#[test]
fn word_symbol_limits() {
    check(&[
        (r"SYMBOL 128512 \u", "\u{f600}"),
        (r"SYMBOL 0x1F600 \u", "\u{f600}"),
        (r"SYMBOL 92 \j", "\\"),
        (r"SYMBOL 126 \j", "~"),
        (r"SYMBOL 0x82A0 \j", "\u{3042}"),
        // Truncated into a control character or a lone surrogate: nothing.
        (r"SYMBOL 0x1000A \u", ""),
        (r"SYMBOL 0x1D800 \u", ""),
    ]);
}

//! A number is one item in the linear format, as Word reads it (#6229): the base of a script is the
//! whole number before it (`10^3` is `10³`, not `1`, `0³`), and so is a fraction's operand. A number is
//! a run of digits; a `.` or `,` continues it only when a digit follows (`2.5`, `1,000`), and a leading
//! `.` before a digit starts one (`.5`). Letters and signs are separate items. Every expected tree is the
//! one Word for Windows (build 16.0.20527) builds from the same text.

use wordcraft_doc::math::{FracKind, MNode, MRun, ScriptKind, parse_linear};

fn r(t: &str) -> MNode {
    MNode::Run(MRun::new(t))
}

fn a(t: &str) -> Vec<MNode> {
    vec![r(t)]
}

fn sup(base: &str, s: &str) -> MNode {
    MNode::Script { kind: ScriptKind::Sup, base: a(base), sub: Vec::new(), sup: a(s) }
}

fn sub(base: &str, s: &str) -> MNode {
    MNode::Script { kind: ScriptKind::Sub, base: a(base), sub: a(s), sup: Vec::new() }
}

fn frac(num: &str, den: &str) -> MNode {
    MNode::Frac { kind: FracKind::Bar, num: a(num), den: a(den) }
}

#[track_caller]
fn parses(s: &str, want: Vec<MNode>) {
    assert_eq!(parse_linear(s), want, "{s}");
}

#[test]
fn a_whole_number_is_the_base() {
    parses("10^3", vec![sup("10", "3")]);
    parses("123^4", vec![sup("123", "4")]);
    for n in ["2.5", "3.14", "0.5", ".5", "1,000", "1.2.3", "1,2,3"] {
        parses(&format!("{n}^2"), vec![sup(n, "2")]);
    }
    parses("10_i", vec![sub("10", "i")]);
    parses("10_i^2", vec![MNode::Script { kind: ScriptKind::SubSup, base: a("10"), sub: a("i"), sup: a("2") }]);
    parses("10^3_i", vec![MNode::Script { kind: ScriptKind::SubSup, base: a("10"), sub: a("i"), sup: a("3") }]);
    parses("10^-3", vec![sup("10", "-3")]);
    parses("10^(3)", vec![sup("10", "3")]);
}

#[test]
fn what_comes_before_the_number_stays_outside() {
    parses("a10^2", vec![r("a"), sup("10", "2")]);
    parses("x+10^3", vec![r("x+"), sup("10", "3")]);
    parses("-10^2", vec![r("-"), sup("10", "2")]);
    parses("1e10^2", vec![r("1e"), sup("10", "2")]);
    // A point with no digit after it isn't part of the number.
    parses("2.^2", vec![r("2"), sup(".", "2")]);
}

#[test]
fn letters_after_a_number_are_their_own_base() {
    parses("2x^2", vec![r("2"), sup("x", "2")]);
    parses("10x^2", vec![r("10"), sup("x", "2")]);
    parses("x2^3", vec![r("x"), sup("2", "3")]);
    parses("3.14r^2", vec![r("3.14"), sup("r", "2")]);
    parses("2.5x^2", vec![r("2.5"), sup("x", "2")]);
}

#[test]
fn a_number_script_is_read_by_the_same_rule() {
    parses("x^.5", vec![sup("x", ".5")]);
    parses("x^2.5", vec![sup("x", "2.5")]);
    parses("x^0.5", vec![sup("x", "0.5")]);
    parses("x_1,000", vec![sub("x", "1,000")]);
    parses("x_1.2.3", vec![sub("x", "1.2.3")]);
    parses("x^2.", vec![sup("x", "2"), r(".")]);
    // Letters after the digits stay in the script (#1609).
    parses("x_10a", vec![sub("x", "10a")]);
    parses("2^10", vec![sup("2", "10")]);
}

#[test]
fn a_whole_number_is_a_fraction_operand() {
    parses("2.5/3", vec![frac("2.5", "3")]);
    parses("1,000/2", vec![frac("1,000", "2")]);
    parses("x/2.5", vec![frac("x", "2.5")]);
    parses("0.5/0.25", vec![frac("0.5", "0.25")]);
    parses("10/2^3", vec![MNode::Frac { kind: FracKind::Bar, num: a("10"), den: vec![sup("2", "3")] }]);
}

#[test]
fn a_comma_between_digits_in_brackets_joins_them() {
    let [MNode::Run(f), MNode::Delim { beg: Some('('), elems, .. }] = &parse_linear("f(1,2^3)")[..] else { panic!() };
    assert_eq!(f.text, "f");
    assert_eq!(elems, &vec![vec![sup("1,2", "3")]]);
}

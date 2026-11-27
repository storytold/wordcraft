//! The linear format (UnicodeMath) parser: one case per construct in the module doc. Where Word
//! builds the same text (observed with Word's build-up), the expected tree is the one Word builds;
//! where it differs, the case says so.

use super::*;
use crate::math::{ColJc, to_linear};

fn r(t: &str) -> MNode {
    MNode::Run(MRun::new(t))
}

fn a(t: &str) -> Arg {
    vec![r(t)]
}

fn up(t: &str) -> Arg {
    vec![MNode::Run(MRun::plain(t))]
}

fn sup(base: Arg, s: Arg) -> MNode {
    MNode::Script { kind: ScriptKind::Sup, base, sub: Vec::new(), sup: s }
}

fn sub(base: Arg, s: Arg) -> MNode {
    MNode::Script { kind: ScriptKind::Sub, base, sub: s, sup: Vec::new() }
}

fn frac(kind: FracKind, num: Arg, den: Arg) -> MNode {
    MNode::Frac { kind, num, den }
}

/// A delimiter as the parser makes it: one element has no separator.
fn delim(beg: Option<char>, end: Option<char>, elems: Vec<Arg>) -> MNode {
    let sep = (elems.len() > 1).then_some('|');
    MNode::Delim { beg, end, sep, grow: true, shp_match: false, elems }
}

#[track_caller]
fn parses(s: &str, want: Arg) {
    assert_eq!(parse(s), want, "{s}");
}

/// Linear text printed back from the tree parses to the same tree.
#[track_caller]
fn stable(s: &str) {
    let n = parse(s);
    let back = to_linear(&n);
    assert_eq!(parse(&back), n, "{s} → {back}");
}

// --- Fractions. ---------------------------------------------------------------------------------

#[test]
fn fraction_operators() {
    parses("a/b+c", vec![frac(FracKind::Bar, a("a"), a("b")), r("+c")]);
    parses("a⁄b", vec![frac(FracKind::Skewed, a("a"), a("b"))]);
    parses("a∕b", vec![frac(FracKind::Linear, a("a"), a("b"))]);
    parses("a¦b", vec![frac(FracKind::NoBar, a("a"), a("b"))]);
    // A parenthesised operand loses its parentheses; `/` groups to the left (as in Word).
    parses("(a+b)/(c+d)", vec![frac(FracKind::Bar, a("a+b"), a("c+d"))]);
    parses("a/b/c", vec![frac(FracKind::Bar, vec![frac(FracKind::Bar, a("a"), a("b"))], a("c"))]);
    // Placeholders are empty arguments; a missing denominator is empty.
    parses("⬚/⬚", vec![frac(FracKind::Bar, vec![], vec![])]);
    parses("a/", vec![frac(FracKind::Bar, a("a"), vec![])]);
}

// --- Scripts. -----------------------------------------------------------------------------------

#[test]
fn scripts() {
    parses("x^(-1)", vec![sup(a("x"), a("-1"))]);
    parses("x_i^2", vec![MNode::Script { kind: ScriptKind::SubSup, base: a("x"), sub: a("i"), sup: a("2") }]);
    // Either order makes one sub-superscript.
    parses("a^b_c", vec![MNode::Script { kind: ScriptKind::SubSup, base: a("a"), sub: a("c"), sup: a("b") }]);
    parses("x_(i_j)", vec![sub(a("x"), vec![sub(a("i"), a("j"))])]);
}

/// `^`/`_` with nothing before them are pre-scripts (module doc). Word's build-up doesn't make an
/// `m:sPre` from this text.
#[test]
fn pre_scripts() {
    parses("_1^n▒Y", vec![MNode::Script { kind: ScriptKind::Pre, base: a("Y"), sub: a("1"), sup: a("n") }]);
}

// --- Radicals, n-ary operators, functions, limits. ----------------------------------------------

#[test]
fn radicals() {
    parses("√x", vec![MNode::Rad { deg: vec![], deg_hide: true, e: a("x") }]);
    parses("√(3&x)", vec![MNode::Rad { deg: a("3"), deg_hide: false, e: a("x") }]);
    parses("√", vec![MNode::Rad { deg: vec![], deg_hide: true, e: vec![] }]);
}

#[test]
fn nary_operators() {
    let nary = |chr, sub, sup, e| MNode::Nary { chr, lim_loc: None, grow: false, sub_hide: false, sup_hide: false, sub, sup, e };
    parses("∑_(i=1)^n▒i^2", vec![nary('∑', a("i=1"), a("n"), vec![sup(a("i"), a("2"))])]);
    parses("∫_0^1▒f(x)dx", vec![nary('∫', a("0"), a("1"), vec![r("f"), delim(Some('('), Some(')'), vec![a("x")]), r("dx")])]);
}

#[test]
fn function_names() {
    parses("sin x", vec![MNode::Func { name: up("sin"), e: a("x") }]);
    parses("log_2 x", vec![MNode::Func { name: vec![sub(up("log"), a("2"))], e: a("x") }]);
    parses("lim┬(n→∞) a_n", vec![MNode::Func { name: vec![MNode::Lim { upper: false, e: up("lim"), lim: a("n→∞") }], e: vec![sub(a("a"), a("n"))] }]);
}

#[test]
fn limits_on_any_base() {
    parses("x┬k", vec![MNode::Lim { upper: false, e: a("x"), lim: a("k") }]);
    parses("x┴k", vec![MNode::Lim { upper: true, e: a("x"), lim: a("k") }]);
    parses("a/b┬c", vec![frac(FracKind::Bar, a("a"), vec![MNode::Lim { upper: false, e: a("b"), lim: a("c") }])]);
}

// --- Brackets. ----------------------------------------------------------------------------------

#[test]
fn brackets() {
    parses("(a│b)", vec![delim(Some('('), Some(')'), vec![a("a"), a("b")])]);
    parses("|x|+‖v‖", vec![delim(Some('|'), Some('|'), vec![a("x")]), r("+"), delim(Some('‖'), Some('‖'), vec![a("v")])]);
    parses("f'(x)", vec![r("f'"), delim(Some('('), Some(')'), vec![a("x")])]);
    // `├`…`┤` make one-sided or reversed brackets (module doc; Word's build-up leaves `├]a[┤` as
    // text, but builds `{█(x@y)┤`).
    parses("├]a[┤", vec![delim(Some(']'), Some('['), vec![a("a")])]);
    parses("{█(x@y)┤", vec![delim(Some('{'), None, vec![vec![MNode::EqArr { rows: vec![a("x"), a("y")] }]])]);
    // An unclosed bracket closes at the end; stray closers are text.
    parses("(a", vec![delim(Some('('), None, vec![a("a")])]);
    parses("a)", a("a)"));
    parses("]x", a("]x"));
    parses(")a(", vec![r(")a"), delim(Some('('), None, vec![vec![]])]);
}

// --- Prefix structures and accents. -------------------------------------------------------------

#[test]
fn prefix_structures() {
    parses("▭(a)", vec![MNode::BorderBox { hide: [false; 4], strike: [false; 4], e: a("a") }]);
    parses("□(a)", vec![MNode::Boxed { e: a("a") }]);
    parses("¯(ab)", vec![MNode::Bar { top: true, e: a("ab") }]);
    parses("▁(ab)", vec![MNode::Bar { top: false, e: a("ab") }]);
    parses("⏞(a+b)", vec![MNode::GroupChr { chr: '⏞', top: true, e: a("a+b") }]);
    parses("⏟(a+b)", vec![MNode::GroupChr { chr: '⏟', top: false, e: a("a+b") }]);
}

#[test]
fn combining_accents() {
    parses("A\u{20D7}", vec![MNode::Acc { chr: '\u{20D7}', e: a("A") }]);
    parses("x\u{302}", vec![MNode::Acc { chr: '\u{302}', e: a("x") }]);
    assert!(is_combining('\u{302}') && is_combining('\u{20D7}'));
    assert!(!is_combining('x') && !is_combining('^'));
}

// --- Arrays and numbering. ----------------------------------------------------------------------

#[test]
fn arrays() {
    parses("█(a=b@c=d)", vec![MNode::EqArr { rows: vec![a("a=b"), a("c=d")] }]);
    parses("█(a@b", vec![MNode::EqArr { rows: vec![a("a"), a("b")] }]);
    // Column alignments are left to the writer (it writes centred columns).
    parses("■(a&b@c&d)", vec![MNode::Matrix { rows: vec![vec![a("a"), a("b")], vec![a("c"), a("d")]], col_jc: Vec::<ColJc>::new() }]);
}

#[test]
fn a_top_level_hash_numbers_the_equation() {
    let number = delim(Some('('), Some(')'), vec![a("1")]);
    parses("E=mc^2#(1)", vec![MNode::EqArr { rows: vec![vec![r("E=m"), sup(a("c"), a("2")), r("#"), number]] }]);
    assert!(matches!(&parse("a=b#")[..], [MNode::EqArr { rows }] if rows.len() == 1));
    assert!(!matches!(&parse("a=b")[..], [MNode::EqArr { .. }]));
}

// --- Helpers. -----------------------------------------------------------------------------------

#[test]
fn closers_pair_with_openers() {
    for (o, c) in [('(', ')'), ('[', ']'), ('{', '}'), ('⟨', '⟩'), ('〈', '〉'), ('⌈', '⌉'), ('⌊', '⌋'), ('⟦', '⟧'), ('〖', '〗')]
    {
        assert_eq!(closer(o), Some(c));
        assert!(is_closer(c));
    }
    assert_eq!(closer('a'), None);
    assert_eq!(closer('|'), None, "| closes itself, by matching");
}

#[test]
fn strip_parens_unwraps_one_plain_group() {
    let inner = a("a+b");
    assert_eq!(strip_parens(vec![delim(Some('('), Some(')'), vec![inner.clone()])]), inner);
    assert_eq!(strip_parens(vec![delim(None, None, vec![inner.clone()])]), inner, "invisible brackets");
    let square = vec![delim(Some('['), Some(']'), vec![inner.clone()])];
    assert_eq!(strip_parens(square.clone()), square, "only round or invisible brackets");
    let two = vec![delim(Some('('), Some(')'), vec![a("a"), a("b")])];
    assert_eq!(strip_parens(two.clone()), two, "only one element");
    let more = vec![delim(Some('('), Some(')'), vec![inner]), r("c")];
    assert_eq!(strip_parens(more.clone()), more, "only a lone group");
}

// --- Round trip through `to_linear`. ------------------------------------------------------------

#[test]
fn every_construct_prints_back_to_the_same_tree() {
    for s in [
        "a/b+c",
        "(a+b)/(c+d)",
        "a/b/c",
        "a⁄b",
        "a∕b",
        "a¦b",
        "⬚/⬚",
        "x_i^2",
        "a^b_c",
        "x^(-1)",
        "x_(i_j)",
        "_1^n▒Y",
        "√x",
        "√(3&x)",
        "∑_(i=1)^n▒i^2",
        "∫_0^1▒f(x)dx",
        "sin x",
        "log_2 x",
        "lim┬(n→∞) a_n",
        "x┬k",
        "x┴k",
        "a/b┬c",
        "(a│b)",
        "|x|+‖v‖",
        "f'(x)",
        "├]a[┤",
        "{█(x@y)┤",
        "(a",
        ")a(",
        "▭(a)",
        "□(a)",
        "¯(ab)",
        "▁(ab)",
        "⏞(a+b)",
        "⏟(a+b)",
        "A\u{20D7}",
        "x\u{302}",
        "█(a=b@c=d)",
        "■(a&b@c&d)",
        "E=mc^2#(1)",
    ] {
        stable(s);
    }
}

#[test]
fn truncated_input_parses() {
    for s in ["√", "(a", "a/", "█(a@b", "x_", "■(", "├", "┤", "〖", "#", "▒", "/", "^", "_"] {
        let n = parse(s);
        assert_eq!(parse(&to_linear(&n)), n, "{s:?} → {:?}", to_linear(&n));
    }
}

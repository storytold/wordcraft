//! Stacked scripts in the linear format nest to the right, as Word builds them (#6074): the
//! superscript of `a^b^c` is `b^c`, so `a^b^c` is `a^(b^c)`. Mixed scripts still make one
//! sub-superscript, and brackets still group to the left.

use wordcraft_doc::math::{MNode, MRun, ScriptKind, parse_linear};

fn a(t: &str) -> Vec<MNode> {
    vec![MNode::Run(MRun::new(t))]
}

fn sup(base: Vec<MNode>, s: Vec<MNode>) -> MNode {
    MNode::Script { kind: ScriptKind::Sup, base, sub: Vec::new(), sup: s }
}

fn sub(base: Vec<MNode>, s: Vec<MNode>) -> MNode {
    MNode::Script { kind: ScriptKind::Sub, base, sub: s, sup: Vec::new() }
}

#[test]
fn stacked_scripts_nest_to_the_right() {
    assert_eq!(parse_linear("a^b^c"), vec![sup(a("a"), vec![sup(a("b"), a("c"))])]);
    assert_eq!(parse_linear("a_b_c"), vec![sub(a("a"), vec![sub(a("b"), a("c"))])]);
    assert_eq!(parse_linear("x^2^3"), vec![sup(a("x"), vec![sup(a("2"), a("3"))])]);
    assert_eq!(parse_linear("a^b^c^d"), vec![sup(a("a"), vec![sup(a("b"), vec![sup(a("c"), a("d"))])])]);
}

#[test]
fn mixed_scripts_and_brackets_are_unchanged() {
    // `a_b^c` and `a^b_c` are one sub-superscript, as in Word.
    let subsup = vec![MNode::Script { kind: ScriptKind::SubSup, base: a("a"), sub: a("b"), sup: a("c") }];
    assert_eq!(parse_linear("a_b^c"), subsup);
    let subsup = vec![MNode::Script { kind: ScriptKind::SubSup, base: a("a"), sub: a("c"), sup: a("b") }];
    assert_eq!(parse_linear("a^b_c"), subsup);
    // Brackets group to the left, with a visible delimiter, as in Word.
    let [MNode::Script { kind: ScriptKind::Sup, base, sup: c, .. }] = &parse_linear("(a^b)^c")[..] else { panic!() };
    assert_eq!(c, &a("c"));
    assert!(matches!(&base[..], [MNode::Delim { beg: Some('('), elems, .. }] if elems[0] == vec![sup(a("a"), a("b"))]), "{base:?}");
}

#[test]
fn stacked_scripts_on_a_function_name_nest_to_the_right() {
    let name = |s: &str| vec![MNode::Run(MRun::plain(s))];
    let func = |n: Vec<MNode>| vec![MNode::Func { name: n, e: a("x") }];
    assert_eq!(parse_linear("sin^2^3 x"), func(vec![sup(name("sin"), vec![sup(a("2"), a("3"))])]));
    assert_eq!(parse_linear("log_a_b x"), func(vec![sub(name("log"), vec![sub(a("a"), a("b"))])]));
    // Mixed scripts on a name are still one sub-superscript.
    let subsup = MNode::Script { kind: ScriptKind::SubSup, base: name("log"), sub: a("2"), sup: a("3") };
    assert_eq!(parse_linear("log_2^3 x"), func(vec![subsup]));
}

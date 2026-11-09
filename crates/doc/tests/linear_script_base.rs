//! A script whose base is itself a script survives linear text (#6075). `to_linear` puts a base that
//! isn't simple in invisible brackets, `〖a^b〗^c`, as Word linearizes it, and the parser treats
//! `〖…〗` as a script base as grouping only, as Word builds it: no delimiter is left.

use wordcraft_doc::math::{MNode, MRun, ScriptKind, parse_linear, to_linear};

fn a(t: &str) -> Vec<MNode> {
    vec![MNode::Run(MRun::new(t))]
}

fn sup(base: Vec<MNode>, s: Vec<MNode>) -> MNode {
    MNode::Script { kind: ScriptKind::Sup, base, sub: Vec::new(), sup: s }
}

#[test]
fn a_script_on_a_script_prints_back_the_same() {
    // sSup(sSup(a, b), c), as Word files hold it.
    let left = vec![sup(vec![sup(a("a"), a("b"))], a("c"))];
    assert_eq!(to_linear(&left), "〖a^b〗^c");
    assert_eq!(parse_linear(&to_linear(&left)), left);
    // Subscripts and a longer base too.
    let sub =
        vec![MNode::Script { kind: ScriptKind::Sub, base: vec![sup(a("x"), a("2")), MNode::Run(MRun::new("+y"))], sub: a("i"), sup: Vec::new() }];
    assert_eq!(parse_linear(&to_linear(&sub)), sub, "{}", to_linear(&sub));
}

#[test]
fn invisible_brackets_only_group_a_script_base() {
    assert_eq!(parse_linear("〖a+b〗^2"), vec![sup(a("a+b"), a("2"))]);
    // Visible brackets stay.
    let [MNode::Script { base, .. }] = &parse_linear("(a+b)^2")[..] else { panic!() };
    assert!(matches!(&base[..], [MNode::Delim { beg: Some('('), .. }]), "{base:?}");
}

#[test]
fn half_typed_scripts_print_back_the_same() {
    for s in ["x^2^", "x_i_", "(a^b)^c", "a^b"] {
        let n = parse_linear(s);
        assert_eq!(parse_linear(&to_linear(&n)), n, "{s} → {}", to_linear(&n));
    }
}

#[test]
fn an_invisible_delimiter_as_the_base_is_kept() {
    // A base that is itself an invisible delimiter (LaTeX `\left.a\right.^2` makes one) is written
    // `〖〖a〗〗^2`: the outer pair only groups, the inner one reads back as that delimiter.
    let invisible = MNode::Delim { beg: None, end: None, sep: None, grow: true, shp_match: false, elems: vec![a("a")] };
    let n = vec![sup(vec![invisible], a("2"))];
    assert_eq!(parse_linear(&to_linear(&n)), n, "{}", to_linear(&n));
    let latex = wordcraft_doc::math_latex::parse_latex(r"\left.a\right.^2");
    assert!(matches!(&latex[..], [MNode::Script { base, .. }] if matches!(&base[..], [MNode::Delim { beg: None, end: None, .. }])), "{latex:?}");
    assert_eq!(parse_linear(&to_linear(&latex)), latex, "{}", to_linear(&latex));
}

#[test]
fn an_operator_as_the_base_prints_back_the_same() {
    // A script after an operator reads as a pre-script, so an operator base is grouped: `〖⊗〗_i`.
    for op in ["⊗", "×", "∗", "*", "<", "&", "#"] {
        let n = vec![MNode::Script { kind: ScriptKind::Sub, base: a(op), sub: a("i"), sup: Vec::new() }];
        assert_eq!(parse_linear(&to_linear(&n)), n, "{op}: {}", to_linear(&n));
        let n = vec![sup(a(op), a("2"))];
        assert_eq!(parse_linear(&to_linear(&n)), n, "{op}: {}", to_linear(&n));
    }
}

//! A script base of several letters survives linear text (#6228). The parser attaches a script to the
//! last character before it (`ab^2` is `a`, `b²`), so `to_linear` writes such a base in invisible
//! brackets, `〖mc〗^2`, as Word linearizes it. Single characters, numbers and upright function
//! names stay bare.

use wordcraft_doc::math::{MNode, MRun, ScriptKind, parse_linear, to_linear};

fn a(t: &str) -> Vec<MNode> {
    vec![MNode::Run(MRun::new(t))]
}

fn sup(base: Vec<MNode>, s: Vec<MNode>) -> MNode {
    MNode::Script { kind: ScriptKind::Sup, base, sub: Vec::new(), sup: s }
}

#[test]
fn a_base_of_several_letters_prints_back_whole() {
    for base in ["ab", "mc", "x1", "αβ"] {
        let n = vec![sup(a(base), a("2"))];
        assert_eq!(to_linear(&n), format!("〖{base}〗^2"));
        assert_eq!(parse_linear(&to_linear(&n)), n, "{base}");
    }
    // After other text, and as the base of a base.
    let e = vec![MNode::Run(MRun::new("E=")), sup(a("mc"), a("2"))];
    assert_eq!(to_linear(&e), "E=〖mc〗^2");
    assert_eq!(parse_linear(&to_linear(&e)), e);
    let nested = vec![sup(vec![sup(a("ab"), a("2"))], a("3"))];
    assert_eq!(parse_linear(&to_linear(&nested)), nested, "{}", to_linear(&nested));
}

#[test]
fn single_characters_numbers_and_function_names_stay_bare() {
    assert_eq!(to_linear(&[sup(a("x"), a("2"))]), "x^2");
    // Word writes a number base bare too (`10^3`).
    assert_eq!(to_linear(&[sup(a("10"), a("3"))]), "10^3");
    assert_eq!(to_linear(&[sup(a("2.5"), a("2"))]), "2.5^2");
    let sin = vec![sup(vec![MNode::Run(MRun::plain("sin"))], a("2"))];
    assert_eq!(to_linear(&sin), "sin^2");
}

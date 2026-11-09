//! An n-ary operator's empty but shown limit survives linear text (#6076): half-typed `∑_` has an
//! empty lower limit that is shown, and `to_linear` writes it as `_()` rather than leaving it out,
//! which read back as a hidden limit.

use wordcraft_doc::math::{MNode, MRun, parse_linear, to_linear};

#[test]
fn an_empty_shown_limit_prints_back_the_same() {
    for s in ["∑_", "∑^", "∫_0^▒x", "∑_()^n▒i"] {
        let n = parse_linear(s);
        assert_eq!(parse_linear(&to_linear(&n)), n, "{s} → {}", to_linear(&n));
    }
}

#[test]
fn hidden_and_filled_limits_print_as_before() {
    let nary = |sub_hide, sup_hide, sub: &str, sup: &str| {
        let arg = |t: &str| if t.is_empty() { Vec::new() } else { vec![MNode::Run(MRun::new(t))] };
        vec![MNode::Nary { chr: '∑', lim_loc: None, grow: false, sub_hide, sup_hide, sub: arg(sub), sup: arg(sup), e: arg("x") }]
    };
    assert_eq!(to_linear(&nary(true, true, "", "")), "∑▒x");
    assert_eq!(to_linear(&nary(false, false, "i", "n")), "∑_i^n▒x");
    assert_eq!(to_linear(&nary(false, true, "", "")), "∑_()▒x");
}

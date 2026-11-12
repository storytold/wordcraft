use super::super::{format, mapping::Mapping, trace::Sample};
use std::collections::HashMap;

/// Parse a trace-format fixture using the shared work allowance.
fn format(channels: &str, work: &mut usize) -> Option<format::Format> {
    let xml = format!(r#"<traceFormat xmlns="http://www.w3.org/2003/InkML">{channels}</traceFormat>"#);
    format::read(&crate::xml::parse_strict(xml.as_bytes()).ok()?, work)
}

/// Compile a forward or reverse-only transform with explicit source and target channels.
fn compile(mapping: &str, source: &str, target: &str, inverse: bool, work: &mut usize) -> Option<Mapping> {
    let unknown = if inverse { r#"<mapping type="unknown"/>"# } else { "" };
    let xml = format!(
        r#"<canvasTransform xmlns="http://www.w3.org/2003/InkML" xmlns:m="http://www.w3.org/1998/Math/MathML">{unknown}{mapping}</canvasTransform>"#
    );
    let root = crate::xml::parse_strict(xml.as_bytes()).ok()?;
    Mapping::transform(&root, &format(source, work)?, &format(target, work)?, &HashMap::new(), work)
}

/// Evaluate a mapping into a fresh sample with the requested output channel count.
fn apply(map: &Mapping, input: &[Option<f64>], outputs: usize, work: &mut usize) -> Option<Sample> {
    let mut output = vec![None; outputs];
    map.apply(&input.to_vec(), &mut output, work)?;
    Some(output)
}

/// Wrap variable bindings and a MathML expression in a mapping fixture.
fn formula(bindings: &str, expression: &str) -> String {
    format!(r#"<mapping type="mathml">{bindings}<m:math>{expression}</m:math></mapping>"#)
}

/// Build a table mapping with explicit bindings, policies and rows.
fn table(bindings: &str, attrs: &str, rows: &str) -> String {
    format!(r#"<mapping type="table">{bindings}<table {attrs}>{rows}</table></mapping>"#)
}

const ONE: &str = r#"<channel name="X"/>"#;
const BIT: &str = r#"<channel name="X" type="boolean"/>"#;
const INDEX: &str = r#"<channel name="X" type="integer" min="0" max="3"/>"#;
const PAIR: &str = r#"<bind source="X" column="1"/><bind target="X" column="2"/>"#;
const VARIABLE: &str = r#"<bind source="X" variable="x"/><bind target="X"/>"#;

/// Boolean tables use exact states and reject interpolation, numeric Boolean rows and duplicates.
#[test]
fn boolean_tables_use_exact_lookup_without_implicit_interpolation() {
    let mapping = table(PAIR, "", "F T,T F");
    let map = compile(&mapping, ONE, BIT, false, &mut 1_000_000).unwrap();
    for (value, expected) in [(0.0, 1.0), (1.0, 0.0)] {
        assert_eq!(apply(&map, &[Some(value)], 1, &mut 100_000), Some(vec![Some(expected)]));
    }
    for value in [-1.0, 0.25, 0.5, 0.75, 2.0, f64::NAN, f64::INFINITY] {
        assert!(apply(&map, &[Some(value)], 1, &mut 100_000).is_none());
    }
    for attrs in [r#"interpolation="linear""#, r#"interpolation="floor""#, r#"interpolation="cubic""#] {
        assert!(compile(&table(PAIR, attrs, "F T,T F"), BIT, BIT, false, &mut 1_000_000).is_none());
    }
    for rows in ["F 1,T 0", "0 T,1 F", "F T,T F,F F"] {
        assert!(compile(&table(PAIR, "", rows), BIT, BIT, false, &mut 1_000_000).is_none());
    }
}

/// Boolean tensor lookup requires a complete grid and known, exact source states.
#[test]
fn boolean_tensor_tables_require_complete_grids_and_exact_source_values() {
    let source = r#"<channel name="A" type="boolean"/><channel name="B" type="boolean"/>"#;
    let target = r#"<channel name="S" type="boolean"/>"#;
    let bindings = r#"<bind source="A" column="1"/><bind source="B" column="2"/><bind target="S" column="3"/>"#;
    let map = compile(&table(bindings, "", "F F F,F T T,T F T,T T F"), source, target, false, &mut 1_000_000).unwrap();
    for a in [0.0, 1.0] {
        for b in [0.0, 1.0] {
            assert_eq!(apply(&map, &[Some(a), Some(b)], 1, &mut 100_000), Some(vec![Some(if a != b { 1.0 } else { 0.0 })]));
        }
    }
    assert!(apply(&map, &[Some(0.5), Some(1.0)], 1, &mut 100_000).is_none());
    assert!(apply(&map, &[Some(1.0), None], 1, &mut 100_000).is_none());
    assert!(compile(&table(bindings, "", "F F F,F T T,T F T"), source, target, false, &mut 1_000_000).is_none());
}

/// Boolean formula and table inverses recover both states but reject unknown or fractional input.
#[test]
fn inverse_boolean_formulas_and_tables_recover_both_states() {
    let mappings = [formula(VARIABLE, "<m:apply><m:not/><m:ci>x</m:ci></m:apply>"), table(PAIR, "", "F T,T F")];
    for mapping in mappings {
        let map = compile(&mapping, BIT, BIT, true, &mut 1_000_000).unwrap();
        for (input, expected) in [(0.0, 1.0), (1.0, 0.0)] {
            assert_eq!(apply(&map, &[Some(input)], 1, &mut 100_000), Some(vec![Some(expected)]));
        }
        assert!(apply(&map, &[None], 1, &mut 100_000).is_none());
        assert!(apply(&map, &[Some(0.5)], 1, &mut 100_000).is_none());
    }
}

/// Transitive Boolean dependencies invert jointly regardless of product order or nesting.
#[test]
fn inverse_coupled_boolean_products_join_transitive_dependencies() {
    let canvas = r#"<channel name="A" type="boolean"/><channel name="B" type="boolean"/><channel name="C" type="boolean"/>"#;
    let recorded = r#"<channel name="P" type="boolean"/><channel name="Q" type="boolean"/><channel name="R" type="boolean"/>"#;
    let p = formula(
        r#"<bind source="A" variable="a"/><bind source="B" variable="b"/><bind target="P"/>"#,
        "<m:apply><m:xor/><m:ci>a</m:ci><m:ci>b</m:ci></m:apply>",
    );
    let q = formula(
        r#"<bind source="B" variable="b"/><bind source="C" variable="c"/><bind target="Q"/>"#,
        "<m:apply><m:xor/><m:ci>b</m:ci><m:ci>c</m:ci></m:apply>",
    );
    let r = formula(r#"<bind source="C" variable="c"/><bind target="R"/>"#, "<m:ci>c</m:ci>");
    for body in [format!("{p}{r}{q}"), format!("<mapping type=\"product\">{q}{r}</mapping>{p}")] {
        let mapping = format!(r#"<mapping type="product">{body}</mapping>"#);
        let inverse = compile(&mapping, recorded, canvas, true, &mut 1_000_000).unwrap();
        let forward = compile(&mapping, canvas, recorded, false, &mut 1_000_000).unwrap();
        for bits in 0..8 {
            let values: Vec<_> = (0..3).map(|i| Some(f64::from((bits >> i) & 1))).collect();
            let observed = apply(&forward, &values, 3, &mut 100_000).unwrap();
            assert_eq!(apply(&inverse, &observed, 3, &mut 100_000), Some(values));
        }
        assert!(apply(&inverse, &[Some(1.0), None, Some(1.0)], 3, &mut 100_000).is_none());
    }
}

/// Mixed products recover continuous coordinates and discrete contact without fabricating missing states.
#[test]
fn inverse_mixed_products_keep_continuous_coordinate_and_discrete_contact_components() {
    let canvas = r#"<channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="S" type="boolean"/>"#;
    let recorded = r#"<channel name="U"/><channel name="V"/><channel name="P" type="boolean"/>"#;
    let coordinates = r#"<mapping type="affine"><bind source="X" column="1"/><bind source="Y" column="2"/><bind target="U" column="1"/><bind target="V" column="2"/><affine>2 1 0,1 -1 0</affine></mapping>"#;
    let contact = formula(r#"<bind source="S" variable="s"/><bind target="P"/>"#, "<m:apply><m:not/><m:ci>s</m:ci></m:apply>");
    let map = compile(
        &format!(r#"<mapping type="product"><mapping type="product">{contact}</mapping>{coordinates}</mapping>"#),
        recorded,
        canvas,
        true,
        &mut 1_000_000,
    )
    .unwrap();
    let result = apply(&map, &[Some(14.0), Some(1.0), Some(0.0)], 3, &mut 100_000).unwrap();
    assert!((result[0].unwrap() - 5.0).abs() < 1e-12 && (result[1].unwrap() - 4.0).abs() < 1e-12);
    assert_eq!(result[2], Some(1.0));
    assert!(apply(&map, &[Some(14.0), Some(1.0), None], 3, &mut 100_000).is_none());
}

/// Constant observations constrain an inverse but cannot create a recovered coordinate by themselves.
#[test]
fn inverse_products_check_constant_observations_without_inventing_coordinates() {
    let recorded = r#"<channel name="X"/><channel name="Z" type="integer"/>"#;
    let varying = r#"<mapping type="affine"><bind source="X" column="1"/><bind target="X" column="1"/><affine>2 0</affine></mapping>"#;
    let constant = formula(r#"<bind target="Z"/>"#, "<m:cn>7.2</m:cn>");
    let map = compile(&format!(r#"<mapping type="product">{constant}{varying}</mapping>"#), recorded, ONE, true, &mut 1_000_000).unwrap();
    assert_eq!(apply(&map, &[Some(4.0), Some(7.0)], 1, &mut 100_000), Some(vec![Some(2.0)]));
    assert_eq!(apply(&map, &[Some(4.0), None], 1, &mut 100_000), Some(vec![Some(2.0)]));
    assert!(apply(&map, &[Some(4.0), Some(8.0)], 1, &mut 100_000).is_none());
    assert!(compile(&format!(r#"<mapping type="product">{constant}</mapping>"#), recorded, ONE, true, &mut 1_000_000).is_none());
}

/// Nonlinear integer inversion enumerates declared discrete domains rather than guessing continuous roots.
#[test]
fn inverse_integer_formulas_use_declared_discrete_domains_not_continuous_guesses() {
    let expression = "<m:apply><m:plus/><m:apply><m:times/><m:ci>x</m:ci><m:ci>x</m:ci></m:apply><m:ci>x</m:ci></m:apply>";
    let mapping = formula(VARIABLE, expression);
    let map = compile(&mapping, ONE, INDEX, true, &mut 1_000_000).unwrap();
    for x in 0..4 {
        assert_eq!(apply(&map, &[Some(f64::from(x * x + x))], 1, &mut 100_000), Some(vec![Some(f64::from(x))]));
    }
    assert!(apply(&map, &[Some(3.0)], 1, &mut 100_000).is_none());
    assert!(compile(&mapping, ONE, &INDEX.replace(" type=\"integer\"", ""), true, &mut 1_000_000).is_none());
    assert!(compile(&mapping, ONE, &INDEX.replace(" max=\"3\"", ""), true, &mut 1_000_000).is_none());
    assert!(compile(&mapping, ONE, &INDEX.replace(" min=\"0\"", ""), true, &mut 1_000_000).is_none());
    let canvas = INDEX.replace("min=\"0\" max=\"3\"", "min=\".1\" max=\"2.9\"");
    let map = compile(&mapping, ONE, &canvas, true, &mut 1_000_000).unwrap();
    assert_eq!(apply(&map, &[Some(6.0)], 1, &mut 100_000), Some(vec![Some(2.0)]));
    assert!(apply(&map, &[Some(0.0)], 1, &mut 100_000).is_none());
}

/// Finite-domain inversion supports nonmonotone and step tables when a unique preimage exists.
#[test]
fn inverse_nonmonotone_and_step_tables_have_unique_finite_preimages() {
    for policy in ["linear", "floor", "middle", "ceiling"] {
        let map = compile(&table(PAIR, &format!(r#"interpolation="{policy}""#), "0 2,1 0,2 3,3 1"), ONE, INDEX, true, &mut 1_000_000).unwrap();
        for (input, expected) in [(2.0, 0.0), (0.0, 1.0), (3.0, 2.0), (1.0, 3.0)] {
            assert_eq!(apply(&map, &[Some(input)], 1, &mut 100_000), Some(vec![Some(expected)]));
        }
        assert!(apply(&map, &[Some(1.5)], 1, &mut 100_000).is_none());
    }
    let map = compile(&table(PAIR, r#"apply="relative""#, "0 3,1 -1,2 -1,3 -1"), ONE, INDEX, true, &mut 1_000_000).unwrap();
    assert_eq!(apply(&map, &[Some(3.0)], 1, &mut 100_000), Some(vec![Some(0.0)]));
}

/// Quantized observations recover only unique preimages and reject collisions or absent states.
#[test]
fn inverse_quantized_outputs_reject_collisions_but_keep_unique_preimages() {
    let recorded = r#"<channel name="X" type="integer"/>"#;
    let map = compile(r#"<mapping type="affine"><affine>.6 0</affine></mapping>"#, recorded, &INDEX.replace("3", "2"), true, &mut 1_000_000).unwrap();
    assert_eq!(apply(&map, &[Some(0.0)], 1, &mut 100_000), Some(vec![Some(0.0)]));
    assert!(apply(&map, &[Some(1.0)], 1, &mut 100_000).is_none());
    assert!(apply(&map, &[Some(2.0)], 1, &mut 100_000).is_none());
    assert!(apply(&map, &[None], 1, &mut 100_000).is_none());
    let canvas = r#"<channel name="X" type="integer" min="-2" max="2"/>"#;
    let map = compile(r#"<mapping type="affine"><affine>.5 0</affine></mapping>"#, recorded, canvas, true, &mut 1_000_000).unwrap();
    assert_eq!(apply(&map, &[Some(0.0)], 1, &mut 100_000), Some(vec![Some(0.0)]));
    for value in [-1.0, 1.0] {
        assert!(apply(&map, &[Some(value)], 1, &mut 100_000).is_none());
    }
}

/// Joint quotient and remainder observations resolve quantization that either output alone cannot.
#[test]
fn inverse_joint_discrete_observations_can_resolve_individual_quantization() {
    let recorded = r#"<channel name="P" type="integer"/><channel name="Q" type="integer"/>"#;
    let p = formula(r#"<bind source="X" variable="x"/><bind target="P"/>"#, "<m:apply><m:quotient/><m:ci>x</m:ci><m:cn>2</m:cn></m:apply>");
    let q = formula(r#"<bind source="X" variable="x"/><bind target="Q"/>"#, "<m:apply><m:rem/><m:ci>x</m:ci><m:cn>2</m:cn></m:apply>");
    let map = compile(&format!(r#"<mapping type="product">{p}{q}</mapping>"#), recorded, INDEX, true, &mut 1_000_000).unwrap();
    for x in 0..4 {
        assert_eq!(apply(&map, &[Some(f64::from(x / 2)), Some(f64::from(x % 2))], 1, &mut 100_000), Some(vec![Some(f64::from(x))]));
    }
    assert!(apply(&map, &[Some(1.0), None], 1, &mut 100_000).is_none());
    assert!(apply(&map, &[Some(0.0), Some(2.0)], 1, &mut 100_000).is_none());
}

/// Missing observations and floating-point near-collisions never select an ambiguous discrete state.
#[test]
fn inverse_discrete_constraints_treat_missing_observations_and_near_collisions_conservatively() {
    let recorded = r#"<channel name="P"/><channel name="Q"/>"#;
    let bindings = r#"<bind source="X" column="1"/><bind target="P" column="2"/><bind target="Q" column="3"/>"#;
    let map = compile(&table(bindings, "", "0 42 2,1 42 0,2 42 3,3 42 1"), recorded, INDEX, true, &mut 1_000_000).unwrap();
    assert_eq!(apply(&map, &[None, Some(3.0)], 1, &mut 100_000), Some(vec![Some(2.0)]));
    assert!(apply(&map, &[Some(41.0), Some(3.0)], 1, &mut 100_000).is_none());
    assert!(apply(&map, &[Some(42.0), None], 1, &mut 100_000).is_none());
    let domain = INDEX.replace("3", "1");
    let map = compile(r#"<mapping type="affine"><affine>1e-15 1</affine></mapping>"#, ONE, &domain, true, &mut 1_000_000).unwrap();
    assert!(apply(&map, &[Some(1.0)], 1, &mut 100_000).is_none());
    let map = compile(r#"<mapping type="affine"><affine>1e-12 1</affine></mapping>"#, ONE, &domain, true, &mut 1_000_000).unwrap();
    assert_eq!(apply(&map, &[Some(1.0)], 1, &mut 100_000), Some(vec![Some(0.0)]));
}

/// Finite-domain proofs reject any candidate whose forward evaluation or normalization fails.
#[test]
fn inverse_finite_domain_proofs_refuse_unevaluated_candidates() {
    let recorded = r#"<channel name="X" type="integer"/>"#;
    let mapping = formula(VARIABLE, "<m:apply><m:divide/><m:cn>1</m:cn><m:ci>x</m:ci></m:apply>");
    let canvas = r#"<channel name="X" type="integer" min="-1" max="1"/>"#;
    assert!(compile(&mapping, recorded, canvas, true, &mut 1_000_000).is_none());
    let large = "<m:apply><m:power/><m:cn>2</m:cn><m:apply><m:times/><m:ci>x</m:ci><m:cn>100</m:cn></m:apply></m:apply>";
    let expression = format!("<m:apply><m:minus/>{large}{large}</m:apply>");
    let canvas = INDEX.replace("3", "1");
    assert!(compile(&formula(VARIABLE, &expression), recorded, &canvas, true, &mut 1_000_000).is_none());
    let mapping = formula(VARIABLE, "<m:apply><m:floor/><m:ci>x</m:ci></m:apply>");
    let recorded = r#"<channel name="X" type="integer" min="1" max="3"/>"#;
    assert!(compile(&mapping, recorded, INDEX, true, &mut 1_000_000).is_none());
}

/// Build Boolean channels and a formula encoding their bit pattern as one observed number.
fn encoded_bits(bits: usize) -> (String, String) {
    let mut channels = String::new();
    let mut bindings = String::from(r#"<bind target="X"/>"#);
    let mut terms = String::new();
    for i in 0..bits {
        channels.push_str(&format!(r#"<channel name="B{i}" type="boolean"/>"#));
        bindings.push_str(&format!(r#"<bind source="B{i}" variable="b{i}"/>"#));
        terms.push_str(&format!("<m:apply><m:times/><m:ci>b{i}</m:ci><m:cn>{}</m:cn></m:apply>", 1 << i));
    }
    (channels, formula(&bindings, &format!("<m:apply><m:plus/>{terms}</m:apply>")))
}

/// Finite-domain enumeration enforces its state cap even when more work is available.
#[test]
fn inverse_finite_domain_cardinality_is_bounded_independently_of_the_work_allowance() {
    let (canvas, mapping) = encoded_bits(10);
    let map = compile(&mapping, ONE, &canvas, true, &mut 2_000_000).unwrap();
    assert_eq!(apply(&map, &[Some(513.0)], 10, &mut 100_000), Some((0..10).map(|i| Some(f64::from((513 >> i) & 1))).collect()));
    let (canvas, mapping) = encoded_bits(11);
    assert!(compile(&mapping, ONE, &canvas, true, &mut 2_000_000).is_none());
}

/// Discrete compilation and evaluation charge exact work on both successful and failed operations.
#[test]
fn discrete_compilation_and_evaluation_debit_success_and_failure_work() {
    let mapping = formula(VARIABLE, "<m:apply><m:plus/><m:apply><m:times/><m:ci>x</m:ci><m:ci>x</m:ci></m:apply><m:ci>x</m:ci></m:apply>");
    let canvas = INDEX;
    let mut work = 1_000_000;
    let map = compile(&mapping, ONE, canvas, true, &mut work).unwrap();
    let used = 1_000_000 - work;
    assert!(compile(&mapping, ONE, canvas, true, &mut (used - 1)).is_none());
    let mut exact = used;
    assert!(compile(&mapping, ONE, canvas, true, &mut exact).is_some());
    let mut work = 100_000;
    assert!(apply(&map, &[Some(2.0)], 1, &mut work).is_some());
    let used = 100_000 - work;
    assert!(apply(&map, &[Some(2.0)], 1, &mut (used - 1)).is_none());
    let mut exact = used;
    assert!(apply(&map, &[Some(2.0)], 1, &mut exact).is_some());
    let before = work;
    assert!(apply(&map, &[Some(1.0)], 1, &mut work).is_none());
    assert!(work < before);
}

proptest::proptest! {
    /// Generated finite tables recover a state only when its observed value has a unique preimage.
    #[test]
    fn finite_table_inverse_never_guesses_among_colliding_preimages(values in proptest::collection::vec(0u8..8, 1..9)) {
        let rows=values.iter().enumerate().map(|(i,v)|format!("{i} {v}")).collect::<Vec<_>>().join(",");
        let canvas=format!(r#"<channel name="X" type="integer" min="0" max="{}"/>"#,values.len()-1);
        let map=compile(&table(PAIR,"",&rows),ONE,&canvas,true,&mut 1_000_000).unwrap();
        for value in 0u8..8 {
            let mut candidates=values.iter().enumerate().filter(|(_,v)|**v==value);
            let candidate=candidates.next();
            let expected=if candidates.next().is_none() {candidate.map(|(i,_)|vec![Some(i as f64)])} else {None};
            proptest::prop_assert_eq!(apply(&map,&[Some(f64::from(value))],1,&mut 100_000),expected);
        }
    }
}

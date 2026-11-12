use super::super::{format, mapping::Mapping};
use std::collections::HashMap;

/// Parse declared channels, or use the application format for an empty fixture.
fn format(channels: &str, work: &mut usize) -> Option<format::Format> {
    if channels.is_empty() {
        return Some(format::Format::default());
    }
    let xml = format!(r#"<traceFormat xmlns="http://www.w3.org/2003/InkML">{channels}</traceFormat>"#);
    format::read(&crate::xml::parse_strict(xml.as_bytes()).ok()?, work)
}

/// Compile a reverse-only canvas transform against canvas and recorded formats.
fn compile(reverse: &str, canvas: &str, recorded: &str, work: &mut usize) -> Option<Mapping> {
    let xml = format!(
        r#"<canvasTransform xmlns="http://www.w3.org/2003/InkML" xmlns:m="http://www.w3.org/1998/Math/MathML"><mapping type="unknown"/>{reverse}</canvasTransform>"#
    );
    let root = crate::xml::parse_strict(xml.as_bytes()).ok()?;
    Mapping::transform(&root, &format(recorded, work)?, &format(canvas, work)?, &HashMap::new(), work)
}

/// Apply a two-coordinate mapping and require both recovered values.
fn sample(map: &Mapping, x: f64, y: f64, work: &mut usize) -> Option<[f64; 2]> {
    let mut output = vec![None; 2];
    map.apply(&vec![Some(x), Some(y)], &mut output, work)?;
    Some([output.first().copied().flatten()?, output.get(1).copied().flatten()?])
}

/// Build canvas channels with custom X constraints and a default Y channel.
fn canvas(attrs: &str) -> String {
    format!(r#"<channel name="X" {attrs}/><channel name="Y"/>"#)
}
/// Build a product applying a MathML expression to X and preserving Y.
fn formula(expression: &str) -> String {
    format!(
        r#"<mapping type="product"><mapping type="mathml"><bind source="X" variable="x"/><bind target="X"/><m:math>{expression}</m:math></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping>"#
    )
}
/// Build an X table transform with interpolation and relative policies, preserving Y.
fn table(rows: &str, interpolation: &str, apply: &str) -> String {
    format!(
        r#"<mapping type="product"><mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="{interpolation}" apply="{apply}">{rows}</table></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping>"#
    )
}
/// Compare recovered coordinates with a scale-relative tolerance.
fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9 * expected.abs().max(1.0), "{actual} != {expected}");
}

/// Linear MathML products solve repeated variables, cross-coupling and constant offsets.
#[test]
fn inverse_mathml_affine_products_solve_repeated_and_cross_coupled_variables() {
    let reverse = r#"<mapping type="product"><mapping type="mathml"><bind source="X" variable="x"/><bind source="Y" variable="y"/><bind target="X"/><m:math><m:apply><m:plus/><m:ci>x</m:ci><m:ci>x</m:ci><m:ci>y</m:ci></m:apply></m:math></mapping><mapping type="mathml"><bind source="X" variable="x"/><bind source="Y" variable="y"/><bind target="Y"/><m:math><m:apply><m:minus/><m:ci>x</m:ci><m:ci>y</m:ci></m:apply></m:math></mapping></mapping>"#;
    let map = compile(reverse, "", "", &mut 1_000_000).unwrap();
    let [x, y] = sample(&map, 14.0, 1.0, &mut 100_000).unwrap();
    close(x, 5.0);
    close(y, 4.0);
    let map = compile(
        &formula("<m:apply><m:divide/><m:apply><m:plus/><m:ci>x</m:ci><m:cn>3</m:cn></m:apply><m:cn>2</m:cn></m:apply>"),
        "",
        "",
        &mut 1_000_000,
    )
    .unwrap();
    close(sample(&map, 4.0, 7.0, &mut 100_000).unwrap()[0], 5.0);
}

/// Supported exponential, logarithmic, reciprocal and odd-power inverses recover signed values.
#[test]
fn inverse_mathml_exponentials_logarithms_reciprocals_and_odd_powers() {
    for (expression, input, expected) in [
        ("<m:apply><m:exp/><m:ci>x</m:ci></m:apply>", 2.0f64.exp(), 2.0),
        ("<m:apply><m:ln/><m:ci>x</m:ci></m:apply>", 2.0, 2.0f64.exp()),
        ("<m:apply><m:log/><m:logbase><m:cn>2</m:cn></m:logbase><m:ci>x</m:ci></m:apply>", 3.0, 8.0),
        ("<m:apply><m:power/><m:cn>2</m:cn><m:ci>x</m:ci></m:apply>", 8.0, 3.0),
        ("<m:apply><m:divide/><m:cn>4</m:cn><m:ci>x</m:ci></m:apply>", -2.0, -2.0),
        ("<m:apply><m:power/><m:ci>x</m:ci><m:cn>3</m:cn></m:apply>", -8.0, -2.0),
        ("<m:apply><m:root/><m:degree><m:cn>3</m:cn></m:degree><m:ci>x</m:ci></m:apply>", -2.0, -8.0),
        ("<m:apply><m:root/><m:degree><m:cn>0.3333333333333333</m:cn></m:degree><m:ci>x</m:ci></m:apply>", -8.0, -2.0),
        ("<m:apply><m:power/><m:ci>x</m:ci><m:cn>-3</m:cn></m:apply>", -0.125, -2.0),
        ("<m:apply><m:root/><m:degree><m:cn>-3</m:cn></m:degree><m:ci>x</m:ci></m:apply>", -0.5, -8.0),
    ] {
        let map = compile(&formula(expression), "", "", &mut 1_000_000).unwrap();
        close(sample(&map, input, 7.0, &mut 100_000).unwrap()[0], expected);
    }
}

/// Inversion reverses composed nonlinear steps and constant arithmetic in the correct order.
#[test]
fn inverse_mathml_composes_non_linear_steps_and_constant_arithmetic() {
    let expression = "<m:apply><m:minus/><m:cn>10</m:cn><m:apply><m:times/><m:cn>3</m:cn><m:apply><m:exp/><m:apply><m:divide/><m:ci>x</m:ci><m:cn>2</m:cn></m:apply></m:apply></m:apply></m:apply>";
    let map = compile(&formula(expression), "", "", &mut 1_000_000).unwrap();
    for i in -20..20 {
        let x = f64::from(i) * 0.25;
        let y = 10.0 - 3.0 * (x / 2.0).exp();
        close(sample(&map, y, 0.0, &mut 100_000).unwrap()[0], x);
    }
    let expression = "<m:apply><m:abs/><m:apply><m:exp/><m:ci>x</m:ci></m:apply></m:apply>";
    let map = compile(&formula(expression), "", "", &mut 1_000_000).unwrap();
    close(sample(&map, 2.0f64.exp(), 0.0, &mut 100_000).unwrap()[0], 2.0);
}

/// Even powers and absolute values need a declared branch and reject observations outside their image.
#[test]
fn inverse_even_powers_and_absolute_values_require_an_unambiguous_branch() {
    for expression in ["<m:apply><m:power/><m:ci>x</m:ci><m:cn>2</m:cn></m:apply>", "<m:apply><m:abs/><m:ci>x</m:ci></m:apply>"] {
        assert!(compile(&formula(expression), "", "", &mut 1_000_000).is_none());
        let value = if expression.contains("power") { 4.0 } else { 2.0 };
        for (range, sign) in [(r#"min="0" max="10""#, 1.0), (r#"min="-10" max="0""#, -1.0)] {
            let map = compile(&formula(expression), &canvas(range), "", &mut 1_000_000).unwrap();
            close(sample(&map, value, 0.0, &mut 100_000).unwrap()[0], sign * 2.0);
            assert!(sample(&map, -1e-100, 0.0, &mut 100_000).is_none());
        }
    }
}

/// Root inverses reject negative observations and enforce the declared source range.
#[test]
fn inverse_roots_reject_negative_output_and_respect_declared_ranges() {
    let expression = "<m:apply><m:root/><m:ci>x</m:ci></m:apply>";
    let map = compile(&formula(expression), "", "", &mut 1_000_000).unwrap();
    close(sample(&map, 3.0, 0.0, &mut 100_000).unwrap()[0], 9.0);
    for value in [-1.0, -1e-100] {
        assert!(sample(&map, value, 0.0, &mut 100_000).is_none());
    }
    let map = compile(&formula(expression), &canvas(r#"min="2" max="10""#), "", &mut 1_000_000).unwrap();
    assert!(sample(&map, 1.0, 0.0, &mut 100_000).is_none());
    assert!(sample(&map, 4.0, 0.0, &mut 100_000).is_none());
}

/// Trigonometric inverses honor declared monotone branches, periods and tangent singularities.
#[test]
fn inverse_trigonometric_maps_honor_period_and_branch_bounds() {
    for (op, low, high, x) in [
        ("sin", -1.0, 1.0, 0.4),
        ("sin", 2.0, 4.0, 3.1),
        ("sin", 5.0, 7.0, 6.0),
        ("sin", -4.0, -2.0, -3.0),
        ("cos", 0.1, 3.0, 2.0),
        ("cos", 3.2, 6.0, 4.0),
        ("cos", -3.0, -0.1, -2.0),
        ("tan", 2.0, 4.0, 3.0),
    ] {
        let expression = format!("<m:apply><m:{op}/><m:ci>x</m:ci></m:apply>");
        assert!(compile(&formula(&expression), "", "", &mut 1_000_000).is_none());
        let range = canvas(&format!(r#"min="{low}" max="{high}""#));
        let map = compile(&formula(&expression), &range, "", &mut 1_000_000).unwrap();
        let value = match op {
            "sin" => f64::sin(x),
            "cos" => f64::cos(x),
            _ => f64::tan(x),
        };
        close(sample(&map, value, 0.0, &mut 100_000).unwrap()[0], x);
    }
    let tan = formula("<m:apply><m:tan/><m:ci>x</m:ci></m:apply>");
    assert!(compile(&tan, &canvas(r#"min="1" max="2""#), "", &mut 1_000_000).is_none());
}

/// Arc-function inverses recover valid samples and reject observations outside their image.
#[test]
fn inverse_arc_functions_refuse_values_outside_their_images() {
    for (op, at) in [("arcsin", f64::asin as fn(f64) -> f64), ("arccos", f64::acos), ("arctan", f64::atan)] {
        let map = compile(&formula(&format!("<m:apply><m:{op}/><m:ci>x</m:ci></m:apply>")), "", "", &mut 1_000_000).unwrap();
        close(sample(&map, at(0.4), 0.0, &mut 100_000).unwrap()[0], 0.4);
        assert!(sample(&map, 10.0, 0.0, &mut 100_000).is_none());
        if op == "arccos" {
            assert!(sample(&map, -1e-100, 0.0, &mut 100_000).is_none());
        }
    }
}

/// Continuous inversion refuses discrete, constant and multibranch maps rather than inventing geometry.
#[test]
fn inverse_mathml_discrete_constant_and_multi_branch_maps_are_refused() {
    for expression in [
        "<m:apply><m:floor/><m:ci>x</m:ci></m:apply>",
        "<m:apply><m:ceiling/><m:ci>x</m:ci></m:apply>",
        "<m:apply><m:power/><m:ci>x</m:ci><m:cn>0</m:cn></m:apply>",
        "<m:apply><m:times/><m:ci>x</m:ci><m:cn>0</m:cn></m:apply>",
        "<m:apply><m:max/><m:ci>x</m:ci><m:cn>0</m:cn></m:apply>",
        "<m:apply><m:quotient/><m:ci>x</m:ci><m:cn>2</m:cn></m:apply>",
        "<m:apply><m:plus/><m:apply><m:exp/><m:ci>x</m:ci></m:apply><m:apply><m:sin/><m:ci>x</m:ci></m:apply></m:apply>",
    ] {
        assert!(
            compile(&formula(expression), "", "", &mut 1_000_000).is_none_or(|map| sample(&map, 0.0, 0.0, &mut 100_000).is_none()),
            "{expression}"
        );
    }
    let map = formula("<m:apply><m:times/><m:ci>x</m:ci><m:cn>0</m:cn></m:apply>");
    let body = format!(
        r#"<context><canvasTransform xmlns:m="http://www.w3.org/1998/Math/MathML"><mapping type="unknown"/>{map}</canvasTransform></context><trace>0 0,0 1</trace>"#
    );
    assert!(super::ink(&body).is_none());
    assert!(super::ink(&body.replace("<m:cn>0</m:cn>", "<m:cn>2</m:cn>")).is_some());
}

/// Numeric safety caps cannot substitute for declared ranges in branch proofs.
#[test]
fn inverse_branch_proofs_do_not_treat_numeric_limits_as_declared_ranges() {
    for expression in [
        "<m:apply><m:min/><m:ci>x</m:ci><m:cn>1000000000</m:cn></m:apply>",
        "<m:apply><m:max/><m:ci>x</m:ci><m:cn>-1000000000</m:cn></m:apply>",
        "<m:apply><m:min/><m:apply><m:exp/><m:ci>x</m:ci></m:apply><m:cn>1000000000</m:cn></m:apply>",
        "<m:apply><m:min/><m:apply><m:divide/><m:cn>1</m:cn><m:ci>x</m:ci></m:apply><m:cn>1000000000</m:cn></m:apply>",
    ] {
        assert!(compile(&formula(expression), "", "", &mut 1_000_000).is_none());
    }
    let expression = "<m:apply><m:min/><m:ci>x</m:ci><m:cn>10</m:cn></m:apply>";
    let map = compile(&formula(expression), &canvas(r#"min="0" max="10""#), "", &mut 1_000_000).unwrap();
    close(sample(&map, 4.0, 0.0, &mut 100_000).unwrap()[0], 4.0);
    let expression = "<m:apply><m:max/><m:ci>x</m:ci><m:cn>0</m:cn></m:apply>";
    let map = compile(&formula(expression), &canvas(r#"min="0" max="10""#), "", &mut 1_000_000).unwrap();
    close(sample(&map, 4.0, 0.0, &mut 100_000).unwrap()[0], 4.0);
}

/// Relative linear table inversion includes the identity term and rejects a constant resulting map.
#[test]
fn inverse_relative_linear_tables_incorporate_the_identity_term() {
    for (rows, input, expected) in [("0 1,10 11", 11.0, 5.0), ("0 20,10 -10", 10.0, 5.0)] {
        let map = compile(&table(rows, "linear", "relative"), "", "", &mut 1_000_000).unwrap();
        close(sample(&map, input, 7.0, &mut 100_000).unwrap()[0], expected);
    }
    assert!(compile(&table("0 10,10 0", "linear", "relative"), "", "", &mut 1_000_000).is_none());
}

/// Cubic inversion solves the original curve in both directions, including relative tables.
#[test]
fn inverse_cubic_tables_solve_the_original_curve_not_a_swapped_knot_spline() {
    for (rows, apply) in
        [("0 0,1 1,2 4,3 9", "absolute"), ("0 0,1 -1,2 -4,3 -9", "absolute"), ("0 0,1 2,2 8,3 18", "relative"), ("0 0,1 -2,2 -8,3 -18", "relative")]
    {
        let reverse = table(rows, "cubic", apply);
        let map = compile(&reverse, "", "", &mut 1_000_000).unwrap();
        let xml = format!(r#"<ink xmlns="http://www.w3.org/2003/InkML">{reverse}</ink>"#);
        let root = crate::xml::parse_strict(xml.as_bytes()).unwrap();
        let forward = Mapping::read(
            root.child("ink:mapping").unwrap(),
            &format::Format::default(),
            &format::Format::default(),
            &HashMap::new(),
            &mut 1_000_000,
            0,
        )
        .unwrap();
        for i in 0..=24 {
            let x = f64::from(i) / 8.0;
            let [y, _] = sample(&forward, x, 7.0, &mut 100_000).unwrap();
            close(sample(&map, y, 7.0, &mut 100_000).unwrap()[0], x);
        }
        assert!(sample(&map, 100.0, 0.0, &mut 100_000).is_none());
    }
}

/// Monotone knots do not justify inversion when the cubic turns between them.
#[test]
fn inverse_cubic_tables_detect_turning_points_between_monotone_knots() {
    assert!(compile(&table("0 0,1 1,2 1.01,3 2", "cubic", "absolute"), "", "", &mut 1_000_000).is_none());
    assert!(compile(&table("0 0,1 -1,2 -1.01,3 -2", "cubic", "absolute"), "", "", &mut 1_000_000).is_none());
}

/// Integer inverses reject hidden quantization and observations without integral preimages.
#[test]
fn inverse_integer_channels_cannot_hide_quantization_or_missing_preimages() {
    let discrete = r#"<channel name="X" type="integer"/><channel name="Y" type="integer"/>"#;
    let reverse = r#"<mapping type="affine"><affine>2 0 0,0 1 0</affine></mapping>"#;
    let map = compile(reverse, discrete, discrete, &mut 1_000_000).unwrap();
    assert_eq!(sample(&map, 4.0, 7.0, &mut 100_000), Some([2.0, 7.0]));
    assert!(sample(&map, 3.0, 7.0, &mut 100_000).is_none());
    assert!(compile(&reverse.replace("2 0 0", "0.5 0 0"), discrete, discrete, &mut 1_000_000).is_none());
    assert!(compile(reverse, "", discrete, &mut 1_000_000).is_none());
    let map = compile(reverse, discrete, "", &mut 1_000_000).unwrap();
    assert!(sample(&map, 3.0, 7.0, &mut 100_000).is_none());
    let map = compile(&formula("<m:apply><m:plus/><m:ci>x</m:ci><m:ci>x</m:ci></m:apply>"), discrete, discrete, &mut 1_000_000).unwrap();
    assert_eq!(sample(&map, 4.0, 7.0, &mut 100_000), Some([2.0, 7.0]));
    assert!(sample(&map, 3.0, 7.0, &mut 100_000).is_none());
}

/// Rectangular affine inverses reconstruct coordinates and validate redundant known observations.
#[test]
fn inverse_rectangular_affine_maps_validate_redundant_observations() {
    let recorded = r#"<channel name="X"/><channel name="Y"/><channel name="Z"/>"#;
    for (matrix, values, expected) in [
        ("2 0 1,0 3 -2,1 1 0", vec![Some(11.0), Some(10.0), Some(9.0)], vec![Some(5.0), Some(4.0)]),
        ("0 0 7,1 0 0,0 1 0", vec![Some(7.0), Some(5.0), Some(4.0)], vec![Some(5.0), Some(4.0)]),
    ] {
        let reverse = format!(r#"<mapping type="affine"><affine>{matrix}</affine></mapping>"#);
        let map = compile(&reverse, "", recorded, &mut 1_000_000).unwrap();
        let mut result = vec![None; 2];
        map.apply(&values, &mut result, &mut 100_000).unwrap();
        for (actual, expected) in result.iter().zip(&expected) {
            close(actual.unwrap(), expected.unwrap());
        }
        let mut invalid = values.clone();
        invalid[0] = Some(100.0);
        assert!(map.apply(&invalid, &mut result, &mut 100_000).is_none());
    }
    let map = compile(r#"<mapping type="affine"><affine>2 0 1,0 3 -2,1 1 0</affine></mapping>"#, "", recorded, &mut 1_000_000).unwrap();
    let mut result = vec![None; 2];
    map.apply(&vec![Some(11.0), Some(10.0), None], &mut result, &mut 100_000).unwrap();
    close(result[0].unwrap(), 5.0);
    close(result[1].unwrap(), 4.0);
    assert!(compile(r#"<mapping type="affine"><affine>1 1 0,2 2 0,3 3 0</affine></mapping>"#, "", recorded, &mut 1_000_000).is_none());
}

/// A reconstructing table output recovers the input while remaining observations constrain it.
#[test]
fn inverse_univariate_tables_use_a_reconstructing_output_and_validate_the_rest() {
    let recorded = r#"<channel name="X"/><channel name="Y"/><channel name="Z"/>"#;
    for interpolation in ["linear", "cubic"] {
        let reverse = format!(
            r#"<mapping type="product"><mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><bind target="Z" column="3"/><table interpolation="{interpolation}">0 99 0,1 99 2,2 99 4,3 99 6</table></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping>"#
        );
        let map = compile(&reverse, "", recorded, &mut 1_000_000).unwrap();
        let mut result = vec![None; 2];
        map.apply(&vec![Some(99.0), Some(7.0), Some(3.0)], &mut result, &mut 100_000).unwrap();
        assert_eq!(result, [Some(1.5), Some(7.0)]);
        assert!(map.apply(&vec![Some(98.0), Some(7.0), Some(3.0)], &mut result, &mut 100_000).is_none());
        map.apply(&vec![None, Some(7.0), Some(3.0)], &mut result, &mut 100_000).unwrap();
        assert_eq!(result, [Some(1.5), Some(7.0)]);
    }
}

/// Analytic inverse compilation, evaluation and proofs obey exact shared work budgets.
#[test]
fn inverse_compilation_evaluation_and_proofs_use_the_shared_budget() {
    for reverse in [formula("<m:apply><m:exp/><m:ci>x</m:ci></m:apply>"), table("0 0,1 1,2 4,3 9", "cubic", "absolute")] {
        let mut work = 1_000_000;
        assert!(compile(&reverse, "", "", &mut work).is_some());
        let used = 1_000_000 - work;
        assert!(compile(&reverse, "", "", &mut (used - 1)).is_none());
        let mut exact = used;
        let map = compile(&reverse, "", "", &mut exact).unwrap();
        let mut work = 100_000;
        assert!(sample(&map, 2.0, 7.0, &mut work).is_some());
        let used = 100_000 - work;
        assert!(sample(&map, 2.0, 7.0, &mut (used - 1)).is_none());
        let mut exact = used;
        assert!(sample(&map, 2.0, 7.0, &mut exact).is_some());
    }
}

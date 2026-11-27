use super::{Cx, Env, Frame, Pass, Val, check_display, check_name, compare_texts, eval_expr, eval_lenient, run_stmt, run_stmt_in};
use crate::array::{Data, Grid, element_name};
use crate::{CalcError, Expr, Op, SolveKind, Stmt, Value, parse};
use craft_units::{Dim, Precision, Quantity, UnitError, UnitSystem, format};

fn run(env: &mut Env, src: &str) -> Result<Option<Quantity>, CalcError> {
    run_value(env, src).map(|v| v.map(one))
}

fn run_value(env: &mut Env, src: &str) -> Result<Option<Value>, CalcError> {
    let stmt = parse(src).unwrap_or_else(|e| panic!("{src:?} did not parse: {e}"));
    run_stmt(env, &stmt)
}

/// The single value of `v`.
fn one(v: Value) -> Quantity {
    match v {
        Value::One(q) => q,
        other => panic!("expected a single value, got {other:?}"),
    }
}

/// Run `src` in a fresh env and return its shown value.
fn val(src: &str) -> Quantity {
    shown(&mut Env::new(), src)
}

fn shown(env: &mut Env, src: &str) -> Quantity {
    match run(env, src) {
        Ok(Some(q)) => q,
        other => panic!("{src:?} did not show a value: {other:?}"),
    }
}

fn err(src: &str) -> CalcError {
    match run(&mut Env::new(), src) {
        Err(e) => e,
        other => panic!("{src:?} should fail: {other:?}"),
    }
}

fn close(q: Quantity, value: f64, dim: Dim) {
    assert_eq!(q.dim, dim, "dimension of {q:?}");
    assert!((q.value - value).abs() <= 1e-12 * value.abs().max(1.0), "{} vs {value}", q.value);
}

fn syntax_msg(e: CalcError) -> String {
    match e {
        CalcError::Syntax(m) => m,
        other => panic!("expected a syntax error, got {other:?}"),
    }
}

const AREA: Dim = Dim { exp: [2, 0, 0, 0, 0, 0, 0, 0] };

#[test]
fn stress_example() {
    let mut env = Env::new();
    assert_eq!(run(&mut env, "P := 1500 lbf").ok(), Some(None));
    assert_eq!(run(&mut env, "A_b := 2.5 in^2").ok(), Some(None));
    let q = shown(&mut env, "σ := P/A_b =");
    let f = format(q, None, UnitSystem::UsCustomary, Precision::new(3)).expect("formats");
    assert_eq!((f.number.as_str(), f.unit.as_str()), ("600", "psi"));
}

#[test]
fn mixed_systems() {
    close(val("len := 1 in + 25.4 mm ="), 0.0508, Dim::LENGTH);
}

#[test]
fn unit_mismatch_message() {
    assert!(matches!(err("x := 1 in + 1 lbf ="), CalcError::Unit(UnitError::Mismatch { .. })));
}

#[test]
fn undefined_message() {
    let e = err("y := z + 1 =");
    assert_eq!(e, CalcError::Undefined("z".into()));
    assert_eq!(e.to_string(), "z isn't defined");
}

#[test]
fn user_function() {
    let mut env = Env::new();
    assert_eq!(run(&mut env, "f(x, y) := x^2 + y").ok(), Some(None));
    close(shown(&mut env, "f(3 m, 1 m^2) ="), 10.0, AREA);
}

#[test]
fn unit_names_cannot_be_variables() {
    let mut env = Env::new();
    for src in ["m := 5 kg", "A := 2 in^2", "kN := 1", "sin := 1", "f(m) := m", "min := 1", "π := 3", "e := 2", "pi := 3"] {
        assert!(matches!(run(&mut env, src), Err(CalcError::ReservedName(_))), "{src}");
    }
    assert!(run(&mut env, "A_b := 2 in^2").is_ok());
    close(shown(&mut env, "2 m ="), 2.0, Dim::LENGTH);
    // A refused definition leaves nothing behind.
    assert_eq!(env.names(), vec!["A_b".to_string()]);
}

#[test]
fn trig_takes_angles() {
    close(val("sin(30 deg) ="), 0.5, Dim::ONE);
    close(val("cos(0) ="), 1.0, Dim::ONE);
    assert!(matches!(err("sin(1 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
}

#[test]
fn non_finite_results_are_errors() {
    for src in ["10^400 =", "1/0 =", "sqrt(-1) =", "0/0 =", "ln(0) =", "ln(-1) =", "10^300 * 10^300 =", "tan(90 deg) * 10^300 * 10^300 ="] {
        assert_eq!(err(src), CalcError::NonFinite, "{src}");
    }
    // A hand-built non-finite literal is caught too.
    let e = Expr::Num(f64::INFINITY);
    assert_eq!(eval_expr(&Env::new(), &e), Err(CalcError::NonFinite));
}

#[test]
fn recursion_bound() {
    let mut env = Env::new();
    assert!(run(&mut env, "f(x) := f(x)").is_ok());
    assert_eq!(run(&mut env, "f(1) ="), Err(CalcError::TooComplex));
}

#[test]
fn upstream() {
    let mut env = Env::new();
    env.mark_failed("a");
    assert_eq!(run(&mut env, "b := a + 1 ="), Err(CalcError::Upstream));
    assert_eq!(run(&mut env, "a(2) ="), Err(CalcError::Upstream));
    // Defining it afterwards clears the mark.
    env.define_var("a", Quantity::dimensionless(1.0));
    close(shown(&mut env, "a + 1 ="), 2.0, Dim::ONE);
}

#[test]
fn name_resolution_order() {
    let mut env = Env::new();
    // Parameters win over variables; constants and units resolve last.
    env.define_var("x", Quantity::dimensionless(100.0));
    assert_eq!(run(&mut env, "fa(x) := x + 1").ok(), Some(None));
    close(shown(&mut env, "fa(1) ="), 2.0, Dim::ONE);
    close(val("π ="), std::f64::consts::PI, Dim::ONE);
    close(val("pi ="), std::f64::consts::PI, Dim::ONE);
    close(val("e ="), std::f64::consts::E, Dim::ONE);
    close(val("5 min ="), 300.0, Dim::TIME);
}

#[test]
fn functions_see_globals_but_not_the_callers_parameters() {
    let mut env = Env::new();
    run(&mut env, "k := 10").expect("defines");
    run(&mut env, "fa(x) := x * k").expect("defines");
    run(&mut env, "fb(k) := fa(k)").expect("defines");
    close(shown(&mut env, "fb(2) ="), 20.0, Dim::ONE);
    // `fc` has no parameter `y`, and the caller's `y` does not leak in.
    run(&mut env, "fc(x) := x + y").expect("defines");
    run(&mut env, "fd(y) := fc(1)").expect("defines");
    assert_eq!(run(&mut env, "fd(5) ="), Err(CalcError::Undefined("y".into())));
}

#[test]
fn offset_units_only_as_a_lone_literal() {
    close(val("212 °F ="), 373.15, Dim::TEMPERATURE);
    close(val("100 °C ="), 373.15, Dim::TEMPERATURE);
    close(val("-40 °F ="), 233.15, Dim::TEMPERATURE);
    close(val("(5 °C) ="), 278.15, Dim::TEMPERATURE);
    // Two absolute temperatures can't be added or subtracted, but an interval can be added.
    for src in ["100 °C + 5 °C =", "100 °C - 5 °F ="] {
        assert_eq!(err(src), CalcError::Unit(UnitError::AffineTemperature), "{src}");
    }
    close(val("100 °C + 5 Δ°C ="), 378.15, Dim::TEMPERATURE);
    close(val("5 K + 100 °C ="), 378.15, Dim::TEMPERATURE);
    close(val("100 °C - 5 Δ°C ="), 368.15, Dim::TEMPERATURE);
    close(val("100 °C - 5 K ="), 368.15, Dim::TEMPERATURE);
    for src in ["5 K - 100 °C =", "5 Δ°C - 100 °C =", "5 Δ°F - 100 °C ="] {
        assert_eq!(err(src), CalcError::Unit(UnitError::AffineTemperature), "{src}");
    }
    close(val("5 Δ°F + 100 °C ="), 373.15 + 5.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    close(val("100 °C + 5 °R ="), 373.15 + 5.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    // An absolute literal beside a variable is refused; variables carry whether they are absolute
    // (see `offset_display_needs_an_absolute_temperature`).
    let mut env = Env::new();
    run(&mut env, "T_1 := 100 °C").expect("defines");
    run(&mut env, "T_2 := 20 °C").expect("defines");
    close(shown(&mut env, "T_1 - T_2 ="), 80.0, Dim::TEMPERATURE);
    close(shown(&mut env, "T_1 + 5 K ="), 378.15, Dim::TEMPERATURE);
    for src in ["T_1 + 5 °C =", "100 °C + T_1 =", "T_1 - 5 °F =", "100 °C + 5 m ="] {
        assert_eq!(run(&mut env, src), Err(CalcError::Unit(UnitError::AffineTemperature)), "{src}");
    }
    for src in ["°F =", "1 / °F =", "5 °F / s =", "x := 2 °F^2 =", "3 + °C =", "-(5 °F) =", "(5 °F)^2 =", "2 (5 °F) =", "(5 °F) / 2 ="] {
        assert_eq!(err(src), CalcError::Unit(UnitError::AffineTemperature), "{src}");
    }
    let mut env = Env::new();
    run(&mut env, "t := 3").expect("defines");
    assert_eq!(run(&mut env, "t °F ="), Err(CalcError::Unit(UnitError::AffineTemperature)));
    // Added to an interval, an absolute temperature stays absolute.
    close(val("100 °C + 5 K ="), 378.15, Dim::TEMPERATURE);
    // A plain temperature unit is fine: 10 K, and a rankine.
    close(val("10 K ="), 10.0, Dim::TEMPERATURE);
    // Hand-built quantity literals take the same path.
    let q = eval_expr(&Env::new(), &Expr::Quantity(32.0, "°F".into())).map(one).expect("evaluates");
    close(q, 273.15, Dim::TEMPERATURE);
}

#[test]
fn functions_are_not_values() {
    let mut env = Env::new();
    run(&mut env, "f(x) := x").expect("defines");
    run(&mut env, "P := 2").expect("defines");
    assert_eq!(syntax_msg(run(&mut env, "f + 1 =").unwrap_err()), "f is a function — call it like f(x)");
    assert_eq!(syntax_msg(run(&mut env, "f^2 (3) =").unwrap_err()), "f is a function — call it like f(x)");
    assert_eq!(syntax_msg(run(&mut env, "sin + 1 =").unwrap_err()), "sin is a function — call it like sin(x)");
    assert!(syntax_msg(run(&mut env, "P(2) =").unwrap_err()).contains("P isn't a function"));
    assert!(syntax_msg(run(&mut env, "m(2) =").unwrap_err()).contains("m isn't a function"));
    assert_eq!(run(&mut env, "nope(2) ="), Err(CalcError::Undefined("nope".into())));
}

#[test]
fn user_function_arity() {
    let mut env = Env::new();
    run(&mut env, "f(x, y) := x + y").expect("defines");
    assert_eq!(run(&mut env, "f(1) ="), Err(CalcError::Arity("f".into(), 2)));
    assert_eq!(run(&mut env, "f(1, 2, 3) ="), Err(CalcError::Arity("f".into(), 2)));
}

#[test]
fn exponential_work_is_bounded() {
    // Each level calls the one below twice: 2^30 evaluations unless the work is capped.
    let mut env = Env::new();
    run(&mut env, "g0(x) := x").expect("defines");
    for i in 1..=30 {
        run(&mut env, &format!("g{i}(x) := g{}(x) + g{}(x)", i - 1, i - 1)).expect("defines");
    }
    assert_eq!(run(&mut env, "g30(1) ="), Err(CalcError::TooComplex));
}

#[test]
fn deep_expressions_evaluate_and_hostile_depth_is_refused() {
    // Parser-limit shapes: 30 nested parentheses (its nesting limit is 64) and a 1000-term sum
    // (a tree 1000 high) evaluate fine.
    let nested = format!("{}1{} =", "(-".repeat(30), ")".repeat(30));
    assert!(run(&mut Env::new(), &nested).is_ok());
    let sum = format!("{} =", vec!["1"; 1000].join("+"));
    close(val(&sum), 1000.0, Dim::ONE);
    // A hand-built tree far deeper than the parser allows is refused, not a stack overflow.
    let mut e = Expr::Num(1.0);
    for _ in 0..3000 {
        e = Expr::Unary(Op::Neg, Box::new(e));
    }
    assert_eq!(eval_expr(&Env::new(), &e), Err(CalcError::TooComplex));
    // Function bodies nested through calls share one depth budget.
    let mut env = Env::new();
    let tail = " + 1".repeat(440);
    run(&mut env, "n0(x) := x").expect("defines");
    for i in 1..30 {
        run(&mut env, &format!("n{i}(x) := n{}(x){tail}", i - 1)).expect("defines");
    }
    assert_eq!(run(&mut env, "n29(1) ="), Err(CalcError::TooComplex));
    // Shallow enough chains still work.
    close(shown(&mut env, "n3(1) ="), 1.0 + 3.0 * 440.0, Dim::ONE);
}

#[test]
fn show_units_are_validated() {
    close(val("1 in = ? mm"), 0.0254, Dim::LENGTH);
    assert!(matches!(err("1 in = ? lbf"), CalcError::Unit(UnitError::Mismatch { .. })));
    assert!(matches!(err("1 in = ? bogus"), CalcError::Unit(UnitError::UnknownUnit(_))));
    // A failed show leaves a defined name undefined.
    let mut env = Env::new();
    assert!(run(&mut env, "a := 1 in = ? lbf").is_err());
    assert!(env.names().is_empty());
    assert!(syntax_msg(err("f(x) := x =")).contains("function"));
}

#[test]
fn definitions_without_show_return_none() {
    let mut env = Env::new();
    assert_eq!(run(&mut env, "a := 3").ok(), Some(None));
    assert_eq!(env.names(), vec!["a".to_string()]);
    run(&mut env, "f(x) := x").expect("defines");
    assert_eq!(env.names(), vec!["a".to_string(), "f".to_string()]);
}

#[test]
fn powers() {
    close(val("(2 m)^3 ="), 8.0, Dim::LENGTH.powi(3).expect("dim"));
    close(val("2^0.5 ="), 2f64.sqrt(), Dim::ONE);
    close(val("4 m^2 ^ 1 ="), 4.0, AREA);
    assert!(matches!(err("2^(1 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
    assert!(matches!(err("(2 m)^0.5 ="), CalcError::Unit(UnitError::NotDimensionless(_))));
    close(val("|-3 m| ="), 3.0, Dim::LENGTH);
}

#[test]
fn builtins_unary() {
    close(val("sqrt(4 m^2) ="), 2.0, Dim::LENGTH);
    close(val("√9 ="), 3.0, Dim::ONE);
    assert!(matches!(err("sqrt(4 m) ="), CalcError::Unit(UnitError::Fractional)));
    close(val("abs(-2 kg) ="), 2.0, Dim::MASS);
    close(val("exp(0) ="), 1.0, Dim::ONE);
    close(val("ln(e) ="), 1.0, Dim::ONE);
    close(val("log(1000) ="), 3.0, Dim::ONE);
    close(val("log(8, 2) ="), 3.0, Dim::ONE);
    close(val("log(1000, 10) ="), 3.0, Dim::ONE);
    close(val("log(81, 3) ="), 4.0, Dim::ONE);
    assert!(matches!(err("exp(1 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
    close(val("sinh(0) + cosh(0) + tanh(0) ="), 1.0, Dim::ONE);
    close(val("floor(2.7) + ceil(2.1) ="), 5.0, Dim::ONE);
    assert!(matches!(err("floor(2.7 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
}

#[test]
fn inverse_trig_returns_angles() {
    close(val("asin(1) ="), std::f64::consts::FRAC_PI_2, Dim::ANGLE);
    close(val("acos(1) ="), 0.0, Dim::ANGLE);
    close(val("atan(1) ="), std::f64::consts::FRAC_PI_4, Dim::ANGLE);
    close(val("atan2(1 m, 1 m) ="), std::f64::consts::FRAC_PI_4, Dim::ANGLE);
    close(val("atan2(1 m, 1000 mm) ="), std::f64::consts::FRAC_PI_4, Dim::ANGLE);
    assert!(matches!(err("atan2(1 m, 1 s) ="), CalcError::Unit(UnitError::Mismatch { .. })));
    assert_eq!(err("asin(2) ="), CalcError::NonFinite);
    // An angle result goes back into trig.
    close(val("sin(asin(0.5)) ="), 0.5, Dim::ONE);
}

#[test]
fn min_max_round_mod() {
    close(val("min(3 m, 1 ft, 2 m) ="), 0.3048, Dim::LENGTH);
    close(val("max(3 m, 1 ft, 2 m) ="), 3.0, Dim::LENGTH);
    close(val("max(1) ="), 1.0, Dim::ONE);
    assert!(matches!(err("min(1 m, 1 s) ="), CalcError::Unit(UnitError::Mismatch { .. })));
    assert_eq!(err("min() ="), CalcError::Arity("min".into(), 1));
    close(val("round(2.5) ="), 3.0, Dim::ONE);
    close(val("round(-2.5) ="), -3.0, Dim::ONE);
    close(val("round(2.71828, 2) ="), 2.72, Dim::ONE);
    close(val("round(1234, -2) ="), 1200.0, Dim::ONE);
    assert!(syntax_msg(err("round(1.5, 0.5) =")).contains("whole number"));
    assert!(syntax_msg(err("round(1.5, 99) =")).contains("-15 to 15"));
    close(val("mod(7 m, 3 m) ="), 1.0, Dim::LENGTH);
    close(val("mod(-1, 3) ="), 2.0, Dim::ONE);
    close(val("mod(5, -3) ="), -1.0, Dim::ONE);
    // A remainder too small to move the divisor is 0, never exactly the divisor.
    close(val("mod(-1/10^20 m, 3 m) ="), 0.0, Dim::LENGTH);
    assert!(val("mod(-1/10^20, 3) =").value < 3.0);
    assert_eq!(err("mod(5, 0) ="), CalcError::NonFinite);
    assert!(matches!(err("mod(1 m, 1 s) ="), CalcError::Unit(UnitError::Mismatch { .. })));
}

/// `q` is `value` to relative `tol`, in dimension `dim`.
fn close_rel(q: Quantity, value: f64, dim: Dim, tol: f64) {
    assert_eq!(q.dim, dim, "dimension of {q:?}");
    assert!((q.value - value).abs() <= tol * value.abs().max(1.0), "{} vs {value}", q.value);
}

/// Two quantities are equal as measures: the same dimension, values within rounding.
fn same_quantity(a: Quantity, b: Quantity) {
    assert_eq!(a.dim, b.dim);
    assert!((a.value - b.value).abs() <= 1e-12 * b.value.abs().max(1e-300), "{} vs {}", a.value, b.value);
}

#[test]
fn ceil_to_multiple() {
    same_quantity(val("Ceil(2.06 in, 0.125 in) ="), val("2.125 in ="));
    close(val("Ceil(7.2, 1) ="), 8.0, Dim::ONE);
    // Mathcad's hicks_si_p_2.68.xmcd: 1.9 in up to a multiple of 25.4 mm is 2 in, which is 50.8 mm.
    same_quantity(val("Ceil(1.9 in, 25.4 mm) ="), val("50.8 mm ="));
    same_quantity(val("Floor(2.06 in, 0.125 in) ="), val("2 in ="));
    same_quantity(val("Round(2.06 in, 0.125 in) ="), val("2 in ="));
    // Halves go away from zero.
    close(val("Round(2.0625, 0.125) ="), 2.125, Dim::ONE);
    close(val("Round(-2.0625, 0.125) ="), -2.125, Dim::ONE);
    // A value already on a multiple stays, whatever the unit conversions do to the last digit.
    same_quantity(val("Ceil(0.375 in, 0.125 in) ="), val("0.375 in ="));
    same_quantity(val("Floor(0.375 in, 3.175 mm) ="), val("0.375 in ="));
    same_quantity(val("Ceil(0.75 in, 0.125 in) ="), val("0.75 in ="));
    same_quantity(val("Ceil(50.8 mm, 2 in) ="), val("50.8 mm ="));
    same_quantity(val("Round(2.0625 in, 0.125 in) ="), val("2.125 in ="));
    same_quantity(val("Floor(-0.375 in, 0.125 in) ="), val("-0.375 in ="));
    same_quantity(val("Ceil(-0.375 in, 0.125 in) ="), val("-0.375 in ="));
    // The snap only absorbs rounding noise: a ratio just outside it still moves, and a tiny
    // ratio is not an exact multiple of anything.
    close(val("Ceil(2.0000001, 1) ="), 3.0, Dim::ONE);
    close(val("Ceil(1000000.0000001, 1) ="), 1_000_001.0, Dim::ONE);
    close(val("Floor(0.9999999, 1) ="), 0.0, Dim::ONE);
    close(val("Ceil(1/10^13, 1) ="), 1.0, Dim::ONE);
    close(val("Floor(-1/10^13, 1) ="), -1.0, Dim::ONE);
    close(val("Round(1/10^13, 1) ="), 0.0, Dim::ONE);
    // Element-wise over a grid, like the other scalar built-ins.
    let mut env = Env::new();
    run(&mut env, "v := [1.2, 2.5, 3.9]").expect("defines v");
    close_all(&grid(&mut env, "Ceil(v, 1) =").1, &[2.0, 3.0, 4.0], Dim::ONE);
}

#[test]
fn ceil_negative_and_bad_steps() {
    close(val("Ceil(-2.06, 0.125) ="), -2.0, Dim::ONE);
    close(val("Floor(-2.06, 0.125) ="), -2.125, Dim::ONE);
    for src in ["Ceil(2, 0) =", "Floor(2, -1) =", "Round(2, 0) =", "Round(2 in, -1 in) ="] {
        let msg = syntax_msg(err(src));
        let name = src.split('(').next().expect("a name");
        assert!(msg.contains(&format!("{name}'s step must be greater than 0")), "{src}: {msg}");
    }
    assert!(matches!(err("Ceil(2 in, 1 lbf) ="), CalcError::Unit(UnitError::Mismatch { .. })));
    assert_eq!(err("Ceil(2) ="), CalcError::Arity("Ceil".into(), 2));
    assert_eq!(err("Round(1, 2, 3) ="), CalcError::Arity("Round".into(), 2));
}

#[test]
fn erf_matches_mathcad() {
    // Molar Mass.xmcd.
    close_rel(val("0.5·√π·12000·(1 + erf(15000/12000)) ="), 20449.510423409225, Dim::ONE, 1e-12);
    close_rel(val("erf(0) ="), 0.0, Dim::ONE, 1e-15);
    assert!(matches!(err("erf(2 in) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
}

#[test]
fn reciprocal_trig() {
    close(val("csc(30 deg) ="), 2.0, Dim::ONE);
    close(val("cot(45 deg) ="), 1.0, Dim::ONE);
    close_rel(val("secant(60 deg) ="), 2.0, Dim::ONE, 1e-12);
    close_rel(val("atanh(0.5) ="), 0.5493061443340548, Dim::ONE, 1e-14);
    assert_eq!(err("csc(0) ="), CalcError::NonFinite);
    assert_eq!(err("atanh(1) ="), CalcError::NonFinite);
    assert!(matches!(err("cot(1 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
}

#[test]
fn bessel() {
    close_rel(val("J0(1) ="), 0.7651976865579666, Dim::ONE, 1e-14);
    close_rel(val("J1(1) ="), 0.44005058574493355, Dim::ONE, 1e-14);
    assert!(matches!(err("J0(1 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
}

#[test]
fn lowercase_unchanged() {
    close(val("ceil(2.1) ="), 3.0, Dim::ONE);
    close(val("floor(2.7) ="), 2.0, Dim::ONE);
    close(val("round(2.345, 2) ="), 2.35, Dim::ONE);
    assert!(matches!(err("ceil(2.1, 1) ="), CalcError::Arity(..)));
}

#[test]
fn new_scalar_function_names_reserved() {
    for name in ["erf", "csc", "cot", "secant", "atanh", "J0", "J1", "Ceil", "Floor", "Round"] {
        assert_eq!(check_name(name), Err(CalcError::ReservedName(name.into())), "{name}");
        assert_eq!(err(&format!("{name} := 1")), CalcError::ReservedName(name.into()), "{name}");
    }
}

#[test]
fn arity_errors() {
    assert_eq!(err("sin() ="), CalcError::Arity("sin".into(), 1));
    assert_eq!(err("sqrt(1, 2) ="), CalcError::Arity("sqrt".into(), 1));
    assert_eq!(err("atan2(1) ="), CalcError::Arity("atan2".into(), 2));
    assert_eq!(err("mod(1, 2, 3) ="), CalcError::Arity("mod".into(), 2));
    assert_eq!(err("round(1, 2, 3) ="), CalcError::Arity("round".into(), 2));
    assert_eq!(err("log() ="), CalcError::Arity("log".into(), 1));
    assert_eq!(err("log(1, 2, 3) ="), CalcError::Arity("log".into(), 2));
}

#[test]
fn check_name_rules() {
    for bad in ["m", "mm", "kN", "psi", "lbf", "sin", "atan2", "round", "mod", "π", "pi", "e", "min", "max", "°F"] {
        assert_eq!(check_name(bad), Err(CalcError::ReservedName(bad.into())), "{bad}");
    }
    for ok in ["x", "A_b", "σ", "P", "f", "foo", "L1"] {
        assert!(check_name(ok).is_ok(), "{ok}");
    }
}

#[test]
fn redefinition_replaces_the_previous_entry() {
    let mut env = Env::new();
    env.define_var("a", Quantity::dimensionless(1.0));
    env.define_fn("a", vec!["x".into()], Expr::Name("x".into()));
    close(shown(&mut env, "a(4) ="), 4.0, Dim::ONE);
    env.mark_failed("a");
    assert_eq!(run(&mut env, "a(4) ="), Err(CalcError::Upstream));
    assert_eq!(env.names(), vec!["a".to_string()]);
}

#[test]
fn offset_display_needs_an_absolute_temperature() {
    let not_absolute = |u: &str| CalcError::NotAbsolute(u.to_string());
    // A difference shown on an offset scale would be −253.15 °C: refused.
    assert_eq!(err("dT := 20 Δ°C = ? °C"), not_absolute("°C"));
    let mut env = Env::new();
    run(&mut env, "T_1 := 70 °F").expect("defines");
    run(&mut env, "T_2 := 100 °F").expect("defines");
    assert_eq!(run(&mut env, "T_2 - T_1 = ? °F"), Err(not_absolute("°F")));
    assert_eq!(run(&mut env, "T_2 - T_1 = ? °C"), Err(not_absolute("°C")));
    // The difference itself is fine in a difference unit, or a plain one.
    close(shown(&mut env, "T_2 - T_1 = ? Δ°F"), 30.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    close(shown(&mut env, "T_2 - T_1 = ? K"), 30.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    // Absolute temperatures, and an absolute temperature plus or minus a difference, show.
    close(shown(&mut env, "T := 70 °F = ? °C"), 294.261_111_111_111_1, Dim::TEMPERATURE);
    close(shown(&mut env, "T_1 + 5 K = ? °C"), 294.261_111_111_111_1 + 5.0, Dim::TEMPERATURE);
    close(shown(&mut env, "T_1 + (T_2 - T_1)/2 = ? °F"), (85.0 - 32.0) * 5.0 / 9.0 + 273.15, Dim::TEMPERATURE);
    close(shown(&mut env, "T_3 := T_2 - 10 Δ°F = ? °F"), (90.0 - 32.0) * 5.0 / 9.0 + 273.15, Dim::TEMPERATURE);
    close(shown(&mut env, "T_3 = ? °C"), (90.0 - 32.0) * 5.0 / 9.0 + 273.15, Dim::TEMPERATURE);
    close(val("100 °C + 5 K = ? °C"), 378.15, Dim::TEMPERATURE);
    close(val("T_K := 300 K = ? °C"), 300.0, Dim::TEMPERATURE);
    // Anything else made from temperatures is not known to be absolute.
    for src in ["(T_1 + T_2)/2 = ? °C", "2*T_1 = ? °C", "-T_1 + 600 K = ? °F", "5 K + 3 K = ? °C", "|T_1| = ? °C"] {
        assert_eq!(run(&mut env, src), Err(not_absolute(src.rsplit(' ').next().unwrap())), "{src}");
    }
    // User functions pass it through their parameters and result.
    run(&mut env, "warm(t) := t + 5 Δ°C").expect("defines");
    run(&mut env, "twice(t) := 2*t").expect("defines");
    close(shown(&mut env, "warm(T_1) = ? °C"), 294.261_111_111_111_1 + 5.0, Dim::TEMPERATURE);
    assert_eq!(run(&mut env, "twice(T_1) = ? °C"), Err(not_absolute("°C")));
    assert_eq!(run(&mut env, "warm(5 Δ°C) = ? °C"), Err(not_absolute("°C")));
    // The larger of two absolute temperatures is one of them.
    close(shown(&mut env, "max(T_1, T_2) = ? °F"), 310.927_777_777_777_8, Dim::TEMPERATURE);
    assert_eq!(run(&mut env, "max(T_2 - T_1, 1 K) = ? °F"), Err(not_absolute("°F")));
}

#[test]
fn engineering_constants_carry_their_units() {
    let accel = Dim::LENGTH.div(Dim::TIME.powi(2).unwrap()).unwrap();
    close(val("a := g_0 ="), 9.80665, accel);
    close(val("v := c_0 ="), 299_792_458.0, Dim::LENGTH.div(Dim::TIME).unwrap());
    let r = val("r := R_u =");
    assert!((r.value - 8.314_462_618).abs() < 1e-9, "{}", r.value);
    // A weight: 10 kg under standard gravity is 98.0665 N, and 1 lbm weighs 1 lbf.
    close(val("F_w := 10 kg·g_0 ="), 98.0665, val("n_1 := 1 N =").dim);
    let mut env = Env::new();
    let w = shown(&mut env, "F_1 := 1 lbm·g_0 =");
    let f = format(w, Some(&craft_units::parse_unit("lbf").unwrap()), UnitSystem::UsCustomary, Precision::new(6)).unwrap();
    assert_eq!(f.number, "1");
    // The constants are taken: they can't be redefined.
    for name in ["g_0", "c_0", "R_u"] {
        assert!(matches!(check_name(name), Err(CalcError::ReservedName(_))), "{name}");
    }
    assert!(check_name("g_1").is_ok(), "other subscripts are free");
}

#[test]
fn g_is_gravity_when_the_document_says_so() {
    let accel = Dim::LENGTH.div(Dim::TIME.powi(2).unwrap()).unwrap();
    let mut env = Env::new();
    env.set_g_is_gravity(true);
    close(shown(&mut env, "a_1 := g ="), 9.80665, accel);
    close(shown(&mut env, "F_w := 5 kg·g ="), 49.03325, val("n_1 := 1 N =").dim);
    close(shown(&mut env, "m_1 := 5 gm ="), 0.005, Dim::MASS);
    close(shown(&mut env, "m_2 := 5 kg ="), 5.0, Dim::MASS);
    // `g` as a display unit would read as gravity: refused, with the way out named.
    let e = run(&mut env, "m_3 := 5 kg = ? g").expect_err("g is gravity here");
    assert!(e.to_string().contains("gm"), "{e}");
    assert!(run(&mut env, "m_4 := 5 kg = ? gm").is_ok());
    // Off (SI): g is the gram again.
    close(val("m_5 := 5 g ="), 0.005, Dim::MASS);
}

// ---------------------------------------------------------------------------------------------
// Grids (vectors and matrices)
// ---------------------------------------------------------------------------------------------

/// `values` in `unit`, as SI quantities.
fn quantities(values: &[f64], unit: &str) -> Vec<Quantity> {
    let scale = craft_units::parse_unit(unit).expect("a unit");
    values.iter().map(|&v| Quantity::new(v * scale.factor, scale.dim)).collect()
}

/// Define the column vector `name` of `values` in `unit`.
fn column(env: &mut Env, name: &str, values: &[f64], unit: &str) {
    env.define_array(name, values.len(), 1, quantities(values, unit)).expect("defines");
}

/// The grid `src` shows: its shape and its elements.
fn grid(env: &mut Env, src: &str) -> ((usize, usize), Vec<Quantity>) {
    match run_value(env, src) {
        Ok(Some(Value::Grid { rows, cols, items })) => ((rows, cols), items),
        other => panic!("{src:?} did not show a grid: {other:?}"),
    }
}

/// Every element of `items` is close to the matching `want` (SI) and has dimension `dim`.
fn close_all(items: &[Quantity], want: &[f64], dim: Dim) {
    assert_eq!(items.len(), want.len(), "{items:?}");
    for (q, w) in items.iter().zip(want) {
        close(*q, *w, dim);
    }
}

const INCH: f64 = 0.0254;

#[test]
fn vectors_add_element_wise() {
    let mut env = Env::new();
    column(&mut env, "D", &[1.0, 2.0, 3.0], "in");
    column(&mut env, "t", &[0.1, 0.2, 0.3], "in");
    let (shape, items) = grid(&mut env, "D + t =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[1.1 * INCH, 2.2 * INCH, 3.3 * INCH], Dim::LENGTH);
    // A name holding a grid evaluates as that grid.
    assert_eq!(run(&mut env, "D_t := D + t").ok(), Some(None));
    let (shape, items) = grid(&mut env, "D_t - t =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[INCH, 2.0 * INCH, 3.0 * INCH], Dim::LENGTH);
    // A shown unit must fit every element; one that doesn't names the element and defines nothing.
    assert!(run_value(&mut env, "D = ? mm").is_ok());
    let e = run_value(&mut env, "u := D = ? lbf").expect_err("inches aren't force");
    assert!(e.to_string().starts_with("element 0: "), "{e}");
    assert_eq!(run(&mut env, "u ="), Err(CalcError::Undefined("u".into())));
}

#[test]
fn single_value_broadcasts() {
    let mut env = Env::new();
    column(&mut env, "D", &[1.0, 2.0, 3.0], "in");
    let (shape, items) = grid(&mut env, "2·D =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[2.0 * INCH, 4.0 * INCH, 6.0 * INCH], Dim::LENGTH);
    close_all(&grid(&mut env, "D/2 =").1, &[0.5 * INCH, INCH, 1.5 * INCH], Dim::LENGTH);
    close_all(&grid(&mut env, "D^2 =").1, &[INCH * INCH, 4.0 * INCH * INCH, 9.0 * INCH * INCH], AREA);
    close_all(&grid(&mut env, "1 in - D =").1, &[0.0, -INCH, -2.0 * INCH], Dim::LENGTH);
    close_all(&grid(&mut env, "-D =").1, &[-INCH, -2.0 * INCH, -3.0 * INCH], Dim::LENGTH);
    close_all(&grid(&mut env, "|1 in - D| =").1, &[0.0, INCH, 2.0 * INCH], Dim::LENGTH);
    // A single value's unit rules still hold for every element.
    let e = run_value(&mut env, "D + 1 lbf =").expect_err("can't add force to length");
    assert!(e.to_string().starts_with("element 0: "), "{e}");
    // Single values are unchanged: their errors aren't about an element.
    assert!(matches!(err("x := 1 in + 1 lbf ="), CalcError::Unit(UnitError::Mismatch { .. })));
}

#[test]
fn shape_mismatch_is_named() {
    let mut env = Env::new();
    column(&mut env, "D", &[1.0, 2.0, 3.0], "in");
    column(&mut env, "E", &[1.0, 2.0, 3.0, 4.0], "in");
    let e = run_value(&mut env, "D + E =").expect_err("shapes differ");
    assert_eq!(e, CalcError::Shape { left: (3, 1), right: (4, 1) });
    assert_eq!(e.to_string(), "shapes don't match: 3 rows × 1 vs 4 rows × 1");
    // A row isn't a column.
    env.define_array("R", 1, 3, quantities(&[1.0, 2.0, 3.0], "in")).expect("defines");
    assert_eq!(run_value(&mut env, "D·R ="), Err(CalcError::Shape { left: (3, 1), right: (1, 3) }));
}

#[test]
fn element_error_names_the_element() {
    let mixed = vec![quantities(&[1.0], "in")[0], quantities(&[2.0], "lbf")[0]];
    for (origin, at) in [(0, "element 1: "), (1, "element 2: ")] {
        let mut env = Env::new();
        env.set_origin(origin);
        env.define_array("P", 2, 1, mixed.clone()).expect("defines");
        column(&mut env, "Q", &[1.0, 1.0], "in");
        let e = run_value(&mut env, "P + Q =").expect_err("lbf + in");
        assert!(e.to_string().starts_with(at), "origin {origin}: {e}");
        assert!(matches!(e, CalcError::Element { ref error, .. } if matches!(**error, CalcError::Unit(UnitError::Mismatch { .. }))), "{e:?}");
    }
    // A vector element has one index (a row vector counts along its columns); a matrix element two.
    assert_eq!(element_name(3, 1, 2, 0), "2");
    assert_eq!(element_name(1, 3, 2, 1), "3");
    assert_eq!(element_name(2, 3, 4, 0), "[1, 1]");
    assert_eq!(element_name(2, 3, 5, 1), "[2, 3]");
    // An origin other than 0 or 1 is 0.
    let mut env = Env::new();
    env.set_origin(7);
    env.define_array("P", 2, 1, mixed).expect("defines");
    column(&mut env, "Q", &[1.0, 1.0], "in");
    assert!(run_value(&mut env, "P + Q =").expect_err("lbf + in").to_string().starts_with("element 1: "));
}

#[test]
fn builtins_apply_element_wise() {
    let mut env = Env::new();
    column(&mut env, "v", &[4.0, 9.0], "1");
    column(&mut env, "r", &[1.26, 2.34], "1");
    let (shape, items) = grid(&mut env, "sqrt(v) =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[2.0, 3.0], Dim::ONE);
    close_all(&grid(&mut env, "round(r, 1) =").1, &[1.3, 2.3], Dim::ONE);
    close_all(&grid(&mut env, "mod(v, 4) =").1, &[0.0, 1.0], Dim::ONE);
    close_all(&grid(&mut env, "log(100, v) =").1, &[100f64.ln() / 4f64.ln(), 100f64.ln() / 9f64.ln()], Dim::ONE);
    // Arity is checked before any element is.
    assert_eq!(run_value(&mut env, "sqrt(v, v) ="), Err(CalcError::Arity("sqrt".into(), 1)));
    // A per-element error names the element.
    column(&mut env, "w", &[1.0, -1.0], "1");
    assert!(run_value(&mut env, "sqrt(w) =").expect_err("sqrt(-1)").to_string().starts_with("element 1: "));
    // min and max reduce an array to one value rather than applying element-wise.
    close(shown(&mut env, "max(v) ="), 9.0, Dim::ONE);
    close(shown(&mut env, "min(v, 1) ="), 1.0, Dim::ONE);
}

#[test]
fn user_functions_apply_element_wise() {
    let mut env = Env::new();
    column(&mut env, "v", &[1.0, 2.0], "1");
    run(&mut env, "f(x) := x^2 + 1").expect("defines");
    let (shape, items) = grid(&mut env, "f(v) =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[2.0, 5.0], Dim::ONE);
}

#[test]
fn arrays_are_bounded() {
    let mut env = Env::new();
    let e = env.define_array("big", 10_001, 1, vec![Quantity::dimensionless(1.0); 10_001]).expect_err("too large");
    assert_eq!(e, CalcError::TooLarge);
    assert_eq!(e.to_string(), "more than 10,000 elements");
    assert!(env.define_array("big", 100, 101, Vec::new()).is_err_and(|e| e == CalcError::TooLarge));
    // The cap is checked before the size, and an overflowing shape is too large rather than a panic.
    assert_eq!(Grid::<Val>::new(101, 100, Vec::new()), Err(CalcError::TooLarge));
    assert_eq!(Grid::<Val>::new(usize::MAX, 2, Vec::new()), Err(CalcError::TooLarge));
    // A size that doesn't match the shape, or no elements at all, is refused.
    assert!(Grid::<Val>::new(2, 2, Vec::new()).is_err());
    assert!(Grid::<Val>::new(0, 1, Vec::new()).is_err());
    // 10,000 is allowed, and so is arithmetic on it.
    column(&mut env, "D", &vec![1.0; 10_000], "in");
    assert_eq!(grid(&mut env, "D + D =").0, (10_000, 1));
}

#[test]
fn element_operations_count_against_the_budget() {
    let stmt = |src: &str| parse(src).expect("parses");
    let mut env = Env::new();
    column(&mut env, "D", &vec![1.0; 10_000], "in");
    column(&mut env, "N", &vec![4.0; 10_000], "1");
    // Each costs one step per node, 10,000 for reading the grid and 10,000 for the element-wise
    // operation (an operator, a negation, a built-in).
    for (src, nodes) in [("x := 2·D", 3), ("y := -D", 2), ("z := sqrt(N)", 2)] {
        let pass = Pass::new(u64::MAX);
        run_stmt_in(&mut env, &stmt(src), &pass).expect("runs");
        assert_eq!(pass.spent.get(), nodes + 20_000, "{src}");
        // The read fits in this budget; the operation doesn't.
        assert_eq!(run_stmt_in(&mut env, &stmt(src), &Pass::new(15_000)), Err(CalcError::TooMuchWork), "{src}");
    }
    assert_eq!(run_stmt_in(&mut env, &stmt("w := 2·D"), &Pass::new(5_000)), Err(CalcError::TooMuchWork));
    // Single values cost what they did: one step per node.
    let pass = Pass::new(u64::MAX);
    run_stmt_in(&mut env, &stmt("k := 2·3"), &pass).expect("runs");
    assert_eq!(pass.spent.get(), 3);
}

// ---------------------------------------------------------------------------------------------
// Literals, indexes, ranges and indexed definitions
// ---------------------------------------------------------------------------------------------

/// An env holding `D := [0.840, 1.050, 1.315] in` and `M := [1, 2; 3, 4]`, numbered from `origin`.
fn pipes(origin: u8) -> Env {
    let mut env = Env::new();
    env.set_origin(origin);
    run(&mut env, "D := [0.840, 1.050, 1.315] in").expect("defines D");
    run(&mut env, "M := [1, 2; 3, 4]").expect("defines M");
    env
}

fn index_range_msg(e: CalcError) -> String {
    match e {
        CalcError::IndexRange(m) => m,
        other => panic!("expected an index range error, got {other:?}"),
    }
}

#[test]
fn literal_evaluates() {
    let mut env = pipes(0);
    let (shape, items) = grid(&mut env, "D =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[0.840 * INCH, 1.050 * INCH, 1.315 * INCH], Dim::LENGTH);
    // Each element keeps its own unit.
    let (shape, items) = grid(&mut env, "[2 in, 50 mm] =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[2.0 * INCH, 0.050], Dim::LENGTH);
    // A matrix, row by row.
    let (shape, items) = grid(&mut env, "M =");
    assert_eq!(shape, (2, 2));
    close_all(&items, &[1.0, 2.0, 3.0, 4.0], Dim::ONE);
    // Elements are expressions.
    close_all(&grid(&mut env, "[1 + 1, D[0]/(1 in)] =").1, &[2.0, 0.840], Dim::ONE);
    // An element that is itself a grid is refused, whichever way it is written.
    for src in ["[2 [1]] =", "[D, 1] =", "[1, 2·D] ="] {
        assert_eq!(syntax_msg(run_value(&mut env, src).expect_err(src)), "a matrix element can't be a vector", "{src}");
    }
}

#[test]
fn index_reads_an_element() {
    let mut env = pipes(0);
    close(shown(&mut env, "D[0] ="), 0.840 * INCH, Dim::LENGTH);
    close(shown(&mut env, "D[2] ="), 1.315 * INCH, Dim::LENGTH);
    close(shown(&mut env, "M[1, 0] ="), 3.0, Dim::ONE);
    close(shown(&mut env, "M[0, 1] ="), 2.0, Dim::ONE);
    // An index is an expression, and a column vector may also be given a row and a column.
    close(shown(&mut env, "D[3 - 2] ="), 1.050 * INCH, Dim::LENGTH);
    close(shown(&mut env, "D[1, 0] ="), 1.050 * INCH, Dim::LENGTH);
    // A function's parameter may be indexed.
    run(&mut env, "f(x) := x[1]").expect("defines f");
    close(shown(&mut env, "f(D) ="), 1.050 * INCH, Dim::LENGTH);

    let mut env = pipes(1);
    close(shown(&mut env, "D[1] ="), 0.840 * INCH, Dim::LENGTH);
    close(shown(&mut env, "D[3] ="), 1.315 * INCH, Dim::LENGTH);
    close(shown(&mut env, "M[2, 1] ="), 3.0, Dim::ONE);
}

#[test]
fn index_must_be_a_whole_number_in_range() {
    let mut env = pipes(0);
    for src in ["D[1.5] =", "D[1 in] =", "D[1 rad] =", "D[[0]] =", "M[0.5, 0] =", "M[0, 1 in] ="] {
        assert_eq!(syntax_msg(run_value(&mut env, src).expect_err(src)), "an index is a whole number, like D[2]", "{src}");
    }
    // Within 1e-9 of a whole number is that number.
    close(shown(&mut env, "D[1 + 10^-12] ="), 1.050 * INCH, Dim::LENGTH);
    assert_eq!(index_range_msg(run_value(&mut env, "D[3] =").expect_err("past the end")), "index 3 is outside D (0 to 2)");
    assert_eq!(index_range_msg(run_value(&mut env, "D[-1] =").expect_err("before the start")), "index -1 is outside D (0 to 2)");
    assert_eq!(index_range_msg(run_value(&mut env, "D[10^300] =").expect_err("far past the end")).split(' ').next(), Some("index"));
    assert_eq!(index_range_msg(run_value(&mut env, "M[1] =").expect_err("one index")), "M is a matrix — give a row and a column, like M[1, 0]");
    assert_eq!(
        index_range_msg(run_value(&mut env, "M[2, 0] =").expect_err("no third row")),
        "index [2, 0] is outside M (rows 0 to 1, columns 0 to 1)"
    );
    run(&mut env, "x := 5").expect("defines x");
    assert_eq!(index_range_msg(run_value(&mut env, "x[0] =").expect_err("not a vector")), "x isn't a vector");
    assert_eq!(index_range_msg(run_value(&mut env, "π[0] =").expect_err("not a vector")), "π isn't a vector");
    assert_eq!(run_value(&mut env, "z[0] ="), Err(CalcError::Undefined("z".into())));
    run(&mut env, "f(x) := x").expect("defines f");
    assert!(matches!(run_value(&mut env, "f[0] ="), Err(CalcError::Syntax(_))));
    env.mark_failed("F");
    assert_eq!(run_value(&mut env, "F[0] ="), Err(CalcError::Upstream));
    // Counting from 1, 0 is outside.
    let mut env = pipes(1);
    assert_eq!(index_range_msg(run_value(&mut env, "D[0] =").expect_err("before the start")), "index 0 is outside D (1 to 3)");
}

#[test]
fn ranges_evaluate() {
    let mut env = Env::new();
    // A range includes both ends.
    let (shape, items) = grid(&mut env, "i := 0..5 =");
    assert_eq!(shape, (6, 1));
    close_all(&items, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], Dim::ONE);
    close_all(&grid(&mut env, "x := 1, 1.5..3 =").1, &[1.0, 1.5, 2.0, 2.5, 3.0], Dim::ONE);
    close_all(&grid(&mut env, "len := 0 in, 0.5 in..1 in =").1, &[0.0, 0.5 * INCH, INCH], Dim::LENGTH);
    // With no step, a range counts down as well as up.
    close_all(&grid(&mut env, "c := 5..0 =").1, &[5.0, 4.0, 3.0, 2.0, 1.0, 0.0], Dim::ONE);
    // The ends are expressions; a range of one element is a vector.
    run(&mut env, "n := 3").expect("defines n");
    close_all(&grid(&mut env, "k := 0..n-1 =").1, &[0.0, 1.0, 2.0], Dim::ONE);
    assert_eq!(grid(&mut env, "o := 2..2 =").0, (1, 1));
    // A step that doesn't land on the last value stops short of it.
    close_all(&grid(&mut env, "ev := 0, 2..5 =").1, &[0.0, 2.0, 4.0], Dim::ONE);
    // 10,000 elements is allowed.
    assert_eq!(grid(&mut env, "big := 1..10000 =").0, (10_000, 1));
    // A step that lands on the last value includes it, however the step rounds.
    let tenths = grid(&mut env, "x := 0, 0.1..1 =").1;
    assert_eq!(tenths.len(), 11, "{tenths:?}");
    assert_eq!(tenths.last().map(|q| q.value), Some(1.0));
}

/// The ends of a range, and so its step, carry rounding error in proportion to their size: a
/// range of large values whose step lands on its last value still ends on it.
#[test]
fn a_range_of_large_values_keeps_its_last_element() {
    let mut env = Env::new();
    for (src, count, first, last) in [
        ("x := 100000, 100000.1..100002 =", 21, 100_000.0, 100_002.0),
        ("x := 1000000, 1000000.01..1000000.1 =", 11, 1_000_000.0, 1_000_000.1),
        ("x := 100002, 100001.9..100000 =", 21, 100_002.0, 100_000.0),
        ("x := 500000, 500000.001..500000.1 =", 101, 500_000.0, 500_000.1),
        ("x := 250000, 250000.05..250010 =", 201, 250_000.0, 250_010.0),
    ] {
        let (shape, items) = grid(&mut env, src);
        assert_eq!(shape, (count, 1), "{src}");
        assert_eq!(items.first().map(|q| q.value), Some(first), "{src}");
        // The last element is the last value as written, exactly.
        assert_eq!(items.last().map(|q| q.value), Some(last), "{src}");
    }
    // A step that doesn't land still stops short of the last value, at any size.
    let (shape, items) = grid(&mut env, "x := 1000000, 1000002..1000005 =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[1_000_000.0, 1_000_002.0, 1_000_004.0], Dim::ONE);
    // Counting by 1 from a large value is exact.
    let (shape, items) = grid(&mut env, "x := 1000000000000..1000000000005 =");
    assert_eq!(shape, (6, 1));
    assert_eq!(items.last().map(|q| q.value), Some(1e12 + 5.0));
    // The small ranges are as they were.
    close_all(&grid(&mut env, "x := 1, 1.5..3 =").1, &[1.0, 1.5, 2.0, 2.5, 3.0], Dim::ONE);
    close_all(&grid(&mut env, "x := 0..5 =").1, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], Dim::ONE);
    // A step too small for its values to tell where the range ends is an error, not a guess:
    // 1e15 + 0.1 is 1e15 + 0.125 in a double.
    let fine = run(&mut env, "x := 1e15, 1e15 + 0.1..1e15 + 1").expect_err("a step below the values' precision");
    assert_eq!(syntax_msg(fine), "a range's step is too small next to its values to tell exactly where it ends");
}

#[test]
fn bad_ranges_are_errors() {
    let mut env = Env::new();
    column(&mut env, "D", &[1.0, 2.0], "in");
    let zero = run(&mut env, "x := 1, 1..3").expect_err("zero step");
    assert!(matches!(zero, CalcError::Syntax(_)), "{zero:?}");
    let away = run(&mut env, "x := 1, 0..3").expect_err("wrong direction");
    assert!(matches!(away, CalcError::Syntax(_)), "{away:?}");
    assert!(matches!(run(&mut env, "x := 0..5 in"), Err(CalcError::Unit(UnitError::Mismatch { .. }))));
    assert!(matches!(run(&mut env, "x := 0 in, 1..5 in"), Err(CalcError::Unit(UnitError::Mismatch { .. }))));
    assert_eq!(run(&mut env, "x := 0..20000"), Err(CalcError::TooLarge));
    // A count too large for any integer is too large too, not a panic or a wrap.
    assert_eq!(run(&mut env, "x := 0, 10^-300..1"), Err(CalcError::TooLarge));
    assert_eq!(run(&mut env, "x := 0, 10^-300..10^300"), Err(CalcError::TooLarge));
    // A range with units needs a step: counting by 1 in SI units would be a wrong-looking answer.
    assert!(matches!(run(&mut env, "x := 0 in..1 in"), Err(CalcError::Syntax(_))));
    // The ends are single values.
    assert!(matches!(run(&mut env, "x := D..5"), Err(CalcError::Syntax(_))));
    // A range defines a plain variable only.
    for src in ["f(x) := 0..x", "t[i] := 0..5"] {
        assert_eq!(syntax_msg(run(&mut env, src).expect_err(src)), "a range defines a variable, like i := 0..5", "{src}");
    }
    let built = Expr::Binary(
        Op::Add,
        Box::new(Expr::Range { first: Box::new(Expr::Num(0.0)), second: None, last: Box::new(Expr::Num(2.0)) }),
        Box::new(Expr::Num(1.0)),
    );
    assert_eq!(syntax_msg(eval_expr(&env, &built).expect_err("a range inside an expression")), "a range defines a variable, like i := 0..5");
    // Nothing failed defined anything.
    assert_eq!(env.names(), vec!["D".to_string()]);
    // The range's elements are charged before they are built.
    let pass = Pass::new(u64::MAX);
    run_stmt_in(&mut env, &parse("r := 0..9999").expect("parses"), &pass).expect("runs");
    assert_eq!(pass.spent.get(), 3 + 10_000);
    assert_eq!(run_stmt_in(&mut env, &parse("r := 0..9999").expect("parses"), &Pass::new(5_000)), Err(CalcError::TooMuchWork));
}

#[test]
fn indexed_definition_fills_a_vector() {
    let mut env = pipes(0);
    run(&mut env, "i := 0..2").expect("defines i");
    assert_eq!(run(&mut env, "t[i] := 2·D[i]"), Ok(None));
    let (shape, items) = grid(&mut env, "t =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[1.68 * INCH, 2.1 * INCH, 2.63 * INCH], Dim::LENGTH);
    // Shown as it is defined; `i` is each element in turn.
    close_all(&grid(&mut env, "sq[i] := i^2 =").1, &[0.0, 1.0, 4.0], Dim::ONE);
    // Counting from 1.
    let mut env = pipes(1);
    run(&mut env, "i := 1..3").expect("defines i");
    run(&mut env, "t[i] := 2·D[i]").expect("defines t");
    close_all(&grid(&mut env, "t =").1, &[1.68 * INCH, 2.1 * INCH, 2.63 * INCH], Dim::LENGTH);
    // 10,000 elements fit in one recalculation's budget: reading D[i] doesn't copy D.
    let mut env = Env::new();
    column(&mut env, "D", &vec![1.0; 10_000], "in");
    run(&mut env, "i := 0..9999").expect("defines i");
    let pass = Pass::new(super::MAX_PASS_STEPS);
    run_stmt_in(&mut env, &parse("t[i] := 2·D[i]").expect("parses"), &pass).expect("fits the budget");
    assert_eq!(grid(&mut env, "t =").0, (10_000, 1));
}

#[test]
fn indexed_definition_is_strict() {
    let mut env = pipes(0);
    run(&mut env, "i := 1..2").expect("defines i");
    assert_eq!(syntax_msg(run(&mut env, "t[i] := 2·D[i]").expect_err("t[0] unset")), "t[0] isn't set by i — the range must cover every element");
    // The body giving a grid for an element is an error.
    run(&mut env, "j := 0..2").expect("defines j");
    assert!(matches!(run(&mut env, "u[j] := D"), Err(CalcError::Syntax(_))));
    // The range variable must be a vector of whole, plain numbers.
    run(&mut env, "hf := 0, 0.5..2").expect("defines hf");
    run(&mut env, "q := 0 in, 1 in..2 in").expect("defines q");
    run(&mut env, "k := 1").expect("defines k");
    for src in ["v[hf] := 1", "v[q] := 1", "v[k] := 1", "v[M] := 1"] {
        assert!(matches!(run(&mut env, src), Err(CalcError::Syntax(_))), "{src}");
    }
    // An index before the origin, or one making the vector too large, is refused before any work.
    run(&mut env, "n := -1..2").expect("defines n");
    assert!(matches!(run(&mut env, "v[n] := 1"), Err(CalcError::IndexRange(_))));
    run(&mut env, "w := [0, 20000]").expect("defines w");
    assert_eq!(run(&mut env, "v[w] := 1"), Err(CalcError::TooLarge));
    assert_eq!(run(&mut env, "v[z] := 1"), Err(CalcError::Undefined("z".into())));
    // An element's error names the element.
    let e = run(&mut env, "v[j] := 1/(j - 1)").expect_err("1/0");
    assert_eq!(e, CalcError::Element { at: "1".into(), error: Box::new(CalcError::NonFinite) });
    // Nothing failed defined anything.
    for name in ["t", "u", "v"] {
        assert!(!env.names().contains(&name.to_string()), "{name}");
    }
}

#[test]
fn nested_indexes_are_bounded_not_a_stack_overflow() {
    let mut env = Env::new();
    run(&mut env, "Z := [0]").expect("defines Z");
    // As deep as the parser allows.
    let src = format!("{}0{} =", "Z[".repeat(60), "]".repeat(60));
    close(shown(&mut env, &src), 0.0, Dim::ONE);
    // Hand-built: just inside the evaluator's depth limit evaluates, far past it is refused. An
    // index or literal level costs less stack than a user-function call level (eval → call →
    // call_user → eval), which already needs about 2.7 MB at the depth limit; 4 MB covers both.
    let deep = move || {
        let nest = |n: usize| (0..n).fold(Expr::Num(0.0), |e, _| Expr::Index("Z".into(), vec![e]));
        assert_eq!(eval_expr(&env, &nest(2000)), Ok(Value::One(Quantity::dimensionless(0.0))));
        assert_eq!(eval_expr(&env, &nest(3000)), Err(CalcError::TooComplex));
        let literals =
            |n: usize| (0..n).fold(Expr::Num(0.0), |e, _| Expr::Index("Z".into(), vec![Expr::Matrix { rows: 1, cols: 1, items: vec![e] }]));
        assert!(eval_expr(&env, &literals(3000)).is_err());
        let matrices = |n: usize| (0..n).fold(Expr::Num(0.0), |e, _| Expr::Matrix { rows: 1, cols: 1, items: vec![e] });
        assert_eq!(eval_expr(&env, &matrices(3000)), Err(CalcError::TooComplex));
    };
    std::thread::Builder::new().stack_size(4 << 20).spawn(deep).expect("spawns").join().expect("no panic");
}

// ---------------------------------------------------------------------------------------------
// Array functions
// ---------------------------------------------------------------------------------------------

/// `q` is close to `want`, dimension and value.
fn close_to(q: Quantity, want: Quantity) {
    close(q, want.value, want.dim);
}

/// One value in `unit`, as an SI quantity.
fn quantity(value: f64, unit: &str) -> Quantity {
    quantities(&[value], unit).into_iter().next().expect("one quantity")
}

/// An env holding `NPS := [0.5, 0.75, 1, 1.5] in` and the flange torques
/// `T := [30, 45, 60, 90] ft·lbf`.
fn flanges() -> Env {
    let mut env = Env::new();
    run(&mut env, "NPS := [0.5, 0.75, 1, 1.5] in").expect("defines NPS");
    run(&mut env, "T := [30, 45, 60, 90] ft·lbf").expect("defines T");
    env
}

#[test]
fn size_functions() {
    let mut env = Env::new();
    run(&mut env, "M := [1, 2; 3, 4; 5, 6]").expect("defines M");
    close(shown(&mut env, "length(M) ="), 6.0, Dim::ONE);
    close(shown(&mut env, "rows(M) ="), 3.0, Dim::ONE);
    close(shown(&mut env, "cols(M) ="), 2.0, Dim::ONE);
    close(shown(&mut env, "length(5 in) ="), 1.0, Dim::ONE);
    assert_eq!(run(&mut env, "length(M, M) ="), Err(CalcError::Arity("length".into(), 1)));
}

#[test]
fn reductions() {
    let mut env = pipes(0);
    close(shown(&mut env, "min([3, 1, 2] in) ="), INCH, Dim::LENGTH);
    close(shown(&mut env, "max(D, 2 in) ="), 2.0 * INCH, Dim::LENGTH);
    close(shown(&mut env, "max(D) ="), 1.315 * INCH, Dim::LENGTH);
    close_to(shown(&mut env, "sum([1, 2] lbf) ="), quantity(3.0, "lbf"));
    close(shown(&mut env, "mean([1, 2, 3] in) ="), 2.0 * INCH, Dim::LENGTH);
    assert!(matches!(run(&mut env, "min([1 in, 2 lbf]) ="), Err(CalcError::Unit(_))));
    assert!(matches!(run(&mut env, "sum([1 in, 2 lbf]) ="), Err(CalcError::Unit(_))));
    assert_eq!(run(&mut env, "sum(D, D) ="), Err(CalcError::Arity("sum".into(), 1)));
}

#[test]
fn products() {
    let mut env = Env::new();
    close_to(shown(&mut env, "dot([1, 2] in, [3, 4] lbf) ="), quantity(11.0, "lbf·in"));
    let (shape, items) = grid(&mut env, "mmul([1, 2; 3, 4], [5; 6]) =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[17.0, 39.0], Dim::ONE);
    // The inner sizes must agree: the message names the arguments, by name when they are names.
    let e = run_value(&mut env, "mmul([1, 2, 3; 4, 5, 6], [1, 2, 3; 4, 5, 6]) =").expect_err("3 columns by 2 rows");
    assert_eq!(syntax_msg(e), "mmul needs A's columns to match B's rows (3 and 2)");
    run(&mut env, "M := [1, 2, 3; 4, 5, 6]").expect("defines M");
    run(&mut env, "X := [1; 2]").expect("defines X");
    let e = run_value(&mut env, "mmul(M, X) =").expect_err("3 columns by 2 rows");
    assert_eq!(syntax_msg(e), "mmul needs M's columns to match X's rows (3 and 2)");
    let (shape, items) = grid(&mut env, "transpose([1, 2, 3]) =");
    assert_eq!(shape, (1, 3));
    close_all(&items, &[1.0, 2.0, 3.0], Dim::ONE);
    let (shape, items) = grid(&mut env, "transpose([1, 2; 3, 4; 5, 6]) =");
    assert_eq!(shape, (2, 3));
    close_all(&items, &[1.0, 3.0, 5.0, 2.0, 4.0, 6.0], Dim::ONE);
    assert_eq!(syntax_msg(run(&mut env, "dot([1, 2], [1, 2, 3]) =").expect_err("lengths")), "dot needs a and b of the same length (2 and 3)");
}

/// An env holding `M := [1, 2, 3; 4, 5, 6; 7, 8, 9]`, numbered from `origin`.
fn nine(origin: u8) -> Env {
    let mut env = Env::new();
    env.set_origin(origin);
    run(&mut env, "M := [1, 2, 3; 4, 5, 6; 7, 8, 9]").expect("defines M");
    env
}

#[test]
fn submatrix_counts_from_index_base() {
    let (shape, items) = grid(&mut nine(0), "submatrix(M, 1, 2, 0, 1) =");
    assert_eq!(shape, (2, 2));
    close_all(&items, &[4.0, 5.0, 7.0, 8.0], Dim::ONE);
    let (shape, items) = grid(&mut nine(1), "submatrix(M, 1, 2, 1, 2) =");
    assert_eq!(shape, (2, 2));
    close_all(&items, &[1.0, 2.0, 4.0, 5.0], Dim::ONE);
    // One row, one column, and one element: a single value is a 1 × 1 matrix.
    let (shape, items) = grid(&mut nine(0), "submatrix(M, 2, 2, 0, 2) =");
    assert_eq!(shape, (1, 3));
    close_all(&items, &[7.0, 8.0, 9.0], Dim::ONE);
    close(shown(&mut nine(0), "submatrix(M, 1, 1, 1, 1) ="), 5.0, Dim::ONE);
    close(shown(&mut Env::new(), "submatrix(7, 0, 0, 0, 0) ="), 7.0, Dim::ONE);
    // Units stay with the elements, and a text matrix gives texts.
    let mut env = Env::new();
    run(&mut env, "Q1 := [1, 2; 3, 4] in").expect("defines Q1");
    let (_, items) = grid(&mut env, "submatrix(Q1, 1, 1, 0, 1) =");
    close_all(&items, &[3.0 * INCH, 4.0 * INCH], Dim { exp: [1, 0, 0, 0, 0, 0, 0, 0] });
    run(&mut env, "T := [\"a\", \"b\"; \"c\", \"d\"]").expect("defines T");
    assert_eq!(
        run_value(&mut env, "submatrix(T, 0, 1, 1, 1) ="),
        Ok(Some(Value::TextGrid { rows: 2, cols: 1, items: vec!["b".into(), "d".into()] }))
    );
    assert_eq!(run_value(&mut env, "submatrix(T, 1, 1, 0, 0) ="), Ok(Some(Value::Text("c".into()))));
}

#[test]
fn submatrix_range_errors_name_what_is_wrong() {
    let msg = |origin: u8, src: &str| match run_value(&mut nine(origin), src) {
        Err(CalcError::IndexRange(m) | CalcError::Syntax(m)) => m,
        other => panic!("{src:?} should be an index or syntax error: {other:?}"),
    };
    assert_eq!(msg(0, "submatrix(M, 1, 3, 0, 1) ="), "submatrix row 3 (jr) is outside M (rows 0 to 2)");
    assert_eq!(msg(0, "submatrix(M, 0, 1, 0, 3) ="), "submatrix column 3 (jc) is outside M (columns 0 to 2)");
    assert_eq!(msg(1, "submatrix(M, 0, 1, 1, 2) ="), "submatrix row 0 (ir) is outside M (rows 1 to 3)");
    assert_eq!(msg(0, "submatrix(M, 0, 1, -1, 1) ="), "submatrix column -1 (ic) is outside M (columns 0 to 2)");
    assert_eq!(msg(0, "submatrix(M, 2, 1, 0, 1) ="), "submatrix needs ir to be at most jr (rows 2 and 1)");
    assert_eq!(msg(0, "submatrix(M, 0, 1, 2, 0) ="), "submatrix needs ic to be at most jc (columns 2 and 0)");
    assert_eq!(msg(0, "submatrix(M, 0.5, 1, 0, 1) ="), "submatrix needs ir to be a whole number");
    assert_eq!(msg(0, "submatrix(M, 0, 1, 0, [1, 2]) ="), "submatrix needs jc to be a whole number");
    assert_eq!(msg(0, "submatrix(M, 0, 1 in, 0, 1) ="), "submatrix needs jr to be a whole number");
    assert!(matches!(err("submatrix([1, 2], 0, 0, 0) ="), CalcError::Arity(..)));
}

#[test]
fn augment_and_stack() {
    let (shape, items) = grid(&mut Env::new(), "augment([1; 2], [3; 4]) =");
    assert_eq!(shape, (2, 2));
    close_all(&items, &[1.0, 3.0, 2.0, 4.0], Dim::ONE);
    let (shape, items) = grid(&mut Env::new(), "stack(transpose([1, 2]), transpose([3, 4])) =");
    assert_eq!(shape, (2, 2));
    close_all(&items, &[1.0, 2.0, 3.0, 4.0], Dim::ONE);
    // More than two, matrices as well as vectors, and single values as 1 × 1.
    let (shape, items) = grid(&mut Env::new(), "augment([1, 2; 3, 4], [5; 6], [7, 8; 9, 10]) =");
    assert_eq!(shape, (2, 5));
    close_all(&items, &[1.0, 2.0, 5.0, 7.0, 8.0, 3.0, 4.0, 6.0, 9.0, 10.0], Dim::ONE);
    // (`[1, 2]` is a column: a row is `transpose([1, 2])`.)
    let (shape, items) = grid(&mut Env::new(), "stack(transpose([1, 2]), [3, 4; 5, 6], transpose([7, 8])) =");
    assert_eq!(shape, (4, 2));
    close_all(&items, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], Dim::ONE);
    let (shape, items) = grid(&mut Env::new(), "stack([1, 2], 3, [4; 5]) =");
    assert_eq!(shape, (5, 1));
    close_all(&items, &[1.0, 2.0, 3.0, 4.0, 5.0], Dim::ONE);
    let (shape, items) = grid(&mut Env::new(), "augment(1, 2) =");
    assert_eq!((shape, items.len()), ((1, 2), 2));
    // The sizes must agree, and the message gives both.
    let mut env = Env::new();
    run(&mut env, "V1 := [1; 2]").expect("defines V1");
    let e = run_value(&mut env, "augment(V1, [1; 2; 3]) =").expect_err("2 rows and 3 rows");
    assert_eq!(syntax_msg(e), "augment needs V1 and argument 2 to have the same number of rows (V1 is 2 × 1, argument 2 is 3 × 1)");
    let e = run_value(&mut env, "augment([1; 2], [3; 4], [5]) =").expect_err("2 rows and 1 row");
    assert_eq!(syntax_msg(e), "augment needs argument 1 and argument 3 to have the same number of rows (argument 1 is 2 × 1, argument 3 is 1 × 1)");
    let e = run_value(&mut env, "stack(transpose([1, 2]), transpose([1, 2, 3])) =").expect_err("2 columns and 3 columns");
    assert_eq!(syntax_msg(e), "stack needs argument 1 and argument 2 to have the same number of columns (argument 1 is 1 × 2, argument 2 is 1 × 3)");
    assert!(matches!(err("augment([1; 2]) ="), CalcError::Arity(..)));
    assert!(matches!(err("stack([1, 2]) ="), CalcError::Arity(..)));
    assert!(matches!(err("augment() ="), CalcError::Arity(..)));
    // Units are kept per element.
    let mut env = Env::new();
    let (shape, items) = grid(&mut env, "augment([1; 2] in, [3; 4] lbf) =");
    assert_eq!(shape, (2, 2));
    let (len, force) = (quantities(&[1.0], "in")[0].dim, quantities(&[1.0], "lbf")[0].dim);
    close(items[0], INCH, len);
    close(items[1], quantities(&[3.0], "lbf")[0].value, force);
    close(items[2], 2.0 * INCH, len);
    close(items[3], quantities(&[4.0], "lbf")[0].value, force);
}

#[test]
fn augment_text() {
    let mut env = Env::new();
    assert_eq!(
        run_value(&mut env, "augment([\"a\"; \"b\"], [\"c\"; \"d\"]) ="),
        Ok(Some(Value::TextGrid { rows: 2, cols: 2, items: vec!["a".into(), "c".into(), "b".into(), "d".into()] }))
    );
    assert_eq!(
        run_value(&mut env, "stack(transpose([\"a\", \"b\"]), \"c\") =").map(|_| ()).map_err(|e| e.to_string()),
        Err("stack needs argument 1 and argument 2 to have the same number of columns (argument 1 is 1 × 2, argument 2 is 1 × 1)".to_string())
    );
    assert_eq!(run_value(&mut env, "stack(\"a\", \"c\") ="), Ok(Some(Value::TextGrid { rows: 2, cols: 1, items: vec!["a".into(), "c".into()] })));
    // Text with numbers is the mixed error, naming where in the result each would sit.
    let e = run_value(&mut env, "augment([\"a\"; \"b\"], [1; 2]) =").expect_err("text with numbers");
    assert_eq!(syntax_msg(e), "element [0, 1] is a number but element [0, 0] is text");
    let e = run_value(&mut env, "augment([1; 2], [\"a\"; \"b\"]) =").expect_err("numbers with text");
    assert_eq!(syntax_msg(e), "element [0, 1] is text but element [0, 0] is a number");
    let e = run_value(&mut env, "stack(1, \"a\") =").expect_err("a number with a text");
    assert_eq!(syntax_msg(e), "element 1 is text but element 0 is a number");
}

#[test]
fn matrix_functions_are_charged_by_element() {
    let stmt = |src: &str| parse(src).unwrap_or_else(|e| panic!("{src:?} did not parse: {e}"));
    let spent = |env: &mut Env, src: &str| {
        let pass = Pass::new(u64::MAX);
        run_stmt_in(env, &stmt(src), &pass).unwrap();
        pass.spent.get()
    };
    let mut env = nine(0);
    // Each is what its operands cost (a name is its 9 elements and one step, a number one step), the
    // elements it makes (4, 18 and 27) and one step for the name defined.
    assert_eq!(spent(&mut env, "r1 := submatrix(M, 0, 1, 0, 1)"), (9 + 1) + 4 + 4 + 1);
    assert_eq!(spent(&mut env, "r2 := augment(M, M)"), 2 * (9 + 1) + 18 + 1);
    assert_eq!(spent(&mut env, "r3 := stack(M, M, M)"), 3 * (9 + 1) + 27 + 1);
    // A result over 10 000 elements is refused before any of it is made, and the work stops a budget.
    run(&mut env, "i := 0..4999").unwrap();
    run(&mut env, "v[i] := 1").unwrap();
    assert_eq!(run_value(&mut env, "augment(v, v, v) ="), Err(CalcError::TooLarge));
    // The size is refused first, before anything else about the matrices is looked at.
    run(&mut env, "tv[i] := \"a\"").unwrap();
    assert_eq!(run_value(&mut env, "augment(v, tv, v) ="), Err(CalcError::TooLarge));
    assert_eq!(run_value(&mut env, "stack(tv, v, v) ="), Err(CalcError::TooLarge));
    let pass = Pass::new(2 * 5_001 + 100);
    assert_eq!(run_stmt_in(&mut env, &stmt("w := stack(v, v)"), &pass), Err(CalcError::TooMuchWork));
    // One step short of the whole: submatrix's operand (5000 elements and one step), its four
    // numbers and the name defined fit in the budget; the 5000 elements it makes don't.
    let src = "w := submatrix(v, 0, 4999, 0, 0)";
    let made = 5_000;
    let total = spent(&mut env, src);
    assert_eq!(total, 5_001 + 4 + 1 + made);
    let pass = Pass::new(total - 1);
    assert_eq!(run_stmt_in(&mut env, &stmt(src), &pass), Err(CalcError::TooMuchWork));
    assert_eq!(pass.spent(), total - made, "it stops at submatrix's own charge, with none of it counted");
}

#[test]
fn lookup_is_exact() {
    let mut env = flanges();
    let t90 = quantity(90.0, "ft·lbf");
    assert_eq!(shown(&mut env, "lookup(NPS, T, 1.5 in) ="), t90);
    close_to(shown(&mut env, "lookup(NPS, T, 38.1 mm) ="), t90);
    assert_eq!(run(&mut env, "lookup(NPS, T, 2 in) ="), Err(CalcError::Syntax("2 in isn't in NPS".into())));
    // A vector x gives one result per element; a miss names the element.
    let (shape, items) = grid(&mut env, "lookup(NPS, T, [0.5, 1] in) =");
    assert_eq!(shape, (2, 1));
    assert_eq!(items, vec![quantity(30.0, "ft·lbf"), quantity(60.0, "ft·lbf")]);
    let e = run_value(&mut env, "lookup(NPS, T, [0.5, 0.6] in) =").expect_err("0.6 in isn't a size");
    assert_eq!(e.to_string(), "element 1: 0.6 in isn't in NPS");
    assert!(matches!(run(&mut env, "lookup(NPS, T, 1 lbf) ="), Err(CalcError::Unit(UnitError::Mismatch { .. }))));
    // A size listed twice matches more than one value: an error, never the first match.
    run(&mut env, "NPS_2 := [0.5, 1.5, 1, 1.5] in").expect("defines NPS_2");
    let e = run(&mut env, "lookup(NPS_2, T, 1.5 in) =").expect_err("two matches");
    assert_eq!(syntax_msg(e), "1.5 in matches more than one value in NPS_2");
    // Equal after conversion counts as a match too.
    let e = run(&mut env, "lookup(NPS_2, T, 38.1 mm) =").expect_err("two matches");
    assert_eq!(syntax_msg(e), "1.5 in matches more than one value in NPS_2");
    close_to(shown(&mut env, "lookup(NPS_2, T, 1 in) ="), quantity(60.0, "ft·lbf"));
}

#[test]
fn linterp_and_spline_never_extrapolate() {
    let mut env = flanges();
    close_to(shown(&mut env, "linterp(NPS, T, 1.25 in) ="), quantity(75.0, "ft·lbf"));
    close_to(shown(&mut env, "linterp(NPS, T, 38.1 mm) ="), quantity(90.0, "ft·lbf"));
    for f in ["linterp", "spline"] {
        let outside = run(&mut env, &format!("{f}(NPS, T, 1.6 in) =")).expect_err("past the end");
        assert_eq!(outside, CalcError::Syntax("1.6 in is outside NPS (0.5 in to 1.5 in)".into()), "{f}");
        let below = run(&mut env, &format!("{f}(NPS, T, 0.25 in) =")).expect_err("before the start");
        assert_eq!(below, CalcError::Syntax("0.25 in is outside NPS (0.5 in to 1.5 in)".into()), "{f}");
    }
    column(&mut env, "X", &[0.0, 1.0, 2.0, 3.0, 4.0], "1");
    column(&mut env, "L", &[1.0, 3.0, 5.0, 7.0, 9.0], "1");
    column(&mut env, "S", &[0.0, 1.0, 4.0, 9.0, 16.0], "1");
    // A natural spline reproduces a line exactly.
    assert_eq!(shown(&mut env, "spline(X, L, 1.5) ="), Quantity::dimensionless(4.0));
    // Through y = x², it gives the knot values at the knots and lies between neighbours elsewhere.
    for k in 0..5 {
        assert_eq!(shown(&mut env, &format!("spline(X, S, {k}) =")).value, f64::from(k * k), "x = {k}");
    }
    let between = shown(&mut env, "spline(X, S, 1.5) =").value;
    assert!(between > 1.0 && between < 4.0, "{between}");
    // The solver itself: the second derivatives through y = x² at 0…4 are 0, 18/7, 12/7, 18/7, 0,
    // so s(1.5) = 2.5 − (3/8)(18/7 + 12/7)/6 = 125/56.
    assert!((between - 125.0 / 56.0).abs() <= 1e-9, "{between}");
    // Uneven spacing (widths 1, 2, 3), where swapping an interval's neighbours' widths changes the
    // answer: through y = x² at 0, 1, 3, 6 the second derivatives are 0, 15/7, 18/7, 0, giving
    // s(2) = 107/28 and s(2.5) = 1367/224 (derived in the task 4 report).
    column(&mut env, "X_u", &[0.0, 1.0, 3.0, 6.0], "1");
    column(&mut env, "S_u", &[0.0, 1.0, 9.0, 36.0], "1");
    for (x, want) in [("2", 107.0 / 28.0), ("2.5", 1367.0 / 224.0)] {
        let got = shown(&mut env, &format!("spline(X_u, S_u, {x}) =")).value;
        assert!((got - want).abs() <= 1e-9, "s({x}) = {got}, want {want}");
    }
    // A vector x gives one result per element.
    let (shape, items) = grid(&mut env, "spline(X, L, [0.5, 1.5, 2.5]) =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[2.0, 4.0, 6.0], Dim::ONE);
    let (shape, items) = grid(&mut env, "linterp(NPS, T, [0.5, 1.25] in) =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[quantity(30.0, "ft·lbf").value, quantity(75.0, "ft·lbf").value], quantity(1.0, "ft·lbf").dim);
    let e = run_value(&mut env, "linterp(NPS, T, [1, 1.6] in) =").expect_err("1.6 in is past the end");
    assert_eq!(e.to_string(), "element 1: 1.6 in is outside NPS (0.5 in to 1.5 in)");
}

#[test]
fn interp_needs_increasing_xs() {
    let mut env = flanges();
    column(&mut env, "NPS_2", &[0.5, 1.0, 1.0, 1.5], "in");
    column(&mut env, "T_3", &[30.0, 45.0, 60.0], "ft·lbf");
    for f in ["linterp", "spline"] {
        let e = run(&mut env, &format!("{f}(NPS_2, T, 1.25 in) =")).expect_err("not increasing");
        assert_eq!(syntax_msg(e), format!("{f} needs NPS_2 strictly increasing — element 2 isn't more than element 1"));
        let e = run(&mut env, &format!("{f}(NPS, T_3, 1 in) =")).expect_err("lengths differ");
        assert_eq!(syntax_msg(e), format!("{f} needs NPS and T_3 of the same length (4 and 3)"));
    }
    let e = run(&mut env, "lookup(NPS, T_3, 1 in) =").expect_err("lengths differ");
    assert_eq!(syntax_msg(e), "lookup needs NPS and T_3 of the same length (4 and 3)");
    // An argument that isn't a bare name is called by what it is.
    let e = run(&mut env, "lookup([1, 2, 3, 4], [1, 2, 3], 1) =").expect_err("lengths differ");
    assert_eq!(syntax_msg(e), "lookup needs xs and ys of the same length (4 and 3)");
    let e = run(&mut env, "linterp([1, 3, 2], [1, 2, 3], 1.5) =").expect_err("not increasing");
    assert_eq!(syntax_msg(e), "linterp needs xs strictly increasing — element 2 isn't more than element 1");
}

/// The pump curve of `Mathcad Verification Simple Pump Model.xmcd`.
fn pump() -> Env {
    let mut env = Env::new();
    run(&mut env, "flowrate := [0, 1000, 2000, 3000, 3500, 4000] gpm").expect("defines flowrate");
    run(&mut env, "head := [230, 228.5, 221, 200.5, 183.5, 157] ft").expect("defines head");
    run(&mut env, "H(Q) := interp(cspline(flowrate, head), flowrate, head, Q)").expect("defines H");
    env
}

#[test]
fn cspline_matches_mathcad_pump_curve() {
    let mut env = pump();
    // Mathcad's saved values, to a relative 1e-12.
    for (q, want) in [(100.0, 229.892), (200.0, 229.796), (500.0, 229.5), (1000.0, 228.5)] {
        let got = shown(&mut env, &format!("H({q} gpm) ="));
        assert_eq!(got.dim, Dim::LENGTH, "H({q})");
        let want = quantity(want, "ft").value;
        assert!((got.value - want).abs() <= 1e-12 * want, "H({q} gpm) = {} vs {want}", got.value);
    }
    run(&mut env, "Qv := [0, 100, 200] gpm").expect("defines Qv");
    let (shape, items) = grid(&mut env, "H(Qv) =");
    assert_eq!(shape, (3, 1));
    for (got, want) in items.iter().zip([230.0, 229.892, 229.796]) {
        let want = quantity(want, "ft").value;
        assert_eq!(got.dim, Dim::LENGTH);
        assert!((got.value - want).abs() <= 1e-12 * want, "{} vs {want}", got.value);
    }
}

#[test]
fn spline_coefficient_vectors_have_the_documented_layout() {
    let mut env = pump();
    for (f, kind) in [("lspline", 1.0), ("pspline", 2.0), ("cspline", 3.0)] {
        run(&mut env, &format!("c := {f}(flowrate, head)")).expect("defines c");
        let (shape, items) = grid(&mut env, "c =");
        // n + 3 elements: the kind, 3 and 0, then a second derivative at each of the 6 points.
        assert_eq!(shape, (9, 1), "{f}");
        close_all(&items[..3], &[kind, 3.0, 0.0], Dim::ONE);
        // The second derivatives are in vy/vx².
        let per_flow_squared = Dim::LENGTH.div(quantity(1.0, "gpm").dim.powi(2).expect("squared")).expect("quotient");
        assert!(items[3..].iter().all(|q| q.dim == per_flow_squared), "{f}: {items:?}");
    }
    // The same positions in a document numbered from 1: element 0 of an lspline's vector is 1
    // (element 1 is 3 whatever the numbering), and base-1 c[5] is base-0 c[4], the second
    // derivative at the second point (not 0, as at the ends of a natural spline).
    run(&mut env, "c := lspline(flowrate, head)").expect("defines c");
    let second = shown(&mut env, "c[4] =");
    assert!(second.value != 0.0, "{second:?}");
    env.set_origin(1);
    close(shown(&mut env, "c[1] ="), 1.0, Dim::ONE);
    assert_eq!(shown(&mut env, "c[5] ="), second);
}

#[test]
fn lspline_is_natural() {
    let mut env = flanges();
    column(&mut env, "xs", &[0.0, 1.0, 3.0, 4.0, 7.0], "1");
    column(&mut env, "ys", &[2.0, -1.0, 4.0, 4.5, 0.0], "1");
    for x in ["0", "0.5", "2", "3.5", "5.25", "7"] {
        let natural = shown(&mut env, &format!("spline(xs, ys, {x}) =")).value;
        let fitted = shown(&mut env, &format!("interp(lspline(xs, ys), xs, ys, {x}) =")).value;
        assert!((natural - fitted).abs() <= 1e-12 * natural.abs().max(1.0), "x = {x}: {natural} vs {fitted}");
    }
}

#[test]
fn pspline_parabolic_ends() {
    let mut env = pump();
    run(&mut env, "c := pspline(flowrate, head)").expect("defines c");
    // M_0 = M_1 and M_5 = M_4; the other second derivatives differ.
    assert_eq!(shown(&mut env, "c[3] =").value, shown(&mut env, "c[4] =").value);
    assert_eq!(shown(&mut env, "c[8] =").value, shown(&mut env, "c[7] =").value);
    assert_ne!(shown(&mut env, "c[4] =").value, shown(&mut env, "c[5] =").value);
    // It passes through every point, and a parabola is reproduced exactly.
    close_to(shown(&mut env, "interp(c, flowrate, head, 2000 gpm) ="), quantity(221.0, "ft"));
    column(&mut env, "X", &[0.0, 1.0, 3.0, 4.0, 7.0], "1");
    column(&mut env, "S", &[0.0, 1.0, 9.0, 16.0, 49.0], "1");
    for x in ["0.5", "2", "3.5", "5.5"] {
        let want: f64 = x.parse::<f64>().expect("a number").powi(2);
        let got = shown(&mut env, &format!("interp(pspline(X, S), X, S, {x}) =")).value;
        assert!((got - want).abs() <= 1e-12 * want, "x = {x}: {got} vs {want}");
        let got = shown(&mut env, &format!("interp(cspline(X, S), X, S, {x}) =")).value;
        assert!((got - want).abs() <= 1e-12 * want, "cspline, x = {x}: {got} vs {want}");
    }
}

#[test]
fn cspline_reproduces_a_cubic_and_small_data() {
    let mut env = Env::new();
    // Not-a-knot-like ends: a cubic is reproduced exactly by a spline whose M runs in a straight
    // line across each end's two intervals.
    column(&mut env, "X", &[0.0, 1.0, 2.5, 3.0, 5.0, 6.0], "1");
    let cubic = |x: f64| 1.0 - 2.0 * x + 0.5 * x * x + 0.25 * x * x * x;
    column(&mut env, "C", &[0.0, 1.0, 2.5, 3.0, 5.0, 6.0].map(cubic), "1");
    for x in ["0.5", "1.75", "2.75", "4", "5.5"] {
        let want = cubic(x.parse().expect("a number"));
        let got = shown(&mut env, &format!("interp(cspline(X, C), X, C, {x}) =")).value;
        assert!((got - want).abs() <= 1e-11 * want.abs().max(1.0), "x = {x}: {got} vs {want}");
    }
    // Three points: the one parabola through them. Two points suit only lspline.
    column(&mut env, "X3", &[1.0, 2.0, 4.0], "1");
    column(&mut env, "Y3", &[1.0, 4.0, 16.0], "1");
    for f in ["cspline", "pspline", "lspline"] {
        assert!(run_value(&mut env, &format!("{f}(X3, Y3) =")).is_ok(), "{f}");
    }
    close(shown(&mut env, "interp(cspline(X3, Y3), X3, Y3, 3) ="), 9.0, Dim::ONE);
    column(&mut env, "X2", &[1.0, 2.0], "1");
    column(&mut env, "Y2", &[1.0, 4.0], "1");
    assert!(run_value(&mut env, "lspline(X2, Y2) =").is_ok());
    for f in ["cspline", "pspline"] {
        assert_eq!(syntax_msg(run_value(&mut env, &format!("{f}(X2, Y2) =")).expect_err("too few")), format!("{f} needs at least 3 points in X2"));
    }
    let e = run_value(&mut env, "cspline([1, 3, 2], [1, 2, 3]) =").expect_err("not increasing");
    assert_eq!(syntax_msg(e), "cspline needs vx strictly increasing — element 2 isn't more than element 1");
    let e = run_value(&mut env, "regress([1, 2, 3], [1, 2], 1) =").expect_err("lengths differ");
    assert_eq!(syntax_msg(e), "regress needs vx and vy of the same length (3 and 2)");
}

#[test]
fn regress_matches_mathcad() {
    let mut env = Env::new();
    run(&mut env, "T := [100, 200, 300, 400, 500]").expect("defines T");
    run(&mut env, "k := [0.25, 0.31, 0.40, 0.51, 0.64]").expect("defines k");
    run(&mut env, "c := regress(T, k, 2)").expect("defines c");
    let (shape, items) = grid(&mut env, "c =");
    // k + 4 elements: the kind (4), 3 and k, then a_0 … a_k.
    assert_eq!(shape, (6, 1));
    close_all(&items[..3], &[4.0, 3.0, 2.0], Dim::ONE);
    // ASTM C547 PolyRegress w Header.xmcd, to a relative 1e-9.
    for (at, want) in [(3, 0.20799999999999994), (4, 0.0002942857142857147), (5, 1.1428571428571426E-06)] {
        let got = shown(&mut env, &format!("c[{at}] =")).value;
        assert!((got - want).abs() <= 1e-9 * want, "c[{at}] = {got} vs {want}");
    }
    // A fit evaluates anywhere, beyond the data too.
    let want = 0.208 + 2.942857e-4 * 600.0 + 1.142857e-6 * 600.0 * 600.0;
    let got = shown(&mut env, "interp(c, T, k, 600) =").value;
    assert!((got - want).abs() <= 1e-6 * want, "{got} vs {want}");
}

#[test]
fn regress_scales_and_keeps_units() {
    let mut env = Env::new();
    // x around 2000: a plain normal-equations solve loses its digits here.
    run(&mut env, "Yrs := [2000, 2001, 2002, 2003, 2004, 2005]").expect("defines Yrs");
    let poly = |t: f64| 3.0 + 0.5 * (t - 2002.0) + 0.25 * (t - 2002.0).powi(2);
    column(&mut env, "v", &[2000.0, 2001.0, 2002.0, 2003.0, 2004.0, 2005.0].map(poly), "1");
    run(&mut env, "c := regress(Yrs, v, 2)").expect("defines c");
    for (x, want) in [2000.0_f64, 2002.0, 2004.0, 2010.0].map(|x| (x, poly(x))) {
        let got = shown(&mut env, &format!("interp(c, Yrs, v, {x}) =")).value;
        assert!((got - want).abs() <= 1e-6, "x = {x}: {got} vs {want}");
    }
    // The units of a_j are those of vy/vx^j.
    column(&mut env, "Dd", &[1.0, 2.0, 3.0, 4.0], "in");
    column(&mut env, "Fo", &[10.0, 20.0, 40.0, 70.0], "lbf");
    let (_, items) = grid(&mut env, "regress(Dd, Fo, 2) =");
    let inch = quantity(1.0, "in").dim;
    let lbf = quantity(1.0, "lbf").dim;
    assert_eq!(items.len(), 6);
    assert_eq!(items[3].dim, lbf);
    assert_eq!(items[4].dim, lbf.div(inch).expect("per length"));
    assert_eq!(items[5].dim, lbf.div(inch.powi(2).expect("squared")).expect("per area"));
    // k = 1 is the straight line of `slope` and `intercept`.
    let (_, line) = grid(&mut env, "regress(Dd, Fo, 1) =");
    close_to(Quantity::new(line[3].value, line[3].dim), shown(&mut env, "intercept(Dd, Fo) ="));
    close_to(Quantity::new(line[4].value, line[4].dim), shown(&mut env, "slope(Dd, Fo) ="));
    // At the data's own points a fit through as many points as it has coefficients gives the data.
    close_to(shown(&mut env, "interp(regress(Dd, Fo, 3), Dd, Fo, 3 in) ="), quantity(40.0, "lbf"));
}

#[test]
fn slope_intercept() {
    let mut env = Env::new();
    run(&mut env, "x := [1, 2, 3] in").expect("defines x");
    run(&mut env, "y := [2, 4, 6] lbf").expect("defines y");
    close_to(shown(&mut env, "slope(x, y) ="), quantity(2.0, "lbf").div(quantity(1.0, "in")).expect("a quotient"));
    close_to(shown(&mut env, "intercept(x, y) ="), quantity(0.0, "lbf"));
    // A line that doesn't pass through the origin, and noisy data: least squares.
    run(&mut env, "u := [0, 1, 2, 3]").expect("defines u");
    run(&mut env, "w := [1, 3, 4, 7]").expect("defines w");
    close(shown(&mut env, "slope(u, w) ="), 1.9, Dim::ONE);
    close(shown(&mut env, "intercept(u, w) ="), 0.9, Dim::ONE);
    // No line through x values that are all the same, or through one point.
    assert!(run(&mut env, "slope([2, 2, 2], [1, 2, 3]) =").is_err());
    assert!(run(&mut env, "intercept([2], [1]) =").is_err());
    let e = run(&mut env, "slope([1, 2, 3], [1, 2]) =").expect_err("lengths differ");
    assert_eq!(syntax_msg(e), "slope needs vx and vy of the same length (3 and 2)");
}

#[test]
fn interp_refuses_outside_spline_data() {
    let mut env = pump();
    let spline_error = run(&mut env, "spline(flowrate, head, 4100 gpm) =").expect_err("outside");
    assert!(syntax_msg(spline_error.clone()).contains("is outside flowrate"), "{spline_error:?}");
    for f in ["cspline", "lspline", "pspline"] {
        let e = run(&mut env, &format!("interp({f}(flowrate, head), flowrate, head, 4100 gpm) =")).expect_err("outside");
        assert_eq!(e, spline_error, "{f}");
        let e = run(&mut env, &format!("interp({f}(flowrate, head), flowrate, head, -1 gpm) =")).expect_err("outside");
        let msg = syntax_msg(e);
        assert!(msg.starts_with("-3.85 in³/sec is outside flowrate"), "{f}: {msg}");
    }
    // The ends themselves are in the data.
    close_to(shown(&mut env, "H(4000 gpm) ="), quantity(157.0, "ft"));
}

#[test]
fn interp_rejects_foreign_vectors() {
    let mut env = Env::new();
    for src in ["xs := [1, 2, 3, 4]", "ys := [2, 3, 5, 9]", "xs2 := [1, 2, 3, 4, 5]", "ys2 := [2, 3, 5, 9, 1]"] {
        run(&mut env, src).expect("defines");
    }
    let phrase = "cspline, lspline, pspline or regress";
    let attempts = [
        // Not from any of them.
        "interp([1, 2, 3], xs, ys, 2) =",
        "interp([1, 2, 3, 4, 5, 6, 7], xs, ys, 2) =",
        "interp(5, xs, ys, 2) =",
        // From a spline over data of another length.
        "interp(cspline(xs, ys), xs2, ys2, 2) =",
        "interp(lspline(xs2, ys2), xs, ys, 2) =",
        // A regress vector whose length isn't its degree + 4, or a header that isn't ours.
        "interp([4, 3, 2, 1, 1], xs, ys, 2) =",
        "interp([9, 3, 0, 1, 1, 1, 1], xs, ys, 2) =",
        "interp([3, 4, 0, 1, 1, 1, 1], xs, ys, 2) =",
        "interp([3, 3, 1, 1, 1, 1, 1], xs, ys, 2) =",
    ];
    for src in attempts {
        let e = run(&mut env, src).expect_err(src);
        assert!(syntax_msg(e).contains(phrase), "{src}");
    }
    // A vector from the data's other units doesn't fit either.
    column(&mut env, "Dd", &[1.0, 2.0, 3.0, 4.0], "in");
    let e = run(&mut env, "interp(cspline(xs, ys), Dd, ys, 2 in) =").expect_err("units differ");
    assert!(syntax_msg(e).contains(phrase));
    // Too short for the header, counted in the right number.
    let e = syntax_msg(run(&mut env, "interp(5, xs, ys, 2) =").expect_err("one element"));
    assert!(e.ends_with("— it has 1 element, not even a header"), "{e}");
    let e = syntax_msg(run(&mut env, "interp([5, 3], xs, ys, 2) =").expect_err("two elements"));
    assert!(e.ends_with("— it has 2 elements, not even a header"), "{e}");
    // A matrix isn't a vector.
    assert!(run(&mut env, "interp([1, 2; 3, 4], xs, ys, 2) =").is_err());
    // Four arguments.
    assert_eq!(run(&mut env, "interp(cspline(xs, ys), xs, ys) =").expect_err("arity"), CalcError::Arity("interp".into(), 4));
}

#[test]
fn interp_per_element_in_cases() {
    let mut env = Env::new();
    for src in ["xs := [0, 10, 20, 30]", "ys := [0, 100, 400, 900]", "x := [5, 15, 40]"] {
        run(&mut env, src).unwrap();
    }
    run(&mut env, "c := cspline(xs, ys)").unwrap();
    // Only the elements that pick the branch can fail in it.
    let v = vector_in(&mut env, "y := interp(c, xs, ys, x) if x ≤ 30, 0 otherwise =");
    assert_eq!(v.len(), 3);
    assert_eq!(v[2], 0.0);
    assert!(v[0] > 0.0 && v[1] > 100.0, "{v:?}");
    assert_eq!(vector_in(&mut env, "y_2 := if(x ≤ 30, interp(c, xs, ys, x), 0) =")[2], 0.0);
    assert_eq!(err_in(&mut env, "b := interp(c, xs, ys, x) if x ≥ 0, 0 otherwise ="), "element 2: 40 is outside xs (0 to 30)");
    // Strict callers keep their errors.
    assert_eq!(err_in(&mut env, "interp(c, xs, ys, x) ="), "element 2: 40 is outside xs (0 to 30)");
    // The coefficient vector is taken whole: a failed element of it fails every element.
    run(&mut env, "z := [4, 0, 9, 1]").unwrap();
    assert_eq!(err_in(&mut env, "b := interp(1/z, xs, ys, x) if x ≤ 30, 0 otherwise ="), "element 1: result isn't a finite number");
    // A regress vector evaluates anywhere, per element too.
    run(&mut env, "r := regress(xs, ys, 2)").unwrap();
    let v = vector_in(&mut env, "q := interp(r, xs, ys, x) if x ≤ 100, 0 otherwise =");
    assert!((v[2] - 1600.0).abs() <= 1e-9, "{v:?}");
}

#[test]
fn regress_degree_checks() {
    let mut env = Env::new();
    run(&mut env, "T := [100, 200, 300, 400, 500]").unwrap();
    run(&mut env, "k := [0.25, 0.31, 0.40, 0.51, 0.64]").unwrap();
    for bad in ["0", "5", "6", "1.5", "-1", "2.000001", "[1, 2]"] {
        let e = run_value(&mut env, &format!("regress(T, k, {bad}) =")).expect_err(bad);
        assert!(syntax_msg(e).contains("whole number from 1 to 4"), "{bad}");
    }
    let e = run_value(&mut env, "regress(T, k, 2 in) =").expect_err("not a plain number");
    assert!(syntax_msg(e).contains("whole number from 1 to 4"));
    for ok in ["1", "2", "4"] {
        assert!(run_value(&mut env, &format!("regress(T, k, {ok}) =")).is_ok(), "{ok}");
    }
    // All x the same: no curve to fit.
    assert!(run_value(&mut env, "regress([3, 3, 3], [1, 2, 4], 1) =").is_err());
    // Two different values can't fix a parabola.
    let e = run_value(&mut env, "regress([1, 2, 2, 2], [1, 2, 3, 4], 2) =").expect_err("too few different values");
    assert_eq!(syntax_msg(e), "regress can't fit degree 2 to vx: it has too few different values");
}

#[test]
fn curve_fitting_charges_the_pass() {
    let mut env = Env::new();
    column(&mut env, "X", &(0..50).map(f64::from).collect::<Vec<_>>(), "1");
    column(&mut env, "Y", &(0..50).map(|k| f64::from(k * k)).collect::<Vec<_>>(), "1");
    // A spline costs its points (and its vector); a fit costs its points times its degree squared.
    fn spent(env: &mut Env, src: &str, limit: u64) -> (Result<Option<Data>, CalcError>, u64) {
        let pass = Pass::new(limit);
        let r = run_stmt_in(env, &parse(src).expect("parses"), &pass);
        (r, pass.spent.get())
    }
    let (r, small) = spent(&mut env, "c := cspline(X, Y)", u64::MAX);
    assert!(r.is_ok());
    assert!(small >= 50, "{small}");
    let (r, fit) = spent(&mut env, "r := regress(X, Y, 10)", u64::MAX);
    assert!(r.is_ok());
    assert!(fit >= 50 * 11 * 11, "{fit}");
    assert_eq!(spent(&mut env, "r := regress(X, Y, 10)", 50 * 11 * 11 - 1).0, Err(CalcError::TooMuchWork));
    assert_eq!(spent(&mut env, "c := cspline(X, Y)", 10).0, Err(CalcError::TooMuchWork));
    // A fit of every point of a long table is refused before a matrix that size is built.
    column(&mut env, "BigX", &(0..2000).map(f64::from).collect::<Vec<_>>(), "1");
    assert_eq!(spent(&mut env, "r := regress(BigX, BigX, 1999)", u64::MAX).0, Err(CalcError::TooLarge));
}

#[test]
fn new_function_names_are_reserved() {
    for name in [
        "sum",
        "length",
        "rows",
        "cols",
        "mean",
        "dot",
        "mmul",
        "transpose",
        "lookup",
        "linterp",
        "spline",
        "cspline",
        "lspline",
        "pspline",
        "regress",
        "interp",
        "slope",
        "intercept",
        "submatrix",
        "augment",
        "stack",
    ] {
        assert_eq!(check_name(name), Err(CalcError::ReservedName(name.into())), "{name}");
    }
}

#[test]
fn products_are_bounded() {
    let stmt = |src: &str| parse(src).expect("parses");
    let mut env = Env::new();
    env.define_array("C", 10_000, 1, vec![Quantity::dimensionless(1.0); 10_000]).expect("defines C");
    env.define_array("R", 1, 10_000, vec![Quantity::dimensionless(1.0); 10_000]).expect("defines R");
    // 10,000 × 10,000 elements: too large, before any of the product is worked out. The pass has
    // paid only for the three nodes and for reading C and R.
    let pass = Pass::new(u64::MAX);
    assert_eq!(run_stmt_in(&mut env, &stmt("p := mmul(C, R)"), &pass), Err(CalcError::TooLarge));
    assert_eq!(pass.spent.get(), 3 + 20_000);
    // A product charges rows · cols · inner.
    env.define_array("A", 2, 3, vec![Quantity::dimensionless(1.0); 6]).expect("defines A");
    env.define_array("B", 3, 2, vec![Quantity::dimensionless(1.0); 6]).expect("defines B");
    let pass = Pass::new(u64::MAX);
    run_stmt_in(&mut env, &stmt("q := mmul(A, B)"), &pass).expect("runs");
    assert_eq!(pass.spent.get(), 3 + 12 + 2 * 2 * 3);
    // One element, but 10,000 products to sum.
    let pass = Pass::new(u64::MAX);
    run_stmt_in(&mut env, &stmt("r := mmul(R, C)"), &pass).expect("runs");
    assert_eq!(pass.spent.get(), 3 + 20_000 + 10_000);
    assert_eq!(run_stmt_in(&mut env, &stmt("r := mmul(R, C)"), &Pass::new(25_000)), Err(CalcError::TooMuchWork));
}

#[test]
fn array_messages_use_the_display_format() {
    let mut env = flanges();
    env.set_display(UnitSystem::Si, Precision::new(4));
    let e = run(&mut env, "linterp(NPS, T, 1.6 in) =").expect_err("past the end");
    assert_eq!(syntax_msg(e), "0.0406 m is outside NPS (0.0127 m to 0.0381 m)");
}

#[test]
fn reductions_of_temperatures_keep_what_they_stand_for() {
    let mut env = Env::new();
    // The smallest, largest or mean of temperatures is one; a sum isn't.
    for src in ["min([300 K, 310 K]) = ? °C", "max([300 K, 310 K], 305 K) = ? °C", "mean([300 K, 310 K]) = ? °C"] {
        assert!(run(&mut env, src).is_ok(), "{src}");
    }
    assert_eq!(run(&mut env, "sum([300 K, 310 K]) = ? °C"), Err(CalcError::NotAbsolute("°C".into())));
    assert_eq!(run(&mut env, "max([300 K, 310 K], 5 Δ°C) = ? °C"), Err(CalcError::NotAbsolute("°C".into())));
}

#[test]
fn array_messages_always_show_a_unit() {
    let mut env = flanges();
    // 10^307 m is too many inches to show: it shows in metres rather than as a bare number.
    let e = run(&mut env, "linterp(NPS, T, 10^307 m) =").expect_err("past the end");
    assert_eq!(syntax_msg(e), "1×10³⁰⁷ m is outside NPS (0.5 in to 1.5 in)");
}

#[test]
fn text_values() {
    let mut env = Env::new();
    assert_eq!(run_value(&mut env, "c := \"OK\" ="), Ok(Some(Value::Text("OK".into()))));
    assert_eq!(
        run_value(&mut env, "v := [\"A\", \"B\"; \"C\", \"D\"] ="),
        Ok(Some(Value::TextGrid { rows: 2, cols: 2, items: vec!["A".into(), "B".into(), "C".into(), "D".into()] }))
    );
    assert_eq!(run_value(&mut env, "v[1, 0] ="), Ok(Some(Value::Text("C".into()))));
    run_value(&mut env, "w := transpose(v)").unwrap();
    assert_eq!(run_value(&mut env, "w[0, 1] ="), Ok(Some(Value::Text("C".into()))));
    assert_eq!(val("length([\"A\", \"B\", \"C\"]) ="), Quantity::dimensionless(3.0));
    assert_eq!(err("\"OK\" + 1 =").to_string(), "\"OK\" is text — it can't be used in arithmetic");
    assert_eq!(err("-\"OK\" =").to_string(), "\"OK\" is text — it can't be used in arithmetic");
    assert_eq!(err("sqrt(\"OK\") =").to_string(), "\"OK\" is text — it can't be used in sqrt");
    assert_eq!(err("sum([\"A\", \"B\"]) =").to_string(), "\"A\" is text — it can't be used in sum");
    assert_eq!(err("\"OK\" = ? in").to_string(), "\"OK\" is text — it can't be used with a unit");
    assert_eq!(err("[\"A\", 2] =").to_string(), "element 1 is a number but element 0 is text");
    let long = "y".repeat(60);
    assert_eq!(err(&format!("\"{long}\" + 1 =")).to_string(), format!("\"{}…\" is text — it can't be used in arithmetic", "y".repeat(40)));
    // Functions pass text through; indexed definitions build text vectors.
    run_value(&mut env, "label(x) := \"size\"").unwrap();
    assert_eq!(run_value(&mut env, "label(2) ="), Ok(Some(Value::Text("size".into()))));
}

#[test]
fn text_in_vectors_and_other_places() {
    let mut env = Env::new();
    run_value(&mut env, "i := 0..2").unwrap();
    run_value(&mut env, "t[i] := \"n\"").unwrap();
    assert_eq!(run_value(&mut env, "t ="), Ok(Some(Value::TextGrid { rows: 3, cols: 1, items: vec!["n".into(), "n".into(), "n".into()] })));
    // A user function maps text through its parameter, and an element can be read back.
    run_value(&mut env, "id(x) := x").unwrap();
    assert_eq!(run_value(&mut env, "id(\"A\") ="), Ok(Some(Value::Text("A".into()))));
    // Numbers and text never share a vector.
    let mixed = |env: &mut Env, src: &str| run_value(env, src).unwrap_err().to_string();
    assert_eq!(mixed(&mut env, "[1, \"A\"] ="), "element 1 is text but element 0 is a number");
    assert_eq!(mixed(&mut env, "[1, 2; 3, \"A\"] ="), "element [1, 1] is text but element [0, 0] is a number");
    // Other places that need a number.
    assert_eq!(mixed(&mut env, "r := \"A\"..3"), "\"A\" is text — it can't be used in a range");
    assert_eq!(mixed(&mut env, "max([\"B\", \"A\"]) ="), "\"B\" is text — it can't be used in max");
    assert_eq!(mixed(&mut env, "mmul([\"B\"], [1]) ="), "\"B\" is text — it can't be used in mmul");
    assert_eq!(mixed(&mut env, "|\"A\"| ="), "\"A\" is text — it can't be used in arithmetic");
    assert_eq!(mixed(&mut env, "[\"A\", \"B\"] + 1 ="), "\"A\" is text — it can't be used in arithmetic");
    assert_eq!(mixed(&mut env, "[t, t] ="), "a matrix element can't be a vector");
    assert_eq!(val("rows([\"A\", \"B\"; \"C\", \"D\"]) ="), Quantity::dimensionless(2.0));
    assert_eq!(val("cols([\"A\", \"B\"; \"C\", \"D\"]) ="), Quantity::dimensionless(2.0));
    // Indexing text that isn't a vector, and past the end, are the usual index errors.
    run_value(&mut env, "c := \"OK\"").unwrap();
    assert_eq!(mixed(&mut env, "c[0] ="), "c isn't a vector");
    assert_eq!(mixed(&mut env, "t[3] ="), "index 3 is outside t (0 to 2)");
    // The error for a mix names the later element first, counted from the origin.
    assert_eq!(crate::array::mixed(2, 1, 1, 0, 1).to_string(), "element 2 is text but element 1 is a number");
    assert_eq!(crate::array::mixed(2, 1, 0, 1, 0).to_string(), "element 1 is a number but element 0 is text");
}

#[test]
fn text_is_charged_by_size() {
    let long = "é".repeat(1000); // 2000 bytes: 62 steps each at one per 32 bytes
    let stmt = |src: &str| parse(src).unwrap_or_else(|e| panic!("{src:?} did not parse: {e}"));
    let spent = |env: &mut Env, src: &str| {
        let pass = Pass::new(u64::MAX);
        run_stmt_in(env, &stmt(src), &pass).unwrap();
        pass.spent.get()
    };
    let mut env = Env::new();
    spent(&mut env, "i := 0..99");
    spent(&mut env, &format!("t[i] := \"{long}\""));
    spent(&mut env, "v[i] := 1");
    // Copying a name: its elements, plus the text's bytes; the name itself is one step.
    assert_eq!(spent(&mut env, "a := v"), 100 + 1);
    assert_eq!(spent(&mut env, "b := t"), 100 + 100 * 62 + 1);
    assert_eq!(spent(&mut env, "c := t[3]"), 62 + 1 + 1);
    assert_eq!(spent(&mut env, "d := transpose(t)"), 2 * (100 + 100 * 62) + 2); // the copy of `t`, and the transposed copy
    // So repeated copies of the text exhaust a budget that the same number of number copies doesn't.
    let (numbers, texts) = (Pass::new(10_000), Pass::new(10_000));
    let mut copies = |name: &str, pass: &Pass| (0..60).try_for_each(|k| run_stmt_in(&mut env, &stmt(&format!("x_{k} := {name}")), pass).map(|_| ()));
    assert_eq!(copies("v", &numbers), Ok(()));
    assert_eq!(copies("t", &texts), Err(CalcError::TooMuchWork));
}

/// Run `src` in a fresh env and return the plain values of the grid it shows.
fn vector(src: &str) -> Vec<f64> {
    let mut env = Env::new();
    let stmt = parse(&format!("{src} =")).unwrap_or_else(|e| panic!("{src:?} did not parse: {e}"));
    match run_stmt(&mut env, &stmt) {
        Ok(Some(Value::Grid { items, .. })) => items.iter().map(|q| q.value).collect(),
        other => panic!("{src:?} did not show a grid: {other:?}"),
    }
}

#[test]
fn comparisons() {
    let t = |src: &str| val(&format!("{src} =")).value;
    assert_eq!(t("38.1 mm == 1.5 in"), 1.0);
    assert_eq!(t("38.1 mm > 1.5 in"), 0.0);
    assert_eq!(t("212 °F ≤ 100 °C"), 1.0);
    assert_eq!(t("100 °C == 212 °F"), 1.0);
    assert_eq!(t("0.1 + 0.2 == 0.3"), 1.0);
    assert_eq!(t("1e-20 == 0"), 0.0);
    assert_eq!(t("0 ≤ 0.5 ≤ 1"), 1.0);
    assert_eq!(t("0 ≤ 1.5 ≤ 1"), 0.0);
    assert_eq!(t("\"A106 B\" == \" A106\u{a0} B \""), 1.0);
    assert_eq!(t("\"a\" ≠ \"A\""), 1.0);
    assert!(matches!(run(&mut Env::new(), "1 in < 1 lbf ="), Err(CalcError::Unit(_))));
    assert_eq!(err("\"a\" < \"b\" =").to_string(), "\"a\" is text — it can't be used with <");
    assert_eq!(err("\"OK\" == 1 =").to_string(), "can't compare \"OK\" with a number");
    let v = vector("[1, 5, 9] > 4");
    assert_eq!(v, vec![0.0, 1.0, 1.0]);
}

#[test]
fn comparison_results_are_plain_one_and_zero() {
    let q = val("3 in < 4 in =");
    assert_eq!(q.dim, Dim::ONE);
    assert_eq!(q.value, 1.0);
    // Text grids compare element by element, and a text broadcasts.
    assert_eq!(vector("[\"OK\", \"NG\", \" OK \"] == \"OK\""), vec![1.0, 0.0, 1.0]);
    assert_eq!(vector("[\"a\", \"b\"] ≠ [\"a\", \"c\"]"), vec![0.0, 1.0]);
    assert_eq!(err("[1, 2] < [1, 2, 3] =").to_string(), "shapes don't match: 2 rows × 1 vs 3 rows × 1");
    assert_eq!(err("[1, 2] < \"a\" =").to_string(), "can't compare \"a\" with a number");
    // A chain over a vector compares every link element by element.
    assert_eq!(vector("0 ≤ [0.5, 2, 1] ≤ 1"), vec![1.0, 0.0, 1.0]);
    assert_eq!(err("1 in == 2 =").to_string(), CalcError::Unit(UnitError::Mismatch { left: Dim::LENGTH, right: Dim::ONE }).to_string());
}

#[test]
fn temperature_comparisons() {
    let mut env = Env::new();
    for src in ["T := 950 °F", "T_K := 700 K", "ΔT := 20 Δ°F", "T_1 := 100 °F", "T_2 := 200 °F"] {
        run(&mut env, src).unwrap();
    }
    assert_eq!(run(&mut env, "T ≤ 900 °F =").unwrap().unwrap().value, 0.0);
    assert_eq!(run(&mut env, "T ≤ 800 K =").unwrap().unwrap().value, 1.0); // 950 °F is 783.15 K
    assert_eq!(run(&mut env, "ΔT ≤ 10 Δ°F =").unwrap().unwrap().value, 0.0);
    assert_eq!(run(&mut env, "T_K > ΔT =").unwrap().unwrap().value, 1.0);
    let msg = "can't compare an absolute temperature with a temperature difference — write an average as T_1 + (T_2 − T_1)/2";
    assert_eq!(run(&mut env, "ΔT ≤ 10 °F =").unwrap_err().to_string(), msg);
    assert_eq!(run(&mut env, "(T_1 + T_2)/2 ≤ 900 °F =").unwrap_err().to_string(), msg);
    assert_eq!(run(&mut env, "T_1 + (T_2 - T_1)/2 ≤ 900 °F =").unwrap().unwrap().value, 1.0);
}

#[test]
fn logic() {
    let t = |src: &str| val(&format!("{src} =")).value;
    assert_eq!(t("1 < 2 and 3 < 4"), 1.0);
    assert_eq!(t("1 > 2 or not 3 > 4"), 1.0);
    assert_eq!(t("0.5 and 1"), 1.0); // nonzero is true
    assert_eq!(err("1 in and 1 =").to_string(), "a condition must be a comparison or a plain number");
    assert_eq!(err("\"OK\" or 1 =").to_string(), "\"OK\" is text — it can't be used as a condition");
    // A single false left side decides `and` without evaluating the right.
    assert_eq!(t("1 > 2 and undefined_name > 0"), 0.0);
    assert_eq!(t("1 < 2 or undefined_name > 0"), 1.0);
    assert_eq!(vector("[1, 0, 2] > 0 and [1, 1, 0] > 0"), vec![1.0, 0.0, 0.0]);
}

#[test]
fn logic_on_vectors_and_not() {
    assert_eq!(vector("not [1, 0, 2]"), vec![0.0, 1.0, 0.0]);
    assert_eq!(vector("[1, 0] > 0 or [0, 0] > 0"), vec![1.0, 0.0]);
    assert_eq!(err("not 1 in =").to_string(), "a condition must be a comparison or a plain number");
    assert_eq!(val("not not 1 =").value, 1.0);
}

#[test]
fn keywords_are_not_names() {
    for word in ["and", "or", "not", "if", "otherwise"] {
        assert_eq!(check_name(word), Err(CalcError::Keyword(word.into())), "{word}");
    }
    assert_eq!(CalcError::Keyword("or".into()).to_string(), "or is a reserved word — choose another name");
    assert!(check_name("order").is_ok());
}

/// The expression of `src` (written without its `=`), parsed.
fn expr_of(src: &str) -> Expr {
    match parse(&format!("{src} =")) {
        Ok(crate::Stmt::Eval { expr, .. }) => expr,
        other => panic!("{src:?} is not an expression: {other:?}"),
    }
}

/// `src` evaluated in `env` with element failures kept per element.
fn lenient_result(env: &Env, src: &str) -> Result<Data, CalcError> {
    let pass = Pass::new(u64::MAX);
    let cx = Cx::new(env, &pass);
    let result = eval_lenient(&cx, &Frame::GLOBAL, &expr_of(src), 0);
    assert_eq!(cx.lenient.get(), 0, "eval_lenient restores the flag");
    result
}

fn lenient(env: &Env, src: &str) -> Data {
    lenient_result(env, src).unwrap_or_else(|e| panic!("{src:?} failed: {e}"))
}

fn lenient_err(env: &Env, src: &str) -> String {
    match lenient_result(env, src) {
        Err(e) => e.to_string(),
        Ok(d) => panic!("{src:?} should fail: {d:?}"),
    }
}

/// The error of running `src` in `env`, as shown.
fn err_in(env: &mut Env, src: &str) -> String {
    match run(env, src) {
        Err(e) => e.to_string(),
        other => panic!("{src:?} should fail: {other:?}"),
    }
}

/// Run `src` in `env` and return the plain values of the grid it shows.
fn vector_in(env: &mut Env, src: &str) -> Vec<f64> {
    match run_value(env, src) {
        Ok(Some(Value::Grid { items, .. })) => items.iter().map(|q| q.value).collect(),
        other => panic!("{src:?} did not show a grid: {other:?}"),
    }
}

fn failed_at<T>(g: &Grid<T>) -> Vec<usize> {
    g.failed.iter().map(|(k, _)| *k).collect()
}

#[test]
fn lenient_grids_keep_element_failures() {
    let mut env = Env::new();
    run(&mut env, "x := [1, 0, 2]").unwrap();
    let d = lenient(&env, "1/x");
    let Data::Grid(g) = &d else { panic!("{d:?}") };
    assert_eq!(g.failed.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [1]);
    // Element-wise work carries the failure along; whole-array use refuses it.
    let Data::Grid(g) = lenient(&env, "2·(1/x) + 1") else { panic!() };
    assert_eq!(g.failed.len(), 1);
    assert_eq!(g.items[2].q.value, 2.0);
    assert!(lenient_err(&env, "sum(1/x)").starts_with("element 1:"));
    // Strict evaluation is unchanged.
    assert!(err_in(&mut env, "1/x =").starts_with("element 1:"));
}

#[test]
fn lenient_failures_are_named_once() {
    let mut env = Env::new();
    run(&mut env, "x := [1, 0, 2]").unwrap();
    let strict = err_in(&mut env, "1/x =");
    assert_eq!(lenient_err(&env, "sum(1/x)"), strict);
    assert_eq!(lenient_err(&env, "sum(2·(1/x) + 1)"), strict);
    // The raw error is kept, not one already naming the element.
    let Data::Grid(g) = lenient(&env, "1/x") else { panic!() };
    assert!(!matches!(g.failed[0].1, CalcError::Element { .. }), "{:?}", g.failed[0].1);
}

#[test]
fn lenient_failures_pass_through_element_wise_operations() {
    let mut env = Env::new();
    run(&mut env, "x := [1, 0, 4]").unwrap();
    run(&mut env, "y := [1, 0, -4]").unwrap();
    run(&mut env, "f(v) := v·2").unwrap();
    // Built-ins applied element by element, comparisons, logic, `not`, user functions.
    for src in
        ["sqrt(1/x)", "-(1/x)", "|1/x|", "1/x > 0", "not (1/x > 0)", "1/x > 0 and x > 0", "x > 0 or 1/x > 0", "f(1/x)", "1/x + 1/x", "1/x < 2 < 3"]
    {
        let Data::Grid(g) = lenient(&env, src) else { panic!("{src}") };
        assert_eq!(failed_at(&g), [1], "{src}");
    }
    // Each element keeps its own failure, in element order.
    let Data::Grid(g) = lenient(&env, "sqrt(y)/y") else { panic!() };
    assert_eq!(failed_at(&g), [1, 2]);
    // The left side's failure wins where both fail.
    let Data::Grid(g) = lenient(&env, "1/y + sqrt(y - 1)") else { panic!() };
    assert_eq!(failed_at(&g), [1, 2]);
    assert_eq!(g.failed[0].1, CalcError::NonFinite);
}

#[test]
fn whole_array_uses_refuse_lenient_failures() {
    let mut env = Env::new();
    run(&mut env, "x := [1, 0, 2]").unwrap();
    run(&mut env, "M := [1, 0; 2, 3]").unwrap();
    run(&mut env, "first(v) := v[0]").unwrap();
    for src in ["sum(1/x)", "length(1/x)", "transpose(1/x)", "max(1/x)", "lookup(x, 1/x, 2)", "first(1/x)", "augment(1/x, x)", "stack(x, 1/x)"] {
        assert!(lenient_err(&env, src).starts_with("element 1:"), "{src}: {}", lenient_err(&env, src));
    }
    assert!(lenient_err(&env, "submatrix(1/x, 0, 1, 0, 0)").starts_with("element 1:"), "{}", lenient_err(&env, "submatrix(1/x, 0, 1, 0, 0)"));
    assert!(lenient_err(&env, "sum(1/M)").starts_with("element [0, 1]:"), "{}", lenient_err(&env, "sum(1/M)"));
    // Work that runs out is never kept per element.
    let big = Env::new();
    let pass = Pass::new(10);
    let cx = Cx::new(&big, &pass);
    assert_eq!(eval_lenient(&cx, &Frame::GLOBAL, &expr_of("1/[1, 0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]"), 0), Err(CalcError::TooMuchWork));
    assert_eq!(cx.lenient.get(), 0);
}

#[test]
fn strict_boundaries_refuse_failed_grids() {
    let env = Env::new();
    let failed = Grid { rows: 2, cols: 1, items: vec![Val::plain(Quantity::dimensionless(1.0)); 2], failed: vec![(1, CalcError::NonFinite)] };
    let d = Data::Grid(failed);
    let want = "element 1: result isn't a finite number";
    assert_eq!(d.clone().strict(0).map_err(|e| e.to_string()), Err(want.to_string()));
    assert_eq!(check_display(&env, "m", &d).map_err(|e| e.to_string()).err().as_deref(), Some(want));
    // Text grids too: a comparison keeps or refuses their failures like numbers.
    let texts =
        Data::TextGrid(Grid { rows: 3, cols: 1, items: vec!["a".into(), String::new(), "b".into()], failed: vec![(1, CalcError::NonFinite)] });
    assert!(texts.clone().strict(0).is_err());
    let pass = Pass::new(u64::MAX);
    let cx = Cx::new(&env, &pass);
    let b = Data::Text("b".into());
    assert_eq!(compare_texts(&cx, &texts, crate::Cmp::Eq, &b).map_err(|e| e.to_string()), Err(want.to_string()));
    cx.lenient.set(1);
    let Ok(Data::Grid(g)) = compare_texts(&cx, &b, crate::Cmp::Eq, &texts) else { panic!() };
    assert_eq!(failed_at(&g), [1]);
    assert_eq!(g.items[2].q.value, 1.0);
}

#[test]
fn piecewise() {
    let mut env = Env::new();
    let y = |env: &mut Env, t: &str| {
        run(env, &format!("T_{} := {t}", t.len())).unwrap();
        run(env, &format!("Y_{} := 0.4 if T_{} ≤ 900 °F, 0.5 if T_{} ≤ 950 °F, 0.7 otherwise =", t.len(), t.len(), t.len())).unwrap().unwrap().value
    };
    assert_eq!(y(&mut env, "900 °F"), 0.4);
    assert_eq!(y(&mut env, "900.1 °F"), 0.5);
    assert_eq!(y(&mut env, "482.2222222222222 °C"), 0.4); // 900 °F
    assert_eq!(y(&mut env, "1000 °F"), 0.7);
    run(&mut env, "T := 1000 °F").unwrap();
    let e = run(&mut env, "Z := 0.4 if T ≤ 900 °F, 0.5 if T ≤ 950 °F").unwrap_err().to_string();
    assert!(e.starts_with("no case applies (T = 1"), "{e}");
}

#[test]
fn per_element_branches() {
    let mut env = Env::new();
    run(&mut env, "x := [4, -1, 9, 0]").unwrap();
    assert_eq!(vector_in(&mut env, "r := sqrt(x) if x ≥ 0, 0 otherwise ="), vec![2.0, 0.0, 3.0, 0.0]);
    assert_eq!(vector_in(&mut env, "q := if(x ≠ 0, 1/x, 0) ="), vec![0.25, -1.0, 1.0 / 9.0, 0.0]);
    assert_eq!(vector_in(&mut env, "a := x ≠ 0 and 1/x > 0.2 ="), vec![1.0, 0.0, 0.0, 0.0]);
    // Whole arrays stay whole inside a branch.
    // (`s` is the second, so the sum is `s_x`.)
    assert_eq!(vector_in(&mut env, "s_x := sum(x) if x > 0, 0 otherwise ="), vec![12.0, 0.0, 12.0, 0.0]);
    // A picked element that fails fails; an unpicked one doesn't.
    assert!(err_in(&mut env, "b := 1/x if x ≥ 0, 0 otherwise =").starts_with("element 3:"));
    // Text verdicts per element, and counting them. (`m` is the metre, so the margins are `margin`.)
    run(&mut env, "margin := [0.1, -0.2, 0.3] in").unwrap();
    run(&mut env, "check := \"OK\" if margin ≥ 0 in, \"NG\" otherwise").unwrap();
    assert_eq!(run_value(&mut env, "check ="), Ok(Some(Value::TextGrid { rows: 3, cols: 1, items: vec!["OK".into(), "NG".into(), "OK".into()] })));
    assert_eq!(run(&mut env, "sum(check == \"NG\") =").unwrap().unwrap().value, 1.0);
    assert_eq!(err_in(&mut env, "mix := \"OK\" if margin ≥ 0 in, 0 otherwise ="), "element 1 is a number but element 0 is text");
    // No otherwise: the unmatched element names itself.
    assert!(err_in(&mut env, "z := 1 if margin > 0 in =").starts_with("element 1: no case applies (margin = "));
    // A single value may be text in one case and a number in another.
    assert_eq!(run_value(&mut env, "w := \"none\" if 1 > 2, 5 otherwise ="), Ok(Some(Value::One(Quantity::dimensionless(5.0)))));
    // Review Focus 4: inside a user function called with a vector.
    run(&mut env, "f(v) := sqrt(v) if v ≥ 0, 0 otherwise").unwrap();
    assert_eq!(vector_in(&mut env, "f([4, -1, 9]) ="), vec![2.0, 0.0, 3.0]);
    // A nested piecewise inside a lenient branch keeps its element failures to itself: g_2 has no
    // case for x ≤ 0, but those elements never pick the branch that calls it. (`g` is the gram.)
    run(&mut env, "g_2(v) := 1 if v > 5, 2 if v > 0").unwrap();
    assert_eq!(vector_in(&mut env, "n := g_2(x) if x > 0, 0 otherwise ="), vec![2.0, 0.0, 1.0, 0.0]);
}

/// The `x` of `lookup`, `linterp` and `spline` is per element in a branch: an element that picks
/// another branch can't fail there, and `x`'s own failed elements fail only their own results.
/// The tables stay whole, and strict callers keep their errors.
#[test]
fn lookups_and_interpolations_fail_only_the_elements_that_pick_them() {
    let mut env = Env::new();
    for src in ["xs := [0, 10, 20]", "ys := [0, 100, 400]", "x := [5, 15, 30]", "k := [1, 2, 9]", "xs_2 := [1, 2, 3]", "ys_2 := [10, 20, 30]"] {
        run(&mut env, src).unwrap();
    }
    // The reproductions: per-element results.
    assert_eq!(vector_in(&mut env, "y := linterp(xs, ys, x) if x ≤ 20, 0 otherwise ="), vec![50.0, 250.0, 0.0]);
    assert_eq!(vector_in(&mut env, "y_2 := if(x ≤ 20, linterp(xs, ys, x), 0) ="), vec![50.0, 250.0, 0.0]);
    assert_eq!(vector_in(&mut env, "y_3 := lookup(xs_2, ys_2, k) if k ≤ 3, 0 otherwise ="), vec![10.0, 20.0, 0.0]);
    let s = vector_in(&mut env, "y_4 := spline(xs, ys, x) if x ≤ 20, 0 otherwise =");
    assert_eq!((s.len(), s[2]), (3, 0.0));
    // `x`'s own failed element (`1/z` at 0) passes through, as in `2·(1/z)`.
    run(&mut env, "z := [4, 0, 9]").unwrap();
    assert_eq!(vector_in(&mut env, "y_5 := 2·(1/z) if z ≠ 0, 0 otherwise ="), vec![0.5, 0.0, 2.0 / 9.0]);
    let v = vector_in(&mut env, "y_6 := linterp([0, 1], [0, 10], 1/z) if z ≠ 0, 0 otherwise =");
    assert_eq!(v.len(), 3);
    assert!((v[0] - 2.5).abs() < 1e-12 && v[1] == 0.0 && (v[2] - 10.0 / 9.0).abs() < 1e-12, "{v:?}");
    assert_eq!(vector_in(&mut env, "y_7 := lookup([0.25, 1/9], [1, 2], 1/z) if z ≠ 0, 0 otherwise ="), vec![1.0, 0.0, 2.0]);
    // A picked element that fails still fails, named once.
    assert_eq!(err_in(&mut env, "b := linterp(xs, ys, x) if x ≥ 0, 0 otherwise ="), "element 2: 30 is outside xs (0 to 20)");
    assert_eq!(err_in(&mut env, "b := linterp([0, 1], [0, 10], 1/z) if z ≥ 0, 0 otherwise ="), "element 1: result isn't a finite number");
    // Strict callers: the errors as before.
    assert_eq!(err_in(&mut env, "linterp(xs, ys, x) ="), "element 2: 30 is outside xs (0 to 20)");
    assert_eq!(err_in(&mut env, "spline(xs, ys, x) ="), "element 2: 30 is outside xs (0 to 20)");
    assert_eq!(err_in(&mut env, "lookup(xs_2, ys_2, k) ="), "element 2: 9 isn't in xs_2");
    // The tables are taken whole, even in a branch: a failed element of `xs` fails every element.
    assert_eq!(err_in(&mut env, "b := lookup(1/z, ys, 1) if z ≠ 0, 0 otherwise ="), "element 1: result isn't a finite number");
}

/// Text lookups follow the same rule, for a text `x` and for a text result.
#[test]
fn text_lookups_fail_only_the_elements_that_pick_them() {
    let mut env = Env::new();
    for src in [
        "Material := [\"A\", \"B\"]",
        "S := [10, 20]",
        "mat := [\"A\", \"C\", \"B\"]",
        "k := [1, 2, 9]",
        "words := [\"ten\", \"twenty\", \"thirty\"]",
        "pair := [\"ten\", \"twenty\"]",
    ] {
        run(&mut env, src).unwrap();
    }
    assert_eq!(vector_in(&mut env, "s_1 := lookup(Material, S, mat) if mat ≠ \"C\", 0 otherwise ="), vec![10.0, 0.0, 20.0]);
    assert_eq!(
        run_value(&mut env, "w := lookup([1, 2, 3], words, k) if k ≤ 3, \"none\" otherwise ="),
        Ok(Some(Value::TextGrid { rows: 3, cols: 1, items: vec!["ten".into(), "twenty".into(), "none".into()] }))
    );
    assert_eq!(
        run_value(&mut env, "w_2 := lookup(Material, pair, mat) if mat ≠ \"C\", \"none\" otherwise ="),
        Ok(Some(Value::TextGrid { rows: 3, cols: 1, items: vec!["ten".into(), "none".into(), "twenty".into()] }))
    );
    assert_eq!(err_in(&mut env, "lookup(Material, S, mat) ="), "element 1: \"C\" isn't in Material");
    // An `x` whose every element failed is no number to refuse against text `xs`: each element
    // keeps its own failure.
    run(&mut env, "z_0 := [0, 0]").unwrap();
    assert_eq!(err_in(&mut env, "b := lookup(Material, S, 1/z_0) if z_0 == 0, 0 otherwise ="), "element 0: result isn't a finite number");
}

/// A chain is the `and` of its links: a link's right operand is worked out only where every
/// earlier link holds, and a failure where an element is already false is ignored.
#[test]
fn a_chain_is_the_and_of_its_links() {
    let mut env = Env::new();
    run(&mut env, "b := 0").unwrap();
    assert_eq!(run(&mut env, "0 < b < 1/b =").unwrap().unwrap().value, 0.0);
    assert_eq!(run(&mut env, "0 < b and b < 1/b =").unwrap().unwrap().value, 0.0);
    // A single false stops the chain before a later operand is worked out.
    assert_eq!(run(&mut env, "1 < 0 < undefined_name =").unwrap().unwrap().value, 0.0);
    assert_eq!(run(&mut env, "5 < 3 < [1, 2] ="), Ok(Some(Quantity::dimensionless(0.0))));
    // A link that is reached and fails still fails.
    assert_eq!(err_in(&mut env, "-1 < b < 1/b ="), "result isn't a finite number");
    // Per element.
    run(&mut env, "v := [0.5, 0, 2]").unwrap();
    assert_eq!(vector_in(&mut env, "0 < v < 1/v ="), vec![1.0, 0.0, 0.0]);
    assert_eq!(vector_in(&mut env, "0 < v and v < 1/v ="), vec![1.0, 0.0, 0.0]);
    assert_eq!(err_in(&mut env, "-1 < v < 1/v ="), "element 1: result isn't a finite number");
    // Three links: the third operand only where the first two hold.
    run(&mut env, "w := [1, 0, -1, 0.5]").unwrap();
    assert_eq!(vector_in(&mut env, "0 < w ≤ 1 < 2/w ="), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(err_in(&mut env, "-1 < w ≤ 1 < 2/w ="), "element 1: result isn't a finite number");
    // In a branch, as a condition.
    assert_eq!(vector_in(&mut env, "c := 1 if 0 < v < 1/v, 2 otherwise ="), vec![1.0, 2.0, 2.0]);
}

#[test]
fn piecewise_details() {
    let mut env = Env::new();
    run(&mut env, "x := [4, -1, 9, 0]").unwrap();
    // Only the picked branch of a single condition is evaluated.
    assert_eq!(run(&mut env, "p := 1 if 1 > 0, undefined_name otherwise =").unwrap().unwrap().value, 1.0);
    assert_eq!(run(&mut env, "p_2 := if(1 < 0, undefined_name, 2) =").unwrap().unwrap().value, 2.0);
    // A later condition that fails for an element already decided doesn't fail it; a still
    // undecided one does, and so does a picked branch that fails as a whole.
    assert_eq!(vector_in(&mut env, "c := 1 if x ≤ 0, 2 if 1/x > 0.2, 3 otherwise ="), vec![2.0, 1.0, 3.0, 1.0]);
    assert!(err_in(&mut env, "c_2 := 1 if x < 0, 2 if 1/x > 0.2, 3 otherwise =").starts_with("element 3: result isn't a finite number"));
    assert_eq!(err_in(&mut env, "c_3 := [1, 2] if x > 0, 0 otherwise ="), "shapes don't match: 4 rows × 1 vs 2 rows × 1");
    assert_eq!(err_in(&mut env, "c_4 := 1 if x > 0, 2 if [1, 0] > 0, 3 otherwise ="), "shapes don't match: 4 rows × 1 vs 2 rows × 1");
    // ... but an unpicked branch that fails as a whole doesn't matter.
    assert_eq!(vector_in(&mut env, "c_5 := 1 if x > -5, undefined_name otherwise ="), vec![1.0; 4]);
    // A single condition after a vector one decides every element still undecided.
    assert_eq!(vector_in(&mut env, "c_6 := x if x > 5, 7 if 1 > 0 ="), vec![7.0, 7.0, 9.0, 7.0]);
    // Each element keeps what its value stands for as a temperature.
    run(&mut env, "T := 100 °F if x > 0, 200 °F otherwise").unwrap();
    assert!(run_value(&mut env, "T = ? °F").is_ok());
    // A condition must be a plain number; text or a unit value is an error.
    assert_eq!(err_in(&mut env, "d := 1 if \"a\", 2 otherwise ="), "\"a\" is text — it can't be used as a condition");
    assert_eq!(err_in(&mut env, "d := 1 if 1 in, 2 otherwise ="), "a condition must be a comparison or a plain number");
    assert_eq!(err_in(&mut env, "d := 1 if x > 0, 2 if 1 in, 3 otherwise ="), "a condition must be a comparison or a plain number");
    // `if` takes three arguments.
    assert_eq!(err_in(&mut env, "d := if(1, 2) ="), "if expects 3 argument(s)");
    // The no-case message: up to three variables, never units, functions or constants.
    run(&mut env, "a := 1").unwrap();
    run(&mut env, "b := \"x\"").unwrap();
    run(&mut env, "k := 2 in").unwrap();
    run(&mut env, "id(v) := v").unwrap();
    assert_eq!(err_in(&mut env, "n_c := 1 if id(a) > 2 and π > 4 and k > 1 m and b == \"y\" ="), "no case applies (a = 1, k = 2 in, b = \"x\")");
    assert_eq!(err_in(&mut env, "n_c := 1 if 1 > 2 ="), "no case applies");
    // Text grids, matrices and the origin.
    env.set_origin(1);
    run(&mut env, "M := [1, -2; 3, -4]").unwrap();
    assert!(err_in(&mut env, "n_c := 1 if M > 0 =").starts_with("element [1, 2]: no case applies (M = -2)"));
}

#[test]
fn logic_short_circuits_per_element() {
    let mut env = Env::new();
    run(&mut env, "x := [4, -1, 9, 0]").unwrap();
    assert_eq!(vector_in(&mut env, "o := x == 0 or 1/x > 0.2 ="), vec![1.0, 0.0, 0.0, 1.0]);
    // An element the left side doesn't decide takes the right side's failure.
    assert!(err_in(&mut env, "o := x ≠ 4 and 1/x > 0 =").starts_with("element 3: result isn't a finite number"));
    // A left side that decides every element leaves the right side unevaluated.
    assert_eq!(vector_in(&mut env, "o := x > 100 and undefined_name ="), vec![0.0; 4]);
    // The right side broadcasts a single value, and must otherwise have the left's shape.
    assert_eq!(vector_in(&mut env, "o := x > 0 or 1 > 2 ="), vec![1.0, 0.0, 1.0, 0.0]);
    assert_eq!(err_in(&mut env, "o := x > 0 or [1, 0] > 0 ="), "shapes don't match: 4 rows × 1 vs 2 rows × 1");
}

#[test]
fn piecewise_element_errors_are_named_once_at_a_strict_caller() {
    let mut env = Env::new();
    run(&mut env, "x := [4, -1, 9, 0]").unwrap();
    let want = "element 3: result isn't a finite number";
    // A definition that shows its value, and one that doesn't.
    assert_eq!(err_in(&mut env, "b := 1/x if x ≥ 0, 0 otherwise ="), want);
    assert_eq!(err_in(&mut env, "b := 1/x if x ≥ 0, 0 otherwise"), want);
    assert!(env.data("b").is_none(), "nothing failed is defined");
    // An evaluation, through `run_stmt` and through `eval_expr`.
    assert_eq!(err_in(&mut env, "if(x ≥ 0, 1/x, 0) ="), want);
    assert_eq!(eval_expr(&env, &expr_of("if(x ≥ 0, 1/x, 0)")).map_err(|e| e.to_string()), Err(want.to_string()));
    // A function's piecewise body called with a vector.
    run(&mut env, "f(v) := 1/v if v ≥ 0, 0 otherwise").unwrap();
    assert_eq!(err_in(&mut env, "f(x) ="), want);
    // An indexed definition: element by element, a failing element is named once; a body that
    // is a vector is that error, even when one of its elements failed.
    run(&mut env, "i := 0..3").unwrap();
    assert_eq!(err_in(&mut env, "t[i] := 1/x[i] if x[i] ≥ 0, 0 otherwise"), want);
    assert_eq!(err_in(&mut env, "t[i] := 1/x if x ≥ 0, 0 otherwise"), "t[i] is one element — its right side can't be a vector");
    assert_eq!(vector_in(&mut env, "t[i] := 1/x[i] if x[i] > 0, 0 otherwise ="), vec![0.25, 0.0, 1.0 / 9.0, 0.0]);
}

#[test]
fn piecewise_work_is_bounded() {
    // Many cases over a large vector: each condition's work is counted, and an element decided
    // early stops the rest from being tried.
    let mut env = Env::new();
    run(&mut env, "v := 1..10000").unwrap();
    let many = (0..300).map(|k| format!("{k} if v < {k}")).collect::<Vec<_>>().join(", ");
    let stmt = parse(&format!("y := {many}, -1 otherwise")).unwrap();
    assert_eq!(run_stmt_in(&mut env, &stmt, &Pass::new(100_000)).unwrap_err(), CalcError::TooMuchWork);
    // A long chain of nested piecewise functions is bounded by the call depth.
    let mut env = Env::new();
    run(&mut env, "f_0(v) := v").unwrap();
    for k in 1..40 {
        run(&mut env, &format!("f_{k}(v) := f_{}(v) + 1 if v > 0, 0 otherwise", k - 1)).unwrap();
    }
    assert_eq!(err_in(&mut env, "f_39([1, 2]) ="), "too complex");
}

#[test]
fn text_errors_name_a_text_that_did_not_fail() {
    let texts =
        Data::TextGrid(Grid { rows: 3, cols: 1, items: vec![String::new(), "b".into(), "c".into()], failed: vec![(0, CalcError::NonFinite)] });
    assert_eq!(texts.text(), Some("b"));
}

#[test]
fn deep_piecewise_is_refused_not_a_stack_overflow() {
    // Hand-built `if(x > 0, if(x > 0, …), 0)` with a vector `x`, so every level decides per
    // element: just inside the depth limit evaluates, far past it is refused. Each level costs
    // more stack than a call level (eval → call → piecewise → per_element → eval); 8 MB covers it,
    // well inside a recalculation's own stack.
    let mut env = Env::new();
    run(&mut env, "x := [1, -1]").unwrap();
    let deep = move || {
        let condition = expr_of("x > 0");
        let nest = |n: usize| (0..n).fold(Expr::Num(1.0), |e, _| Expr::Call("if".into(), vec![condition.clone(), e, Expr::Num(0.0)]));
        assert_eq!(
            eval_expr(&env, &nest(1000)),
            Ok(Value::Grid { rows: 2, cols: 1, items: vec![Quantity::dimensionless(1.0), Quantity::dimensionless(0.0)] })
        );
        assert_eq!(eval_expr(&env, &nest(3000)), Err(CalcError::TooComplex));
        let cases = |n: usize| {
            (0..n).fold(Expr::Num(1.0), |e, _| Expr::Cases { cases: vec![(e, condition.clone())], otherwise: Some(Box::new(Expr::Num(0.0))) })
        };
        assert_eq!(eval_expr(&env, &cases(3000)), Err(CalcError::TooComplex));
    };
    std::thread::Builder::new().stack_size(8 << 20).spawn(deep).expect("spawns").join().expect("no panic");
}

#[test]
fn no_case_messages_are_written_only_when_they_can_be_seen() {
    let spent = |env: &mut Env, src: &str| {
        let pass = Pass::new(u64::MAX);
        let _ = run_stmt_in(env, &parse(src).unwrap(), &pass);
        pass.spent.get()
    };
    let mut env = Env::new();
    run(&mut env, "v := 1..10000").unwrap();
    run(&mut env, "neg(w) := 5 if w < 2").unwrap();
    run(&mut env, "neg_2(w) := 5 if w < 2, 6 otherwise").unwrap();
    // In a branch only the first element picks, the other 9,999 elements' messages are still
    // written (the branch keeps them), and each counts as work.
    let without = spent(&mut env, "n := neg_2(v) if v < 2, 0 otherwise");
    let with = spent(&mut env, "n := neg(v) if v < 2, 0 otherwise");
    assert!(with >= without + 9_990, "{with} vs {without}");
    let tight = Pass::new(without + 5_000);
    assert_eq!(run_stmt_in(&mut env, &parse("n := neg(v) if v < 2, 0 otherwise").unwrap(), &tight), Err(CalcError::TooMuchWork));
    assert_eq!(run(&mut env, "n := neg(v) if v < 2, 0 otherwise"), Ok(None));
    // A strict caller is told of the first failed element only, so only that message is written.
    let (strict, decided) = (spent(&mut env, "n := 5 if v < 0"), spent(&mut env, "n := 5 if v < 0, 6 otherwise"));
    assert!(strict <= decided + 5, "{strict} vs {decided}");
    assert_eq!(err_in(&mut env, "n := 5 if v < 0"), "element 0: no case applies (v = 1)");
    // The first failed element is reported, whatever failed it.
    run(&mut env, "x := [0, 5, -1]").unwrap();
    assert_eq!(err_in(&mut env, "n := 1/x if x < 1"), "element 0: result isn't a finite number");
    assert_eq!(err_in(&mut env, "n := 1/x if x < 0"), "element 0: no case applies (x = 0)");
}

#[test]
fn statement_boundaries_refuse_a_failed_grid() {
    // No strict evaluation hands these boundaries a grid with failed elements (it names the
    // element itself), so the grid is put in the environment directly, as the smallest seam that
    // reaches each `strict` call.
    let mut env = Env::new();
    let items = vec![Val::plain(Quantity::dimensionless(1.0)), crate::array::placeholder(), Val::plain(Quantity::dimensionless(3.0))];
    let failed = Grid::with_failed(3, 1, items, vec![(1, CalcError::NonFinite)]).unwrap();
    env.entries.insert("f".into(), super::Entry::Var(Data::Grid(failed)));
    let want = "element 1: result isn't a finite number".to_string();
    // `eval_expr`.
    assert_eq!(eval_expr(&env, &Expr::Name("f".into())).map_err(|e| e.to_string()), Err(want.clone()));
    // `run_stmt_in`: an evaluation, and a definition with and without show; nothing is defined.
    assert_eq!(err_in(&mut env, "f ="), want);
    assert_eq!(err_in(&mut env, "y := f ="), want);
    assert_eq!(err_in(&mut env, "y := f"), want);
    assert!(env.data("y").is_none());
    // An indexed body that is a failed grid is a vector, checked before its failures.
    run(&mut env, "i := 0..2").unwrap();
    assert_eq!(err_in(&mut env, "t[i] := f"), "t[i] is one element — its right side can't be a vector");
}

#[test]
fn sums() {
    let mut env = Env::new();
    for src in ["A_b := 0.5 in^2", "r := [2, 3, 4] in", "a := [1, 2, 3]", "b := [4, 5, 6]"] {
        run(&mut env, src).unwrap();
    }
    assert_eq!(run(&mut env, "∑(i = 0..2) a[i]·b[i] =").unwrap().unwrap().value, 32.0);
    assert_eq!(run(&mut env, "∑(i = 0..2) a[i]·b[i] + 1 =").unwrap().unwrap().value, 33.0);
    let i = run(&mut env, "I := ∑(i = 0..length(r) - 1) A_b·r[i]^2 = ? in^4").unwrap().unwrap();
    assert!((i.value - 0.5 * (4.0 + 9.0 + 16.0) * 0.0254_f64.powi(4)).abs() < 1e-15);
    assert_eq!(run(&mut env, "∑(i = 1..3) ∑(j = 1..i) j =").unwrap().unwrap().value, 10.0);
    assert_eq!(run(&mut env, "∑(k = 1, 3..7) k =").unwrap().unwrap().value, 16.0);
    // Two sums may share an index; a defined name may not be one.
    assert_eq!(run(&mut env, "∑(i = 1..2) i + ∑(i = 1..3) i =").unwrap().unwrap().value, 9.0);
    assert_eq!(run(&mut env, "∑(a = 1..2) a =").unwrap_err().to_string(), "the sum's index a is already defined — choose another name");
    assert_eq!(run(&mut env, "∑(i = 1..2) ∑(i = 1..2) i =").unwrap_err().to_string(), "the sum's index i is already defined — choose another name");
    assert!(matches!(run(&mut env, "∑(m = 1..2) m ="), Err(CalcError::ReservedName(_))));
    assert_eq!(run(&mut env, "∑(i = 1..0) i =").unwrap_err().to_string(), "the sum's range is empty — its last value is below its first");
    assert!(run(&mut env, "∑(i = 0..2) (a[i] + r[i]) =").is_err(), "mixed units");
    assert!(matches!(run(&mut env, "∑(i = 1..1e9) i ="), Err(CalcError::TooLarge | CalcError::TooMuchWork)));
    // Review Focus 5: a parameter in the range.
    run(&mut env, "tri(n) := ∑(i = 1..n) i").unwrap();
    assert_eq!(run(&mut env, "tri(4) =").unwrap().unwrap().value, 10.0);
}

#[test]
fn a_sum_binds_its_index_only_in_its_body() {
    let mut env = Env::new();
    // An outer index is a value in the ends of an inner sum, and a parameter too.
    assert_eq!(run(&mut env, "∑(i = 1..3) ∑(j = i..3) j =").unwrap().unwrap().value, 6.0 + 5.0 + 3.0);
    run(&mut env, "tri(n) := ∑(i = 1..n) i").unwrap();
    assert_eq!(run(&mut env, "∑(k = 1..3) tri(k) =").unwrap().unwrap().value, 1.0 + 3.0 + 6.0);
    // A parameter can't be an index, and a function's body doesn't see the caller's index.
    run(&mut env, "bad(n) := ∑(n = 1..2) n").unwrap();
    assert_eq!(err_in(&mut env, "bad(1) ="), "the sum's index n is already defined — choose another name");
    run(&mut env, "leak(x) := x + k").unwrap();
    assert_eq!(err_in(&mut env, "∑(k = 1..2) leak(k) ="), "k isn't defined");
    // The index isn't left behind.
    assert_eq!(err_in(&mut env, "∑(k = 1..2) k + k ="), "k isn't defined");
}

#[test]
fn a_sum_index_must_be_unused_by_functions_and_failed_names() {
    let mut env = Env::new();
    run(&mut env, "f(x) := x").unwrap();
    let msg = "the sum's index f is already defined — choose another name";
    assert_eq!(err_in(&mut env, "∑(f = 1..2) f ="), msg);
    env.mark_failed("w");
    assert_eq!(err_in(&mut env, "∑(w = 1..2) w ="), "the sum's index w is already defined — choose another name");
}

#[test]
fn sum_terms_add_under_the_unit_rules() {
    let mut env = Env::new();
    // Whole vectors add element by element.
    run(&mut env, "M := [1, 2; 3, 4]").unwrap();
    match run_value(&mut env, "∑(i = 1..3) M·i =").unwrap().unwrap() {
        Value::Grid { items, rows, cols } => {
            assert_eq!((rows, cols), (2, 2));
            assert_eq!(items.iter().map(|q| q.value).collect::<Vec<_>>(), vec![6.0, 12.0, 18.0, 24.0]);
        }
        other => panic!("{other:?}"),
    }
    // Shapes must match across terms.
    run(&mut env, "u := [1, 2, 3]").unwrap();
    run(&mut env, "w := [1, 2]").unwrap();
    assert!(run_value(&mut env, "∑(i = 0..1) if(i == 0, u, w) =").is_err());
    // Units must agree, and text is refused with its place.
    assert!(run(&mut env, "∑(i = 1..2) i·1 in =").is_ok());
    assert!(run(&mut env, "∑(i = 1..2) if(i == 1, 1 in, 1 lbf) =").is_err());
    assert_eq!(err_in(&mut env, "∑(i = 1..2) \"a\" ="), "\"a\" is text — it can't be used in a sum");
    // A single term is the term itself, in its own unit.
    let one_term = run(&mut env, "∑(i = 2..2) i·3 in =").unwrap().unwrap();
    close(one_term, 6.0 * 0.0254, Dim { exp: [1, 0, 0, 0, 0, 0, 0, 0] });
}

#[test]
fn sums_are_bounded_work() {
    let mut env = Env::new();
    // 10,000 × 10,000 terms is far past the budget.
    assert!(matches!(run(&mut env, "∑(i = 1..10000) ∑(j = 1..10000) j ="), Err(CalcError::TooComplex | CalcError::TooMuchWork)));
    // A sum nested as deep as the parser allows still evaluates.
    let opens: String = (0..40).map(|k| format!("∑(i_{k} = 1..1) ")).collect();
    let src = format!("{opens}1 =");
    assert_eq!(run(&mut env, &src).unwrap().unwrap().value, 1.0);
}

#[test]
fn percent_unit() {
    close(val("5 % · 200 ="), 10.0, Dim::ONE);
    close(val("5 % ="), 0.05, Dim::ONE);
    close(val("x := 0.25 = ? %"), 0.25, Dim::ONE);
}

#[test]
fn hour_is_hr() {
    close(val("1 hr ="), 3600.0, Dim::TIME);
    let q = val("3412.14 Btu/hr = ? watt");
    assert_eq!(q.dim, val("1 watt =").dim);
    assert!((q.value - 1000.0).abs() < 1e-3, "{}", q.value);
}

#[test]
fn h_hints_hr() {
    let e = err("5 h =");
    assert_eq!(e, CalcError::Undefined("h".into()));
    assert_eq!(e.to_string(), "h isn't defined — for hours, use hr");
    // Any other name has no hint.
    assert_eq!(err("5 hh =").to_string(), "hh isn't defined");
    // As a unit to show a result in, too, alone or inside a compound unit.
    for src in ["t := 2 hr = ? h", "1 kW·hr = ? kW·h", "1 kW = ? BTU/h"] {
        let e = err(src);
        assert_eq!(e, CalcError::Unit(UnitError::UnknownUnit("h".into())), "{src}");
        assert_eq!(e.to_string(), "unknown unit `h` — for hours, use hr", "{src}");
    }
    // Another unknown unit has no hint.
    assert_eq!(err("1 in = ? hh").to_string(), "unknown unit `hh`");
}

#[test]
fn sec_called_hints_secant() {
    let e = syntax_msg(err("sec(60 deg) ="));
    assert_eq!(e, "sec isn't a function — for the secant, use secant(x)");
    // Another unit called keeps the plain message.
    assert_eq!(syntax_msg(err("m(2) =")), "m isn't a function — it can't be called with ( )");
    close(val("secant(60 deg) ="), 2.0, Dim::ONE);
}

#[test]
fn newly_reserved_names() {
    for name in ["bar", "day", "ton", "cal", "mbar", "kbar", "kcal"] {
        let e = err(&format!("{name} := 3"));
        assert_eq!(e, CalcError::ReservedName(name.into()), "{name}");
    }
    // bar takes no prefix: these stay free as names.
    for name in ["ubar", "cbar", "nbar", "Mbar", "µbar"] {
        assert_eq!(check_name(name), Ok(()), "{name}");
    }
}

#[test]
fn reserved_solver_names() {
    for name in ["Given", "Find", "Minerr", "root"] {
        assert_eq!(check_name(name), Err(CalcError::ReservedName(name.into())), "{name}");
        assert_eq!(err(&format!("{name} := 1")), CalcError::ReservedName(name.into()), "{name}");
        assert_eq!(err(&format!("{name}(x) := x")), CalcError::ReservedName(name.into()), "{name}");
        assert_eq!(err(&format!("[{name}; q] := [1; 2]")), CalcError::ReservedName(name.into()), "{name}");
    }
    // A parameter is a name too.
    assert_eq!(err("f(Given) := 1"), CalcError::ReservedName("Given".into()));
    // `Given` can't be called, `root` is a call of its own (two arguments), and `Find` and `Minerr`
    // say what they need.
    assert!(syntax_msg(err("Given(1) =")).contains("isn't a function"));
    assert_eq!(err("root(1) ="), CalcError::Arity("root".into(), 2));
    // Names that merely contain one are free.
    for ok in ["Given_1", "Finder", "roots", "find", "given"] {
        assert_eq!(check_name(ok), Ok(()), "{ok}");
    }
}

#[test]
fn define_many_runs() {
    let mut env = Env::new();
    assert_eq!(run(&mut env, "[r_1; r_2] := [1; 2 in]").ok(), Some(None));
    close(shown(&mut env, "r_1 ="), 1.0, Dim::ONE);
    close(shown(&mut env, "r_2 ="), 0.0254 * 2.0, Dim::LENGTH);
    // Shown, it is the whole column; a unit applies to every element.
    let (shape, items) = grid(&mut env, "[u_1; u_2; u_3] := [1 m; 2 m; 3 m] =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[1.0, 2.0, 3.0], Dim::LENGTH);
    close(shown(&mut env, "u_3 ="), 3.0, Dim::LENGTH);
    let (_, items) = grid(&mut env, "[w_1; w_2] := [1 m; 2 m] = ? mm");
    close_all(&items, &[1.0, 2.0], Dim::LENGTH);
    let e = run_value(&mut env, "[w_3; w_4] := [1 m; 2 m] = ? kg").expect_err("a mass unit for lengths");
    assert!(matches!(e, CalcError::Element { ref error, .. } if matches!(**error, CalcError::Unit(UnitError::Mismatch { .. }))), "{e:?}");
    assert!(!env.names().contains(&"w_3".to_string()));
    // Texts are defined too.
    assert_eq!(run(&mut env, "[t_1; t_2] := [\"OK\"; \"NG\"]").ok(), Some(None));
    assert!(matches!(run_value(&mut env, "t_2 ="), Ok(Some(Value::Text(t))) if t == "NG"));
    // A name defined this way is an ordinary one.
    assert_eq!(run(&mut env, "d := r_1 + 1").ok(), Some(None));
    assert!(env.names().contains(&"r_2".to_string()));
}

#[test]
fn define_many_checks_the_length_and_every_name() {
    let mut env = Env::new();
    let before = env.names();
    let e = run(&mut env, "[r_1; r_2] := [1; 2; 3]").expect_err("3 values for 2 names");
    assert_eq!(syntax_msg(e), "[r_1; r_2] takes a column of 2 values, but the right side has 3");
    let e = run(&mut env, "[r_1; r_2; r_3] := [1; 2]").expect_err("2 values for 3 names");
    assert_eq!(syntax_msg(e), "[r_1; r_2; r_3] takes a column of 3 values, but the right side has 2");
    let e = run(&mut env, "[r_1; r_2] := 5").expect_err("one value");
    assert_eq!(syntax_msg(e), "[r_1; r_2] takes a column of 2 values, but the right side is one value");
    let e = run(&mut env, "[r_1; r_2] := transpose([1; 2])").expect_err("a row");
    assert_eq!(syntax_msg(e), "[r_1; r_2] takes a column of 2 values, but the right side is 1 row × 2 columns");
    let e = run(&mut env, "[r_1; r_2] := \"OK\"").expect_err("a text");
    assert_eq!(syntax_msg(e), "[r_1; r_2] takes a column of 2 values, but the right side is one value");
    // Each name obeys `check_name`, and a refused one leaves nothing behind.
    for (src, name) in [("[r_1; m] := [1; 2]", "m"), ("[sin; r_1] := [1; 2]", "sin"), ("[r_1; Find] := [1; 2]", "Find"), ("[π; r_1] := [1; 2]", "π")]
    {
        assert_eq!(run(&mut env, src).expect_err(src), CalcError::ReservedName(name.into()), "{src}");
    }
    assert_eq!(
        run(&mut env, "[r_1; r__2] := [1; 2]").expect_err("empty subscript").to_string(),
        "r__2 has an empty subscript — write a name like a_b"
    );
    // The body's own error comes first, and nothing is defined on any failure.
    assert!(matches!(run(&mut env, "[r_1; r_2] := [1; zz]"), Err(CalcError::Undefined(_))));
    assert_eq!(env.names(), before);
}

#[test]
fn unresolved_find_errors() {
    let mut env = Env::new();
    for (src, word) in [("x_s := Find(x)", "Find"), ("x_s := Minerr(x, y)", "Minerr"), ("Find(x) =", "Find"), ("[a_1; a_2] := Find(x, y)", "Find")] {
        let e = run(&mut env, src).expect_err(src);
        assert_eq!(syntax_msg(e), format!("{word} must end a Given block"), "{src}");
    }
    // Inside an expression, a call can never end a block.
    for (src, word) in [("2 Minerr(x) =", "Minerr"), ("x_s := Find(x) + 1", "Find"), ("[a_1; a_2] := 2·Find(x, y)", "Find")] {
        let e = run(&mut env, src).expect_err(src);
        assert_eq!(syntax_msg(e), format!("{word} must be the whole right side of the line that ends a Given block"), "{src}");
    }
    assert!(env.names().is_empty());
    // Whatever its arguments are, even ones that don't exist.
    assert_eq!(syntax_msg(err("Find(nothing_here) =")), "Find must end a Given block");
    assert_eq!(syntax_msg(err("Find() =")), "Find must end a Given block");
}

#[test]
fn given_and_constraint_run_quietly() {
    let mut env = Env::new();
    // The sides of an equation are not even evaluated: they may name the unknown.
    for src in ["Given", "x_1 == 2", "p == z + 1 psi", "undefined_name == 1"] {
        assert_eq!(run_stmt_in(&mut env, &parse(src).expect(src), &Pass::new(u64::MAX)).expect(src).map(|d| d.value()), None, "{src}");
    }
    assert!(env.names().is_empty());
}

/// Whether `a` agrees with Mathcad's `b` to three significant figures: `|a − b| ≤ 0.5·10^(⌊log10|b|⌋ − 2)`.
pub(crate) fn sig3(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.5 * 10f64.powi(b.abs().log10().floor() as i32 - 2)
}

/// A solve block as recalculation builds it: `eqs` are `lhs == rhs` sources.
fn solve_expr(kind: SolveKind, unknowns: &[&str], eqs: &[&str]) -> Expr {
    let equations = eqs
        .iter()
        .map(|src| match parse(src).unwrap_or_else(|e| panic!("{src:?} did not parse: {e}")) {
            Stmt::Constraint { lhs, rhs } => (lhs, rhs),
            other => panic!("{src:?} is not an equation: {other:?}"),
        })
        .collect();
    Expr::Solve { kind, unknowns: unknowns.iter().map(|n| (*n).to_string()).collect(), equations }
}

/// Find `unknowns` from `eqs` in `env`.
fn find(env: &Env, unknowns: &[&str], eqs: &[&str]) -> Result<Value, CalcError> {
    eval_expr(env, &solve_expr(SolveKind::Find, unknowns, eqs))
}

fn env_with(defs: &[&str]) -> Env {
    let mut env = Env::new();
    for d in defs {
        run(&mut env, d).unwrap_or_else(|e| panic!("{d:?}: {e}"));
    }
    env
}

fn values_of(v: Value) -> Vec<f64> {
    match v {
        Value::One(q) => vec![q.value],
        Value::Grid { items, .. } => items.iter().map(|q| q.value).collect(),
        other => panic!("expected numbers, got {other:?}"),
    }
}

#[test]
fn sig3_is_three_significant_figures() {
    assert!(sig3(0.014988971068715593, 0.01498897));
    assert!(sig3(0.01499, 0.014988971068715593));
    assert!(!sig3(0.0151, 0.014988971068715593));
    assert!(sig3(1230.0, 1231.0) && !sig3(1240.0, 1231.0));
}

#[test]
fn find_with_units() {
    let env = env_with(&["x := 1 in"]);
    let v = find(&env, &["x"], &["x^2 == 2 in^2"]).expect("solves");
    let Value::One(q) = v else { panic!("one value expected, got {v:?}") };
    // The answer has the guess's dimension, in SI.
    assert_eq!(q.dim, Dim::LENGTH);
    let root2_in = 2f64.sqrt() * 0.0254;
    assert!(sig3(q.value, root2_in), "{} vs {root2_in}", q.value);
    // The equation holds.
    assert!((q.value * q.value - 2.0 * 0.0254 * 0.0254).abs() <= 1e-9 * 2.0 * 0.0254 * 0.0254);
}

#[test]
fn find_two_unknowns_is_a_column_in_order() {
    let env = env_with(&["x := 1", "y := 1"]);
    let eqs = ["x + y == 3", "x - y == 1"];
    let v = find(&env, &["x", "y"], &eqs).expect("solves");
    assert!(matches!(&v, Value::Grid { rows: 2, cols: 1, .. }), "{v:?}");
    let xy = values_of(v);
    assert!(sig3(xy[0], 2.0) && sig3(xy[1], 1.0), "{xy:?}");
    assert!((xy[0] + xy[1] - 3.0).abs() < 1e-9 && (xy[0] - xy[1] - 1.0).abs() < 1e-9);
    // The order follows the unknowns' names.
    let yx = values_of(find(&env, &["y", "x"], &eqs).expect("solves"));
    assert!(sig3(yx[0], 1.0) && sig3(yx[1], 2.0), "{yx:?}");
}

/// A function called from the equations sees the document's guess, not the unknown being tried
/// (its scope is its parameters and the document's names), as the guide says.
#[test]
fn a_function_in_an_equation_sees_the_guess() {
    let mut env = env_with(&["x := 1"]);
    run(&mut env, "shift(a) := a + x").expect("defines");
    // shift(0) is the guess, 1, so x == shift(0) + 3 is x == 4 (were it the unknown, x == x + 3
    // has none).
    let x = values_of(find(&env, &["x"], &["x == shift(0) + 3"]).expect("solves"))[0];
    assert!(sig3(x, 4.0) && (x - 4.0).abs() <= 1e-9, "{x}");
}

#[test]
fn missing_guess_is_asked_for() {
    let env = env_with(&["y := 1"]);
    assert_eq!(syntax_msg(find(&env, &["x"], &["x^2 == 2"]).expect_err("no guess")), "give x a starting value");
    // Every unknown needs one.
    assert_eq!(syntax_msg(find(&env, &["y", "z"], &["y + z == 2"]).expect_err("no guess")), "give z a starting value");
}

#[test]
fn guess_must_be_one_number() {
    for guess in ["x := [1, 2]", "x := \"one\""] {
        let env = env_with(&[guess]);
        let m = syntax_msg(find(&env, &["x"], &["x^2 == 2"]).expect_err(guess));
        assert!(m.contains("x's starting value must be one number"), "{guess}: {m}");
    }
}

#[test]
fn equation_unit_mismatch() {
    // The unit error names the equation, counted from 1.
    let env = env_with(&["p := 1 psi", "z := 2 ft"]);
    let e = find(&env, &["p"], &["p == 3 ft"]).expect_err("psi against ft");
    assert!(matches!(&e, CalcError::Equation { n: 1, error } if matches!(**error, CalcError::Unit(UnitError::Mismatch { .. }))), "{e:?}");
    assert!(e.to_string().starts_with("equation 1: "), "{e}");
    let e = find(&env, &["p"], &["p == 2 psi", "p + 1 psi == z"]).expect_err("psi against ft");
    assert!(matches!(&e, CalcError::Equation { n: 2, error } if matches!(**error, CalcError::Unit(UnitError::Mismatch { .. }))), "{e:?}");
    // Inside a side too.
    let e = find(&env, &["p"], &["p + z == 2 psi"]).expect_err("psi plus ft");
    assert!(matches!(&e, CalcError::Equation { n: 1, .. }), "{e:?}");
    // root has one expression, not equations: its unit error is as it is.
    let mut env = env_with(&["p := 1 psi"]);
    assert!(matches!(run(&mut env, "root(p − 3 ft, p) ="), Err(CalcError::Unit(UnitError::Mismatch { .. }))));
}

#[test]
fn an_equation_side_is_one_number() {
    let env = env_with(&["x := 1", "v := [1, 2]"]);
    let m = syntax_msg(find(&env, &["x"], &["x == v"]).expect_err("a vector side"));
    assert!(m.contains("single numbers"), "{m}");
}

#[test]
fn guess_is_untouched() {
    let mut env = env_with(&["x := 1 in"]);
    find(&env, &["x"], &["x^2 == 2 in^2"]).expect("solves");
    let q = shown(&mut env, "x =");
    close(q, 0.0254, Dim::LENGTH);
}

#[test]
fn a_guess_of_zero_solves() {
    let env = env_with(&["x := 0"]);
    let x = values_of(find(&env, &["x"], &["x^2 + x == 2"]).expect("solves"))[0];
    // From 0 the nearer root is 1 (the other is −2).
    assert!(sig3(x, 1.0) || sig3(x, -2.0), "{x}");
    assert!((x * x + x - 2.0).abs() < 1e-9);
}

#[test]
fn nonfinite_at_the_guess_is_an_error() {
    let env = env_with(&["x := 0"]);
    assert_eq!(find(&env, &["x"], &["1/x == 1"]), Err(CalcError::NonFinite));
}

#[test]
fn nonfinite_at_a_trial_point_is_only_a_bad_trial() {
    // From 4 the first Newton step lands on a negative number, where sqrt is not finite.
    let env = env_with(&["x := 4"]);
    let x = values_of(find(&env, &["x"], &["sqrt(x) == 0.5"]).expect("solves"))[0];
    assert!(sig3(x, 0.25), "{x}");
}

#[test]
fn minerr_takes_the_nearest() {
    let env = env_with(&["x := 1"]);
    let v = eval_expr(&env, &solve_expr(SolveKind::Minerr, &["x"], &["x == 1", "x == 3"])).expect("minimizes");
    assert!(sig3(values_of(v)[0], 2.0));
}

#[test]
fn root_form() {
    let mut env = env_with(&["x := 1"]);
    let q = shown(&mut env, "root(x^2 − 2, x) =");
    assert!(sig3(q.value, 2f64.sqrt()), "{}", q.value);
    assert!((q.value * q.value - 2.0).abs() < 1e-9);
    // In a function, the parameters are visible; the guess is still the document's.
    run(&mut env, "g2(a) := root(x^2 − a, x)").expect("defines");
    assert!(sig3(shown(&mut env, "g2(9) =").value, 3.0));
    assert!(sig3(shown(&mut env, "g2(16) =").value, 4.0));
    // The guess is untouched.
    close(shown(&mut env, "x ="), 1.0, Dim::ONE);
}

#[test]
fn root_with_units() {
    let mut env = env_with(&["x := 1 in"]);
    let q = shown(&mut env, "root(x^2 − 2 in^2, x) =");
    assert_eq!(q.dim, Dim::LENGTH);
    assert!(sig3(q.value, 2f64.sqrt() * 0.0254), "{}", q.value);
}

#[test]
fn root_arity_and_unknown() {
    let mut env = env_with(&["x := 1"]);
    for src in ["root(x) =", "root(x, x, x) =", "root() ="] {
        assert_eq!(run(&mut env, src), Err(CalcError::Arity("root".into(), 2)), "{src}");
    }
    let m = syntax_msg(run(&mut env, "root(x^2 − 2, 3) =").expect_err("not a name"));
    assert!(m.contains("the unknown"), "{m}");
}

#[test]
fn root_without_a_solution() {
    let mut env = env_with(&["x := 1"]);
    let m = err_in(&mut env, "root(x^2 + 1, x) =");
    assert!(m.contains("no solution found near the guesses"), "{m}");
}

#[test]
fn no_solution_message() {
    let env = env_with(&["x := 1"]);
    let m = find(&env, &["x"], &["x^2 == -1"]).expect_err("no real root").to_string();
    assert!(m.starts_with("no solution found near the guesses — the equations are still off by "), "{m}");
    assert!(m.ends_with("; try other guesses"), "{m}");
}

/// The final review's probes: a difference that is small only next to the sides at the guess, or
/// next to a huge guess, is no solution. Each is checked at the answer alone.
#[test]
fn non_solutions_are_never_shown() {
    const NONE: &str = "no solution found near the guesses — the equations are still off by ";
    // From 30, exp(x) + 1 falls to 1 far to the left: 1 against 0.5.
    let env = env_with(&["x := 30"]);
    let m = find(&env, &["x"], &["exp(x) + 1 == 0.5"]).expect_err("exp(x) + 1 is never 0.5").to_string();
    assert!(m.starts_with(NONE), "{m}");
    for guess in ["x := 1e6", "x := 1e3"] {
        let env = env_with(&[guess]);
        let m = find(&env, &["x"], &["x^2 == −1"]).expect_err(guess).to_string();
        assert!(m.starts_with(NONE), "{guess}: {m}");
    }
    for guess in ["x := 1e6", "x := 1e8", "x := 1e3"] {
        let mut env = env_with(&[guess]);
        let m = err_in(&mut env, "root(x^2 + 1, x) =");
        assert!(m.starts_with(NONE), "{guess}: {m}");
    }
    // From 1e3 the solve ends near 0, where the equation misses by all of its size.
    let env = env_with(&["x := 1e3"]);
    let m = find(&env, &["x"], &["x^2 == −1"]).expect_err("no real root").to_string();
    assert!(m.contains("off by 100%"), "{m}");
}

#[test]
fn failures_have_their_messages() {
    use crate::solve::Failure;
    assert_eq!(super::solve_failure(Failure::NotConverged { steps: 200 }).to_string(), "the solve didn't converge in 200 steps — try other guesses");
    assert_eq!(super::solve_failure(Failure::NonFinite), CalcError::NonFinite);
    let off = |worst| super::solve_failure(Failure::NoSolution { worst }).to_string();
    assert_eq!(off(1.0), "no solution found near the guesses — the equations are still off by 100%; try other guesses");
    assert_eq!(off(0.0345), "no solution found near the guesses — the equations are still off by 3.5%; try other guesses");
    assert_eq!(off(0.25), "no solution found near the guesses — the equations are still off by 25%; try other guesses");
    assert_eq!(off(3.0e-7), "no solution found near the guesses — the equations are still off by 3.0e-7 of their size; try other guesses");
    // A percentage from 0.1% up (the most an equation can miss by is 200% of its scale).
    assert_eq!(off(1e-3), "no solution found near the guesses — the equations are still off by 0.10%; try other guesses");
    assert_eq!(off(9.9e-4), "no solution found near the guesses — the equations are still off by 9.9e-4 of their size; try other guesses");
    assert_eq!(off(2.0), "no solution found near the guesses — the equations are still off by 200%; try other guesses");
}

/// A trial point the equations can't be worked out at (a curve read past its last point) is a
/// bad trial, which the solve steps back from; the same error at the guesses is the solve's own.
#[test]
fn a_trial_past_the_data_is_a_bad_trial() {
    let defs = ["x_d := [0; 1; 2; 3; 4]", "y_d := [0; 1; 4; 9; 16]"];
    let curve = "linterp(x_d, y_d, x) == 15";
    // From 0.5 the first full step goes to about 15, past the data's last point, 4.
    let env = env_with(&[defs[0], defs[1], "x := 0.5"]);
    let x = values_of(find(&env, &["x"], &[curve]).expect("solves"));
    assert!(sig3(x[0], 3.0 + 6.0 / 7.0), "{x:?}");
    // The equation holds: on the last segment, y = 9 + 7·(x − 3).
    assert!((9.0 + 7.0 * (x[0] - 3.0) - 15.0).abs() <= 1e-9, "{x:?}");
    // A guess past the data is an error of the equation, shown as it is.
    let env = env_with(&[defs[0], defs[1], "x := 5"]);
    let e = find(&env, &["x"], &[curve]).expect_err("the guess is outside the data");
    assert!(e.to_string().contains("is outside x_d"), "{e}");
}

#[test]
fn solve_counts_work() {
    let mut env = env_with(&["x := 1"]);
    let stmt = parse("root(x^2 − 2, x) =").expect("parses");
    assert_eq!(run_stmt_in(&mut env, &stmt, &Pass::new(40)), Err(CalcError::TooMuchWork));
    // With room, the same statement works and its work grows with the iterations.
    let pass = Pass::new(u64::MAX);
    assert!(run_stmt_in(&mut env, &stmt, &pass).is_ok());
    assert!(pass.spent() > 40, "{}", pass.spent());
}

/// The pump worksheet's Colebrook block as the function `f_turb(Rn, ε_r) := Find(f_t)`.
fn pump_env() -> Env {
    let mut env = env_with(&["f_t := 0.1"]);
    let body = solve_expr(SolveKind::Find, &["f_t"], &["1/√f_t == −2·log(ε_r/3.7 + 2.51/(Rn·√f_t))"]);
    env.define_fn("f_turb", vec!["Rn".into(), "ε_r".into()], body);
    env
}

#[test]
fn block_as_function() {
    let mut env = pump_env();
    let (rn, eps) = (955_288.768_467_370_2_f64, 0.000_225_535_647_162_009_8_f64);
    let f = shown(&mut env, "f_turb(955288.76846737019, 0.00022553564716200979) =");
    assert_eq!(f.dim, Dim::ONE);
    // Mathcad's saved value in Mathcad Verification Simple Pump Model.xmcd.
    assert!(sig3(f.value, 0.014988971068715593), "{}", f.value);
    // The equation holds.
    let (lhs, rhs) = (1.0 / f.value.sqrt(), -2.0 * (eps / 3.7 + 2.51 / (rn * f.value.sqrt())).log10());
    assert!((lhs - rhs).abs() <= 1e-9 * lhs.abs(), "{lhs} vs {rhs}");
    // The guess is still the guess.
    close(shown(&mut env, "f_t ="), 0.1, Dim::ONE);
}

fn square_env() -> Env {
    let mut env = env_with(&["x := 1"]);
    env.define_fn("s", vec!["a".into()], solve_expr(SolveKind::Find, &["x"], &["x^2 == a"]));
    env
}

#[test]
fn block_function_over_vector() {
    let mut env = square_env();
    let v = vector_in(&mut env, "s([4; 9]) =");
    assert!(sig3(v[0], 2.0) && sig3(v[1], 3.0), "{v:?}");
    assert!((v[0] * v[0] - 4.0).abs() < 1e-9 && (v[1] * v[1] - 9.0).abs() < 1e-9);
    // A single call is a single value, as before.
    assert!(sig3(shown(&mut env, "s(25) =").value, 5.0));
    // Strict: the error names the element that has no solution, counted from the origin.
    let m = err_in(&mut env, "s([4; −1]) =");
    assert!(m.starts_with("element 1: no solution found near the guesses"), "{m}");
    env.set_origin(1);
    let m = err_in(&mut env, "s([4; −1]) =");
    assert!(m.starts_with("element 2: no solution found near the guesses"), "{m}");
}

#[test]
fn block_function_per_element_in_cases() {
    let mut env = square_env();
    run(&mut env, "v := [4; −1; 9]").expect("defines");
    // Only the elements that pick the branch can fail in it.
    let v = vector_in(&mut env, "y := s(v) if v > 0, 0 otherwise =");
    assert!(sig3(v[0], 2.0) && v[1] == 0.0 && sig3(v[2], 3.0), "{v:?}");
    // Picked by the failing element, it is that element's error.
    let m = err_in(&mut env, "z := s(v) if v > −5, 0 otherwise =");
    assert!(m.starts_with("element 1: no solution found near the guesses"), "{m}");
}

#[test]
fn block_function_broadcasts_arguments() {
    let mut env = env_with(&["x := 1"]);
    env.define_fn("t", vec!["a".into(), "b".into()], solve_expr(SolveKind::Find, &["x"], &["x^2 == a·b"]));
    let v = vector_in(&mut env, "t([1; 4], 4) =");
    assert!(sig3(v[0], 2.0) && sig3(v[1], 4.0), "{v:?}");
    let v = vector_in(&mut env, "t(4, [1; 9]) =");
    assert!(sig3(v[0], 2.0) && sig3(v[1], 6.0), "{v:?}");
    assert!(matches!(run(&mut env, "t([1; 4], [1; 2; 3]) ="), Err(CalcError::Shape { .. })));
}

#[test]
fn block_function_with_several_unknowns_takes_no_vector() {
    let mut env = env_with(&["x := 1", "y := 1"]);
    env.define_fn("h", vec!["a".into()], solve_expr(SolveKind::Find, &["x", "y"], &["x + y == a", "x - y == 1"]));
    let v = vector_in(&mut env, "h(3) =");
    assert!(sig3(v[0], 2.0) && sig3(v[1], 1.0), "{v:?}");
    let m = syntax_msg(run(&mut env, "h([3; 5]) =").expect_err("a vector"));
    assert!(m.contains("several unknowns"), "{m}");
}

#[test]
fn a_solve_has_at_most_fifty_unknowns() {
    // Refused before the guesses are even looked up.
    let names: Vec<String> = (0..51).map(|k| format!("u_{k}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let m = syntax_msg(find(&Env::new(), &refs, &["u_0 == 1"]).expect_err("too many"));
    assert_eq!(m, "a solve can have at most 50 unknowns");
    // Fifty are allowed (here the equation fixes only the first).
    let defs: Vec<String> = names.iter().take(50).map(|n| format!("{n} := 1")).collect();
    let env = env_with(&defs.iter().map(String::as_str).collect::<Vec<_>>());
    let v = find(&env, &refs[..50], &["u_0 == 3"]).expect("solves");
    assert!(sig3(values_of(v)[0], 3.0));
}

#[test]
fn a_solves_linear_algebra_is_work() {
    // Cheap equations, many unknowns: each step's matrix work is n · (n + m) = 40 · 41.
    let names: Vec<String> = (0..40).map(|k| format!("u_{k}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let defs: Vec<String> = names.iter().map(|n| format!("{n} := 1")).collect();
    let mut env = env_with(&defs.iter().map(String::as_str).collect::<Vec<_>>());
    let stmt = Stmt::Define { name: "r".into(), index: None, params: vec![], body: solve_expr(SolveKind::Find, &refs, &["u_0 == 3"]), show: None };
    assert_eq!(run_stmt_in(&mut env, &stmt, &Pass::new(1_000)), Err(CalcError::TooMuchWork));
    let pass = Pass::new(u64::MAX);
    run_stmt_in(&mut env, &stmt, &pass).expect("solves with room");
    assert!(pass.spent() >= 40 * 41, "{}", pass.spent());
}

#[test]
fn a_block_function_may_take_a_vector_whole() {
    let mut env = env_with(&["x := 1"]);
    env.define_fn("f", vec!["c".into()], solve_expr(SolveKind::Find, &["x"], &["x == sum(c)"]));
    run(&mut env, "c := [1; 2; 3]").expect("defines");
    // One solve with the vector whole, not a column of per-element solutions.
    assert!(sig3(shown(&mut env, "f(c) =").value, 6.0));
    env.define_fn("f_i", vec!["c".into()], solve_expr(SolveKind::Find, &["x"], &["x == c[1]"]));
    assert!(sig3(shown(&mut env, "f_i(c) =").value, 2.0));
}

#[test]
fn root_over_a_vector_is_per_element() {
    let mut env = env_with(&["x := 1"]);
    run(&mut env, "g2(a) := root(x^2 − a, x)").expect("defines");
    let v = vector_in(&mut env, "g2([9; 16]) =");
    assert!(sig3(v[0], 3.0) && sig3(v[1], 4.0), "{v:?}");
    let m = err_in(&mut env, "g2([9; −1]) =");
    assert!(m.starts_with("element 1: no solution found near the guesses"), "{m}");
}

#[test]
fn an_unknown_that_is_a_parameter_starts_from_the_argument() {
    // No global x: the argument is the guess, as in Mathcad, so the sign picks the root.
    let mut env = Env::new();
    run(&mut env, "q_r(x) := root(x^2 − 2, x)").expect("defines");
    assert!(sig3(shown(&mut env, "q_r(1) =").value, 2f64.sqrt()));
    assert!(sig3(shown(&mut env, "q_r(−1) =").value, -(2f64.sqrt())));
    // A vector of guesses is solved per element.
    let v = vector_in(&mut env, "q_r([1; −1]) =");
    assert!(sig3(v[0], 2f64.sqrt()) && sig3(v[1], -(2f64.sqrt())), "{v:?}");
}

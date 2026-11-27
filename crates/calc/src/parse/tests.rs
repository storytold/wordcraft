use super::parse;
use crate::{CalcError, Cmp, Expr, Logic, Op, Show, Stmt, defined_name, free_names};
use proptest::prelude::*;

fn num(v: f64) -> Expr {
    Expr::Num(v)
}
fn name(n: &str) -> Expr {
    Expr::Name(n.into())
}
fn bin(op: Op, a: Expr, b: Expr) -> Expr {
    Expr::Binary(op, Box::new(a), Box::new(b))
}
fn neg(a: Expr) -> Expr {
    Expr::Unary(Op::Neg, Box::new(a))
}
fn call(n: &str, args: Vec<Expr>) -> Expr {
    Expr::Call(n.into(), args)
}
fn matrix(rows: usize, cols: usize, items: Vec<Expr>) -> Expr {
    Expr::Matrix { rows, cols, items }
}
fn index(n: &str, at: Vec<Expr>) -> Expr {
    Expr::Index(n.into(), at)
}
fn range(first: Expr, second: Option<Expr>, last: Expr) -> Expr {
    Expr::Range { first: Box::new(first), second: second.map(Box::new), last: Box::new(last) }
}
fn show(unit: Option<&str>) -> Show {
    Show { unit: unit.map(str::to_string) }
}
/// The expression of an `Eval` (the statement must end in `=`).
fn expr_of(src: &str) -> Expr {
    match parse(&format!("{src} =")) {
        Ok(Stmt::Eval { expr, .. }) => expr,
        other => panic!("{src:?} did not parse as an Eval: {other:?}"),
    }
}
/// The body of a definition with no `=`.
fn body_of(src: &str) -> Expr {
    match parse(src) {
        Ok(Stmt::Define { body, show: None, .. }) => body,
        other => panic!("{src:?} did not parse as a definition: {other:?}"),
    }
}
/// The message of a syntax error.
fn syntax_msg(src: &str) -> String {
    match parse(src) {
        Err(CalcError::Syntax(m)) => m,
        other => panic!("{src:?} should be a syntax error, got {other:?}"),
    }
}
fn syntax(src: &str) {
    assert!(matches!(parse(src), Err(CalcError::Syntax(_))), "{src:?} should be a syntax error, got {:?}", parse(src));
}

#[test]
fn define_with_units() {
    assert_eq!(
        parse("P := 1500 lbf"),
        Ok(Stmt::Define { name: "P".into(), index: None, params: vec![], body: bin(Op::Mul, num(1500.0), name("lbf")), show: None })
    );
}

#[test]
fn define_and_show() {
    assert_eq!(
        parse("σ := P/A_b ="),
        Ok(Stmt::Define { name: "σ".into(), index: None, params: vec![], body: bin(Op::Div, name("P"), name("A_b")), show: Some(show(None)) })
    );
}

#[test]
fn define_and_show_with_unit() {
    let want =
        Stmt::Define { name: "σ".into(), index: None, params: vec![], body: bin(Op::Div, name("P"), name("A_b")), show: Some(show(Some("ksi"))) };
    assert_eq!(parse("σ := P/A_b = ? ksi"), Ok(want.clone()));
    assert_eq!(parse("σ ≔ P/A_b = ksi"), Ok(want));
}

#[test]
fn eval_with_unit() {
    let want = Ok(Stmt::Eval { expr: name("σ"), show: show(Some("ksi")) });
    assert_eq!(parse("σ = ? ksi"), want);
    assert_eq!(parse("σ = ksi"), want);
    assert_eq!(parse("σ ="), Ok(Stmt::Eval { expr: name("σ"), show: show(None) }));
    assert_eq!(parse("σ = ?"), Ok(Stmt::Eval { expr: name("σ"), show: show(None) }));
    assert_eq!(parse("σ =   "), Ok(Stmt::Eval { expr: name("σ"), show: show(None) }));
}

#[test]
fn show_unit_is_verbatim_and_unparsed() {
    for unit in ["lbf/in^2", "kN·m", "°F", "Δ°F", "µm", "m/s^2", "% weird (unit)"] {
        assert_eq!(parse(&format!("x = ? {unit}")), Ok(Stmt::Eval { expr: name("x"), show: show(Some(unit)) }));
    }
}

#[test]
fn function_def() {
    assert_eq!(
        parse("f(x, y) := x^2 + y"),
        Ok(Stmt::Define {
            name: "f".into(),
            index: None,
            params: vec!["x".into(), "y".into()],
            body: bin(Op::Add, bin(Op::Pow, name("x"), num(2.0)), name("y")),
            show: None
        })
    );
}

#[test]
fn subscripted_greek() {
    assert_eq!(
        parse("σ_allow := 20 ksi"),
        Ok(Stmt::Define { name: "σ_allow".into(), index: None, params: vec![], body: bin(Op::Mul, num(20.0), name("ksi")), show: None })
    );
    assert_eq!(expr_of("P_1 + A_b"), bin(Op::Add, name("P_1"), name("A_b")));
}

#[test]
fn precedence() {
    assert_eq!(expr_of("a + b*c^2"), bin(Op::Add, name("a"), bin(Op::Mul, name("b"), bin(Op::Pow, name("c"), num(2.0)))));
    assert_eq!(expr_of("a^b^c"), bin(Op::Pow, name("a"), bin(Op::Pow, name("b"), name("c"))));
    assert_eq!(expr_of("a - b - c"), bin(Op::Sub, bin(Op::Sub, name("a"), name("b")), name("c")));
    assert_eq!(expr_of("a / b / c"), bin(Op::Div, bin(Op::Div, name("a"), name("b")), name("c")));
    assert_eq!(expr_of("(a + b) * c"), bin(Op::Mul, bin(Op::Add, name("a"), name("b")), name("c")));
}

#[test]
fn unary_minus_binds_looser_than_pow() {
    assert_eq!(expr_of("-x^2"), neg(bin(Op::Pow, name("x"), num(2.0))));
    assert_eq!(expr_of("2^-x"), bin(Op::Pow, num(2.0), neg(name("x"))));
    assert_eq!(expr_of("-a*b"), bin(Op::Mul, neg(name("a")), name("b")));
    assert_eq!(expr_of("a - -b"), bin(Op::Sub, name("a"), neg(name("b"))));
    assert_eq!(expr_of("+a"), name("a"));
    assert_eq!(expr_of("a \u{2212} b"), bin(Op::Sub, name("a"), name("b")));
}

#[test]
fn free_names_excludes_params() {
    let s = parse("f(x) := x*k + g(x)").unwrap();
    assert_eq!(free_names(&s), vec!["k".to_string(), "g".to_string()]);
    assert_eq!(defined_name(&s), Some("f"));
}

#[test]
fn free_names_in_first_use_order_without_duplicates() {
    let s = parse("c := b*a + |b| + sqrt(d) + a").unwrap();
    assert_eq!(free_names(&s), vec!["b", "a", "sqrt", "d"]);
    let s = parse("x + y*x =").unwrap();
    assert_eq!(free_names(&s), vec!["x", "y"]);
    assert_eq!(defined_name(&s), None);
    assert!(free_names(&parse("2 + 3 =").unwrap()).is_empty());
}

#[test]
fn hostile_sources_error_not_panic() {
    assert_eq!(parse(&"(".repeat(200)), Err(CalcError::TooComplex));
    assert_eq!(parse(&format!("x:={}", "1+".repeat(60_000))), Err(CalcError::TooComplex));
    assert!(matches!(parse(":= ="), Err(CalcError::Syntax(_))));
    assert!(matches!(parse("\u{0}"), Err(CalcError::Syntax(_))));
}

#[test]
fn source_size_limit_is_16_kib() {
    let at_limit = format!("x := 1{}", " ".repeat(16 * 1024 - 6));
    assert_eq!(at_limit.len(), 16 * 1024);
    assert!(parse(&at_limit).is_ok());
    assert_eq!(parse(&format!("{at_limit} ")), Err(CalcError::TooComplex));
}

#[test]
fn nesting_depth_limit() {
    // Nesting (parentheses, calls, bars, prefix signs, power operands) is accepted to 64 levels
    // and refused at 65.
    let ok = |src: String| assert!(parse(&src).is_ok(), "{} should parse", &src[..src.len().min(40)]);
    let too_complex = |src: String| assert_eq!(parse(&src), Err(CalcError::TooComplex), "{}", &src[..src.len().min(40)]);

    let parens = |n: usize| format!("{}x{} =", "(".repeat(n), ")".repeat(n));
    ok(parens(64));
    too_complex(parens(65));

    let negations = |k: usize| format!("x := {}1", "-".repeat(k));
    ok(negations(64));
    too_complex(negations(65));

    let powers = |k: usize| format!("x := {}1", "2^".repeat(k));
    ok(powers(64));
    too_complex(powers(65));

    let abs = |k: usize| format!("x := {}1{}", "|".repeat(k), "|".repeat(k));
    ok(abs(64));
    too_complex(abs(65));

    let calls = |k: usize| format!("x := {}1{}", "f(".repeat(k), ")".repeat(k));
    ok(calls(64));
    too_complex(calls(65));

    // Far beyond any limit.
    too_complex(parens(5_000));
}

#[test]
fn tree_height_limit_allows_long_flat_expressions() {
    // A flat chain builds a left-leaning tree: k operators are k + 1 levels tall, up to 1024.
    let sum = |k: usize| format!("x := {}1", "1+".repeat(k));
    assert!(parse(&sum(500)).is_ok());
    assert!(parse(&sum(1023)).is_ok());
    assert!(parse(&sum(1024)) == Err(CalcError::TooComplex));
    assert!(parse(&sum(1100)) == Err(CalcError::TooComplex));
    assert!(parse(&sum(5_000)) == Err(CalcError::TooComplex));
    let product = format!("x := {}a", "a ".repeat(500));
    assert!(parse(&product).is_ok());
    let product = format!("x := {}a", "a ".repeat(1100));
    assert!(parse(&product) == Err(CalcError::TooComplex));
    // The height of a nested group counts towards the same limit.
    assert!(parse(&format!("x := (1+1)+{}1", "1+".repeat(1022))) == Err(CalcError::TooComplex));
    assert!(parse(&format!("x := (1+1)+{}1", "1+".repeat(1021))).is_ok());
}

#[test]
fn adjacent_numbers_are_an_error() {
    for src in ["1 500 lbf", "1 000", "x 2", "(a) 2", "|a| 2", "2 3", "x² 2", "f(x) 2", "1.5 2.5"] {
        match parse(&format!("{src} =")) {
            Err(CalcError::Syntax(m)) => assert_eq!(m, "two values in a row \u{2014} did you mean `*`?", "{src}"),
            other => panic!("{src:?} gave {other:?}"),
        }
    }
    // A number before a name or a bracket is still implicit multiplication.
    assert_eq!(expr_of("2 P"), bin(Op::Mul, num(2.0), name("P")));
    assert_eq!(expr_of("5 in"), bin(Op::Mul, num(5.0), name("in")));
    assert_eq!(expr_of("P A"), bin(Op::Mul, name("P"), name("A")));
    assert_eq!(expr_of("2 (a+b)"), bin(Op::Mul, num(2.0), bin(Op::Add, name("a"), name("b"))));
    assert_eq!(expr_of("a (2)"), bin(Op::Mul, name("a"), num(2.0)));
    assert_eq!(expr_of("2 √x"), bin(Op::Mul, num(2.0), call("sqrt", vec![name("x")])));
}

#[test]
fn superscript_digits_are_postfix_powers() {
    assert_eq!(expr_of("x²"), expr_of("x^2"));
    assert_eq!(expr_of("x²"), bin(Op::Pow, name("x"), num(2.0)));
    assert_eq!(expr_of("in³"), bin(Op::Pow, name("in"), num(3.0)));
    assert_eq!(expr_of("s⁻¹"), expr_of("s^-1"));
    assert_eq!(expr_of("s⁻¹"), bin(Op::Pow, name("s"), neg(num(1.0))));
    assert_eq!(expr_of("(a+b)²"), bin(Op::Pow, bin(Op::Add, name("a"), name("b")), num(2.0)));
    assert_eq!(expr_of("|a|²"), bin(Op::Pow, Expr::Abs(Box::new(name("a"))), num(2.0)));
    assert_eq!(expr_of("5²"), bin(Op::Pow, num(5.0), num(2.0)));
    assert_eq!(expr_of("f(x)²"), bin(Op::Pow, call("f", vec![name("x")]), num(2.0)));
    assert_eq!(expr_of("√x²"), bin(Op::Pow, call("sqrt", vec![name("x")]), num(2.0)));
    for (sup, digit) in [("⁰", 0.0), ("¹", 1.0), ("²", 2.0), ("³", 3.0), ("⁴", 4.0), ("⁵", 5.0), ("⁶", 6.0), ("⁷", 7.0), ("⁸", 8.0), ("⁹", 9.0)]
    {
        assert_eq!(expr_of(&format!("x{sup}")), bin(Op::Pow, name("x"), num(digit)), "{sup}");
    }
    // It binds like `^`: tighter than multiplication and than a prefix minus.
    assert_eq!(expr_of("2 m²"), bin(Op::Mul, num(2.0), bin(Op::Pow, name("m"), num(2.0))));
    assert_eq!(expr_of("kN/m²"), bin(Op::Div, name("kN"), bin(Op::Pow, name("m"), num(2.0))));
    assert_eq!(expr_of("-x²"), neg(bin(Op::Pow, name("x"), num(2.0))));
    assert_eq!(expr_of("lbf/in²"), expr_of("lbf/in^2"));
    // A subscript digit is still part of a name.
    assert_eq!(expr_of("x₂"), name("x₂"));
}

#[test]
fn superscript_errors() {
    for src in ["x¹²", "x⁻¹²", "x⁻", "x ²", "²", "x² ²", "a + ²", "x⁻⁻1"] {
        let r = parse(&format!("{src} ="));
        assert!(matches!(r, Err(CalcError::Syntax(_))), "{src:?} gave {r:?}");
    }
    match parse("x¹² =") {
        Err(CalcError::Syntax(m)) => assert!(m.contains("single digit"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn en_dash_is_minus() {
    assert_eq!(expr_of("a \u{2013} b"), bin(Op::Sub, name("a"), name("b")));
    assert_eq!(expr_of("\u{2013}a"), neg(name("a")));
}

#[test]
fn implicit_multiplication_has_the_precedence_of_multiplication() {
    // Adjacency groups like `*`, left to right (the Mathcad convention), so `1/2 x` is `(1/2) x`,
    // not `1/(2 x)`.
    assert_eq!(expr_of("1/2 x"), bin(Op::Mul, bin(Op::Div, num(1.0), num(2.0)), name("x")));
    assert_eq!(expr_of("1/2 x"), expr_of("1/2*x"));
    assert_eq!(expr_of("a/b c"), bin(Op::Mul, bin(Op::Div, name("a"), name("b")), name("c")));
}

#[test]
fn token_count_is_capped() {
    // Under 16 kB of source, but more than 10 000 tokens.
    let src = format!("x := {}1", "-".repeat(12_000));
    assert!(src.len() < 16 * 1024);
    assert_eq!(parse(&src), Err(CalcError::TooComplex));
    assert_eq!(parse(&",".repeat(12_000)), Err(CalcError::TooComplex));
}

#[test]
fn numbers() {
    assert_eq!(expr_of("1.5e3"), num(1500.0));
    assert_eq!(expr_of("2E-4"), num(2e-4));
    assert_eq!(expr_of("1e+2"), num(100.0));
    assert_eq!(expr_of(".5"), num(0.5));
    assert_eq!(expr_of("7."), num(7.0));
    assert_eq!(expr_of("2.75"), num(2.75));
    syntax("1e999 =");
    syntax("1.5.3 =");
    syntax(". =");
}

#[test]
fn number_then_e_is_a_name_not_an_exponent() {
    assert_eq!(expr_of("2eV"), bin(Op::Mul, num(2.0), name("eV")));
    assert_eq!(expr_of("2e"), bin(Op::Mul, num(2.0), name("e")));
    assert_eq!(expr_of("2e3 m"), bin(Op::Mul, num(2000.0), name("m")));
}

#[test]
fn implicit_multiplication() {
    assert_eq!(expr_of("2 P"), bin(Op::Mul, num(2.0), name("P")));
    assert_eq!(expr_of("2P"), bin(Op::Mul, num(2.0), name("P")));
    assert_eq!(expr_of("P A"), bin(Op::Mul, name("P"), name("A")));
    assert_eq!(expr_of("2 (a + b)"), bin(Op::Mul, num(2.0), bin(Op::Add, name("a"), name("b"))));
    assert_eq!(expr_of("(a)(b)"), bin(Op::Mul, name("a"), name("b")));
    assert_eq!(expr_of("a b c"), bin(Op::Mul, bin(Op::Mul, name("a"), name("b")), name("c")));
    assert_eq!(expr_of("2x^2"), bin(Op::Mul, num(2.0), bin(Op::Pow, name("x"), num(2.0))));
    assert_eq!(expr_of("x^2 y"), bin(Op::Mul, bin(Op::Pow, name("x"), num(2.0)), name("y")));
    assert_eq!(expr_of("a + 2 b"), bin(Op::Add, name("a"), bin(Op::Mul, num(2.0), name("b"))));
    // A minus after a factor is subtraction, not a negative factor.
    assert_eq!(expr_of("a -b"), bin(Op::Sub, name("a"), name("b")));
}

#[test]
fn multiplication_and_division_symbols() {
    let want = bin(Op::Mul, name("a"), name("b"));
    for sym in ["*", "·", "×", "\u{22C5}"] {
        assert_eq!(expr_of(&format!("a {sym} b")), want, "{sym}");
    }
    assert_eq!(expr_of("a ÷ b"), bin(Op::Div, name("a"), name("b")));
    assert_eq!(expr_of("5 kN·m"), bin(Op::Mul, bin(Op::Mul, num(5.0), name("kN")), name("m")));
    assert_eq!(expr_of("10 lbf/in^2"), bin(Op::Div, bin(Op::Mul, num(10.0), name("lbf")), bin(Op::Pow, name("in"), num(2.0))));
}

#[test]
fn call_needs_the_parenthesis_right_after_the_name() {
    assert_eq!(expr_of("f(x)"), call("f", vec![name("x")]));
    assert_eq!(expr_of("f (x)"), bin(Op::Mul, name("f"), name("x")));
    assert_eq!(expr_of("max(a, 2 b, c)"), call("max", vec![name("a"), bin(Op::Mul, num(2.0), name("b")), name("c")]));
    assert_eq!(expr_of("f()"), call("f", vec![]));
    assert_eq!(expr_of("sin(x)^2"), bin(Op::Pow, call("sin", vec![name("x")]), num(2.0)));
    assert_eq!(expr_of("2 f(x)"), bin(Op::Mul, num(2.0), call("f", vec![name("x")])));
    assert_eq!(expr_of("f(g(x), 1)"), call("f", vec![call("g", vec![name("x")]), num(1.0)]));
    syntax("f(a b c, ) =");
    syntax("f(a,, b) =");
    syntax("f(a =");
}

#[test]
fn square_root_symbol() {
    assert_eq!(expr_of("√x"), call("sqrt", vec![name("x")]));
    assert_eq!(expr_of("√(a + b)"), call("sqrt", vec![bin(Op::Add, name("a"), name("b"))]));
    assert_eq!(expr_of("√x^2"), bin(Op::Pow, call("sqrt", vec![name("x")]), num(2.0)));
    assert_eq!(expr_of("2√x"), bin(Op::Mul, num(2.0), call("sqrt", vec![name("x")])));
    assert_eq!(expr_of("√√x"), call("sqrt", vec![call("sqrt", vec![name("x")])]));
    syntax("√ =");
}

#[test]
fn absolute_value() {
    assert_eq!(expr_of("|x|"), Expr::Abs(Box::new(name("x"))));
    assert_eq!(expr_of("|a - b|"), Expr::Abs(Box::new(bin(Op::Sub, name("a"), name("b")))));
    assert_eq!(expr_of("2|x|"), bin(Op::Mul, num(2.0), Expr::Abs(Box::new(name("x")))));
    assert_eq!(expr_of("|a| |b|"), bin(Op::Mul, Expr::Abs(Box::new(name("a"))), Expr::Abs(Box::new(name("b")))));
    assert_eq!(expr_of("|a| + |b|"), bin(Op::Add, Expr::Abs(Box::new(name("a"))), Expr::Abs(Box::new(name("b")))));
    assert_eq!(expr_of("|(|x|)|"), Expr::Abs(Box::new(Expr::Abs(Box::new(name("x"))))));
    assert_eq!(expr_of("|f(|x|, 1)|"), Expr::Abs(Box::new(call("f", vec![Expr::Abs(Box::new(name("x"))), num(1.0)]))));
    syntax("|x =");
    syntax("x| =");
    syntax("|| =");
}

#[test]
fn unit_names_lex_as_names() {
    assert_eq!(expr_of("20 °F"), bin(Op::Mul, num(20.0), name("°F")));
    assert_eq!(expr_of("5 Δ°F"), bin(Op::Mul, num(5.0), name("Δ°F")));
    assert_eq!(expr_of("5 ∆°C"), bin(Op::Mul, num(5.0), name("∆°C")));
    assert_eq!(expr_of("3 µm"), bin(Op::Mul, num(3.0), name("µm")));
    assert_eq!(expr_of("3 μm"), bin(Op::Mul, num(3.0), name("μm")));
    assert_eq!(expr_of("2 Ω"), bin(Op::Mul, num(2.0), name("Ω")));
    assert_eq!(expr_of("α_1 + x2"), bin(Op::Add, name("α_1"), name("x2")));
}

#[test]
fn percent_lexes_as_a_unit_name() {
    assert_eq!(expr_of("5 %"), bin(Op::Mul, num(5.0), name("%")));
    assert_eq!(expr_of("5 % · 200"), bin(Op::Mul, bin(Op::Mul, num(5.0), name("%")), num(200.0)));
    assert_eq!(body_of("x := 0.25 %"), bin(Op::Mul, num(0.25), name("%")));
}

#[test]
fn definition_heads() {
    assert_eq!(defined_name(&parse("A_b := 2 in^2").unwrap()), Some("A_b"));
    assert_eq!(defined_name(&parse("g(a) := a").unwrap()), Some("g"));
    // Whitespace and the ≔ spelling are fine; a define with show keeps both.
    let s = parse("  g ( a )≔a  =  ").unwrap();
    assert_eq!(s, Stmt::Define { name: "g".into(), index: None, params: vec!["a".into()], body: name("a"), show: Some(show(None)) });
    syntax("f() := 1");
    syntax("f(1) := 1");
    syntax("f(x, x) := 1");
    syntax("f(x y) := 1");
    syntax("f(x := 1");
    syntax("f(x) + 1 := 2");
    syntax("1 := 2");
    syntax("a b := 2");
    syntax("x := ");
    syntax(":= 3");
    syntax("x := y := 3");
    syntax("x := 3 + =");
}

#[test]
fn malformed_statements_are_syntax_errors() {
    for src in
        ["", "   ", "=", "a +", "a + + =", "* a =", "a b", "a + b", "(a + b =", "a + b) =", "a $ b =", "a : b =", "a ) b =", "()", "( ) =", "a := ("]
    {
        let r = parse(src);
        assert!(matches!(r, Err(CalcError::Syntax(_))), "{src:?} gave {r:?}");
    }
}

#[test]
fn syntax_messages_are_readable() {
    let msg = |src: &str| match parse(src) {
        Err(CalcError::Syntax(m)) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(msg("a $ b ="), "unexpected character `$`");
    assert!(msg("\u{0}").contains("unexpected character"));
    assert!(!msg("a + b").is_empty());
}

#[test]
fn quantity_variant_is_never_produced() {
    fn has_quantity(e: &Expr) -> bool {
        match e {
            Expr::Quantity(..) => true,
            Expr::Num(_) | Expr::Text(_) | Expr::Name(_) => false,
            Expr::Unary(_, a) | Expr::Abs(a) | Expr::Not(a) => has_quantity(a),
            Expr::Binary(_, a, b) | Expr::Logic(_, a, b) => has_quantity(a) || has_quantity(b),
            Expr::Compare { first, rest } => has_quantity(first) || rest.iter().any(|(_, e)| has_quantity(e)),
            Expr::Call(_, args) | Expr::Index(_, args) => args.iter().any(has_quantity),
            Expr::Matrix { items, .. } => items.iter().any(has_quantity),
            Expr::Range { first, second, last } => has_quantity(first) || second.as_deref().is_some_and(has_quantity) || has_quantity(last),
            Expr::Cases { cases, otherwise } => {
                cases.iter().any(|(v, c)| has_quantity(v) || has_quantity(c)) || otherwise.as_deref().is_some_and(has_quantity)
            }
            Expr::Sum { first, second, last, body, .. } => {
                has_quantity(first) || second.as_deref().is_some_and(has_quantity) || has_quantity(last) || has_quantity(body)
            }
            Expr::Solve { equations, .. } => equations.iter().any(|(lhs, rhs)| has_quantity(lhs) || has_quantity(rhs)),
        }
    }
    assert!(!has_quantity(&expr_of("5 kN·m + 2 ksi")));
    assert!(!has_quantity(&body_of("y := 5 kN if a > 2 in, 3 m otherwise")));
}

#[test]
fn literals_parse() {
    // A one-dimensional list is always a column.
    assert_eq!(expr_of("[1, 2, 3]"), matrix(3, 1, vec![num(1.0), num(2.0), num(3.0)]));
    assert_eq!(expr_of("[1, 2; 3, 4]"), matrix(2, 2, vec![num(1.0), num(2.0), num(3.0), num(4.0)]));
    assert_eq!(expr_of("[1; 2]"), matrix(2, 1, vec![num(1.0), num(2.0)]));
    assert_eq!(expr_of("[7]"), matrix(1, 1, vec![num(7.0)]));
    assert_eq!(expr_of("[2 in, 50 mm]"), matrix(2, 1, vec![bin(Op::Mul, num(2.0), name("in")), bin(Op::Mul, num(50.0), name("mm"))]));
    assert_eq!(expr_of("[0.840, 1.050] in"), bin(Op::Mul, matrix(2, 1, vec![num(0.84), num(1.05)]), name("in")));
    assert_eq!(body_of("D := [0.840, 1.050] in"), expr_of("[0.840, 1.050] in"));
    // Elements are whole expressions, and `|…|` and calls work inside them.
    assert_eq!(
        expr_of("[a + 1, |b|, f(c, d)]"),
        matrix(3, 1, vec![bin(Op::Add, name("a"), num(1.0)), Expr::Abs(Box::new(name("b"))), call("f", vec![name("c"), name("d")])])
    );
    // A literal after a value is a product, like a parenthesis.
    assert_eq!(expr_of("2 [1, 2]"), bin(Op::Mul, num(2.0), matrix(2, 1, vec![num(1.0), num(2.0)])));
    assert_eq!(expr_of("[1] [2]"), bin(Op::Mul, matrix(1, 1, vec![num(1.0)]), matrix(1, 1, vec![num(2.0)])));
    syntax("[1] 2 =");
}

#[test]
fn ragged_rows_are_an_error() {
    assert_eq!(syntax_msg("[1, 2; 3] ="), "row 2 has 1 element; row 1 has 2");
    assert_eq!(syntax_msg("x := [1; 2, 3]"), "row 2 has 2 elements; row 1 has 1");
}

#[test]
fn empty_and_nested_literals_are_errors() {
    for src in ["[] =", "[[1], 2] =", "[1, [2]] =", "[([1]), 2] =", "[1, 2 =", "[1,, 2] =", "[1, 2;] =", "[; 1] =", "1, 2] ="] {
        syntax(src);
    }
}

#[test]
fn indexing_needs_no_space() {
    assert_eq!(expr_of("D[2]"), index("D", vec![num(2.0)]));
    assert_eq!(expr_of("M[i, j]"), index("M", vec![name("i"), name("j")]));
    assert_eq!(expr_of("D [2]"), bin(Op::Mul, name("D"), matrix(1, 1, vec![num(2.0)])));
    assert_eq!(syntax_msg("f(x)[2] ="), "only a name can be indexed, like D[2]");
    for src in ["(a)[2] =", "D[1][2] =", "[1][0] =", "f(x)[2] [3] ="] {
        assert_eq!(syntax_msg(src), "only a name can be indexed, like D[2]", "{src}");
    }
    // After anything else, a touching `[` starts a literal factor, like `2(a+b)`.
    assert_eq!(expr_of("2[1, 2]"), bin(Op::Mul, num(2.0), matrix(2, 1, vec![num(1.0), num(2.0)])));
    assert_eq!(expr_of("x²[1]"), bin(Op::Mul, bin(Op::Pow, name("x"), num(2.0)), matrix(1, 1, vec![num(1.0)])));
    assert_eq!(expr_of("|a|[0]"), bin(Op::Mul, Expr::Abs(Box::new(name("a"))), matrix(1, 1, vec![num(0.0)])));
    // An index is a whole expression; it binds like a call, tighter than `^`.
    assert_eq!(expr_of("D_o[i + 1]^2"), bin(Op::Pow, index("D_o", vec![bin(Op::Add, name("i"), num(1.0))]), num(2.0)));
    assert_eq!(expr_of("2 D[0]"), bin(Op::Mul, num(2.0), index("D", vec![num(0.0)])));
    assert_eq!(expr_of("[D[0], D[1]]"), matrix(2, 1, vec![index("D", vec![num(0.0)]), index("D", vec![num(1.0)])]));
    for src in ["D[] =", "M[1, 2, 3] =", "D[1 =", "D[1) ="] {
        syntax(src);
    }
}

#[test]
fn range_dots_are_not_decimal_points() {
    assert_eq!(body_of("i := 0..5"), range(num(0.0), None, num(5.0)));
    assert_eq!(body_of("x := 1.5..3"), range(num(1.5), None, num(3.0)));
    assert_eq!(body_of("x := 1, 1.5..3"), range(num(1.0), Some(num(1.5)), num(3.0)));
    assert_eq!(body_of("y := 1.5"), num(1.5));
    assert_eq!(body_of("x := .5..2."), range(num(0.5), None, num(2.0)));
    assert_eq!(body_of("x := 1e2..2e2"), range(num(100.0), None, num(200.0)));
    // The ends are signed values, with units and powers; a range may be shown.
    assert_eq!(body_of("x := -5 .. -1"), range(neg(num(5.0)), None, neg(num(1.0))));
    assert_eq!(
        body_of("x := 0 in, 0.5 in..1 in"),
        range(bin(Op::Mul, num(0.0), name("in")), Some(bin(Op::Mul, num(0.5), name("in"))), bin(Op::Mul, num(1.0), name("in")))
    );
    assert_eq!(body_of("x := 0..2^n"), range(num(0.0), None, bin(Op::Pow, num(2.0), name("n"))));
    // `..` is the loosest operator: each value is a whole expression.
    assert_eq!(body_of("i := 0..n-1"), range(num(0.0), None, bin(Op::Sub, name("n"), num(1.0))));
    assert_eq!(body_of("i := 0..length(D)-1"), range(num(0.0), None, bin(Op::Sub, call("length", vec![name("D")]), num(1.0))));
    assert_eq!(body_of("x := 1, 1+0.5..n"), range(num(1.0), Some(bin(Op::Add, num(1.0), num(0.5))), name("n")));
    assert_eq!(body_of("x := 2·0..5"), range(bin(Op::Mul, num(2.0), num(0.0)), None, num(5.0)));
    assert_eq!(body_of("x := -1 + a..b/2"), range(bin(Op::Add, neg(num(1.0)), name("a")), None, bin(Op::Div, name("b"), num(2.0))));
    assert_eq!(body_of("x := 0..(n - 1)"), range(num(0.0), None, bin(Op::Sub, name("n"), num(1.0))));
    assert!(matches!(parse("i := 0..5 ="), Ok(Stmt::Define { show: Some(_), .. })));
    syntax("x := 1.5.3");
    syntax("x := 0...5");
    assert_eq!(syntax_msg("x := 0.."), "the expression ends too soon");
    assert_eq!(syntax_msg("x := 0...5"), "`...` isn't valid; a range is written like 0..5");
    syntax("x := ..5");
    assert_eq!(syntax_msg("x := 1, 2"), "a list needs brackets, like [1, 2]; a range with a step is written like 1, 1.5..3");
    syntax("x := 1, 2, 3..4");
    syntax("x := 0..1..2");
    syntax("x := 1 2..3");
}

#[test]
fn ranges_are_whole_right_hand_sides() {
    for src in [
        "x := (0..5)",
        "0..5 =",
        "f(0..5) =",
        "x := [0..5]",
        "x := D[0..1]",
        "x := |0..1|",
        "x := 0..1..2",
        "x := 2·(0..5)",
        "x := 1, (2..3)",
        "x := 0..f(1..2)",
    ] {
        assert_eq!(syntax_msg(src), "a range like 0..5 can only be the whole right side of a definition", "{src}");
    }
}

#[test]
fn indexed_definition_head() {
    assert_eq!(
        parse("t[i] := 2·D[i]"),
        Ok(Stmt::Define {
            name: "t".into(),
            index: Some("i".into()),
            params: vec![],
            body: bin(Op::Mul, num(2.0), index("D", vec![name("i")])),
            show: None
        })
    );
    assert!(matches!(parse("t_o[k] ≔ k ="), Ok(Stmt::Define { index: Some(k), show: Some(_), .. }) if k == "k"));
    for src in ["t[i](x) := 1", "t(x)[i] := 1", "t[0] := 1", "t[i, j] := 1", "t [i] := 1", "t[] := 1", "t[i := 1", "t[i]] := 1"] {
        syntax(src);
    }
}

#[test]
fn free_names_see_new_nodes() {
    let names = free_names(&parse("t[i] := D_o[i]·k + [a, b]").unwrap());
    let set: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(set, ["D_o", "i", "k", "a", "b"].into_iter().collect());
    assert_eq!(names.len(), set.len(), "duplicates in {names:?}");
    assert_eq!(free_names(&parse("x := a, b..c").unwrap()), ["a", "b", "c"]);
    assert_eq!(free_names(&parse("M[r, c] =").unwrap()), ["M", "r", "c"]);
}

#[test]
fn literal_and_index_limits() {
    // Literals and indexes nest like parentheses: 64 levels deep, one more is too complex.
    let ok = |src: String| assert!(parse(&src).is_ok(), "{} should parse", &src[..src.len().min(40)]);
    let too_complex = |src: String| assert_eq!(parse(&src), Err(CalcError::TooComplex), "{}", &src[..src.len().min(40)]);
    let indexes = |k: usize| format!("x := {}1{}", "D[".repeat(k), "]".repeat(k));
    ok(indexes(64));
    too_complex(indexes(65));
    let literals = |k: usize| format!("x := {}1{}", "[f(".repeat(k / 2), ")]".repeat(k / 2));
    ok(literals(64));
    too_complex(literals(66));
    // Their height counts towards the same limit, one level for the literal itself.
    assert!(parse(&format!("x := [1+1]+{}1", "1+".repeat(1021))) == Err(CalcError::TooComplex));
    assert!(parse(&format!("x := [1+1]+{}1", "1+".repeat(1020))).is_ok());
    assert!(parse(&format!("x := D[1+1]+{}1", "1+".repeat(1021))) == Err(CalcError::TooComplex));
    assert!(parse(&format!("x := D[1+1]+{}1", "1+".repeat(1020))).is_ok());
    // A long literal is bounded by the token limit.
    assert_eq!(parse(&format!("x := [{}1]", "1,".repeat(6_000))), Err(CalcError::TooComplex));
    // A range end nests to the same depth as any other right side.
    ok(format!("x := 0..{}1", "-".repeat(64)));
    too_complex(format!("x := 0..{}1", "-".repeat(65)));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_text_never_panics(src in "\\PC{0,200}") {
        let _ = parse(&src);
    }

    #[test]
    fn operator_soup_never_panics(src in "[a-zA-Z0-9_ ().,;|:=?√·×*/^+\\-≔°Δµσ\\[\\]]{0,300}") {
        if let Ok(s) = parse(&src) {
            let _ = free_names(&s);
            let _ = defined_name(&s);
        }
        let _ = crate::render::display(&src, None, &wordcraft_doc::calc::CalcState::Fresh);
    }
}

#[test]
fn error_messages() {
    assert_eq!(CalcError::Undefined("k".into()).to_string(), "k isn't defined");
    assert_eq!(CalcError::Arity("sqrt".into(), 1).to_string(), "sqrt expects 1 argument(s)");
    assert_eq!(CalcError::Duplicate { name: "P".into(), page: Some(3) }.to_string(), "P is already defined on page 3 — choose a unique name");
    assert_eq!(
        CalcError::Duplicate { name: "P".into(), page: None }.to_string(),
        "P is already defined elsewhere in the document — choose a unique name"
    );
    assert_eq!(CalcError::Circular("a, b".into()).to_string(), "a, b depend on each other");
    assert_eq!(CalcError::from(craft_units::UnitError::Overflow).to_string(), "dimension exponent overflow");
}

#[test]
fn text_literals() {
    assert_eq!(body_of("c := \"OK\""), Expr::Text("OK".into()));
    assert_eq!(body_of("c := “Use Schedule 80”"), Expr::Text("Use Schedule 80".into()));
    assert_eq!(body_of("c := [\"A\", \"B\"]"), Expr::Matrix { rows: 2, cols: 1, items: vec![Expr::Text("A".into()), Expr::Text("B".into())] });
    assert_eq!(parse("c := \"OK").unwrap_err().to_string(), "a text needs a closing quote, like \"OK\"");
    assert_eq!(parse(&format!("c := \"{}\"", "x".repeat(1001))).unwrap_err().to_string(), "a text is at most 1,000 characters");
    assert!(parse("c := 2 \"OK\"").is_err(), "text never multiplies by adjacency");
}

/// The expression of `src =`.
fn expr(src: &str) -> Expr {
    expr_of(src)
}

#[test]
fn comparisons_and_logic_parse() {
    let cmp = |a: Expr, rest: Vec<(Cmp, Expr)>| Expr::Compare { first: Box::new(a), rest };
    let n = |s: &str| Expr::Name(s.into());
    assert_eq!(expr("a <= b + 1"), cmp(n("a"), vec![(Cmp::Le, Expr::Binary(Op::Add, Box::new(n("b")), Box::new(Expr::Num(1.0))))]));
    assert_eq!(expr("0 ≤ x < 1"), cmp(Expr::Num(0.0), vec![(Cmp::Le, n("x")), (Cmp::Lt, Expr::Num(1.0))]));
    assert_eq!(expr("a == b"), cmp(n("a"), vec![(Cmp::Eq, n("b"))]));
    assert_eq!(expr("a != b"), expr("a ≠ b"));
    assert_eq!(expr("a >= b"), expr("a ≥ b"));
    assert_eq!(expr("not a < b and c or d"), expr("((¬(a < b)) ∧ c) ∨ d"));
    assert_eq!(expr("a ∧ b"), Expr::Logic(Logic::And, Box::new(n("a")), Box::new(n("b"))));
    // `==` is not the show `=`; a statement can still end in `=`.
    assert!(matches!(parse("a == b ="), Ok(Stmt::Eval { .. })));
    assert_eq!(parse("or := 5").unwrap_err().to_string(), "or is a reserved word — choose another name");
    assert_eq!(parse("f(if) := 5").unwrap_err().to_string(), "if is a reserved word — choose another name");
    assert!(parse("x := 1 ! 2").unwrap_err().to_string().contains("`!`"));
}

#[test]
fn logic_binds_looser_than_comparison() {
    assert_eq!(expr("not a < b and c ≥ d or e"), expr("((not (a < b)) and (c ≥ d)) or e"));
    // `or` is loosest, then `and`, then `not`, then comparisons, then sums.
    assert_eq!(expr("a or b and c"), expr("a or (b and c)"));
    assert_eq!(expr("a < b + 1"), expr("a < (b + 1)"));
    assert_eq!(expr("a or b or c"), expr("(a or b) or c"));
    // A keyword is not a name: it starts no product.
    assert!(parse("2 and =").is_err());
    assert!(parse("x := 2 or 3 4").is_err());
    assert!(parse("if =").is_err());
    assert!(parse("x := 1 otherwise 2").is_err());
    // A long chain or a deep `not` is bounded, not recursive without limit.
    let chain = vec!["a"; 3000].join(" < ");
    assert!(matches!(parse(&format!("{chain} =")), Ok(Stmt::Eval { .. })));
    assert_eq!(parse(&format!("{}a =", "not ".repeat(100))), Err(CalcError::TooComplex));
}

#[test]
fn comparison_names_are_free_names() {
    let s = parse("x := a < b and not c or d == a").unwrap();
    assert_eq!(free_names(&s), vec!["a", "b", "c", "d"]);
}

#[test]
fn cases_parse() {
    let Stmt::Define { body: Expr::Cases { cases, otherwise }, show, .. } =
        parse("Y := 0.4 if T ≤ 900 °F, 0.5 if T ≤ 950 °F, 0.7 otherwise =").unwrap()
    else {
        panic!()
    };
    assert_eq!(cases.len(), 2);
    assert_eq!(otherwise.as_deref(), Some(&Expr::Num(0.7)));
    assert!(show.is_some());
    assert!(matches!(body_of("c := \"OK\" if m ≥ 0 in"), Expr::Cases { otherwise: None, .. }));
    assert!(matches!(body_of("y := if(x > 0, 1, 2)"), Expr::Call(n, a) if n == "if" && a.len() == 3));
    let misplaced = "a list of cases (a if c, b otherwise) can only be the whole right side of a definition";
    assert_eq!(parse("y := (1 if a, 2 otherwise) + 1").unwrap_err().to_string(), misplaced);
    assert_eq!(parse("1 if a, 2 otherwise =").unwrap_err().to_string(), misplaced);
    assert!(parse("y := 1 otherwise").is_err());
    assert!(parse("y := 1 if a, 2 otherwise, 3 if b").is_err());
    assert!(!free_names(&parse("y := if(x > 0, 1, 2)").unwrap()).contains(&"if".to_string()));
}

#[test]
fn cases_details() {
    let misplaced = "a list of cases (a if c, b otherwise) can only be the whole right side of a definition";
    // Each part of a case is a whole expression: commas end conditions, and a condition may use
    // `and`, `or` and comparisons.
    let Expr::Cases { cases, otherwise } = body_of("y := a + 1 if 0 ≤ x < 1 and z, b otherwise") else { panic!() };
    assert_eq!(cases, vec![(expr("a + 1"), expr("0 ≤ x < 1 and z"))]);
    assert_eq!(otherwise.as_deref(), Some(&Expr::Name("b".into())));
    // A function definition may be piecewise; `if(` inside a case is the function.
    assert!(matches!(parse("f(v) := sqrt(v) if v ≥ 0, 0 otherwise"), Ok(Stmt::Define { body: Expr::Cases { .. }, .. })));
    assert!(matches!(body_of("y := if(a, 1, 2) if b"), Expr::Cases { cases, .. } if matches!(&cases[0].0, Expr::Call(n, _) if n == "if")));
    // Anywhere else a case keyword is the misplaced-case error.
    for src in ["y := f(1 if a)", "y := [1 if a, 2]", "y := |1 if a|", "if(a, 1, 2) if b =", "y := 0..5 if a", "y := otherwise", "y := if a"] {
        assert_eq!(syntax_msg(src), misplaced, "{src}");
    }
    // Every case but the last needs `if`, and `otherwise` comes last.
    assert!(parse("y := 1 if a, 2").is_err());
    assert!(parse("y := 1 if a otherwise").is_err());
    assert!(parse("y := 1 if a, 2 otherwise, 3 otherwise").is_err());
    assert!(parse("y := 1 if a, 0..2 if b").is_err());
    // Names in every value and condition are used, in order.
    assert_eq!(free_names(&parse("y := a if b > c, d otherwise").unwrap()), vec!["a", "b", "c", "d"]);
    assert_eq!(free_names(&parse("y := if(p, q, r)").unwrap()), vec!["p", "q", "r"]);
    // Many cases are bounded by the token limit, not by recursion.
    let many = (0..800).map(|k| format!("{k} if x < {k}")).collect::<Vec<_>>().join(", ");
    assert!(matches!(parse(&format!("y := {many}, 0 otherwise")), Ok(Stmt::Define { body: Expr::Cases { .. }, .. })));
}

#[test]
fn an_if_call_is_a_factor() {
    // `if(` touching its parenthesis is the function, so after a value it is an implied product,
    // like `2 sqrt(x)`; `if` followed by anything else starts a case.
    assert_eq!(body_of("y := 2 if(x > 0, 1, 2)"), body_of("y := 2·if(x > 0, 1, 2)"));
    assert!(matches!(body_of("y := 2 if(x > 0, 1, 2)"), Expr::Binary(Op::Mul, _, b) if matches!(&*b, Expr::Call(n, a) if n == "if" && a.len() == 3)));
    let Expr::Cases { cases, otherwise } = body_of("y := 2 if x > 0, 3 otherwise") else { panic!() };
    assert_eq!(cases, vec![(Expr::Num(2.0), expr("x > 0"))]);
    assert_eq!(otherwise.as_deref(), Some(&Expr::Num(3.0)));
    let Expr::Cases { cases, .. } = body_of("y := 2 if (x > 0), 3 otherwise") else { panic!() };
    assert_eq!(cases[0].1, expr("x > 0"));
    // In an evaluation too.
    assert_eq!(expr("2 if(x > 0, 1, 2)"), expr("2·if(x > 0, 1, 2)"));
}

fn sum(index: &str, first: Expr, second: Option<Expr>, last: Expr, body: Expr) -> Expr {
    Expr::Sum { index: index.into(), first: Box::new(first), second: second.map(Box::new), last: Box::new(last), body: Box::new(body) }
}

#[test]
fn a_sum_parses_its_range_and_one_product_term() {
    let term = bin(Op::Mul, index("a", vec![name("i")]), index("b", vec![name("i")]));
    assert_eq!(expr("∑(i = 1..n) a[i]·b[i]"), sum("i", num(1.0), None, name("n"), term.clone()));
    // The sum covers one product term: what follows a `+` is outside it.
    assert_eq!(expr("∑(i = 1..n) a[i]·b[i] + 1"), bin(Op::Add, sum("i", num(1.0), None, name("n"), term.clone()), num(1.0)));
    assert_eq!(expr("∑(i = 1..n) (a[i] + 1)"), sum("i", num(1.0), None, name("n"), bin(Op::Add, index("a", vec![name("i")]), num(1.0))));
    // Powers and quotients stay inside the term; comparisons and logic do not.
    assert_eq!(expr("∑(i = 1..3) i^2/2"), sum("i", num(1.0), None, num(3.0), bin(Op::Div, bin(Op::Pow, name("i"), num(2.0)), num(2.0))));
    assert_eq!(
        expr("∑(i = 1..3) i > 2"),
        Expr::Compare { first: Box::new(sum("i", num(1.0), None, num(3.0), name("i"))), rest: vec![(Cmp::Gt, num(2.0))] }
    );
    // A step: the second value.
    assert_eq!(expr("∑(k = 1, 3..7) k"), sum("k", num(1.0), Some(num(3.0)), num(7.0), name("k")));
    // The ends are whole expressions.
    assert_eq!(expr("∑(i = 0..length(r) - 1) i"), sum("i", num(0.0), None, bin(Op::Sub, call("length", vec![name("r")]), num(1.0)), name("i")));
    // Nested; two sums side by side; a sum after an operator, and as a factor.
    assert_eq!(expr("∑(i = 1..3) ∑(j = 1..i) j"), sum("i", num(1.0), None, num(3.0), sum("j", num(1.0), None, name("i"), name("j"))));
    assert_eq!(
        expr("∑(i = 1..2) i + ∑(i = 1..3) i"),
        bin(Op::Add, sum("i", num(1.0), None, num(2.0), name("i")), sum("i", num(1.0), None, num(3.0), name("i")))
    );
    assert_eq!(expr("2 ∑(i = 1..3) i"), bin(Op::Mul, num(2.0), sum("i", num(1.0), None, num(3.0), name("i"))));
    // A definition's right side, with and without a result.
    assert!(matches!(body_of("I := ∑(i = 0..n) i"), Expr::Sum { .. }));
    assert!(matches!(parse("I := ∑(i = 0..n) i = ? in^4"), Ok(Stmt::Define { show: Some(_), .. })));
    assert_eq!(body_of("tri(n) := ∑(i = 1..n) i"), sum("i", num(1.0), None, name("n"), name("i")));
}

#[test]
fn a_malformed_sum_is_a_syntax_error() {
    for src in [
        "∑ i =",
        "∑(i) i =",
        "∑(i = 1) i =",
        "∑(i = 1..) i =",
        "∑(i = 1..3 i =",
        "∑(i = 1..3) =",
        "∑(1 = 1..3) 1 =",
        "∑(i = 1, 2, 3..4) i =",
        "∑(i = 1..3..4) i =",
        "∑(i = 1, 2) i =",
        "∑(i == 1..3) i =",
        "∑(and = 1..3) 1 =",
    ] {
        assert!(parse(src).is_err(), "{src:?} should not parse: {:?}", parse(src));
    }
    // A sum's `=` is not the show `=`: what follows the first is read as a range.
    assert!(matches!(parse("∑(i = 1..3) i = ? in"), Ok(Stmt::Eval { show: Show { unit: Some(_) }, .. })));
}

#[test]
fn the_names_of_a_sum() {
    // The index is used (the dependency rule) but never as a name inside the body; the ends and
    // the rest of the body count.
    let stmt = parse("t := ∑(i = a..n) b[i]·c + i =").unwrap();
    assert_eq!(free_names(&stmt), vec!["i", "a", "n", "b", "c"]);
    // Without the sum, `i` outside it is the plain name.
    assert_eq!(free_names(&parse("∑(i = 1..2) i + i =").unwrap()), vec!["i"]);
    // An inner sum's ends see the outer index, as the body does.
    assert_eq!(free_names(&parse("∑(i = 1..3) ∑(j = i..m) j·i·x =").unwrap()), vec!["i", "j", "m", "x"]);
    // The index of one sum is a name in a later sum's ends.
    assert_eq!(free_names(&parse("∑(i = 1..2) i + ∑(j = i..4) j =").unwrap()), vec!["i", "j"]);
}

#[test]
fn nested_sums_are_bounded() {
    let opens: String = (0..200).map(|k| format!("∑(i_{k} = 1..2) ")).collect();
    assert_eq!(parse(&format!("{opens}1 =")), Err(CalcError::TooComplex));
    let in_ends: String = (0..200).map(|k| format!("∑(i_{k} = 1..")).collect();
    let closes: String = (0..200).map(|_| ") 1").collect();
    assert_eq!(parse(&format!("{in_ends}3{closes} =")), Err(CalcError::TooComplex));
}

#[test]
fn a_keyword_cannot_be_a_sums_index() {
    for word in ["and", "or", "not", "if", "otherwise"] {
        assert_eq!(parse(&format!("∑({word} = 1..3) 1 =")), Err(CalcError::Keyword(word.to_string())), "{word}");
    }
    assert_eq!(parse("∑(and = 1..3) 1 =").unwrap_err().to_string(), "and is a reserved word — choose another name");
    // Anything else that isn't a name is still a syntax error.
    syntax("∑(1 = 1..3) 1 =");
}

#[test]
fn parse_given() {
    assert_eq!(parse("Given"), Ok(Stmt::Given));
    assert_eq!(parse(" Given "), Ok(Stmt::Given));
    for src in ["given", "Given x", "GIVEN", "Given Given", "x Given"] {
        syntax(src);
    }
    assert_eq!(free_names(&Stmt::Given), Vec::<String>::new());
    assert_eq!(defined_name(&Stmt::Given), None);
}

#[test]
fn parse_constraint() {
    assert_eq!(parse("a·x == b + 1"), Ok(Stmt::Constraint { lhs: bin(Op::Mul, name("a"), name("x")), rhs: bin(Op::Add, name("b"), num(1.0)) }));
    let stmt = parse("p == 5 psi").expect("an equation");
    assert!(matches!(stmt, Stmt::Constraint { .. }));
    assert_eq!(free_names(&stmt), ["p", "psi"]);
    assert_eq!(defined_name(&stmt), None);
    assert_eq!(syntax_msg("a == b == c"), "an equation has one ==");
    assert_eq!(syntax_msg("a == b ≠ c == d"), "an equation has one ==");
    // Every other root expression with no `=` keeps the old error, and one with `=` is still a result.
    for src in ["a < b", "a == b < c", "a ≠ b", "a + b", "not a == b", "a == b and c"] {
        assert_eq!(syntax_msg(src), "end with `=` to show the result, or use `:=` to define a name", "{src}");
    }
    assert!(matches!(parse("a == b ="), Ok(Stmt::Eval { .. })));
}

#[test]
fn parse_define_many() {
    let names = |list: &[&str]| list.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    assert_eq!(parse("[a; b] := v"), Ok(Stmt::DefineMany { names: names(&["a", "b"]), body: name("v"), show: None }));
    assert_eq!(
        parse("[a; b] := Find(x, y) ="),
        Ok(Stmt::DefineMany { names: names(&["a", "b"]), body: call("Find", vec![name("x"), name("y")]), show: Some(show(None)) })
    );
    assert_eq!(
        parse("[σ_1; σ_2; c] := v = ? psi"),
        Ok(Stmt::DefineMany { names: names(&["σ_1", "σ_2", "c"]), body: name("v"), show: Some(show(Some("psi"))) })
    );
    let stmt = parse("[a; b] := v + w").expect("defines two names");
    assert_eq!(defined_name(&stmt), None);
    assert_eq!(crate::ast::defined_names(&stmt), ["a", "b"]);
    assert_eq!(free_names(&stmt), ["v", "w"]);
    for src in ["[a, b] := v", "[] := v", "[a; 2] := v", "[a] := v", "[a; b := v", "[a; b;] := v", "[a;; b] := v", "[a; b](c) := v", "[a; f(x)] := v"]
    {
        syntax(src);
    }
    assert_eq!(syntax_msg("[a; a] := v"), "`a` is repeated");
    // The hint's example isn't circular: the unknowns are other names than the results.
    assert_eq!(syntax_msg("[a] := v"), "a definition of several names is written like [a; b] := Find(x, y)");
    // A keyword where a name belongs is the keyword error, as in any other head.
    assert_eq!(parse("[a; or] := v"), Err(CalcError::Keyword("or".into())));
}

#[test]
fn a_solve_block_uses_its_equations_names_and_its_unknowns() {
    let eq = |l: Expr, r: Expr| (l, r);
    let solve = Expr::Solve {
        kind: crate::SolveKind::Find,
        unknowns: vec!["x".into(), "y".into(), "k".into()],
        equations: vec![eq(bin(Op::Mul, name("a"), name("x")), name("b")), eq(name("y"), bin(Op::Add, name("x"), name("c")))],
    };
    let stmt = Stmt::Define { name: "r".into(), index: None, params: vec![], body: solve.clone(), show: None };
    // The equations first, in order, then the unknowns the equations didn't use.
    assert_eq!(free_names(&stmt), ["a", "x", "b", "y", "c", "k"]);
    // Inside another expression it is the same.
    let wrapped = Stmt::Eval { expr: bin(Op::Add, solve, name("z")), show: show(None) };
    assert_eq!(free_names(&wrapped), ["a", "x", "b", "y", "c", "k", "z"]);
    // An equation with a sum keeps the sum's index to itself.
    let sum = Expr::Sum {
        index: "i".into(),
        first: Box::new(num(1.0)),
        second: None,
        last: Box::new(num(3.0)),
        body: Box::new(index("v", vec![name("i")])),
    };
    let with_sum = Expr::Solve { kind: crate::SolveKind::Minerr, unknowns: vec!["x".into()], equations: vec![(sum, name("x"))] };
    assert_eq!(free_names(&Stmt::Eval { expr: with_sum, show: show(None) }), ["i", "v", "x"]);
}

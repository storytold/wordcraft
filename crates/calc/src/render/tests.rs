use super::*;
use craft_units::{Precision, Quantity, UnitSystem, format};
use proptest::prelude::*;

use crate::array::MAX_ELEMENTS;

fn f(number: &str, unit: &str) -> Formatted {
    Formatted { number: number.into(), unit: unit.into() }
}

fn grid(rows: usize, cols: usize, items: Vec<Formatted>, unit: Option<&str>, total_rows: usize) -> Shown {
    Shown::Grid { rows, cols, items, unit: unit.map(str::to_string), total_rows, total_cols: cols }
}

fn shown_linear(shown: &Shown) -> String {
    rendered(&parse("x ="), "x =", Some(shown), &CalcState::Fresh).linear
}

#[test]
fn a_single_value_reads_back() {
    for one in [f("600", "psi"), f("1", ""), f("-1.5×10^-5", "m"), f("6×10^6", "J/(mol·K)")] {
        let shown = Shown::One(one);
        assert_eq!(shown_result(&shown_linear(&shown)), Some(shown));
    }
}

#[test]
fn a_cut_matrix_has_a_dots_row_under_every_column() {
    let items = (0..100).map(|k| f(&k.to_string(), "")).collect();
    let shown = grid(50, 2, items, Some("in"), 70);
    let linear = shown_linear(&shown);
    assert!(linear.starts_with("x=[■(0&1@2&3@"), "{linear}");
    assert!(linear.ends_with("@98&99@⋮&⋮)] \"in\" \"(70 rows)\""), "{linear}");
    assert_eq!(shown_result(&linear), Some(shown));
}

#[test]
fn a_wide_result_has_a_dots_column_after_its_first_fifty() {
    let items: Vec<Formatted> = (0..50).map(|k| f(&k.to_string(), "")).collect();
    let wide = Shown::Grid { rows: 1, cols: 50, items: items.clone(), unit: Some("in".into()), total_rows: 1, total_cols: 10_000 };
    let linear = shown_linear(&wide);
    assert!(linear.ends_with("&48&49&⋯)] \"in\" \"(10000 columns)\""), "{linear}");
    assert_eq!(shown_result(&linear), Some(wide));
    // Both cut: the dots meet in a corner, and the size names both.
    let both = Shown::Grid { rows: 2, cols: 25, items, unit: None, total_rows: 60, total_cols: 60 };
    let linear = shown_linear(&both);
    let cut_row = format!("{}&⋱", vec!["⋮"; 25].join("&"));
    assert!(linear.contains("&24&⋯@25&") && linear.ends_with(&format!("&49&⋯@{cut_row})] \"(60 rows × 60 columns)\"")), "{linear}");
    assert_eq!(shown_result(&linear), Some(both.clone()));
    let math = rendered(&parse("x ="), "x =", Some(&both), &CalcState::Fresh).math;
    let Some(MNode::Delim { elems, .. }) = math.iter().rfind(|n| matches!(n, MNode::Delim { .. })) else { panic!("{math:?}") };
    let Some(MNode::Matrix { rows, .. }) = elems.first().and_then(|e| e.first()) else { panic!("{elems:?}") };
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.len() == 26), "{rows:?}");
    let text = |cell: &Arg| match cell.as_slice() {
        [MNode::Run(r)] => r.text.clone(),
        other => format!("{other:?}"),
    };
    assert_eq!(rows.iter().map(|r| r.last().map(text)).collect::<Vec<_>>(), [Some("⋯".into()), Some("⋯".into()), Some("⋱".into())]);
}

#[test]
fn the_cells_are_built_whole_however_many() {
    // The most cells ever shown, each as long as a cell gets: far more than the linear parser reads
    // in one go (it keeps 20 of these 50 rows), and each is drawn all the same.
    let items: Vec<Formatted> = (0..2_500).map(|k| f(&format!("-1.235×10^-{}", k % 20 + 1), if k % 2 == 0 { "kN·m" } else { "°F" })).collect();
    let shown = grid(50, 50, items, None, 50);
    let math = rendered(&parse("x ="), "x =", Some(&shown), &CalcState::Fresh).math;
    let Some(MNode::Delim { elems, .. }) = math.iter().rfind(|n| matches!(n, MNode::Delim { .. })) else { panic!("{math:?}") };
    let Some(MNode::Matrix { rows, .. }) = elems.first().and_then(|e| e.first()) else { panic!("{elems:?}") };
    assert_eq!(rows.len(), 50);
    assert!(rows.iter().all(|r| r.len() == 50));
    assert_eq!(shown_result(&shown_linear(&shown)), Some(shown));
}

#[test]
fn the_result_matrix_is_the_one_the_linear_text_describes() {
    // Built cell by cell, the tree is the one Word's linear format gives for the same text.
    for (items, text) in
        [(vec![f("0.84", ""), f("-1.5×10^-5", "")], "[■(0.84@-1.5×〖10〗^(-5))]"), (vec![f("2", ""), f("6×10^6", "")], "[■(2@6×〖10〗^6)]")]
    {
        let shown = grid(2, 1, items, None, 2);
        let math = rendered(&parse("x ="), "x =", Some(&shown), &CalcState::Fresh).math;
        assert_eq!(math.last(), restore_pieces(parse_linear(text), &[], &[]).first(), "{text}");
    }
}

#[test]
fn what_rendered_never_writes_reads_as_nothing() {
    for bad in [
        "x=[■()]",
        "x=[■(1@2&3)]",
        "x=[■(1@⋮)]",
        "x=[■(1@⋮)] \"in\"",
        "x=[■(1@2)] \"(5 rows)\"",
        "x=[■(⋮@1)] \"(5 rows)\"",
        "x=[■(1@⋮&2)] \"(5 rows)\"",
        "x=[■(1@⋮)] \"(1 rows)\"",
        "x=[■(1@⋮)] \"(99999999999999999999 rows)\"",
        "x=[■(1 \"in\"@2)] \"in\"",
        "x=[■(a@2)]",
        "x=[■(1@2)",
        "x=[■(1@2)] \"in",
        "x=[■(1 \"in@2)]",
        "x=[■(1@2)] \"in\" \"lbf\"",
        "x=[■(1&⋯)] \"(5 rows)\"",
        "x=[■(1&⋯)]",
        "x=[■(1&2)] \"(5 columns)\"",
        "x=[■(1&⋯)] \"(1 columns)\"",
        "x=[■(1&⋯@⋮&⋮)] \"(5 rows × 5 columns)\"",
        "x=[■(1&⋯@⋮&⋱)] \"(5 rows)\"",
        "x=[■(1&⋯@⋮&⋱)] \"(5 rows × 5 cols)\"",
        "x=[■(1&⋯@⋮&⋱)] \"(200 rows × 200 columns)\"",
        "x=[■(⋯)] \"(5 columns)\"",
        "x=[■(1)] \"(5 rows × 5 columns)\" \"in\"",
    ] {
        assert_eq!(shown_result(bad), None, "{bad}");
    }
    // More rows than are ever shown, or more cells than a grid holds.
    assert_eq!(shown_result(&format!("x=[■({}1)]", "1@".repeat(50))), None);
    assert_eq!(shown_result(&format!("x=[■({}1)]", "1&".repeat(MAX_ELEMENTS))), None);
    assert_eq!(shown_result(&format!("x=[■({}1)]", "1&".repeat(MAX_SHOWN_COLS))), None);
    assert_eq!(shown_result(&format!("x=[■({}1)]", "1@".repeat(1_000_000))), None);
}

fn number() -> impl Strategy<Value = String> {
    let value = prop_oneof![any::<f64>(), -1e3..1e3_f64, -1e-3..1e-3_f64, Just(0.0)];
    (value.prop_filter("finite", |v| v.is_finite()), 0u8..=15)
        .prop_map(|(v, p)| format(Quantity::dimensionless(v), None, UnitSystem::Si, Precision::new(p)).map_or_else(|_| "0".to_string(), |f| f.number))
}

const UNITS: [&str; 10] = ["", "in", "lbf", "kN·m", "J/(mol·K)", "ft^2", "°F", "Δ°C", "kN m", "1/s"];

/// A grid as [`rendered`] is given one: at most 50 rows and 50 columns shown of `total_rows` ×
/// `total_cols`, and a shared unit (never empty) only when the cells carry none.
fn shown_grid() -> impl Strategy<Value = Shown> {
    let cols = prop_oneof![1usize..=4, 1usize..=MAX_SHOWN_COLS];
    let cuts = (any::<bool>(), any::<bool>(), any::<u16>(), any::<u16>());
    (1usize..=MAX_SHOWN_ROWS, cols, any::<bool>(), cuts).prop_flat_map(|(rows, cols, shared, (cut_rows, cut_cols, more_rows, more_cols))| {
        // A cut grid holds more than it shows, and no more than any grid holds.
        let least_cols = cols + usize::from(cut_cols);
        let total_rows = if cut_rows { rows + 1 + usize::from(more_rows) % (MAX_ELEMENTS / least_cols - rows) } else { rows };
        let total_cols = if cut_cols { cols + 1 + usize::from(more_cols) % (MAX_ELEMENTS / total_rows - cols) } else { cols };
        let cells = prop::collection::vec((number(), 0..UNITS.len()), rows * cols);
        let unit = if shared { (1..UNITS.len()).prop_map(Some).boxed() } else { Just(None).boxed() };
        (cells, unit).prop_map(move |(cells, unit)| {
            let unit = unit.and_then(|k| UNITS.get(k)).map(|u| u.to_string());
            let items = cells
                .into_iter()
                .map(|(number, k)| Formatted {
                    number,
                    unit: if unit.is_some() { String::new() } else { UNITS.get(k).copied().unwrap_or("").to_string() },
                })
                .collect();
            Shown::Grid { rows, cols, items, unit, total_rows, total_cols }
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn every_grid_reads_back(shown in shown_grid()) {
        let linear = shown_linear(&shown);
        prop_assert_eq!(shown_result(&linear), Some(shown.clone()));
        // And with an equation number after it.
        prop_assert_eq!(shown_result(&format!("{linear}#")), Some(shown));
    }

    #[test]
    fn hostile_results_never_panic(s in "[\\[■(@&)\\]\" 0-9.⋮⋯⋱×〖〗^\\-=a-z#]{0,300}") {
        let _ = shown_result(&s);
    }
}

fn display_of(src: &str, shown: Option<&Shown>) -> Rendered {
    rendered(&parse(src), src, shown, &CalcState::Fresh)
}

/// Every run in a math tree, at any depth.
fn runs(nodes: &[MNode]) -> Vec<MRun> {
    let mut out = Vec::new();
    let mut todo: Vec<&MNode> = nodes.iter().collect();
    while let Some(n) = todo.pop() {
        match n {
            MNode::Run(r) => out.push(r.clone()),
            MNode::Frac { num, den, .. } => todo.extend(num.iter().chain(den.iter())),
            MNode::Script { base, sub, sup, .. } => todo.extend(base.iter().chain(sub.iter()).chain(sup.iter())),
            MNode::Rad { deg, e, .. } => todo.extend(deg.iter().chain(e.iter())),
            MNode::Delim { elems, .. } => todo.extend(elems.iter().flatten()),
            MNode::Func { name, e } => todo.extend(name.iter().chain(e.iter())),
            MNode::Matrix { rows, .. } => todo.extend(rows.iter().flatten().flatten()),
            _ => {}
        }
    }
    out
}

#[test]
fn text_results_round_trip() {
    for shown in [
        Shown::Text("OK".into()),
        Shown::Text("Use t = 0.2 in (min) & check @ site".into()),
        Shown::TextGrid { rows: 3, cols: 1, items: vec!["OK".into(), "a = b".into(), "x)]".into()], total_rows: 3, total_cols: 1 },
        Shown::TextGrid { rows: 50, cols: 1, items: vec!["NG".into(); 50], total_rows: 60, total_cols: 1 },
    ] {
        assert_eq!(shown_result(&shown_linear(&shown)), Some(shown));
    }
}

#[test]
fn text_shows_in_quotes() {
    let d = display_of("c := \"OK\" =", Some(&Shown::Text("OK".into())));
    assert!(d.linear.ends_with("=\"OK\""), "{}", d.linear);
    assert!(runs(&d.math).iter().any(|r| r.text == "\"OK\"" && r.sty == Some(MSty::Plain)));
}

#[test]
fn malformed_text_results_read_back_as_nothing() {
    for linear in
        ["x=\"", "x=\"a\"b\"", "x=\"a\" \"in\"", "x=[■(\"a\"@1)]", "x=[■(1@\"a\")]", "x=[■(\"a\")] \"in\"", "x=[■(\"a\"", "x=[■(\"a)]", "x=\"a\"=\"b"]
    {
        assert_eq!(shown_result(linear), None, "{linear}");
    }
    assert_eq!(shown_result("x≔\"a=b\"≈\"c\""), Some(Shown::Text("c".into())));
}

#[test]
fn the_largest_text_grid_reads_back() {
    // Every cell shown, each as long as a text gets, of 4-byte characters: the longest result.
    let text = "😀".repeat(crate::array::MAX_TEXT_CHARS);
    let shown = Shown::TextGrid { rows: 50, cols: 50, items: vec![text; 2_500], total_rows: 50, total_cols: 50 };
    let linear = shown_linear(&shown);
    assert!(linear.len() > 4 << 20 && linear.len() < MAX_RESULT_BYTES, "{}", linear.len());
    assert_eq!(shown_result(&linear), Some(shown));
}

#[test]
fn the_result_is_found_from_the_end() {
    // An unbalanced quote in the expression (stored text edited outside WordCraft) before the result.
    assert_eq!(shown_result("a \"b ≔ 2=\"OK\""), Some(Shown::Text("OK".into())));
    assert_eq!(shown_result("a \"b ≔ 2=600 \"psi\""), Some(Shown::One(f("600", "psi"))));
    assert_eq!(
        shown_result("\"\"\" x=[■(\"a=b\"@\"c\")]"),
        Some(Shown::TextGrid { rows: 2, cols: 1, items: vec!["a=b".into(), "c".into()], total_rows: 2, total_cols: 1 })
    );
    // Nothing from the end back to a relation within the longest result: no result.
    assert_eq!(shown_result(&format!("x={}", "a".repeat(MAX_RESULT_BYTES + 1))), None);
}

#[test]
fn comparisons_and_logic_display() {
    let lin = |src: &str| display_of(src, None).linear;
    assert!(lin("c := a <= b").contains("a≤b"));
    assert!(lin("c := a != b").contains("a≠b"));
    assert!(lin("c := a and not b or d").contains("a∧¬b∨d"));
    assert!(lin("c := 2·(a < b)").contains("2·(a<b)") || lin("c := 2·(a < b)").contains("2(a<b)"));
    let eq = display_of("c := a == b", None);
    assert!(runs(&eq.math).iter().any(|r| r.text == "=" && r.sty == Some(MSty::Bold)));
}

#[test]
fn comparison_parentheses_follow_precedence() {
    let lin = |src: &str| display_of(src, None).linear;
    assert!(lin("c := a < b ≤ c").contains("a<b≤c"));
    assert!(lin("c := a == b").contains("a==b"));
    assert!(lin("c := (a or b) and d").contains("(a∨b)∧d"));
    assert!(lin("c := a or (b and d)").contains("a∨b∧d"));
    assert!(lin("c := a or (b or d)").contains("a∨(b∨d)"));
    assert!(lin("c := not (a and b)").contains("¬(a∧b)"));
    // `not` binds looser than a comparison, but `¬a<b` would read as `(¬a)<b`: the comparison
    // keeps its parentheses, however it was typed. A bare name needs none.
    assert!(lin("c := not (a < b)").contains("≔¬(a<b)"), "{}", lin("c := not (a < b)"));
    assert!(lin("c := not a < b").contains("≔¬(a<b)"));
    assert!(lin("c := not a == 2").contains("≔¬(a==2)"));
    assert!(lin("c := not a").contains("≔¬a"));
    assert!(!lin("c := not a").contains('('));
    assert!(lin("c := not (a or b)").contains("¬(a∨b)"));
    assert!(lin("c := a + (b < d)").contains("a+(b<d)"));
    assert!(lin("c := (a < b) < d").contains("(a<b)<d"));
    assert!(lin("c := x < (a or b)").contains("x<(a∨b)"));
}

#[test]
fn a_comparison_result_reads_back() {
    // `==` is part of the expression; the result is read from the last `=`.
    let one = Shown::One(f("1", ""));
    let d = display_of("a == 1 =", Some(&one));
    assert!(d.linear.contains("a==1=1"), "{}", d.linear);
    assert_eq!(shown_result(&d.linear), Some(one));
    // A display that lost its result: neither `=` of `==` is the result's marker.
    assert_eq!(shown_result("x≔a==1"), None);
    assert_eq!(shown_result("x≔a==1#"), None);
    assert_eq!(shown_result("x≔(a==1)+(b==2)"), None);
    // With its result, still read from the last `=`.
    assert_eq!(shown_result("x≔a==1=1"), Some(Shown::One(f("1", ""))));
}

#[test]
fn looser_operators_keep_their_parentheses_as_operands() {
    let lin = |src: &str| display_of(src, None).linear;
    // Left and right of a sum.
    assert!(lin("c := (a < b) + 1").contains("(a<b)+1"));
    assert!(lin("c := (x > 0) + (y > 0)").contains("(x>0)+(y>0)"));
    assert!(lin("c := (p or q) - 1").contains("(p∨q)-1"));
    assert!(lin("c := (not p) - 1").contains("(¬p)-1"));
    assert!(lin("c := 1 + (a < b)").contains("1+(a<b)"));
    // Products, quotients, powers, negation, roots, calls, indexes and absolute values.
    assert!(lin("c := (a < b)·2").contains("(a<b)·2"));
    assert!(lin("c := 2·(a < b)").contains("(a<b)"));
    assert!(lin("c := (a < b)/2").contains("(a<b)/2"));
    assert!(lin("c := 2/(a < b)").contains("2/(a<b)"));
    assert!(lin("c := (a < b)^2").contains("(a<b)^2"));
    assert!(lin("c := 2^(a < b)").contains("2^(a<b)"));
    assert!(lin("c := -(a < b)").contains("-(a<b)"));
    assert!(lin("c := √(a < b)").contains("√(a<b)"));
    assert!(lin("c := f(a < b, p or q)").contains("(a<b,p∨q)"));
    assert!(lin("c := D[a < b]").contains("(a<b)"));
    assert!(lin("c := |a < b|").contains("|a<b|"));
}

/// The first node in a math tree, depth first, for which `want` holds.
fn find_node(nodes: &[MNode], want: impl Fn(&MNode) -> bool) -> Option<&MNode> {
    let mut todo: Vec<&MNode> = nodes.iter().rev().collect();
    while let Some(n) = todo.pop() {
        if want(n) {
            return Some(n);
        }
        let children: Vec<&MNode> = match n {
            MNode::Frac { num, den, .. } => num.iter().chain(den.iter()).collect(),
            MNode::Script { base, sub, sup, .. } => base.iter().chain(sub.iter()).chain(sup.iter()).collect(),
            MNode::Rad { deg, e, .. } => deg.iter().chain(e.iter()).collect(),
            MNode::Delim { elems, .. } => elems.iter().flatten().collect(),
            MNode::Func { name, e } => name.iter().chain(e.iter()).collect(),
            MNode::Matrix { rows, .. } => rows.iter().flatten().flatten().collect(),
            _ => Vec::new(),
        };
        todo.extend(children.into_iter().rev());
    }
    None
}

#[test]
fn cases_show_as_a_brace() {
    let d = display_of("Y := 0.4 if T ≤ 900 °F, 0.7 otherwise", None);
    let brace = find_node(&d.math, |n| matches!(n, MNode::Delim { beg: Some('{'), end: None, .. }));
    let Some(MNode::Delim { elems, .. }) = brace else { panic!("{:?}", d.math) };
    assert!(
        matches!(elems.first().and_then(|e| e.first()), Some(MNode::Matrix { rows, .. }) if rows.len() == 2 && rows.iter().all(|r| r.len() == 2))
    );
    let words: Vec<String> = runs(&d.math).into_iter().map(|r| r.text).collect();
    assert!(words.iter().any(|w| w.trim() == "if") && words.iter().any(|w| w.trim() == "otherwise"));
    assert!(display_of("y := if(x > 0, 1, 2)", None).linear.contains("if(x>0,1,2)"));
}

#[test]
fn cases_linear_text_and_runs() {
    let d = display_of("Y := 0.4 if T ≤ 900 °F, 0.5 if T ≤ 950 °F, 0.7 otherwise =", Some(&Shown::One(f("0.5", ""))));
    assert!(d.linear.starts_with("Y≔{■(0.4&\"if \"T≤900 \"°F\"@0.5&\"if \"T≤950 \"°F\"@0.7&\"otherwise\")┤=0.5"), "{}", d.linear);
    // The words are upright; the condition's unit keeps its own run.
    let rs = runs(&d.math);
    for word in ["if ", "otherwise"] {
        assert!(rs.iter().any(|r| r.text == word && r.sty == Some(MSty::Plain)), "{word}: {rs:?}");
    }
    assert!(rs.iter().any(|r| r.text.trim() == "°F"));
    // No otherwise: one row per case. A piecewise function shows the same way.
    let d = display_of("f(v) := 1 if v > 0, 2 if v < 0", None);
    assert!(d.linear.contains("f(v)≔{■(1&\"if \"v>0@2&\"if \"v<0)┤"), "{}", d.linear);
    let brace = find_node(&d.math, |n| matches!(n, MNode::Matrix { .. }));
    assert!(matches!(brace, Some(MNode::Matrix { rows, .. }) if rows.len() == 2));
    // `if(` shows as an upright function name.
    let d = display_of("y := if(x > 0, 1, 2)", None);
    assert!(runs(&d.math).iter().any(|r| r.text == "if" && r.sty == Some(MSty::Plain)));
}

#[test]
fn sums_show_as_nary() {
    let d = display_of("I := ∑(i = 0..n) a[i]·b[i]", None);
    let Some(MNode::Nary { chr: '∑', sub, sup, e, .. }) = find_node(&d.math, |n| matches!(n, MNode::Nary { .. })) else { panic!("{:?}", d.math) };
    assert!(!sub.is_empty() && !sup.is_empty() && !e.is_empty());
}

#[test]
fn a_sum_shows_its_limits_and_a_grouped_term() {
    let d = display_of("I := ∑(i = 0..n) a[i]·b[i]", None);
    assert_eq!(d.linear, "I≔∑_(i=0)^n▒〖a_i·b_i〗");
    // A second value shows after the first; the term stays grouped before what follows.
    let d = display_of("∑(k = 1, 3..7) k + 1 =", Some(&Shown::One(f("17", ""))));
    assert_eq!(d.linear, "∑_(k=1,3)^7▒〖k〗+1=17");
    // The limits and the term are trees of their own, with the index in the lower limit.
    let Some(MNode::Nary { sub, e, .. }) = find_node(&d.math, |n| matches!(n, MNode::Nary { .. })) else { panic!("{:?}", d.math) };
    let text: String = runs(sub).iter().map(|r| r.text.clone()).collect();
    assert_eq!(text, "k=1,3");
    assert_eq!(runs(e).iter().map(|r| r.text.clone()).collect::<String>(), "k");
    // Nested sums nest their trees; a sum under a power keeps its parentheses.
    let d = display_of("x := ∑(i = 1..3) ∑(j = 1..i) j", None);
    assert_eq!(d.linear, "x≔∑_(i=1)^3▒〖∑_(j=1)^i▒〖j〗〗");
    let d = display_of("x := (∑(i = 1..3) i)^2", None);
    assert_eq!(d.linear, "x≔(∑_(i=1)^3▒〖i〗)^2");
}

#[test]
fn a_sum_of_terms_keeps_its_parentheses() {
    let grouped = display_of("x := ∑(i = 1..3) (i + 1)", None);
    let outside = display_of("x := ∑(i = 1..3) i + 1", None);
    assert!(grouped.linear.contains("▒〖(i+1)〗"), "{}", grouped.linear);
    assert_eq!(outside.linear, "x≔∑_(i=1)^3▒〖i〗+1");
    assert_ne!(grouped.linear, outside.linear);
    // The parentheses are in the tree too, not only the linear text.
    let Some(MNode::Nary { e, .. }) = find_node(&grouped.math, |n| matches!(n, MNode::Nary { .. })) else { panic!("{:?}", grouped.math) };
    assert!(matches!(e.as_slice(), [MNode::Delim { beg: Some('('), end: Some(')'), .. }]), "{e:?}");
    // A comparison is grouped too; a product, power or lone name is not.
    let d = display_of("x := ∑(i = 0..2) (a[i] > 0)", None);
    assert!(d.linear.contains("▒〖(a_i>0)〗"), "{}", d.linear);
    for body in ["a[i]·b[i]", "a[i]^2", "-a[i]", "a[i]/2"] {
        let d = display_of(&format!("x := ∑(i = 0..2) {body}"), None);
        let Some(MNode::Nary { e, .. }) = find_node(&d.math, |n| matches!(n, MNode::Nary { .. })) else { panic!("{:?}", d.math) };
        assert!(!matches!(e.as_slice(), [MNode::Delim { .. }]), "{body}: {e:?}");
    }
}

#[test]
fn given_displays_as_upright_text() {
    let d = display_of("Given", None);
    assert_eq!(d.linear, "Given");
    assert!(runs(&d.math).iter().any(|r| r.text == "Given" && r.sty == Some(MSty::Plain)), "{:?}", d.math);
    // It shows no result, and there is none to read back.
    assert_eq!(shown_result(&d.linear), None);
    assert!(!shows_value(&Stmt::Given));
}

#[test]
fn a_lone_equation_displays_with_the_bold_equals() {
    let d = display_of("a·x == b + 1", None);
    assert_eq!(d.linear, "a·x==b+1");
    assert!(runs(&d.math).iter().any(|r| r.text == "=" && r.sty == Some(MSty::Bold)), "{:?}", d.math);
    // Nothing after it is a result, even when one is offered.
    let one = Shown::One(f("1", ""));
    assert_eq!(display_of("a == b", Some(&one)).linear, "a==b");
    assert_eq!(shown_result("a==b"), None);
    assert!(!shows_value(&parse("a == b").expect("an equation")));
    // Units and functions in its sides display as they do anywhere else.
    assert_eq!(display_of("p == 5 psi", None).linear, "p==5 \"psi\"");
    assert_eq!(display_of("p == sqrt(z)", None).linear, "p==√z");
}

#[test]
fn a_multiple_definition_displays_a_column_of_names() {
    let d = display_of("[r_1; r_2] := v", None);
    assert_eq!(d.linear, "[■(r_1@r_2)]≔v");
    let Some(MNode::Delim { beg: Some('['), end: Some(']'), elems, .. }) = d.math.first() else { panic!("{:?}", d.math) };
    let Some(MNode::Matrix { rows, .. }) = elems.first().and_then(|e| e.first()) else { panic!("{elems:?}") };
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.len() == 1), "{rows:?}");
    assert_eq!(shown_result(&d.linear), None);
    assert!(!shows_value(&parse("[a; b] := v").expect("defines two names")));
    // Long names stay upright and whole (`xmax` has no function name found inside).
    assert_eq!(display_of("[x_max; σ_allow] := v", None).linear, "[■(x_max@σ_allow)]≔v");
}

#[test]
fn a_shown_multiple_definition_reads_back() {
    let shown = grid(2, 1, vec![f("1", "m"), f("2", "m")], None, 2);
    let d = display_of("[r_1; r_2] := Find(x, y) =", Some(&shown));
    assert_eq!(d.linear, "[■(r_1@r_2)]≔Find(x,y)=[■(1 \"m\"@2 \"m\")]");
    assert!(parse("[a; b] := v =").is_ok_and(|s| shows_value(&s)));
    assert_eq!(shown_result(&d.linear), Some(shown));
    // `Find` is upright, like any function name.
    assert!(runs(&d.math).iter().any(|r| r.text == "Find" && r.sty == Some(MSty::Plain)), "{:?}", d.math);
}

#[test]
fn a_resolved_solve_displays_as_its_call() {
    let solve =
        Expr::Solve { kind: SolveKind::Minerr, unknowns: vec!["x".into(), "y_2".into()], equations: vec![(Expr::Name("x".into()), Expr::Num(1.0))] };
    let stmt = Stmt::Define { name: "r".into(), index: None, params: vec![], body: solve, show: None };
    let d = rendered(&Ok(stmt), "", None, &CalcState::Fresh);
    assert_eq!(d.linear, "r≔Minerr(x,y_2)");
}

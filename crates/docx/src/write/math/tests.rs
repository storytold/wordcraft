//! The OMML writer: content and values are checked by writing a tree and reading it back with
//! [`crate::read::math::read_omath`]; element order, which the reader can't see (it looks children
//! up by name), is checked against exact XML. The golden strings were checked against the
//! ECMA-376 shared-math schema (`CT_*` sequences); they come from this writer, not from Word.

use super::*;
use wordcraft_doc::math::to_linear;

const RED: Rgb = Rgb(0xFF, 0, 0);
const GREEN: Rgb = Rgb(0, 0x80, 0);
const BLUE: Rgb = Rgb(0, 0, 0xFF);

fn r(t: &str) -> MNode {
    MNode::Run(MRun::new(t))
}

fn a(t: &str) -> Arg {
    vec![r(t)]
}

fn delim(beg: Option<char>, end: Option<char>, elems: Vec<Arg>) -> MNode {
    MNode::Delim { beg, end, sep: Some('|'), grow: true, shp_match: false, elems }
}

fn math(nodes: Arg) -> Math {
    Math { nodes, ..Default::default() }
}

/// `write_equation` into a fresh writer, without the XML declaration.
fn write(m: &Math, linear: &str, display: bool, color: Option<Rgb>) -> String {
    let mut w = W::default();
    write_equation(&mut w, linear, display, m, color);
    w.s
}

fn xml(nodes: &Arg) -> String {
    write(&math(nodes.clone()), "", false, None)
}

/// Read written OMML back (inline or display), as the DOCX reader does.
fn read(xml: &str) -> Math {
    let root = crate::xml::parse_strict(wrap_ns(xml).as_bytes()).unwrap();
    match root.child("m:oMathPara") {
        Some(p) => crate::read::math::read_omath(p.child("m:oMath").unwrap(), crate::read::math::para_jc(p)),
        None => crate::read::math::read_omath(root.child("m:oMath").unwrap(), MathJc::CenterGroup),
    }
}

fn round_trip(nodes: &Arg) -> Arg {
    read(&xml(nodes)).nodes
}

#[track_caller]
fn assert_round_trip(label: &str, nodes: Arg) {
    assert_eq!(round_trip(&nodes), nodes, "{label}: {}", xml(&nodes));
}

/// The equation's runs.
fn runs(nodes: &Arg) -> Vec<MRun> {
    let mut out = Vec::new();
    let mut n = nodes.clone();
    wordcraft_doc::math_edit::each_run_mut(&mut n, &mut |r| out.push(r.clone()));
    out
}

// --- Round trip: every structure and option comes back as written. -----------------------------

#[test]
fn every_structure_round_trips() {
    use MNode::*;
    for kind in [FracKind::Bar, FracKind::Skewed, FracKind::Linear, FracKind::NoBar] {
        assert_round_trip("frac", vec![Frac { kind, num: a("a"), den: a("b") }]);
    }
    assert_round_trip("empty frac", vec![Frac { kind: FracKind::Bar, num: vec![], den: vec![] }]);
    assert_round_trip("sup", vec![Script { kind: ScriptKind::Sup, base: a("x"), sub: vec![], sup: a("2") }]);
    assert_round_trip("sub", vec![Script { kind: ScriptKind::Sub, base: a("x"), sub: a("i"), sup: vec![] }]);
    assert_round_trip("subsup", vec![Script { kind: ScriptKind::SubSup, base: a("x"), sub: a("i"), sup: a("2") }]);
    assert_round_trip("pre", vec![Script { kind: ScriptKind::Pre, base: a("x"), sub: a("i"), sup: a("2") }]);
    assert_round_trip("sqrt", vec![Rad { deg: vec![], deg_hide: true, e: a("x") }]);
    assert_round_trip("root", vec![Rad { deg: a("3"), deg_hide: false, e: a("x") }]);
    assert_round_trip("empty degree shown", vec![Rad { deg: vec![], deg_hide: false, e: a("x") }]);
    for (chr, lim_loc) in [('∫', None), ('∑', Some(LimLoc::UndOvr)), ('∏', Some(LimLoc::SubSup))] {
        assert_round_trip("nary", vec![Nary { chr, lim_loc, grow: false, sub_hide: false, sup_hide: false, sub: a("i"), sup: a("n"), e: a("x") }]);
    }
    assert_round_trip(
        "nary flags",
        vec![Nary { chr: '∑', lim_loc: None, grow: true, sub_hide: true, sup_hide: true, sub: vec![], sup: vec![], e: a("x") }],
    );
    assert_round_trip("parens", vec![delim(Some('('), Some(')'), vec![a("a")])]);
    assert_round_trip("one-sided", vec![delim(Some('{'), None, vec![a("a")])]);
    assert_round_trip("no opening", vec![delim(None, Some(']'), vec![a("a")])]);
    assert_round_trip("empty element", vec![delim(Some('('), Some(')'), vec![vec![]])]);
    assert_round_trip(
        "brackets, separator, two elements",
        vec![Delim { beg: Some('['), end: Some(']'), sep: Some(';'), grow: true, shp_match: false, elems: vec![a("a"), a("b")] }],
    );
    assert_round_trip(
        "no separator",
        vec![Delim { beg: Some('('), end: Some(')'), sep: None, grow: true, shp_match: false, elems: vec![a("a"), a("b")] }],
    );
    assert_round_trip(
        "fixed size, matched shape",
        vec![Delim { beg: Some('|'), end: Some('|'), sep: Some('|'), grow: false, shp_match: true, elems: vec![a("a")] }],
    );
    assert_round_trip("func", vec![Func { name: vec![MNode::Run(MRun::plain("sin"))], e: a("x") }]);
    assert_round_trip("lim low", vec![Lim { upper: false, e: vec![MNode::Run(MRun::plain("lim"))], lim: a("n→∞") }]);
    assert_round_trip("lim upp", vec![Lim { upper: true, e: a("x"), lim: a("k") }]);
    assert_round_trip("hat", vec![Acc { chr: '\u{302}', e: a("x") }]);
    assert_round_trip("vector", vec![Acc { chr: '\u{20D7}', e: a("v") }]);
    assert_round_trip("overbar", vec![Bar { top: true, e: a("x") }]);
    assert_round_trip("underbar", vec![Bar { top: false, e: a("x") }]);
    assert_round_trip("border box", vec![BorderBox { hide: [false; 4], strike: [false; 4], e: a("x") }]);
    assert_round_trip("border box flags", vec![BorderBox { hide: [true, false, true, false], strike: [false, true, false, true], e: a("x") }]);
    assert_round_trip("box", vec![Boxed { e: a("x") }]);
    assert_round_trip("underbrace", vec![GroupChr { chr: '\u{23DF}', top: false, e: a("x") }]);
    assert_round_trip("overbrace", vec![GroupChr { chr: '\u{23DE}', top: true, e: a("x") }]);
    assert_round_trip("eqarr", vec![EqArr { rows: vec![a("a=b"), a("c=d")] }]);
    assert_round_trip("numbered eqarr", vec![EqArr { rows: vec![a("a=b#(1)")] }]);
    assert_round_trip("matrix", vec![Matrix { rows: vec![vec![a("a"), a("b")], vec![a("c"), a("d")]], col_jc: vec![ColJc::Left, ColJc::Right] }]);
    assert_round_trip("phantom", vec![Phant { show: true, zero_wid: false, zero_asc: false, zero_desc: false, e: a("x") }]);
    assert_round_trip("hidden phantom", vec![Phant { show: false, zero_wid: true, zero_asc: true, zero_desc: true, e: a("x") }]);
}

#[test]
fn nested_structures_round_trip() {
    let quadratic = parse_linear("x=(-b±√(b^2-4ac))/2a");
    assert_round_trip("quadratic", quadratic);
    let sum = parse_linear("∑_(i=1)^n▒(i^2)/(n+1)");
    assert_round_trip("sum", sum);
}

#[test]
fn run_properties_round_trip() {
    let with = |f: &dyn Fn(&mut MRun)| {
        let mut x = MRun::new("ab");
        f(&mut x);
        vec![MNode::Run(x)]
    };
    for s in [MSty::Plain, MSty::Bold, MSty::Italic, MSty::BoldItalic] {
        assert_round_trip("sty", with(&|x| x.sty = Some(s)));
    }
    for s in [MScr::Script, MScr::Fraktur, MScr::DoubleStruck, MScr::SansSerif, MScr::Monospace] {
        assert_round_trip("scr", with(&|x| x.scr = s));
    }
    assert_round_trip("lit", with(&|x| x.lit = true));
    assert_round_trip(
        "normal text in a font",
        with(&|x| {
            x.nor = true;
            x.font = Some("Arial".into());
        }),
    );
    assert_round_trip("size", with(&|x| x.size = Some(11.0)));
    assert_round_trip("half-point size", with(&|x| x.size = Some(10.5)));
    assert_round_trip("colour", with(&|x| x.color = Some(RED)));
    assert_round_trip("break", with(&|x| x.brk = Some(0)));
    assert_round_trip("aligned break", with(&|x| x.brk = Some(3)));
    for t in ["a<b&c\"d'e>f", "  a b  ", " ", "a\tb\nc", "𝑥𝟙", "\u{F061}"] {
        assert_round_trip(t, a(t));
    }
}

// --- Normalisation: what the reader gives back differently, on purpose. ------------------------

#[test]
fn reading_back_normalises_runs() {
    // Equal neighbours merge (`merge_runs`); an empty run is no run.
    assert_eq!(round_trip(&vec![r("a"), r("b")]), a("ab"));
    assert_eq!(round_trip(&vec![r("a"), r(""), r("b")]), a("ab"));
    assert_eq!(round_trip(&a("")), vec![]);
    // Characters XML can't carry are dropped.
    assert_eq!(round_trip(&a("a\u{1}b")), a("ab"));
    // Sizes are half points, within 1–1638 pt.
    let sized = |s: f32| vec![MNode::Run(MRun { size: Some(s), ..MRun::new("x") })];
    assert_eq!(round_trip(&sized(10.3)), sized(10.5));
    assert_eq!(round_trip(&sized(0.5)), sized(1.0));
    assert_eq!(round_trip(&sized(2000.0)), sized(1638.0));
    // A font belongs to normal text only; normal text without one is written in Cambria Math,
    // as Word writes it (layout treats that as the text font).
    let font = vec![MNode::Run(MRun { font: Some("Arial".into()), ..MRun::new("x") })];
    assert_eq!(round_trip(&font), a("x"));
    let nor = vec![MNode::Run(MRun { nor: true, ..MRun::new("x") })];
    assert_eq!(round_trip(&nor), vec![MNode::Run(MRun { nor: true, font: Some("Cambria Math".into()), ..MRun::new("x") })]);
}

#[test]
fn reading_back_normalises_structures() {
    use MNode::*;
    // A script keeps only the slots its kind has.
    let stray = vec![Script { kind: ScriptKind::Sup, base: a("x"), sub: a("i"), sup: a("2") }];
    assert_eq!(round_trip(&stray), vec![Script { kind: ScriptKind::Sup, base: a("x"), sub: vec![], sup: a("2") }]);
    // A delimiter has at least one element.
    assert_eq!(round_trip(&vec![delim(Some('('), Some(')'), vec![])]), vec![delim(Some('('), Some(')'), vec![vec![]])]);
    // A matrix gets one column alignment per column and rectangular rows (#3643).
    let cells = vec![vec![a("a"), a("b")], vec![a("c")]];
    let padded = vec![vec![a("a"), a("b")], vec![a("c"), vec![]]];
    assert_eq!(round_trip(&vec![Matrix { rows: cells, col_jc: vec![] }]), vec![Matrix { rows: padded, col_jc: vec![ColJc::Center; 2] }]);
    assert_eq!(round_trip(&vec![Matrix { rows: vec![], col_jc: vec![] }]), vec![Matrix { rows: vec![], col_jc: vec![ColJc::Center] }]);
}

/// Every linear-format equation saves and reopens as the same tree, once matrices have their
/// column alignments filled in.
#[test]
fn linear_equations_survive_a_save() {
    fn fill_col_jc(a: Arg) -> Arg {
        a.into_iter()
            .map(|n| match n {
                MNode::Matrix { rows, col_jc } if col_jc.is_empty() => {
                    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
                    let rows = rows.into_iter().map(|row| row.into_iter().map(fill_col_jc).collect()).collect();
                    MNode::Matrix { rows, col_jc: vec![ColJc::Center; cols] }
                }
                other => wordcraft_doc::math_linear::map_args(other, &fill_col_jc),
            })
            .collect()
    }
    for s in [
        "x=(-b±√(b^2-4ac))/2a",
        "〖a+b〗^2",
        "├]a[┤",
        "{█(x@y)┤",
        "(a│b)",
        "▭(a)+□(b)+¯(c)+▁(d)",
        "⏞(a+b)+⏟(c+d)",
        "⬚/⬚",
        "a⁄b+a∕b+a¦b",
        "_1^n▒Y",
        "x┬k+x┴k",
        "A⃗+x̂",
        "log_2⁡x",
        "lim┬(n→∞)⁡a_n",
        ")a(",
        "E=mc^2#(1)",
        "■(a&b@c&d)",
        "√(3&x)",
        "∫_0^1▒f(x)dx",
        "|x|+‖v‖",
    ] {
        let n = parse_linear(s);
        assert_eq!(round_trip(&n), fill_col_jc(n.clone()), "{s} → {}", to_linear(&n));
    }
}

// --- Where `write_equation` takes the equation from. --------------------------------------------

#[test]
fn kept_omml_is_written_back_unchanged() {
    let kept = "<m:oMath><m:r><m:t>KEPT</m:t></m:r></m:oMath>";
    let m = Math { nodes: a("NODES"), omml: kept.into(), ..Default::default() };
    assert_eq!(write(&m, "linear", false, None), kept);
}

#[test]
fn kept_xml_that_is_not_an_omath_is_rewritten_from_the_nodes() {
    let m = Math { nodes: a("NODES"), omml: "<m:r><m:t>X</m:t></m:r>".into(), ..Default::default() };
    let out = write(&m, "linear", false, None);
    assert_eq!(read(&out).nodes, a("NODES"));
}

#[test]
fn without_nodes_the_linear_text_is_written() {
    let out = write(&Math::default(), "a/b", false, None);
    assert_eq!(read(&out).nodes, parse_linear("a/b"));
    assert_eq!(write(&Math::default(), "", false, None), "<m:oMath></m:oMath>");
}

#[test]
fn an_equation_colour_rewrites_the_kept_omml() {
    let kept_red = r#"<m:oMath><m:r><w:rPr><w:color w:val="FF0000"/></w:rPr><m:t>a</m:t></m:r></m:oMath>"#;
    let nodes = vec![r("a"), MNode::Run(MRun { color: Some(GREEN), ..MRun::new("g") })];
    let m = Math { nodes, omml: kept_red.into(), ..Default::default() };
    let colours = |c: Option<Rgb>| runs(&read(&write(&m, "", false, c)).nodes).into_iter().map(|r| (r.text, r.color)).collect::<Vec<_>>();
    // The same colour keeps the source XML.
    assert_eq!(write(&m, "", false, Some(RED)), kept_red);
    // Another colour goes on every run without its own (#1467).
    assert_eq!(colours(Some(BLUE)), vec![("a".to_string(), Some(BLUE)), ("g".to_string(), Some(GREEN))]);
    // No colour takes the old one off.
    assert_eq!(colours(None), vec![("a".to_string(), None), ("g".to_string(), Some(GREEN))]);
}

#[test]
fn kept_omml_without_one_colour_is_kept_without_an_equation_colour() {
    for kept in [
        "<m:oMath><m:r><m:t>a</m:t></m:r></m:oMath>",
        r#"<m:oMath><m:r><w:rPr><w:color w:val="FF0000"/></w:rPr><m:t>a</m:t></m:r><m:r><m:t>b</m:t></m:r></m:oMath>"#,
    ] {
        let m = Math { nodes: a("ab"), omml: kept.into(), ..Default::default() };
        assert_eq!(write(&m, "", false, None), kept);
    }
}

#[test]
fn display_justification_round_trips() {
    for jc in [MathJc::CenterGroup, MathJc::Center, MathJc::Left, MathJc::Right] {
        let out = write(&Math { nodes: a("x"), jc, ..Default::default() }, "", true, None);
        assert_eq!(read(&out).jc, jc, "{out}");
        assert_eq!(out.contains("m:oMathParaPr"), jc != MathJc::CenterGroup, "{out}");
    }
    assert!(!write(&math(a("x")), "", false, None).contains("oMathPara"));
}

// --- Equation numbers. --------------------------------------------------------------------------

fn rows(rs: &[&str]) -> Math {
    math(vec![MNode::EqArr { rows: rs.iter().map(|s| a(s)).collect() }])
}

#[test]
fn automatic_numbers_are_written_out() {
    // A `#` with nothing after it gets the next number; a written number counts too, as in
    // layout's `auto_numbers`.
    let mut n = 0;
    let out = resolve_numbers(&rows(&["a=b#", "c=d", "e=f#(9)", "g#"]), true, &mut n).unwrap();
    assert_eq!(n, 3);
    assert_eq!(to_linear(&out.nodes), "█(a=b#(1)@c=d@e=f#(9)@g#(3))");
    assert!(out.omml.is_empty(), "the numbered copy is written from its nodes");
    // The counter carries on into the next equation.
    let out = resolve_numbers(&rows(&["h#"]), true, &mut n).unwrap();
    assert_eq!((n, to_linear(&out.nodes)), (4, "h#(4)".to_string()), "a one-row array prints without █");
}

#[test]
fn equations_without_automatic_numbers_are_left_alone() {
    let mut n = 5;
    assert!(resolve_numbers(&rows(&["a=b#(7)"]), true, &mut n).is_none());
    assert_eq!(n, 6, "a written number still counts");
    let mut n = 0;
    assert!(resolve_numbers(&rows(&["a#"]), false, &mut n).is_none(), "inline");
    assert!(resolve_numbers(&math(a("a=b#")), true, &mut n).is_none(), "not an equation array");
    let nested = math(vec![MNode::EqArr { rows: vec![vec![MNode::Frac { kind: FracKind::Bar, num: a("a#"), den: a("b") }]] }]);
    assert!(resolve_numbers(&nested, true, &mut n).is_none(), "a # inside a structure isn't a number mark");
    assert_eq!(n, 0);
}

// --- Settings: m:mathPr. ------------------------------------------------------------------------

fn math_pr_round_trip(m: &MathProps) -> MathProps {
    let mut w = W::default();
    math_pr(&mut w, m);
    let root = crate::xml::parse_strict(wrap_ns(&w.s).as_bytes()).unwrap();
    crate::read::math::read_math_pr(root.child("m:mathPr").unwrap())
}

#[test]
fn math_properties_round_trip() {
    for m in [
        MathProps::default(),
        MathProps { brk_bin: BrkBin::After, brk_bin_sub: BrkBinSub::PlusMinus, ..Default::default() },
        MathProps { brk_bin: BrkBin::Repeat, brk_bin_sub: BrkBinSub::MinusPlus, ..Default::default() },
        MathProps { wrap_indent: 0.0, ..Default::default() },
        MathProps { wrap_indent: 12.35, ..Default::default() },
        MathProps { wrap_right: true, ..Default::default() },
    ] {
        assert_eq!(math_pr_round_trip(&m), m);
    }
}

#[test]
fn math_properties_are_clamped() {
    let indent = |v: f32| math_pr_round_trip(&MathProps { wrap_indent: v, ..Default::default() }).wrap_indent;
    assert_eq!(indent(5000.0), 1584.0);
    assert_eq!(indent(-3.0), 0.0);
    assert_eq!(indent(f32::NAN), wordcraft_doc::math::DEFAULT_WRAP_INDENT);
    // `wrapIndent` and `wrapRight` are a choice: aligning right drops the indent.
    let right = math_pr_round_trip(&MathProps { wrap_right: true, wrap_indent: 10.0, ..Default::default() });
    assert_eq!(right, MathProps { wrap_right: true, ..Default::default() });
}

// --- Limits. ------------------------------------------------------------------------------------

#[test]
fn matrix_caps_hold_on_save() {
    let wide = vec![MNode::Matrix { rows: vec![vec![a("x"); MAX_MATRIX_COLS + 6]; 3], col_jc: vec![] }];
    let back = round_trip(&wide);
    let [MNode::Matrix { rows, col_jc }] = &back[..] else { panic!("{back:?}") };
    assert_eq!((rows.len(), rows[0].len(), col_jc.len()), (3, MAX_MATRIX_COLS, MAX_MATRIX_COLS));
    let tall = vec![MNode::Matrix { rows: vec![vec![a("x")]; MAX_MATRIX_ROWS + 5], col_jc: vec![] }];
    let back = round_trip(&tall);
    let [MNode::Matrix { rows, .. }] = &back[..] else { panic!("{back:?}") };
    assert_eq!(rows.len(), MAX_MATRIX_ROWS);
}

fn fraction_chain(depth: usize) -> Arg {
    let mut cur = a("LEAF");
    for _ in 0..depth {
        cur = vec![MNode::Frac { kind: FracKind::Bar, num: cur, den: a("d") }];
    }
    cur
}

#[test]
fn a_tree_past_the_depth_cap_writes_well_formed_xml() {
    for depth in [MAX_DEPTH, MAX_DEPTH + 1, 4 * MAX_DEPTH] {
        let out = xml(&fraction_chain(depth));
        assert!(crate::xml::parse_strict(wrap_ns(&out).as_bytes()).is_ok(), "depth {depth}");
    }
    assert!(!xml(&fraction_chain(MAX_DEPTH + 1)).contains("LEAF"), "the writer stops at MAX_DEPTH");
}

// --- Golden XML: element order and what is left out. --------------------------------------------

const CTRL: &str = r#"<m:ctrlPr><w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/><w:i/></w:rPr></m:ctrlPr>"#;

/// `m:r` for `x` with no properties of its own.
fn rx(t: &str) -> String {
    format!(r#"<m:r><w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/></w:rPr><m:t xml:space="preserve">{t}</m:t></m:r>"#)
}

/// The golden string with `{C}` standing for [`CTRL`] and `{a}`… for [`rx`] runs.
fn expand(s: &str) -> String {
    let mut out = s.replace("{C}", CTRL);
    for t in ["a", "b", "c", "d", "x", "i", "n", "2", "3", "k"] {
        out = out.replace(&format!("{{{t}}}"), &rx(t));
    }
    format!("<m:oMath>{out}</m:oMath>")
}

#[track_caller]
fn assert_golden(label: &str, nodes: Arg, expected: &str) {
    assert_eq!(xml(&nodes), expand(expected), "{label}");
}

#[test]
fn golden_runs() {
    assert_golden("plain run", a("x"), "{x}");
    let all = MRun { sty: Some(MSty::Bold), scr: MScr::Script, lit: true, brk: Some(2), color: Some(RED), size: Some(11.0), ..MRun::new("x") };
    assert_golden(
        "every math run property",
        vec![MNode::Run(all)],
        r#"<m:r><m:rPr><m:lit/><m:scr m:val="script"/><m:sty m:val="b"/><m:brk m:alnAt="2"/></m:rPr><w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/><w:color w:val="FF0000"/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr><m:t xml:space="preserve">x</m:t></m:r>"#,
    );
    let nor = MRun { nor: true, font: Some("Arial".into()), brk: Some(0), ..MRun::new("x y") };
    assert_golden(
        "normal text",
        vec![MNode::Run(nor)],
        r#"<m:r><m:rPr><m:nor/><m:brk/></m:rPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/></w:rPr><m:t xml:space="preserve">x y</m:t></m:r>"#,
    );
}

#[test]
fn golden_fractions_scripts_and_radicals() {
    use MNode::*;
    assert_golden(
        "bar fraction",
        vec![Frac { kind: FracKind::Bar, num: a("a"), den: a("b") }],
        "<m:f><m:fPr>{C}</m:fPr><m:num>{a}</m:num><m:den>{b}</m:den></m:f>",
    );
    assert_golden(
        "skewed fraction, empty denominator",
        vec![Frac { kind: FracKind::Skewed, num: a("a"), den: vec![] }],
        r#"<m:f><m:fPr><m:type m:val="skw"/>{C}</m:fPr><m:num>{a}</m:num><m:den/></m:f>"#,
    );
    assert_golden(
        "sup",
        vec![Script { kind: ScriptKind::Sup, base: a("x"), sub: vec![], sup: a("2") }],
        "<m:sSup><m:sSupPr>{C}</m:sSupPr><m:e>{x}</m:e><m:sup>{2}</m:sup></m:sSup>",
    );
    assert_golden(
        "sub",
        vec![Script { kind: ScriptKind::Sub, base: a("x"), sub: a("i"), sup: vec![] }],
        "<m:sSub><m:sSubPr>{C}</m:sSubPr><m:e>{x}</m:e><m:sub>{i}</m:sub></m:sSub>",
    );
    assert_golden(
        "subsup",
        vec![Script { kind: ScriptKind::SubSup, base: a("x"), sub: a("i"), sup: a("2") }],
        "<m:sSubSup><m:sSubSupPr>{C}</m:sSubSupPr><m:e>{x}</m:e><m:sub>{i}</m:sub><m:sup>{2}</m:sup></m:sSubSup>",
    );
    // CT_SPre: sub, sup, then the base.
    assert_golden(
        "pre",
        vec![Script { kind: ScriptKind::Pre, base: a("x"), sub: a("i"), sup: a("2") }],
        "<m:sPre><m:sPrePr>{C}</m:sPrePr><m:sub>{i}</m:sub><m:sup>{2}</m:sup><m:e>{x}</m:e></m:sPre>",
    );
    assert_golden(
        "square root",
        vec![Rad { deg: vec![], deg_hide: true, e: a("x") }],
        r#"<m:rad><m:radPr><m:degHide m:val="1"/>{C}</m:radPr><m:deg/><m:e>{x}</m:e></m:rad>"#,
    );
    assert_golden(
        "cube root",
        vec![Rad { deg: a("3"), deg_hide: false, e: a("x") }],
        "<m:rad><m:radPr>{C}</m:radPr><m:deg>{3}</m:deg><m:e>{x}</m:e></m:rad>",
    );
}

#[test]
fn golden_operators_and_delimiters() {
    use MNode::*;
    let nary = |chr, lim_loc, on| Nary { chr, lim_loc, grow: on, sub_hide: on, sup_hide: on, sub: a("i"), sup: a("n"), e: a("x") };
    assert_golden(
        "integral",
        vec![nary('∫', None, false)],
        "<m:nary><m:naryPr>{C}</m:naryPr><m:sub>{i}</m:sub><m:sup>{n}</m:sup><m:e>{x}</m:e></m:nary>",
    );
    assert_golden(
        "sum, every property",
        vec![nary('∑', Some(LimLoc::UndOvr), true)],
        r#"<m:nary><m:naryPr><m:chr m:val="∑"/><m:limLoc m:val="undOvr"/><m:grow m:val="1"/><m:subHide m:val="1"/><m:supHide m:val="1"/>{C}</m:naryPr><m:sub>{i}</m:sub><m:sup>{n}</m:sup><m:e>{x}</m:e></m:nary>"#,
    );
    assert_golden("parentheses", vec![delim(Some('('), Some(')'), vec![a("x")])], "<m:d><m:dPr>{C}</m:dPr><m:e>{x}</m:e></m:d>");
    assert_golden(
        "delimiter, every property",
        vec![Delim { beg: Some('['), end: None, sep: Some(';'), grow: false, shp_match: true, elems: vec![a("a"), a("b")] }],
        r#"<m:d><m:dPr><m:begChr m:val="["/><m:sepChr m:val=";"/><m:endChr m:val=""/><m:grow m:val="0"/><m:shp m:val="match"/>{C}</m:dPr><m:e>{a}</m:e><m:e>{b}</m:e></m:d>"#,
    );
    assert_golden(
        "function",
        vec![Func { name: a("a"), e: a("x") }],
        "<m:func><m:funcPr>{C}</m:funcPr><m:fName>{a}</m:fName><m:e>{x}</m:e></m:func>",
    );
    assert_golden(
        "lower limit",
        vec![Lim { upper: false, e: a("a"), lim: a("n") }],
        "<m:limLow><m:limLowPr>{C}</m:limLowPr><m:e>{a}</m:e><m:lim>{n}</m:lim></m:limLow>",
    );
    assert_golden(
        "upper limit",
        vec![Lim { upper: true, e: a("a"), lim: a("k") }],
        "<m:limUpp><m:limUppPr>{C}</m:limUppPr><m:e>{a}</m:e><m:lim>{k}</m:lim></m:limUpp>",
    );
}

#[test]
fn golden_accents_bars_and_boxes() {
    use MNode::*;
    assert_golden("accent", vec![Acc { chr: '\u{302}', e: a("x") }], "<m:acc><m:accPr><m:chr m:val=\"\u{302}\"/>{C}</m:accPr><m:e>{x}</m:e></m:acc>");
    assert_golden("overbar", vec![Bar { top: true, e: a("x") }], r#"<m:bar><m:barPr><m:pos m:val="top"/>{C}</m:barPr><m:e>{x}</m:e></m:bar>"#);
    assert_golden("underbar", vec![Bar { top: false, e: a("x") }], r#"<m:bar><m:barPr><m:pos m:val="bot"/>{C}</m:barPr><m:e>{x}</m:e></m:bar>"#);
    assert_golden(
        "border box",
        vec![BorderBox { hide: [false; 4], strike: [false; 4], e: a("x") }],
        "<m:borderBox><m:borderBoxPr>{C}</m:borderBoxPr><m:e>{x}</m:e></m:borderBox>",
    );
    assert_golden(
        "border box, every property",
        vec![BorderBox { hide: [true; 4], strike: [true; 4], e: a("x") }],
        r#"<m:borderBox><m:borderBoxPr><m:hideTop m:val="1"/><m:hideBot m:val="1"/><m:hideLeft m:val="1"/><m:hideRight m:val="1"/><m:strikeH m:val="1"/><m:strikeV m:val="1"/><m:strikeBLTR m:val="1"/><m:strikeTLBR m:val="1"/>{C}</m:borderBoxPr><m:e>{x}</m:e></m:borderBox>"#,
    );
    assert_golden("box", vec![Boxed { e: a("x") }], "<m:box><m:boxPr>{C}</m:boxPr><m:e>{x}</m:e></m:box>");
    assert_golden(
        "underbrace",
        vec![GroupChr { chr: '\u{23DF}', top: false, e: a("x") }],
        "<m:groupChr><m:groupChrPr><m:chr m:val=\"\u{23DF}\"/><m:pos m:val=\"bot\"/><m:vertJc m:val=\"top\"/>{C}</m:groupChrPr><m:e>{x}</m:e></m:groupChr>",
    );
    assert_golden(
        "overbrace",
        vec![GroupChr { chr: '\u{23DE}', top: true, e: a("x") }],
        "<m:groupChr><m:groupChrPr><m:chr m:val=\"\u{23DE}\"/><m:pos m:val=\"top\"/><m:vertJc m:val=\"bot\"/>{C}</m:groupChrPr><m:e>{x}</m:e></m:groupChr>",
    );
    assert_golden(
        "phantom",
        vec![Phant { show: true, zero_wid: false, zero_asc: false, zero_desc: false, e: a("x") }],
        r#"<m:phant><m:phantPr><m:show m:val="1"/>{C}</m:phantPr><m:e>{x}</m:e></m:phant>"#,
    );
    assert_golden(
        "phantom, every property",
        vec![Phant { show: false, zero_wid: true, zero_asc: true, zero_desc: true, e: a("x") }],
        r#"<m:phant><m:phantPr><m:show m:val="0"/><m:zeroWid m:val="1"/><m:zeroAsc m:val="1"/><m:zeroDesc m:val="1"/>{C}</m:phantPr><m:e>{x}</m:e></m:phant>"#,
    );
}

#[test]
fn golden_arrays() {
    use MNode::*;
    assert_golden("eqarr", vec![EqArr { rows: vec![a("a"), a("b")] }], "<m:eqArr><m:eqArrPr>{C}</m:eqArrPr><m:e>{a}</m:e><m:e>{b}</m:e></m:eqArr>");
    assert_golden(
        "numbered eqarr",
        vec![EqArr { rows: vec![vec![r("a"), r("#")]] }],
        r#"<m:eqArr><m:eqArrPr><m:maxDist m:val="1"/>{C}</m:eqArrPr><m:e>{a}<m:r><w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/></w:rPr><m:t xml:space="preserve">#</m:t></m:r></m:e></m:eqArr>"#,
    );
    assert_golden(
        "matrix",
        vec![Matrix { rows: vec![vec![a("a"), a("b")], vec![a("c")]], col_jc: vec![ColJc::Left] }],
        r#"<m:m><m:mPr><m:mcs><m:mc><m:mcPr><m:count m:val="1"/><m:mcJc m:val="left"/></m:mcPr></m:mc><m:mc><m:mcPr><m:count m:val="1"/><m:mcJc m:val="center"/></m:mcPr></m:mc></m:mcs>{C}</m:mPr><m:mr><m:e>{a}</m:e><m:e>{b}</m:e></m:mr><m:mr><m:e>{c}</m:e><m:e/></m:mr></m:m>"#,
    );
}

#[test]
fn golden_display_and_settings() {
    let out = write(&Math { nodes: a("x"), jc: MathJc::Left, ..Default::default() }, "", true, None);
    assert_eq!(out, format!(r#"<m:oMathPara><m:oMathParaPr><m:jc m:val="left"/></m:oMathParaPr>{}</m:oMathPara>"#, expand("{x}")));
    assert_eq!(write(&math(a("x")), "", true, None), format!("<m:oMathPara>{}</m:oMathPara>", expand("{x}")));
    let pr = |m: &MathProps| {
        let mut w = W::default();
        math_pr(&mut w, m);
        w.s
    };
    assert_eq!(pr(&MathProps::default()), r#"<m:mathPr><m:brkBin m:val="before"/><m:brkBinSub m:val="--"/><m:wrapIndent m:val="1440"/></m:mathPr>"#);
    assert_eq!(
        pr(&MathProps { brk_bin: BrkBin::After, brk_bin_sub: BrkBinSub::PlusMinus, wrap_right: true, ..Default::default() }),
        r#"<m:mathPr><m:brkBin m:val="after"/><m:brkBinSub m:val="+-"/><m:wrapRight/></m:mathPr>"#
    );
}

/// The run helper is used by every golden: a run with no properties is exactly `rx`.
#[test]
fn golden_helpers_match_the_writer() {
    assert_eq!(xml(&a("x")), format!("<m:oMath>{}</m:oMath>", rx("x")));
    assert!(expand("{C}").contains("m:ctrlPr"));
}

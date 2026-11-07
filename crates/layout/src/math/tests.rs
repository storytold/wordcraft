use super::*;
use wordcraft_doc::math::Math;

fn rc() -> ResolvedChar {
    wordcraft_doc::styles::StyleSheet::default().resolve_char(None, &Default::default())
}

fn lay(lin: &str, display: bool) -> MathLayout {
    layout_equation(&Math::default(), lin, &rc(), display, 400.0, &MathProps::default(), &mut 0)
}

/// The glyphs of a laid-out equation by line: (baseline, [(x, text)]), top line first.
fn lines(m: &MathLayout) -> Vec<(f32, Vec<(f32, String)>)> {
    let mut out: Vec<(f32, Vec<(f32, String)>)> = Vec::new();
    for it in &m.items {
        if let MItem::Glyphs { glyphs, text, .. } = it
            && let Some(&(_, x, y)) = glyphs.first()
        {
            match out.iter_mut().find(|l| (l.0 - y).abs() < 0.5) {
                Some(l) => l.1.push((x, text.clone())),
                None => out.push((y, vec![(x, text.clone())])),
            }
        }
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    for l in &mut out {
        l.1.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    out
}

fn lay_in(math: &Math, lin: &str, display: bool, avail: f32, mp: &MathProps) -> MathLayout {
    layout_equation(math, lin, &rc(), display, avail, mp, &mut 0)
}

#[test]
fn eq_array_rows_without_ampersand_are_centred() {
    // Word centres a row with no `&` in the array (#2617: a case row "a, n odd" above
    // "|a|, &n even"), whether or not the other rows have one.
    use wordcraft_doc::math::*;
    let run = |s: &str| vec![MNode::Run(MRun::new(s))];
    let w_a = lay("a", true).width;
    for other in ["bbbbbb&c", "bbbbbbc"] {
        let math = Math { nodes: vec![MNode::EqArr { rows: vec![run("a"), run(other)] }], ..Default::default() };
        let m = lay_in(&math, "", true, 400.0, &MathProps::default());
        let ls = lines(&m);
        let xa = ls.first().and_then(|l| l.1.first()).map(|g| g.0).unwrap_or(f32::NAN);
        let x0 = ls.get(1).and_then(|l| l.1.first()).map(|g| g.0).unwrap_or(f32::NAN);
        let want = x0 + (m.width - x0 - w_a) / 2.0;
        assert!((xa - want).abs() < 1.0, "{other}: a at {xa}, want {want} ({ls:?}, width {})", m.width);
        // The lone row doesn't widen the aligned columns.
        assert!(x0.abs() < 0.5 && m.width < lay("bbbbbbc", true).width + 1.0, "{other}: {ls:?}, width {}", m.width);
    }
    // A lone row wider than the aligned columns sets the array's width; the aligned
    // rows are centred under it as one block.
    let math = Math { nodes: vec![MNode::EqArr { rows: vec![run("bbbbbbbbbb"), run("a&c")] }], ..Default::default() };
    let m = lay_in(&math, "", true, 400.0, &MathProps::default());
    let ls = lines(&m);
    let xw = ls.first().and_then(|l| l.1.first()).map(|g| g.0).unwrap_or(f32::NAN);
    let x0 = ls.get(1).and_then(|l| l.1.first()).map(|g| g.0).unwrap_or(f32::NAN);
    assert!(xw.abs() < 0.5 && (m.width - lay("bbbbbbbbbb", true).width).abs() < 1.0, "wide lone row: {ls:?}, width {}", m.width);
    let want = (m.width - w_a - lay("c", true).width) / 2.0;
    assert!(want > 1.0 && (x0 - want).abs() < 1.0, "aligned row at {x0}, want {want} ({ls:?})");
}

#[test]
fn long_display_equations_wrap_before_operators_and_indent() {
    let sum = "y=a+b+c+d+e+f+g+h+i+j+k+l+m+n+o+p+q+r+s+t+u+v+w";
    let mp = MathProps::default();
    let full = lay_in(&Math::default(), sum, true, 1e4, &mp);
    assert_eq!(lines(&full).len(), 1, "fits on one line when there is room");
    let avail = full.width * 0.45;
    assert!(avail > 2.0 * mp.wrap_indent, "room for the indent: {avail}");
    let m = lay_in(&Math::default(), sum, true, avail, &mp);
    let ls = lines(&m);
    assert!(ls.len() >= 2, "{ls:?}");
    assert!((m.width - avail).abs() < 0.01, "the wrapped equation fills the line: {} vs {avail}", m.width);
    for (k, (_, gs)) in ls.iter().enumerate() {
        let (x0, first) = gs.first().cloned().unwrap();
        if k == 0 {
            // Math italic 𝑦 when a font has it; plain y where none does (FreeBSD CI has no fonts).
            assert!(x0.abs() < 0.01 && (first == "𝑦" || first == "y"), "{gs:?}");
        } else {
            // Continuation lines start with the operator, one wrap indent in.
            assert!((x0 - mp.wrap_indent).abs() < 0.01 && first == "+", "line {k}: {gs:?}");
        }
        for (x, t) in gs {
            assert!(*x < avail, "line {k}: {t} at {x} past {avail}");
        }
    }
    // Breaks after the operator, and the operator repeated, as the settings say.
    let after = MathProps { brk_bin: BrkBin::After, ..Default::default() };
    let ls = lines(&lay_in(&Math::default(), sum, true, avail, &after));
    assert!(ls.len() >= 2 && ls[0].1.last().is_some_and(|g| g.1 == "+") && ls[1].1[0].1 != "+", "{ls:?}");
    let repeat = MathProps { brk_bin: BrkBin::Repeat, wrap_right: true, ..Default::default() };
    let m = lay_in(&Math::default(), sum, true, avail, &repeat);
    let ls = lines(&m);
    assert!(ls.len() >= 2 && ls[0].1.last().is_some_and(|g| g.1 == "+") && ls[1].1[0].1 == "+", "{ls:?}");
    // Inline equations never wrap.
    assert_eq!(lines(&lay_in(&Math::default(), sum, false, avail, &mp)).len(), 1);
    // A term wider than the line overflows it rather than breaking inside.
    let frac = lay_in(&Math::default(), "y=(a+b+c+d+e+f+g+h+i+j+k+l+m+n+o+p+q+r+s+t+u+v+w)/2", true, avail, &mp);
    assert!(frac.width > avail && frac.items.iter().any(|i| matches!(i, MItem::Rect { .. })), "{}", frac.width);
}

#[test]
fn manual_breaks_start_new_lines() {
    let mp = MathProps::default();
    let nodes = vec![MNode::Run(MRun::new("a=b")), MNode::Run(MRun { brk: Some(0), ..MRun::new("+c") })];
    let m = lay_in(&Math { nodes, ..Default::default() }, "", true, 400.0, &mp);
    let ls = lines(&m);
    assert_eq!(ls.len(), 2, "{ls:?}");
    assert!((ls[1].1[0].0 - mp.wrap_indent).abs() < 0.01 && ls[1].1[0].1 == "+", "{ls:?}");
    assert!(m.width < 400.0, "a manually broken equation is as wide as its lines");
    // `m:alnAt`: the new line's operator sits under the first line's first operator.
    let nodes = vec![MNode::Run(MRun::new("a=b")), MNode::Run(MRun { brk: Some(1), ..MRun::new("+c") })];
    let ls = lines(&lay_in(&Math { nodes, ..Default::default() }, "", true, 400.0, &mp));
    let eq_x = ls[0].1.iter().find(|g| g.1 == "=").unwrap().0;
    assert!((ls[1].1[0].0 - eq_x).abs() < 0.01, "{ls:?}");
}

#[test]
fn short_display_equations_stay_on_one_line() {
    let mp = MathProps::default();
    let one = lay_in(&Math::default(), "a+b=c", true, 400.0, &mp);
    assert_eq!(lines(&one).len(), 1);
    let plain = lay_in(&Math::default(), "a+b=c", false, 400.0, &mp);
    assert!(one.width < 400.0 && (one.width - plain.width).abs() < 0.01, "{} vs {}", one.width, plain.width);
}

#[test]
fn fraction_is_stacked_with_a_rule() {
    let flat = lay("ab", true);
    let f = lay("a/b", true);
    assert!(f.ascent > flat.ascent + 2.0 && f.descent > flat.descent + 2.0, "{f:?}");
    assert!(f.items.iter().any(|i| matches!(i, MItem::Rect { .. })), "fraction rule");
    assert!(f.width < flat.width * 1.2, "stacked, not side by side");
}

#[test]
fn superscript_is_raised_and_smaller() {
    let m = lay("x^2", false);
    let sizes: Vec<(f32, f32)> = m
        .items
        .iter()
        .filter_map(|i| match i {
            MItem::Glyphs { size, glyphs, .. } => glyphs.first().map(|g| (*size, g.2)),
            _ => None,
        })
        .collect();
    assert_eq!(sizes.len(), 2, "{m:?}");
    assert!(sizes[1].0 < sizes[0].0 && sizes[1].1 > 2.0, "{sizes:?}");
}

#[test]
fn operators_get_spaces_and_unary_minus_does_not() {
    let tight = lay("ab", false);
    let spaced = lay("a+b", false);
    let unary = lay("−b", false);
    let plus_w = lay("+", false).width;
    assert!(spaced.width > tight.width + plus_w + 2.0, "medium spaces around +");
    assert!(unary.width < lay("b", false).width + plus_w + 1.0, "no space after a unary minus");
}

#[test]
fn display_operators_are_big_with_limits_over_and_under() {
    let d = lay("∑_(i=1)^n▒i", true);
    let t = lay("∑_(i=1)^n▒i", false);
    assert!(d.ascent + d.descent > t.ascent + t.descent, "display sum taller");
    assert!(d.width < t.width + 4.0, "limits stacked in display");
}

#[test]
fn delimiters_grow_around_tall_content() {
    let small = lay("(x)", true);
    let big = lay("((a)/(b))", true);
    let h = |m: &MathLayout| m.ascent + m.descent;
    assert!(h(&big) > h(&small) * 1.4, "{} vs {}", h(&big), h(&small));
}

#[test]
fn every_structure_lays_out() {
    use wordcraft_doc::math::*;
    let x = || vec![MNode::Run(MRun::new("x"))];
    let nodes = vec![
        MNode::Acc { chr: '\u{302}', e: x() },
        MNode::Acc { chr: '\u{305}', e: x() },
        MNode::Bar { top: false, e: x() },
        MNode::BorderBox { hide: [false; 4], strike: [true; 4], e: x() },
        MNode::Boxed { e: x() },
        MNode::GroupChr { chr: '⏟', top: false, e: x() },
        MNode::EqArr { rows: vec![vec![MNode::Run(MRun::new("x&=1"))], vec![MNode::Run(MRun::new("y&=22"))]] },
        MNode::Matrix { rows: vec![vec![x(), x()], vec![x(), Vec::new()]], col_jc: vec![] },
        MNode::Phant { show: false, zero_wid: false, zero_asc: false, zero_desc: true, e: x() },
        MNode::Lim { upper: false, e: vec![MNode::Run(MRun::plain("lim"))], lim: x() },
        MNode::Rad { deg: x(), deg_hide: false, e: x() },
        MNode::Script { kind: ScriptKind::Pre, base: x(), sub: x(), sup: x() },
        MNode::Frac { kind: FracKind::Skewed, num: x(), den: x() },
        MNode::Frac { kind: FracKind::Linear, num: x(), den: x() },
        MNode::Frac { kind: FracKind::NoBar, num: x(), den: x() },
        MNode::Delim { beg: None, end: Some('|'), sep: Some('|'), grow: true, shp_match: true, elems: vec![x(), x()] },
        MNode::Run(MRun { text: "text".into(), nor: true, ..Default::default() }),
        MNode::Run(MRun { text: "R".into(), scr: MScr::DoubleStruck, ..Default::default() }),
    ];
    for n in nodes {
        let m = layout_equation(&Math { nodes: vec![n.clone()], ..Default::default() }, "", &rc(), true, 400.0, &MathProps::default(), &mut 0);
        assert!(m.width > 0.0 && m.ascent + m.descent > 0.0, "{n:?} → {m:?}");
        for it in &m.items {
            if let MItem::Glyphs { glyphs, .. } = it {
                assert!(glyphs.iter().all(|g| g.1.is_finite() && g.2.is_finite()));
            }
        }
    }
}

/// #1273: an n-ary operator, delimiter, accent or group character the math font lacks is
/// drawn from a fallback font, not as a missing-glyph box.
#[test]
fn characters_the_math_font_lacks_fall_back() {
    let (face, _) = wordcraft_fonts::math::resolve_math("Cambria Math");
    let Some(c) = ['⨌', '⫿', '⟅', 'ア', '中', '가'].into_iter().find(|c| !face.covers(*c) && FontDb::global().fallback_for(*c, face.id()).is_some())
    else {
        return; // Every candidate is in the math font, or no fallback font is installed.
    };
    let x = || vec![MNode::Run(MRun::new("x"))];
    let nodes = [
        MNode::Nary { chr: c, lim_loc: None, grow: None, sub_hide: true, sup_hide: true, sub: vec![], sup: vec![], e: x() },
        MNode::Delim { beg: Some(c), end: Some(c), sep: None, grow: true, shp_match: false, elems: vec![x()] },
        MNode::Acc { chr: c, e: x() },
        MNode::GroupChr { chr: c, top: true, e: x() },
    ];
    for display in [false, true] {
        for n in &nodes {
            let m = layout_equation(&Math { nodes: vec![n.clone()], ..Default::default() }, "", &rc(), display, 400.0, &MathProps::default(), &mut 0);
            for it in &m.items {
                if let MItem::Glyphs { glyphs, .. } = it {
                    assert!(glyphs.iter().all(|g| g.0 != 0), "{c:?} drew a missing glyph in {n:?}");
                }
            }
        }
    }
}

#[test]
fn caret_slots_cover_every_position() {
    use wordcraft_doc::math_edit::{MathPos, move_right, units};
    let math = Math { nodes: wordcraft_doc::math::parse_linear("x=(-b±√(b^2-4ac))/2a+■(a&b@c&d)"), ..Default::default() };
    let m = layout_equation(&math, "", &rc(), true, 400.0, &MathProps::default(), &mut 0);
    // Walking the caret through the equation, every position has a slot.
    let mut p = MathPos::default();
    let mut seen = 0;
    loop {
        let s = m.slots.iter().find(|s| s.path == p.path && s.off == p.off);
        assert!(s.is_some(), "no slot for {p:?}");
        seen += 1;
        match move_right(&math.nodes, &p) {
            Some(n) => p = n,
            None => break,
        }
        assert!(seen < 500);
    }
    assert_eq!(p.off, units(&math.nodes));
    // Hit testing finds the denominator below the bar.
    let den = m.slots.iter().find(|s| s.path == vec![(1, 1)]).unwrap();
    let hit = m.hit(den.x + 1.0, den.y).unwrap();
    assert_eq!(hit.path, vec![(1, 1)]);
}

#[test]
fn an_arguments_extent_spans_its_caret_positions() {
    let math = Math { nodes: wordcraft_doc::math::parse_linear("y=(a+b)/c"), ..Default::default() };
    let m = layout_equation(&math, "", &rc(), true, 400.0, &MathProps::default(), &mut 0);
    let (nx0, nx1, nup, ndown) = m.arg_extent(&[(1, 0)]).unwrap();
    let (dx0, dx1, dup, ddown) = m.arg_extent(&[(1, 1)]).unwrap();
    // The numerator sits above the baseline and the denominator below it, both inside the equation.
    assert!(nx0 > 0.0 && nx1 > nx0 && nx1 <= m.width, "{nx0} {nx1} {}", m.width);
    assert!(dx1 > dx0 && dx1 <= m.width);
    assert!(-ndown > 0.0 && nup <= m.ascent + 0.01, "numerator {nup} {ndown}");
    assert!(dup < -ndown && ddown > 0.0 && ddown <= m.descent + 0.01, "denominator {dup} {ddown}");
    // The whole equation's argument is as wide as the equation.
    let (x0, x1, ..) = m.arg_extent(&[]).unwrap();
    assert!(x0.abs() < 0.01 && (x1 - m.width).abs() < 0.01, "{x0} {x1} {}", m.width);
    assert!(m.arg_extent(&[(9, 0)]).is_none());
}

#[test]
fn empty_slots_show_placeholders_on_screen_only() {
    let math = Math { nodes: wordcraft_doc::math::parse_linear("⬚/⬚"), ..Default::default() };
    let m = layout_equation(&math, "", &rc(), true, 400.0, &MathProps::default(), &mut 0);
    assert!(m.items.iter().all(|i| matches!(i, MItem::ScreenOnly(_) | MItem::Rect { .. })), "{:?}", m.items);
    assert!(m.slots.iter().any(|s| s.path == vec![(0, 0)] && s.off == 0));
    let empty = layout_equation(&Math::default(), "", &rc(), true, 400.0, &MathProps::default(), &mut 0);
    assert!(empty.width > 10.0 && empty.items.iter().all(|i| matches!(i, MItem::ScreenOnly(_))));
}

#[test]
fn numbered_equations_fill_the_line_with_the_number_at_the_right() {
    let math = Math { nodes: wordcraft_doc::math::parse_linear("E=mc^2#(1)"), ..Default::default() };
    let m = layout_equation(&math, "", &rc(), true, 400.0, &MathProps::default(), &mut 0);
    assert!((m.width - 400.0).abs() < 0.01, "{}", m.width);
    let xs: Vec<f32> = m.items.iter().filter_map(|i| if let MItem::Glyphs { glyphs, .. } = i { glyphs.first().map(|g| g.1) } else { None }).collect();
    let max = xs.iter().cloned().fold(0.0f32, f32::max);
    assert!(max > 350.0, "number at the right margin: {xs:?}");
    let min = xs.iter().cloned().fold(f32::MAX, f32::min);
    assert!(min > 120.0 && min < 220.0, "equation centred: {xs:?}");
    // Automatic numbers count.
    let auto = Math { nodes: wordcraft_doc::math::parse_linear("a=b#"), ..Default::default() };
    let mut n = 4;
    let _ = layout_equation(&auto, "", &rc(), true, 400.0, &MathProps::default(), &mut n);
    assert_eq!(n, 5);
    assert_eq!(auto_numbers(&auto, true), 1);
}

#[test]
fn math_alphabets() {
    assert_eq!(math_alnum('x', false, true, MScr::Roman), '𝑥');
    assert_eq!(math_alnum('h', false, true, MScr::Roman), 'ℎ');
    assert_eq!(math_alnum('R', false, false, MScr::DoubleStruck), 'ℝ');
    assert_eq!(math_alnum('α', false, true, MScr::Roman), '𝛼');
    assert_eq!(math_alnum('2', false, true, MScr::Roman), '2');
    assert_eq!(math_alnum('A', true, false, MScr::Roman), '𝐀');
}

#[test]
fn eqarr_rows_without_an_align_marker_are_centred() {
    use wordcraft_doc::math::*;
    // A row without `&` must not widen the alignment columns; it centres across the
    // array instead of right-aligning at the first column's edge.
    let rows: Vec<Vec<MNode>> = vec![vec![MNode::Run(MRun::new("mmm"))], vec![MNode::Run(MRun::new("x&yyyyyyyy"))]];
    let math = Math { nodes: vec![MNode::EqArr { rows }], ..Default::default() };
    let ls = lines(&lay_in(&math, "", true, 400.0, &MathProps::default()));
    assert_eq!(ls.len(), 2, "{ls:?}");
    let left: Vec<f32> = ls.iter().map(|(_, gs)| gs.iter().map(|(x, _)| *x).fold(f32::MAX, |a, b| if b < a { b } else { a })).collect();
    assert!(left.iter().all(|x| *x >= -0.5), "nothing hangs left of the array: {left:?}");
    // The short row starts right of the aligned row's first segment.
    assert!(ls[0].1.first().is_some_and(|(x, _)| *x > left[1] + 1.0), "short row centred, not flush left: {ls:?}");
}

#[test]
fn eqarr_row_without_an_align_marker_wider_than_the_columns_widens_the_array() {
    use wordcraft_doc::math::*;
    // The `&`-less row is wider than the aligned columns: it must not hang left of the
    // origin, the array is as wide as it, and the aligned rows centre under it.
    let rows: Vec<Vec<MNode>> = vec![vec![MNode::Run(MRun::new("bbbbbbbbbb"))], vec![MNode::Run(MRun::new("a&c"))]];
    let math = Math { nodes: vec![MNode::EqArr { rows }], ..Default::default() };
    let m = lay_in(&math, "", true, 400.0, &MathProps::default());
    let ls = lines(&m);
    assert_eq!(ls.len(), 2, "{ls:?}");
    let left: Vec<f32> = ls.iter().map(|(_, gs)| gs.iter().map(|(x, _)| *x).fold(f32::MAX, f32::min)).collect();
    assert!(left.iter().all(|x| *x >= -0.5), "nothing hangs left of the array: {left:?}");
    assert!(left[0].abs() < 0.5, "the wide row starts at the origin: {left:?}");
    let narrow = lay_in(
        &Math { nodes: vec![MNode::EqArr { rows: vec![vec![MNode::Run(MRun::new("a&c"))]] }], ..Default::default() },
        "",
        true,
        400.0,
        &MathProps::default(),
    );
    assert!(m.width > narrow.width + 1.0, "the array is as wide as its widest row: {} vs {}", m.width, narrow.width);
    assert!(left[1] > 1.0, "the aligned row is centred under the wide one: {left:?}");
}

#[test]
fn hostile_trees_never_panic() {
    use wordcraft_doc::math::*;
    // Deep nesting.
    let mut n = vec![MNode::Run(MRun::new("x"))];
    for _ in 0..500 {
        n = vec![MNode::Frac { kind: FracKind::Bar, num: n.clone(), den: Vec::new() }];
    }
    let _ = layout_equation(&Math { nodes: n, ..Default::default() }, "", &rc(), true, 400.0, &MathProps::default(), &mut 0);
    // Empty everything.
    let empty = vec![
        MNode::Nary { chr: '\0', lim_loc: None, grow: Some(true), sub_hide: false, sup_hide: false, sub: vec![], sup: vec![], e: vec![] },
        MNode::Delim { beg: Some('\u{10FFFF}'), end: None, sep: None, grow: true, shp_match: false, elems: vec![] },
        MNode::Matrix { rows: vec![vec![]], col_jc: vec![] },
        MNode::EqArr { rows: vec![] },
        MNode::Run(MRun { text: String::new(), size: Some(f32::NAN), ..Default::default() }),
    ];
    let m = layout_equation(&Math { nodes: empty, ..Default::default() }, "", &rc(), true, 400.0, &MathProps::default(), &mut 0);
    assert!(m.width.is_finite() && m.ascent.is_finite());
    // Breaks everywhere, odd wrap settings, no room at all.
    let runs: Vec<MNode> = (0..2000).map(|i| MNode::Run(MRun { brk: (i % 3 == 0).then_some((i % 256) as u8), ..MRun::new("−+=") })).collect();
    for avail in [0.0, f32::NAN, 1e9] {
        for brk_bin in [BrkBin::Before, BrkBin::After, BrkBin::Repeat] {
            let mp = MathProps { brk_bin, brk_bin_sub: BrkBinSub::PlusMinus, wrap_indent: f32::INFINITY, wrap_right: avail == 0.0 };
            let m = layout_equation(&Math { nodes: runs.clone(), ..Default::default() }, "", &rc(), true, avail, &mp, &mut 0);
            assert!(m.width.is_finite() && m.descent.is_finite());
        }
    }
}

/// Finding 23: each character's boundary in a run of text was found by walking the run's
/// glyphs from the first.
#[test]
fn a_long_text_run_lays_out_in_linear_time() {
    use crate::tests::linear::assert_linear_steps;
    use wordcraft_doc::math::*;
    let eq = |n: usize| Math { nodes: vec![MNode::Run(MRun { nor: true, ..MRun::new("a".repeat(n)) })], ..Default::default() };
    let lay = |m: &Math| drop(std::hint::black_box(lay_in(m, "", false, 1e6, &MathProps::default())));
    assert_linear_steps("math text run", 400, eq, lay);
}

/// #3208: a math font can lack `MathVariants` vertical-construction data for some n-ary
/// operator (not every open-source math font catalogs size variants for every one) while
/// still having the base glyph itself. The "no variant found" fallback used `want` (the
/// *target* size) standing in for the glyph's own natural size, which made the scale factor
/// `want / want` — always exactly 1.0 — silently undoing the "scale the nearest one" the
/// comment above it says it does, so the operator stayed at its plain, un-enlarged size in
/// display mode. It should instead scale from the glyph's own measured height.
#[test]
fn a_nary_operator_without_a_cataloged_size_variant_still_grows_in_display_mode() {
    let (face, table) = wordcraft_fonts::math::resolve_math("Cambria Math");
    let Some(t) = table else { return }; // no math font installed in this environment
    // Simulate a font whose MATH table has no vertical variants for this operator at all
    // (everything else about the table, including `display_operator_min_height`, stays real).
    let mut bare = (*t).clone();
    bare.vert = Default::default();
    let mut cx = Ctx {
        face,
        t: Some(Arc::new(bare)),
        upem: face.upem.max(1.0) as f32,
        base: 12.0,
        color: Rgb::BLACK,
        text_font: String::new(),
        nodes: 0,
        path: Vec::new(),
        node_stack: Vec::new(),
    };
    let n = MNode::Nary {
        chr: '∫',
        lim_loc: None,
        grow: None,
        sub_hide: true,
        sup_hide: true,
        sub: vec![],
        sup: vec![],
        e: vec![MNode::Run(MRun::new("x"))],
    };
    let display_st = St { level: 0, display: true, cramped: false, upright: false };
    let display_op = &cx.op_atoms(&n, display_st, 0)[0];
    let display_h = display_op.a + display_op.d;
    let inline_st = St { display: false, ..display_st };
    let inline_op = &cx.op_atoms(&n, inline_st, 0)[0];
    let inline_h = inline_op.a + inline_op.d;
    // `want` is itself capped around 1.2em (real variants are picked to be "about 1.2 em
    // tall", matching Word), so even a successful fallback grows only modestly, not
    // dramatically — the bug was that it didn't grow *at all* (exactly 1.0x, every time).
    assert!(
        display_h > inline_h * 1.03,
        "display {display_h} should be measurably taller than inline {inline_h} even with no cataloged size variant for this operator"
    );
}

/// #3208: real Word grows a display n-ary operator to match a taller operand (here, a
/// stacked fraction) even when the file has no `m:grow` on it at all — verified against
/// Word itself on a file this project's own `wordcraft-cli` wrote, where `m:grow` is never
/// written. (Also verified against Word for ∑ with the same operand shape, not asserted
/// here: the substitute math font this test environment resolves to only catalogs two very
/// far apart vertical size variants for ∑, no glyph assembly, so even its *non-grown*
/// display size already lands on the bigger of the two — there's no room left in this
/// particular font's data to show further growth for it specifically.) Measures the
/// operator's own glyph height directly, not the whole equation's: that also includes the
/// operand and so grows with a taller operand either way — the bug was that the *operator
/// itself* stayed small regardless.
#[test]
fn a_display_operator_grows_to_match_a_tall_operand_without_an_explicit_grow() {
    let (face, table) = wordcraft_fonts::math::resolve_math("Cambria Math");
    let Some(t) = table else { return }; // no math font installed in this environment
    let op_height = |e: Vec<MNode>| {
        let mut cx = Ctx {
            face,
            t: Some(t.clone()),
            upem: face.upem.max(1.0) as f32,
            base: 12.0,
            color: Rgb::BLACK,
            text_font: String::new(),
            nodes: 0,
            path: Vec::new(),
            node_stack: Vec::new(),
        };
        let n = MNode::Nary { chr: '∫', lim_loc: None, grow: None, sub_hide: true, sup_hide: true, sub: vec![], sup: vec![], e };
        let st = St { level: 0, display: true, cramped: false, upright: false };
        let op = &cx.op_atoms(&n, st, 0)[0];
        op.a + op.d
    };
    let small = op_height(vec![MNode::Run(MRun::new("x"))]);
    let tall = op_height(vec![MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b"))] }]);
    assert!(
        tall > small * 1.3,
        "operator height with a fraction operand {tall} should clearly exceed its height with a plain letter operand {small}"
    );
}

/// #3217 review: an explicit `m:grow="0"` keeps a display operator at its own size beside a tall
/// operand; only a file that doesn't say gets Word's display default.
#[test]
fn an_explicit_no_grow_keeps_a_display_operator_small_beside_a_tall_operand() {
    let (face, table) = wordcraft_fonts::math::resolve_math("Cambria Math");
    let Some(t) = table else { return }; // no math font installed in this environment
    let op_height = |grow: Option<bool>| {
        let mut cx = Ctx {
            face,
            t: Some(t.clone()),
            upem: face.upem.max(1.0) as f32,
            base: 12.0,
            color: Rgb::BLACK,
            text_font: String::new(),
            nodes: 0,
            path: Vec::new(),
            node_stack: Vec::new(),
        };
        let e = vec![MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b"))] }];
        let n = MNode::Nary { chr: '∫', lim_loc: None, grow, sub_hide: true, sup_hide: true, sub: vec![], sup: vec![], e };
        let st = St { level: 0, display: true, cramped: false, upright: false };
        let op = &cx.op_atoms(&n, st, 0)[0];
        op.a + op.d
    };
    let (unsaid, off, on) = (op_height(None), op_height(Some(false)), op_height(Some(true)));
    assert!(off < unsaid * 0.8, "explicit off {off} stays smaller than the default {unsaid}");
    assert!((unsaid - on).abs() < 0.01, "a file that doesn't say grows like an explicit on: {unsaid} vs {on}");
}

/// `scaled_fallback`'s target can be smaller than a glyph's own natural size (e.g. a tiny
/// `display_operator_min_height` on an unusual font): it must not shrink the glyph below
/// that natural size to chase the target down, only ever grow toward a *larger* one.
#[test]
fn scaled_fallback_never_shrinks_a_glyph_below_its_natural_size() {
    let (face, table) = wordcraft_fonts::math::resolve_math("Cambria Math");
    let Some(t) = table else { return }; // no math font installed in this environment
    let cx = Ctx {
        face,
        t: Some(t),
        upem: face.upem.max(1.0) as f32,
        base: 12.0,
        color: Rgb::BLACK,
        text_font: String::new(),
        nodes: 0,
        path: Vec::new(),
        node_stack: Vec::new(),
    };
    let gid = face.glyph_for('∫');
    let size = 12.0;
    let natural = cx.glyph_id(gid, size, cx.color);
    let shrunk = cx.scaled_fallback(gid, 0.1, size);
    assert!(
        shrunk.a + shrunk.d >= natural.a + natural.d - 0.01,
        "a target far smaller than the glyph's natural size should not shrink it: {} vs natural {}",
        shrunk.a + shrunk.d,
        natural.a + natural.d
    );
}

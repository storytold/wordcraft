use super::*;
use crate::math::{ColJc, FracKind};

#[test]
fn adjacent_radical_fraction_gap() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let mut failures = Vec::new();
    for display in [false, true] {
        for size in [9.0, 14.0, 20.0] {
            let mut cx = Ctx {
                face,
                t: None,
                upem: face.upem as f32,
                base: size,
                color: Rgb::BLACK,
                text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
                nodes: 0,
                path: Vec::new(),
                node_stack: Vec::new(),
            };
            let st = St { level: 0, display, frac_edges: FracEdges::Spaced, cramped: false, upright: false };
            for count in [2, 3] {
                let frac = MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b"))] };
                for body in [vec![frac.clone(); count], vec![MNode::Boxed { e: vec![frac] }; count]] {
                    let plain = cx.arg(&body, st.cramp(), 0);
                    let radical = cx.node(&MNode::Rad { deg: Vec::new(), deg_hide: true, e: body }, st, 0);
                    let bars = |b: &Bx| {
                        b.items
                            .iter()
                            .filter_map(|i| match i {
                                MItem::Rect { x, y, w, .. } => Some((*x, *y, *w)),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    };
                    let plain = bars(&plain);
                    let radical = bars(&radical);
                    assert_eq!(plain.len(), count);
                    assert_eq!(radical.len(), count + 1);
                    for i in 0..count - 1 {
                        let expected = plain[i + 1].0 - plain[i].0 - plain[i].2;
                        let actual = radical[i + 2].0 - radical[i + 1].0 - radical[i + 1].2;
                        assert!(expected > 0.0, "the ordinary fraction bars are separated");
                        assert!((radical[i + 1].1 - radical[i + 2].1).abs() < 0.001, "the bars share a baseline");
                        eprintln!("display={display} size={size} count={count} boundary={i} expected={expected} actual={actual}");
                        if (actual - expected).abs() >= 0.001 {
                            failures.push((display, size, count, i, expected, actual));
                        }
                    }
                    assert!((radical[0].0 - radical[1].0).abs() < 0.001, "the first edge stays radical-facing");
                    let last = radical.last().unwrap();
                    assert!((radical[0].0 + radical[0].2 - last.0 - last.2).abs() < 0.001, "the last edge stays radical-facing");
                }
            }
        }
    }
    assert!(failures.is_empty(), "inner fraction edges lose their ordinary spacing: {failures:?}");
}

#[test]
fn radical_row_side_context() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let mut cx = Ctx {
        face,
        t: None,
        upem: face.upem as f32,
        base: 14.0,
        color: Rgb::BLACK,
        text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
        nodes: 0,
        path: Vec::new(),
        node_stack: Vec::new(),
    };
    let st = St { level: 0, display: false, frac_edges: FracEdges::Spaced, cramped: true, upright: false };
    let tight = St { frac_edges: FracEdges::Tight, ..st };
    let frac = MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b"))] };
    let text = MNode::Run(MRun::new("x"));
    let plain = cx.node(&frac, st, 0).w;
    let pad = cx.em(st) * 0.08;
    let middle = cx.row(&[text.clone(), frac.clone(), text.clone()], tight, 0, 0);
    assert!((middle[1].0.w - plain).abs() < 0.001, "a fraction between ordinary atoms retains both margins");
    let first = cx.row(&[frac.clone(), text.clone()], tight, 0, 0);
    let last = cx.row(&[text, frac], tight, 0, 0);
    let bar_x = |b: &Bx| {
        b.items
            .iter()
            .find_map(|i| match i {
                MItem::Rect { x, .. } => Some(*x),
                _ => None,
            })
            .unwrap()
    };
    assert!((first[0].0.w - (plain - pad)).abs() < 0.001, "only the leading margin is removed");
    assert!((last[1].0.w - (plain - pad)).abs() < 0.001, "only the trailing margin is removed");
    assert!(bar_x(&first[0].0).abs() < 0.001, "leading bar aligns with the radical");
    assert!((bar_x(&last[1].0) - pad * 0.5).abs() < 0.001, "inner-facing bar edge retains its ordinary inset");
}

#[test]
fn radical_container_edges() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let mut cx = Ctx {
        face,
        t: None,
        upem: face.upem as f32,
        base: 14.0,
        color: Rgb::BLACK,
        text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
        nodes: 0,
        path: Vec::new(),
        node_stack: Vec::new(),
    };
    let st = St { level: 0, display: false, frac_edges: FracEdges::Spaced, cramped: true, upright: false };
    let frac = || MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b"))] };
    let text = || vec![MNode::Run(MRun::new("x"))];
    let pad = cx.em(st) * 0.08;
    let cases = [
        ("matrix", MNode::Matrix { rows: vec![vec![vec![frac()], vec![frac()], vec![frac()]]], col_jc: vec![] }, 1.0, 1.0),
        (
            "nary",
            MNode::Nary { chr: '∑', lim_loc: None, grow: false, sub_hide: true, sup_hide: true, sub: vec![], sup: vec![], e: vec![frac()] },
            0.0,
            1.0,
        ),
        ("function", MNode::Func { name: text(), e: vec![frac()] }, 0.0, 1.0),
        ("function name", MNode::Func { name: vec![frac()], e: text() }, 1.0, 0.0),
        ("postscript", MNode::Script { kind: ScriptKind::Sup, base: vec![frac()], sub: vec![], sup: text() }, 1.0, 0.0),
        ("prescript", MNode::Script { kind: ScriptKind::Pre, base: vec![frac()], sub: text(), sup: text() }, 0.0, 1.0),
        ("linear", MNode::Frac { kind: FracKind::Linear, num: vec![frac()], den: vec![frac()] }, 1.0, 1.0),
    ];
    let mut failures = Vec::new();
    for (name, node, left, right) in cases {
        let plain = cx.node(&node, st, 0).w;
        for (edges, removed) in [(FracEdges::LeftTight, left), (FracEdges::RightTight, right), (FracEdges::Tight, left + right)] {
            let packed = cx.node(&node, St { frac_edges: edges, ..st }, 0).w;
            if (plain - packed - removed * pad).abs() >= 0.001 {
                failures.push(format!("{name} {edges:?}: removed {}, expected {}", plain - packed, removed * pad));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn radical_matrix_alignment() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let mut cx = Ctx {
        face,
        t: None,
        upem: face.upem as f32,
        base: 14.0,
        color: Rgb::BLACK,
        text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
        nodes: 0,
        path: Vec::new(),
        node_stack: Vec::new(),
    };
    let st = St { level: 0, display: false, frac_edges: FracEdges::Spaced, cramped: true, upright: false };
    let frac = MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b"))] };
    let bars = |b: &Bx| {
        b.items
            .iter()
            .filter_map(|i| match i {
                MItem::Rect { w, .. } => Some(*w),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let plain = bars(&cx.node(&frac, st, 0))[0];
    let pad = cx.em(st) * 0.08;
    let mut failures = Vec::new();
    for count in [1, 3] {
        for jc in [ColJc::Left, ColJc::Center, ColJc::Right] {
            for edges in [FracEdges::Spaced, FracEdges::LeftTight, FracEdges::RightTight, FracEdges::Tight] {
                let rows = vec![vec![vec![frac.clone()]; count], vec![vec![MNode::Run(MRun::new("xxxxxxxx"))]; count]];
                let matrix = MNode::Matrix { rows, col_jc: vec![jc; count] };
                let packed = bars(&cx.node(&matrix, St { frac_edges: edges, ..st }, 0));
                assert_eq!(packed.len(), count);
                for (i, width) in packed.into_iter().enumerate() {
                    let left = i == 0 && jc == ColJc::Left && matches!(edges, FracEdges::LeftTight | FracEdges::Tight);
                    let right = i + 1 == count && jc == ColJc::Right && matches!(edges, FracEdges::RightTight | FracEdges::Tight);
                    let expected = plain - (usize::from(left) + usize::from(right)) as f32 * pad * 0.5;
                    if (width - expected).abs() >= 0.001 {
                        failures.push(format!("columns={count} {jc:?} {edges:?} column={i}: bar={width}, expected={expected}"));
                    }
                }
            }
        }
    }
    for size in [14.0, 15.0] {
        let run = MNode::Run(MRun { size: Some(size), ..MRun::new("b") });
        let other = MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![run] };
        let other_plain = bars(&cx.node(&other, st, 0))[0];
        let matrix = MNode::Matrix { rows: vec![vec![vec![frac.clone()]], vec![vec![other]]], col_jc: vec![ColJc::Center] };
        let packed = bars(&cx.node(&matrix, St { frac_edges: FracEdges::Tight, ..st }, 0));
        let removed = if size == 14.0 { pad } else { 0.0 };
        for (width, plain) in packed.into_iter().zip([plain, other_plain]) {
            if (width - (plain - removed)).abs() >= 0.001 {
                failures.push(format!("competing centered cells, size={size}: bar={width}, expected={}", plain - removed));
            }
        }
    }
    assert!(failures.is_empty(), "aligned interior edges must retain spacing: {failures:#?}");
}

#[test]
fn radical_delimiter_elements() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let mut cx = Ctx {
        face,
        t: None,
        upem: face.upem as f32,
        base: 14.0,
        color: Rgb::BLACK,
        text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
        nodes: 0,
        path: Vec::new(),
        node_stack: Vec::new(),
    };
    let st = St { level: 0, display: false, frac_edges: FracEdges::Spaced, cramped: true, upright: false };
    let frac = MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b"))] };
    let bars = |b: &Bx| {
        b.items
            .iter()
            .filter_map(|i| match i {
                MItem::Rect { w, .. } => Some(*w),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let plain = bars(&cx.node(&frac, st, 0))[0];
    let pad = cx.em(st) * 0.08;
    let mut failures = Vec::new();
    for count in [1, 2, 3] {
        for sep in [None, Some('|')] {
            for edges in [FracEdges::Spaced, FracEdges::LeftTight, FracEdges::RightTight, FracEdges::Tight] {
                let delim =
                    MNode::Delim { beg: Some('('), end: Some(')'), sep, grow: false, shp_match: false, elems: vec![vec![frac.clone()]; count] };
                let packed = bars(&cx.node(&delim, St { frac_edges: edges, ..st }, 0));
                assert_eq!(packed.len(), count);
                for (i, width) in packed.into_iter().enumerate() {
                    let left = i == 0 && matches!(edges, FracEdges::LeftTight | FracEdges::Tight);
                    let right = i + 1 == count && matches!(edges, FracEdges::RightTight | FracEdges::Tight);
                    let expected = plain - (usize::from(left) + usize::from(right)) as f32 * pad * 0.5;
                    if (width - expected).abs() >= 0.001 {
                        failures.push(format!("elements={count} separator={sep:?} {edges:?} element={i}: bar={width}, expected={expected}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "interior delimiter element edges must retain spacing: {failures:#?}");
}

#[test]
fn enclosed_fraction_edges() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    for display in [false, true] {
        for size in [9.0, 14.0, 20.0] {
            let mut cx = Ctx {
                face,
                t: None,
                upem: face.upem as f32,
                base: size,
                color: Rgb::BLACK,
                text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
                nodes: 0,
                path: Vec::new(),
                node_stack: Vec::new(),
            };
            let st = St { level: 0, display, frac_edges: FracEdges::Spaced, cramped: false, upright: false };
            let tight = St { frac_edges: FracEdges::Tight, ..st };
            assert_eq!(tight.frac().frac_edges, FracEdges::Spaced, "nested arguments keep their spacing");
            assert_eq!(tight.script().frac_edges, FracEdges::Spaced, "scripts keep their spacing");
            assert_eq!(tight.cramp().frac_edges, FracEdges::Tight, "cramping preserves the surrounding edge context");
            let no_bar = MNode::Frac { kind: FracKind::NoBar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b"))] };
            assert_eq!(cx.node(&no_bar, tight, 0).w, cx.node(&no_bar, st, 0).w, "barless fractions keep their spacing");
            let frac = MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b+1"))] };
            let rad = MNode::Rad { deg: Vec::new(), deg_hide: true, e: vec![frac.clone()] };
            let result = cx.node(&rad, st, 0);
            let bars: Vec<_> = result
                .items
                .iter()
                .filter_map(|i| match i {
                    MItem::Rect { x, w, .. } => Some((*x, *w)),
                    _ => None,
                })
                .collect();
            assert_eq!(bars.len(), 2);
            assert!((bars[0].0 - bars[1].0).abs() < 0.001, "radical and fraction start together: {bars:?}");
            assert!((bars[0].1 - bars[1].1).abs() < 0.001, "radical and fraction end together: {bars:?}");

            let nested = MNode::Frac { kind: FracKind::Bar, num: vec![frac.clone()], den: vec![MNode::Run(MRun::new("c"))] };
            let plain = cx.node(&nested, st, 0);
            let inside = cx.node(&MNode::Rad { deg: Vec::new(), deg_hide: true, e: vec![nested] }, st, 0);
            let rule_width = |b: &Bx, index| {
                b.items
                    .iter()
                    .filter_map(|i| match i {
                        MItem::Rect { w, .. } => Some(*w),
                        _ => None,
                    })
                    .nth(index)
                    .unwrap()
            };
            assert_eq!(rule_width(&inside, 1), rule_width(&plain, 0), "nested fraction spacing stays unchanged");

            for body in [
                vec![MNode::Run(MRun::new("(")), frac.clone(), MNode::Run(MRun::new(")"))],
                vec![MNode::Delim { beg: Some('('), end: Some(')'), sep: None, grow: false, shp_match: false, elems: vec![vec![frac.clone()]] }],
            ] {
                let result = cx.node(&MNode::Rad { deg: Vec::new(), deg_hide: true, e: body }, st, 0);
                let (bar_x, bar_w) = result
                    .items
                    .iter()
                    .filter_map(|i| match i {
                        MItem::Rect { x, w, .. } => Some((*x, *w)),
                        _ => None,
                    })
                    .nth(1)
                    .unwrap();
                let (left, right) = ('(', ')');
                let glyph_x = |c: char| {
                    result
                        .items
                        .iter()
                        .find_map(|i| match i {
                            MItem::Glyphs { glyphs, text, .. } if text == &c.to_string() => glyphs.first().map(|g| g.1),
                            _ => None,
                        })
                        .unwrap()
                };
                let left_end = glyph_x(left) + face.advance(face.glyph_for(left)) as f32 * size / face.upem as f32;
                assert!((bar_x - left_end).abs() < 0.001, "no fraction margin after opening bracket");
                assert!((bar_x + bar_w - glyph_x(right)).abs() < 0.001, "no fraction margin before closing bracket");
            }
        }
    }
}

fn edge_test_ctx(face: FaceRef) -> Ctx {
    Ctx {
        face,
        t: None,
        upem: face.upem as f32,
        base: 14.0,
        color: Rgb::BLACK,
        text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
        nodes: 0,
        path: Vec::new(),
        node_stack: Vec::new(),
    }
}

#[test]
fn edge_scan_budget_is_shared() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let mut cx = edge_test_ctx(FaceRef::of(&font));
    let opening = [MNode::Run(MRun::new("⟨((")), MNode::Run(MRun::new("x"))];
    let closing = [MNode::Run(MRun::new("x")), MNode::Run(MRun::new("))⟩"))];
    // Two nodes plus three fence characters and the first non-fence character.
    const SCAN_COST: usize = 6;
    const REPEATS: usize = 2;
    cx.nodes = MAX_NODES - REPEATS * SCAN_COST - 1;
    for remaining in (0..REPEATS).rev() {
        assert_eq!(cx.edge_index(opening.iter().enumerate(), Cls::Open), Some(1));
        assert_eq!(cx.nodes, MAX_NODES - remaining * SCAN_COST - 1, "charge characters, not UTF-8 bytes or nodes alone");
    }
    let visited = std::cell::Cell::new(0);
    let nodes = opening.iter().enumerate().inspect(|_| visited.set(visited.get() + 1));
    assert_eq!(cx.edge_index(nodes, Cls::Open), None, "the next scan exhausts before classifying its first character");
    assert_eq!(cx.nodes, MAX_NODES);
    assert_eq!(visited.get(), 1, "do not inspect the remaining nodes after exhaustion");
    assert_eq!(cx.edge_index(opening.iter().enumerate(), Cls::Open), None);
    assert_eq!(cx.nodes, MAX_NODES, "an exhausted lookup spends no further budget");

    cx.nodes = MAX_NODES - SCAN_COST;
    assert_eq!(cx.edge_index(closing.iter().enumerate().rev(), Cls::Close), Some(0));
    assert_eq!(cx.nodes, MAX_NODES, "reverse discovery charges the same work");
    assert_eq!(cx.edge_index(closing.iter().enumerate().rev(), Cls::Close), None);
    assert_eq!(cx.nodes, MAX_NODES);
}

#[test]
fn radical_scan_exhaustion() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let mut cx = edge_test_ctx(FaceRef::of(&font));
    let st = St { level: 0, display: false, frac_edges: FracEdges::Spaced, cramped: false, upright: false };
    let fraction = MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new("a"))], den: vec![MNode::Run(MRun::new("b"))] };
    let body = [MNode::Rad { deg: vec![], deg_hide: true, e: vec![MNode::Run(MRun::new("⟨((")), fraction] }];
    let rules = |b: &Bx| b.items.iter().filter(|i| matches!(i, MItem::Rect { .. })).count();
    assert_eq!(rules(&cx.arg(&body, st, 0)), 2, "ordinary layout draws the radical and fraction rules");

    // One outer node, then the fence node and its three characters exhaust inspection.
    cx.nodes = MAX_NODES - 5;
    let limited = cx.arg(&body, st, 0);
    assert_eq!(rules(&limited), 0, "the exhausted radicand is not rendered");
    assert_eq!(cx.nodes, MAX_NODES + 1, "the existing row cap stops its first render attempt");
    let spent = cx.nodes;
    assert!(cx.row(&body, St { frac_edges: FracEdges::Tight, ..st }, 0, 0).is_empty());
    assert_eq!(cx.nodes, spent, "an exhausted row returns before edge discovery");
}

#[test]
fn radical_eq_array_insets() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let mut cx = edge_test_ctx(FaceRef::of(&font));
    let fraction = |text| MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Run(MRun::new(text))], den: vec![MNode::Run(MRun::new("b"))] };
    let bars = |b: &Bx| {
        b.items
            .iter()
            .filter_map(|i| match i {
                MItem::Rect { x, w, .. } => Some((*x, *w)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for display in [false, true] {
        let st = St { level: 0, display, frac_edges: FracEdges::Spaced, cramped: false, upright: false };
        let short = fraction("a");
        let wide = fraction("abcdefgh");
        let short_box = cx.arg(std::slice::from_ref(&short), st.cramp(), 0);
        let wide_box = cx.arg(std::slice::from_ref(&wide), st.cramp(), 0);
        let short_bar = bars(&short_box)[0];
        let wide_bar = bars(&wide_box)[0];
        assert!(short_bar.0 > 0.0, "the ordinary fraction has an inset");
        assert!(wide_box.w > short_box.w, "the short row must be centered inside a wider array");
        let array = MNode::EqArr { rows: vec![vec![short], vec![wide]] };
        let plain = cx.arg(std::slice::from_ref(&array), st.cramp(), 0);
        let radical = cx.arg(&[MNode::Rad { deg: vec![], deg_hide: true, e: vec![array] }], st, 0);
        let plain_bars = bars(&plain);
        let packed_bars = bars(&radical);
        assert_eq!(plain_bars.len(), 2);
        assert_eq!(packed_bars.len(), 3);
        let centered = (plain.w - short_box.w) / 2.0;
        assert!(centered > 0.0);
        assert!((plain_bars[0].0 - centered - short_bar.0).abs() < 0.001, "the ordinary array centers the short fraction");
        assert!((packed_bars[0].1 - plain.w).abs() < 0.001, "the radical keeps the array's ordinary frame width");
        assert!((packed_bars[1].1 - short_bar.1).abs() < 0.001, "the inset short row retains its ordinary bar width");
        assert!((packed_bars[2].1 - wide_bar.1).abs() < 0.001, "the wider row conservatively retains both margins");
        let offset = packed_bars[0].0;
        assert!((packed_bars[1].0 - offset - centered - short_bar.0).abs() < 0.001, "centering does not remove the short row's bar inset");
        assert!((packed_bars[2].0 - offset - wide_bar.0).abs() < 0.001, "the wide row keeps its leading inset");
    }
}

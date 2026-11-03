use super::*;

#[test]
fn mixed_size_script_base() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    for size in [9.0, 14.0, 20.0] {
        for ratio in [0.7, 1.0, 1.4] {
            for cramped in [false, true] {
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
                let st = St { level: 0, display: true, cramped, upright: true };
                let run = |text: &str, point_size| {
                    MNode::Run(MRun { size: Some(point_size), sty: Some(wordcraft_doc::math::MSty::Plain), ..MRun::new(text) })
                };
                let base = vec![run("D", size * ratio)];
                let sup = vec![run("3", size)];
                let b = cx.arg(&base, st, 0);
                let p = cx.arg(&sup, st.script(), 0);
                let shift = cx.k(if cramped { C::SuperscriptShiftUpCramped } else { C::SuperscriptShiftUp }, st);
                let mut expected = shift.max(p.d + cx.k(C::SuperscriptBottomMin, st));
                if ratio != 1.0 {
                    expected = expected.max(b.a - cx.k(C::SuperscriptBaselineDropMax, st));
                }
                let result = cx.node(&MNode::Script { kind: ScriptKind::Sup, base, sub: Vec::new(), sup }, st, 0);
                let actual = result
                    .items
                    .iter()
                    .find_map(|item| match item {
                        MItem::Glyphs { text, glyphs, .. } if text == "3" => glyphs.first().map(|g| g.2),
                        _ => None,
                    })
                    .unwrap();
                assert!((actual - expected).abs() < 0.001, "size={size}, ratio={ratio}, cramped={cramped}: {actual} vs {expected}");
            }
        }
    }
}

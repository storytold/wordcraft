use super::*;
use crate::math::{Arc, Ctx, FaceRef, FontDb, FracKind, MItem, MNode, MRun, MathTable, Rgb};

#[test]
fn display_script_fraction_size() {
    let st = St { level: 0, display: true, fraction_size: FracSize::Text, cramped: false, upright: false };
    assert_eq!(st.script().frac().level, 1, "display script fractions retain their argument size");
    let small = St { fraction_size: FracSize::Display, ..st };
    assert_eq!(small.frac().level, 0, "the outer display fraction stays full size");
    assert_eq!(small.frac().frac().level, 1, "smallFrac shrinks nested arguments");
    assert_eq!(small.script().frac().level, 2, "smallFrac also shrinks display script fractions");
    let inline = St { display: false, fraction_size: FracSize::Script, ..st };
    assert_eq!(inline.script().frac().level, 2, "inline script fractions shrink their arguments");
}

#[test]
fn nested_fraction_size() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let mut table = MathTable::default();
    table.script_percent = 73.0;
    table.script_script_percent = 60.0;
    let text = |s: &str| vec![MNode::Run(MRun::new(s))];
    let node = MNode::Frac { kind: FracKind::Bar, num: vec![MNode::Frac { kind: FracKind::Bar, num: text("a"), den: text("b") }], den: text("c") };
    for (display, fraction_size, inner_scale, outer_scale) in
        [(true, FracSize::Text, 1.0, 1.0), (true, FracSize::Display, 0.73, 1.0), (false, FracSize::Script, 0.6, 0.73)]
    {
        for size in [9.0, 14.0, 20.0] {
            let mut cx = Ctx {
                face,
                t: Some(Arc::new(table.clone())),
                upem: face.upem as f32,
                base: size,
                color: Rgb::BLACK,
                text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
                nodes: 0,
                path: Vec::new(),
                node_stack: Vec::new(),
            };
            let st = St { level: 0, display, fraction_size, cramped: false, upright: false };
            let b = cx.arg(std::slice::from_ref(&node), st, 0);
            let sizes: Vec<f32> = b
                .items
                .iter()
                .filter_map(|item| match item {
                    MItem::Glyphs { size, .. } => Some(*size),
                    _ => None,
                })
                .collect();
            let expected = vec![size * inner_scale, size * inner_scale, size * outer_scale];
            assert_eq!(sizes.len(), expected.len());
            for (actual, expected) in sizes.iter().zip(expected) {
                assert!((actual - expected).abs() < 0.001, "display={display}: {sizes:?}");
            }
        }
    }
}

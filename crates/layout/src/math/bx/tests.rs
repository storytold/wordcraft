use super::*;
use crate::math::{Arc, Ctx, FaceRef, FontDb, FracKind, MItem, MNode, MRun, MathTable, Rgb, ScriptKind};

#[test]
fn display_script_fraction_size() {
    let st = St { level: 0, display: true, fraction_size: FracSize::Text, cramped: false, upright: false };
    assert_eq!(st.script().frac().level, 1, "display script fractions retain their argument size");
    let small = St { fraction_size: FracSize::Display, ..st };
    assert_eq!(small.frac().level, 0, "the outer display fraction stays full size");
    assert_eq!(small.frac().frac().level, 1, "smallFrac shrinks nested arguments");
    assert_eq!(small.script().frac().level, 1, "a standalone display script fraction keeps script-sized arguments");
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

#[test]
fn script_fraction_glyph_sizes() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let mut table = MathTable::default();
    table.script_percent = 73.0;
    table.script_script_percent = 60.0;
    let text = |s: &str| vec![MNode::Run(MRun::new(s))];
    // Expected base/argument scales distinguish script entry from fraction nesting.
    for (display, small_frac, nested, base_scale, arg_scale) in [
        (true, false, false, 1.0, 0.73),
        (true, true, false, 1.0, 0.73),
        (true, false, true, 1.0, 0.73),
        (true, true, true, 1.0, 0.60),
        (false, false, false, 1.0, 0.60),
        (false, true, false, 1.0, 0.60),
        (false, false, true, 0.73, 0.60),
        (false, true, true, 0.73, 0.60),
    ] {
        for kind in [FracKind::Bar, FracKind::NoBar] {
            let script = MNode::Script {
                kind: ScriptKind::Sup,
                base: text("x"),
                sub: Vec::new(),
                sup: vec![MNode::Frac { kind, num: text("a"), den: text("b") }],
            };
            let node = if nested { MNode::Frac { kind, num: vec![script], den: text("c") } } else { script };
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
                let fraction_size = if display { if small_frac { FracSize::Display } else { FracSize::Text } } else { FracSize::Script };
                let st = St { level: 0, display, fraction_size, cramped: false, upright: false };
                let result = cx.arg(std::slice::from_ref(&node), st, 0);
                let actual: Vec<f32> = result
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        MItem::Glyphs { size, .. } => Some(*size),
                        _ => None,
                    })
                    .collect();
                let mut expected = vec![size * base_scale, size * arg_scale, size * arg_scale];
                if nested {
                    expected.push(size * base_scale);
                }
                assert_eq!(actual.len(), expected.len(), "display={display} smallFrac={small_frac} nested={nested} kind={kind:?}");
                for (actual, expected) in actual.iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 0.001,
                        "display={display} smallFrac={small_frac} nested={nested} kind={kind:?} size={size}: {actual} != {expected}"
                    );
                }
            }
        }
    }
}

use super::*;
use std::sync::Arc;
use wordcraft_fonts::math::{Construction, MathTable};

#[test]
fn display_operator_minimum() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let upem = face.upem as f32;
    let base_gid = face.glyph_for('S');
    let variants = [face.glyph_for('a'), face.glyph_for('b'), face.glyph_for('c')];
    let mut table = MathTable::default();
    table.display_operator_min_height = upem * 1.5;
    table.vert.insert(
        base_gid,
        Construction { variants: vec![(variants[0], upem), (variants[1], upem * 1.4), (variants[2], upem * 1.6)], ..Default::default() },
    );
    let node = MNode::Nary {
        chr: 'S',
        lim_loc: None,
        grow: false,
        sub_hide: true,
        sup_hide: true,
        sub: Vec::new(),
        sup: Vec::new(),
        e: vec![MNode::Run(MRun::new("x"))],
    };
    for display in [false, true] {
        for size in [9.0, 14.0, 20.0] {
            let mut cx = Ctx {
                face,
                t: Some(Arc::new(table.clone())),
                upem,
                base: size,
                color: Rgb::BLACK,
                text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
                nodes: 0,
                path: Vec::new(),
                node_stack: Vec::new(),
            };
            let st = St { level: 0, display, cramped: false, upright: false };
            let boxes = cx.op_atoms(&node, st, 0);
            let MItem::Glyphs { size: actual_size, glyphs, .. } = &boxes[0].items[0] else { panic!("operator glyph missing") };
            let expected = if display { variants[2] } else { base_gid };
            assert_eq!(glyphs[0].0, expected, "display minimum changes only display operators");
            assert_eq!(*actual_size, size, "the chosen variant keeps its font size");
        }
    }
}

#[test]
fn operator_missing_math_data() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let gid = face.glyph_for('S');
    assert_ne!(gid, 0, "the control needs a real base glyph");
    let mut without_construction = MathTable::default();
    without_construction.display_operator_min_height = face.upem as f32 * 1.5;
    let mut empty_construction = without_construction.clone();
    empty_construction.vert.insert(gid, Construction::default());
    for table in [None, Some(without_construction), Some(empty_construction)] {
        for display in [false, true] {
            for grow in [false, true] {
                let size = wordcraft_doc::styles::StyleSheet::default().resolve_char(None, &Default::default()).size;
                let mut cx = Ctx {
                    face,
                    t: table.clone().map(Arc::new),
                    upem: face.upem as f32,
                    base: size,
                    color: Rgb::BLACK,
                    text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
                    nodes: 0,
                    path: Vec::new(),
                    node_stack: Vec::new(),
                };
                let node = MNode::Nary {
                    chr: 'S',
                    lim_loc: None,
                    grow,
                    sub_hide: true,
                    sup_hide: true,
                    sub: Vec::new(),
                    sup: Vec::new(),
                    e: vec![MNode::Run(MRun::new("x"))],
                };
                let st = St { level: 0, display, cramped: false, upright: false };
                let boxes = cx.op_atoms(&node, st, 0);
                let operator = boxes.first().expect("operator box");
                let MItem::Glyphs { glyphs, size: actual, face: drawn, .. } = operator.items.first().expect("operator glyph") else {
                    panic!("expected a base glyph")
                };
                assert_eq!(*drawn, face);
                assert_eq!(glyphs.first().unwrap().0, gid);
                assert_eq!(*actual, size, "missing constructions do not rescale the base glyph");
                assert!((operator.w - face.advance(gid) as f32 * size / face.upem as f32).abs() < 0.001);
            }
        }
    }
}

use super::*;
use crate::math::{FaceRef, FontDb, Rgb};
use std::sync::Arc;

fn script_kern_bytes(base: u16, script: u16) -> Vec<u8> {
    let gi = 10 + 8 + (C::RadicalKernAfterDegree as usize + 1) * 4 + 2;
    let ki = gi + 8;
    let mut bytes = vec![0u8; ki + 40];
    let mut put = |at, value: u16| bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
    put(0, 1);
    put(4, 10);
    put(6, gi as u16);
    put(10, 73);
    put(12, 60);
    put(gi + 6, 8);
    put(ki, 20);
    put(ki + 2, 2);
    let mut glyphs = [base, script];
    glyphs.sort_unstable();
    for (i, glyph) in glyphs.into_iter().enumerate() {
        let corner = if glyph == base { 2 } else { 1 };
        put(ki + 4 + i * 8 + corner * 2, if glyph == base { 28 } else { 34 });
        put(ki + 24 + i * 2, glyph);
    }
    put(ki + 20, 1);
    put(ki + 22, 2);
    put(ki + 30, (-100i16) as u16);
    put(ki + 36, (-50i16) as u16);
    bytes
}

#[test]
fn subscript_math_kerning() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let font = db.face(wordcraft_fonts::DEFAULT_FAMILY, "Regular");
    let face = FaceRef::of(&font);
    let bytes = script_kern_bytes(face.glyph_for('V') as u16, face.glyph_for('a') as u16);
    for size in [9.0, 14.0, 20.0] {
        let mut cx = Ctx {
            face,
            t: Some(Arc::new(wordcraft_fonts::math::parse(&bytes).unwrap())),
            upem: face.upem as f32,
            base: size,
            color: Rgb::BLACK,
            text_font: wordcraft_fonts::DEFAULT_FAMILY.into(),
            nodes: 0,
            path: Vec::new(),
            node_stack: Vec::new(),
        };
        let st = St { level: 0, display: true, frac_edges: FracEdges::Spaced, cramped: false, upright: false };
        for ratio in [1.0, 1.4] {
            let base_size = size * ratio;
            let base = cx.glyphs(face, "V", base_size, 0, Rgb::BLACK, (false, false));
            let script_size = cx.size(st.script());
            let sub = cx.glyphs(face, "a", script_size, 1, Rgb::BLACK, (false, false));
            let sub_x = |b: &Bx| {
                b.items
                    .iter()
                    .find_map(|i| match i {
                        MItem::Glyphs { glyphs, text, .. } if text == "a" => glyphs.first().map(|g| g.1),
                        _ => None,
                    })
                    .unwrap()
            };
            for boxed_script in [false, true] {
                let mut sub = sub.clone();
                if boxed_script {
                    sub.glyph = None;
                }
                let result = cx.attach(base.clone(), Some(sub), None, st, true);
                let kern = if ratio == 1.0 { (-100.0 * base_size - if boxed_script { 0.0 } else { 50.0 * script_size }) / cx.upem } else { 0.0 };
                assert!((sub_x(&result) - (base.w + kern).max(0.0)).abs() < 0.001, "font corner kerning positions the subscript");
            }
            let mut boxed_base = base.clone();
            boxed_base.glyph = None;
            let result = cx.attach(boxed_base, Some(sub), None, st, false);
            assert_eq!(sub_x(&result), base.w, "a compound base keeps its existing script attachment");
        }
        let mut stepped = bytes.clone();
        let gi = u16::from_be_bytes([stepped[6], stepped[7]]) as usize;
        let ki = gi + 8;
        stepped.resize(ki + 56, 0);
        let mut put = |at, value: i16| stepped[at..at + 2].copy_from_slice(&value.to_be_bytes());
        let mut glyphs = [face.glyph_for('V') as u16, face.glyph_for('a') as u16];
        glyphs.sort_unstable();
        let script_index = glyphs.iter().position(|g| *g == face.glyph_for('a') as u16).unwrap();
        put(ki + 4 + script_index * 8 + 2, 50);
        put(ki + 28, 2);
        for (i, value) in [0i16, 300, -100, -300, 50].into_iter().enumerate() {
            put(ki + 30 + i * 4, value);
        }
        put(ki + 50, 0);
        put(ki + 52, -50);
        cx.t = Some(Arc::new(wordcraft_fonts::math::parse(&stepped).unwrap()));
        let scale = size / cx.upem;
        let mut base = cx.glyphs(face, "V", size, 0, Rgb::BLACK, (false, false));
        base.d = 200.0 * scale;
        let mut sub = cx.glyphs(face, "a", cx.size(st.script()), 1, Rgb::BLACK, (false, false));
        sub.a = 600.0 * scale;
        for (drop, expected) in [(200.0, -100.0), (500.0, -300.0)] {
            let kern = cx.sub_kern(&base, &sub, drop * scale);
            let expected = expected * scale - 50.0 * cx.size(st.script()) / cx.upem;
            assert!((kern - expected).abs() < 0.001, "choose the smaller sum at the two touching heights");
        }
    }
}

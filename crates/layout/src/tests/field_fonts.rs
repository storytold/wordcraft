//! A field result shaped in more than one font: a date picture like `yyyy年M月d日` gives digits in
//! the Latin font and 年月日 in the East Asian one. Each glyph must be drawn in the font it was
//! shaped with; one cluster per object drew them all in the first font's face, with the other
//! fonts' glyph ids (wrong glyphs on screen, and a PDF face krilla couldn't subset). Assertions
//! compare with the same text typed, never font names (fonts differ between machines; CJK fonts
//! may be missing, and then the result is in one font and the test still holds).

use crate::para::{ClKind, Cluster};
use crate::*;

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn first_para(l: &DocLayout) -> Arc<ParaLayout> {
    l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap()
}

/// The face and glyph id of every glyph of `cs`.
fn drawn<'a>(pl: &ParaLayout, cs: impl Iterator<Item = &'a Cluster>) -> Vec<(String, u32)> {
    cs.flat_map(|c| {
        let face = pl.styles.get(c.style as usize).map(|s| format!("{} {}", s.face.family, s.face.style)).unwrap_or_default();
        pl.glyphs.get(c.g0 as usize..c.g1 as usize).unwrap_or_default().iter().map(move |g| (face.clone(), g.gid))
    })
    .collect()
}

const RESULT: &str = "2026年11月3日";

/// "x" followed by a field showing [`RESULT`]; the field's clusters start at byte 1.
fn with_field() -> Document {
    let mut d = Document::from_text("x");
    let field =
        InlineObject::Field { instr: "MERGEFIELD d".into(), result: RESULT.into(), locked: false, dirty: false, code: false, form: String::new() };
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().insert_object(1, field, &CharProps::default()).unwrap();
    d
}

#[test]
fn a_field_result_draws_each_glyph_in_the_font_it_was_shaped_with() {
    let typed = first_para(&lay(&Document::from_text(RESULT)));
    let pl = first_para(&lay(&with_field()));
    let field: Vec<&Cluster> = pl.clusters.iter().filter(|c| c.start == 1).collect();
    assert!(!field.is_empty());
    assert!(field.iter().all(|c| c.kind == ClKind::Text), "text clusters");
    assert!(field.iter().rev().skip(1).all(|c| !c.break_after), "no line break inside the result");
    assert_eq!(drawn(&pl, field.iter().copied()), drawn(&typed, typed.clusters.iter()), "the glyphs and faces of the text typed");
    // Its parts read back as the whole result, in order.
    let k0 = pl.clusters.iter().position(|c| c.start == 1).unwrap();
    let shown: String = pl.shown.iter().filter(|s| s.0 >= k0 && s.0 < k0 + field.len()).map(|s| s.1.as_str()).collect();
    assert_eq!(shown, RESULT);
}

#[test]
fn the_caret_goes_before_or_after_the_field_never_into_it() {
    let pl = first_para(&lay(&with_field()));
    let field: Vec<&Cluster> = pl.clusters.iter().filter(|c| c.start == 1).collect();
    let (before, after) = (pl.x_of(0, 1).unwrap(), pl.x_of(0, field[0].end).unwrap());
    let width: f32 = field.iter().map(|c| c.adv).sum();
    assert!((after - before - width).abs() < 0.01, "after {after}, before {before}, width {width}");
    // A click anywhere on the result lands on one side of it.
    for dx in [0.1, width / 2.0, width - 0.1] {
        let off = pl.off_at_x(0, before + dx);
        assert!(off == 1 || off == field[0].end, "{off}");
    }
}

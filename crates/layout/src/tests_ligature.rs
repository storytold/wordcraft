//! The caret inside a ligature (#1974): a glyph that draws several letters (fi, ffi, lam-alef)
//! gives each letter an even share of its advance, so the caret, clicks and selection edges can
//! land between the letters. A single grapheme cluster (letter plus combining mark) stays atomic.

use super::*;
use wordcraft_doc::{Pos, StoryRef};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn para_layout(l: &DocLayout) -> &ParaLayout {
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, para, .. } = it {
                return para;
            }
        }
    }
    panic!("no paragraph laid out");
}

/// `text` in a bundled font that draws fi and ffi as ligatures, so the tests don't depend on the
/// installed fonts (Liberation Sans, the usual Linux stand-in for the default font, draws none).
fn doc(text: &str) -> Document {
    let mut d = Document::from_text(text);
    let end = d.body.first().and_then(|b| b.as_para()).map_or(0, |p| p.text.len());
    let _ = d.format_range(&Pos::body(0, 0), &Pos::body(0, end), &|c| c.font = Some(LIGATURE_FONT.into()));
    d
}

const LIGATURE_FONT: &str = "Source Serif 4";

fn caret_x(l: &DocLayout, off: usize) -> f32 {
    l.caret(&Pos::body(0, off)).map(|c| c.x).unwrap_or(f32::NAN)
}

/// Does the default font draw `fi` as one glyph? (The tests below need a ligature to mean anything.)
fn has_fi_ligature(l: &DocLayout) -> bool {
    let pl = para_layout(l);
    pl.clusters.len() >= 2 && pl.clusters.first().is_some_and(|c| c.g1 > c.g0) && pl.glyphs.len() < pl.clusters.len()
}

#[test]
fn caret_between_the_letters_of_a_ligature_is_inside_it() {
    crate::test_fonts();
    let l = lay(&doc("fi"));
    assert!(has_fi_ligature(&l), "the default font should draw fi as a ligature");
    let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 1), caret_x(&l, 2));
    assert!(a < b && b < c, "caret x before, inside and after fi: {a} {b} {c}");
    assert!((b - a - (c - b)).abs() < 0.05, "an even split: {a} {b} {c}");
}

/// Width of "fi" (both letters) with character spacing `sp` points per letter.
fn fi_width_with_spacing(sp: Option<f32>) -> (DocLayout, f32) {
    let mut d = doc("fi");
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 2), &|c| c.spacing = sp).unwrap();
    let l = lay(&d);
    let w = caret_x(&l, 2) - caret_x(&l, 0);
    (l, w)
}

#[test]
fn character_spacing_inside_a_ligature_is_kept() {
    crate::test_fonts();
    let (l0, plain) = fi_width_with_spacing(None);
    assert!(has_fi_ligature(&l0), "the default font should draw fi as a ligature");
    for sp in [3.0_f32, -0.5] {
        let (l, w) = fi_width_with_spacing(Some(sp));
        assert!(has_fi_ligature(&l), "fi stays a ligature with spacing {sp}");
        assert!((w - (plain + 2.0 * sp)).abs() < 0.05, "spacing {sp}: width {w}, expected {}", plain + 2.0 * sp);
        let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 1), caret_x(&l, 2));
        assert!(a < b && b < c, "caret inside the ligature with spacing {sp}: {a} {b} {c}");
    }
}

#[test]
fn clicks_inside_a_ligature_land_between_its_letters() {
    crate::test_fonts();
    let l = lay(&doc("fi"));
    let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 1), caret_x(&l, 2));
    let cr = l.caret(&Pos::body(0, 1)).unwrap();
    let y = cr.top + cr.height / 2.0;
    let hit = |x: f32| l.hit(cr.page, x, y, StoryRef::Body).map(|p| p.off);
    assert_eq!(hit(b + 0.2), Some(1));
    assert_eq!(hit(b - 0.2), Some(1));
    assert_eq!(hit(a + 0.2), Some(0));
    assert_eq!(hit(c - 0.2), Some(2));
}

#[test]
fn every_letter_of_a_three_letter_ligature_has_its_own_caret() {
    crate::test_fonts();
    let l = lay(&doc("ffi"));
    assert!(has_fi_ligature(&l), "the default font should draw ffi as a ligature");
    let xs: Vec<f32> = (0..=3).map(|o| caret_x(&l, o)).collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
}

#[test]
fn a_combining_mark_stays_in_its_letters_cluster() {
    crate::test_fonts();
    let l = lay(&doc("fi\u{301}"));
    let pl = para_layout(&l);
    let offs = pl.line_offsets(0);
    assert!(!offs.contains(&2), "no caret between i and its accent: {offs:?}");
}

#[test]
fn selection_edges_inside_a_ligature_use_the_same_positions() {
    crate::test_fonts();
    let d = doc("fi");
    let l = lay(&d);
    let rects = l.selection_rects(&d, &Pos::body(0, 0), &Pos::body(0, 1), 0);
    let r = rects.first().map(|(_, r)| *r).unwrap();
    let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 1), caret_x(&l, 2));
    assert!((r.x - a).abs() < 0.05 && (r.x + r.w - b).abs() < 0.05, "{r:?} {a} {b}");
    assert!(r.x + r.w < c - 0.05, "the highlight of f stops inside the ligature: {r:?} {c}");
}

/// "لا" laid out in an installed Arabic font that draws lam-alef as one glyph (Noto Naskh Arabic,
/// the usual fallback, draws two), or `None` when no such font is installed.
fn lam_alef_ligature() -> Option<DocLayout> {
    ["Noto Sans Arabic", "Noto Kufi Arabic", "Amiri", "Scheherazade New", "DejaVu Sans", "Arial", "Geeza Pro", "Tahoma"].into_iter().find_map(
        |family| {
            let mut d = Document::from_text("لا");
            d.format_range(&Pos::body(0, 0), &Pos::body(0, 4), &|c| {
                c.font = Some(family.into());
                c.font_cs = Some(family.into());
            })
            .ok()?;
            let l = lay(&d);
            let pl = para_layout(&l);
            let glyphs: Vec<u32> = pl.clusters.iter().map(|c| c.g1.saturating_sub(c.g0)).collect();
            (glyphs == [1, 0] && pl.styles.iter().any(|s| s.face.family == family)).then_some(l)
        },
    )
}

#[test]
fn a_right_to_left_ligature_splits_mirrored() {
    crate::test_fonts();
    let Some(l) = lam_alef_ligature() else {
        eprintln!("skipped: no lam-alef ligature font");
        return;
    };
    // Logical offsets 0 (before lam), 2 (between lam and alef), 4 (after alef); the text runs leftward.
    let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 2), caret_x(&l, 4));
    assert!(a > b && b > c, "caret x runs right to left through the ligature: {a} {b} {c}");
    assert!(((a - b) - (b - c)).abs() < 0.05, "an even split: {a} {b} {c}");
}

#[test]
fn a_right_to_left_ligature_is_drawn_from_its_left_edge() {
    crate::test_fonts();
    // The glyph stays on lam, the rightmost letter; it must still start at alef's left edge.
    let Some(l) = lam_alef_ligature() else {
        eprintln!("skipped: no lam-alef ligature font");
        return;
    };
    let pl = para_layout(&l);
    let line = pl.lines.first().unwrap();
    let (k, head) = pl.clusters.iter().enumerate().find(|(_, c)| c.g1 > c.g0).unwrap();
    let glyph = pl.glyphs.get(head.g0 as usize).unwrap();
    let left = (line.c0..line.c1).filter_map(|k| line.cl_left(k)).fold(f32::INFINITY, f32::min);
    let drawn = line.cl_left(k).unwrap() + glyph.dx;
    assert!((drawn - left).abs() < 0.05, "glyph drawn at {drawn}, ligature starts at {left}");
}

#[test]
fn an_emoji_zwj_sequence_stays_one_cluster() {
    crate::test_fonts();
    // 👩‍💻: woman, zero-width joiner, laptop — one grapheme, one caret stop on each side.
    let t = "a\u{1F469}\u{200D}\u{1F4BB}b";
    let l = lay(&Document::from_text(t));
    let offs = para_layout(&l).line_offsets(0);
    assert!(!offs.contains(&5) && !offs.contains(&8), "no caret inside the emoji: {offs:?}");
    let xs: Vec<f32> = [0, 1, 12, 13].iter().map(|&o| caret_x(&l, o)).collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
}

#[test]
fn an_indic_conjunct_stays_one_cluster() {
    crate::test_fonts();
    // क्ष: ka, virama, ssa — one conjunct; the caret never stops inside it.
    let l = lay(&Document::from_text("\u{915}\u{94D}\u{937}"));
    let offs = para_layout(&l).line_offsets(0);
    assert!(!offs.contains(&3) && !offs.contains(&6), "no caret inside the conjunct: {offs:?}");
}

#[test]
fn a_long_word_never_wraps_inside_a_ligature() {
    crate::test_fonts();
    // A narrow page forces an emergency break in the unbroken word; whatever the offset, a line
    // never ends between a ligature's glyph and its glyphless letters.
    let mut seen_ligature = false;
    for k in 0..24 {
        let mut d = doc(&format!("{}ffi", "x".repeat(k)));
        d.last_section.margin_left = 250.0;
        d.last_section.margin_right = 250.0;
        let l = lay(&d);
        let pl = para_layout(&l);
        seen_ligature |= pl.glyphs.len() < pl.clusters.len();
        for line in &pl.lines {
            if let Some(next) = pl.clusters.get(line.c1) {
                let inside = next.g1 <= next.g0 && next.kind == crate::para::ClKind::Text && line.c1 > line.c0;
                assert!(!inside, "k={k}: line ends at {} inside a ligature", line.c1);
            }
        }
    }
    assert!(seen_ligature, "the default font should draw ffi as a ligature");
}

#[test]
fn a_hyphenation_point_never_splits_a_ligature() {
    crate::test_fonts();
    // Soft hyphens sit between the letters of "ffi"; a line never ends between a ligature's glyph
    // and its glyphless letters, whether or not the soft hyphen leaves the ligature intact.
    for k in 0..24 {
        let mut d = doc(&format!("{}f\u{ad}f\u{ad}i", "x".repeat(k)));
        d.last_section.margin_left = 250.0;
        d.last_section.margin_right = 250.0;
        let l = lay(&d);
        let pl = para_layout(&l);
        for line in &pl.lines {
            if let Some(next) = pl.clusters.get(line.c1) {
                let inside = next.g1 <= next.g0 && next.kind == crate::para::ClKind::Text && line.c1 > line.c0;
                assert!(!inside, "k={k}: line ends at {} inside a ligature", line.c1);
            }
        }
    }
}

//! A combining mark the face of its character can't draw (#7865): the character and its marks are
//! drawn together in a face that has them all (CSS Fonts 4 §5.3, cluster matching), and with no
//! such face the mark on its own from a face that has it, except in right-to-left and
//! complex-script text, where it stays with its letter. Marks the face draws and pairs it composes
//! stay with their character. Fixed test fonts: Source Serif 4 lacks U+0336 and U+0338; Source
//! Sans 3 has U+0338; only JetBrains Mono has U+0336.

use std::sync::Arc;

use wordcraft_doc::{CharProps, Document, Paragraph, para_block};
use wordcraft_fonts::{FaceRef, FontDb};

use super::lay;
use super::linear::assert_linear_steps;
use crate::display::DisplayOptions;
use crate::{ParaLayout, Placed};

const SERIF: &str = "Source Serif 4";

fn serif() -> FaceRef {
    crate::test_fonts();
    let face = wordcraft_fonts::word::resolve(SERIF, false, false).face;
    assert_eq!(face.family, SERIF, "the fixed test fonts include {SERIF}");
    face
}

/// A document of one paragraph, `text` set in Source Serif 4; `cs`: with complex-script
/// formatting (`w:cs`), its complex-script font Source Serif 4 too.
fn doc(text: &str, cs: bool) -> Document {
    let chr = CharProps { font: Some(SERIF.into()), font_cs: Some(SERIF.into()), cs: Some(cs), size: Some(12.0), ..Default::default() };
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text(text, chr))];
    d
}

/// [`doc`] laid out: its paragraph and the characters drawn as missing glyphs.
fn laid(text: &str, cs: bool) -> (Arc<ParaLayout>, Vec<char>) {
    crate::test_fonts();
    let d = doc(text, cs);
    let l = lay(&d);
    let missing = l.missing_glyphs(&d, &DisplayOptions::default(), None).chars.iter().map(|(c, _)| *c).collect();
    let para = l
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .find_map(|it| match it {
            Placed::Lines { para, .. } => Some(para.clone()),
            _ => None,
        })
        .expect("the paragraph is laid out");
    (para, missing)
}

/// The face that draws the character at byte `at` of the paragraph.
fn face_at(para: &ParaLayout, at: usize) -> FaceRef {
    let c = para.clusters.iter().find(|c| c.start <= at && at < c.end).expect("a cluster holds the character");
    para.styles[c.style as usize].face
}

#[test]
fn a_mark_its_letters_font_lacks_goes_with_its_letter_to_a_face_that_has_both() {
    let serif = serif();
    let (letter, mark) = ('∞', '\u{338}');
    assert!(serif.covers(letter) && !serif.covers(mark), "{SERIF} has {letter} and lacks U+0338");
    let text = format!("x {letter}{mark} y");
    let (para, missing) = laid(&text, false);
    assert!(missing.is_empty(), "nothing is drawn as a missing glyph, but {missing:?} were");
    let at = text.find(letter).expect("the letter is in the text");
    let face = face_at(&para, at);
    assert_eq!(face.id(), face_at(&para, at + letter.len_utf8()).id(), "the letter and its mark are drawn in one face");
    assert!(face.covers(letter) && face.covers(mark), "that face has both");
    assert_eq!(face_at(&para, 0).id(), serif.id(), "the text before stays in the run's font");
    assert_eq!(face_at(&para, text.len() - 1).id(), serif.id(), "the text after goes back to the run's font");
}

#[test]
fn a_mark_the_font_composes_with_its_letter_stays_in_the_run_font() {
    let serif = serif();
    // The font has no U+0338, but the shaper draws = with it as ≠, which the font has.
    assert!(!serif.covers('\u{338}') && serif.covers('≠'), "{SERIF} lacks U+0338 and has ≠");
    let text = "x =\u{338} y";
    let (para, missing) = laid(text, false);
    assert!(missing.is_empty(), "nothing is drawn as a missing glyph, but {missing:?} were");
    for (at, c) in text.char_indices() {
        assert_eq!(face_at(&para, at).id(), serif.id(), "U+{:04X} stays in the run's font", c as u32);
    }
}

#[test]
fn a_mark_no_face_draws_with_its_letter_is_drawn_from_a_face_that_has_it() {
    let serif = serif();
    let (letter, mark) = ('ǹ', '\u{336}');
    let db = FontDb::global();
    assert!(serif.covers(letter) && !serif.covers(mark), "{SERIF} has {letter} and lacks U+0336");
    assert!(db.fallback_for_run(&[letter, mark], serif.id(), false, false).is_none(), "no face has both");
    assert!(db.fallback_for(mark, serif.id()).is_some(), "a face has U+0336");
    let text = format!("{letter}{mark}");
    let (para, missing) = laid(&text, false);
    assert!(missing.is_empty(), "the mark is drawn, but {missing:?} were missing glyphs");
    assert_eq!(face_at(&para, 0).id(), serif.id(), "the letter stays in the run's font");
    assert!(face_at(&para, letter.len_utf8()).covers(mark), "the mark comes from a face that has it");
}

/// In complex-script text a mark no face draws with its letter stays in the letter's face, so it is
/// never split from its letter (Arabic harakat would lose their place on it).
#[test]
fn in_complex_script_text_a_mark_no_face_draws_with_its_letter_stays_with_it() {
    let serif = serif();
    let (letter, mark) = ('ǹ', '\u{336}');
    assert!(FontDb::global().fallback_for_run(&[letter, mark], serif.id(), false, false).is_none(), "no face has both");
    let (para, _) = laid(&format!("{letter}{mark}"), true);
    assert_eq!(face_at(&para, 0).id(), serif.id(), "the letter stays in the run's font");
    assert_eq!(face_at(&para, letter.len_utf8()).id(), serif.id(), "the mark stays with its letter");
}

/// A letter followed by many marks no face draws with it, in complex-script text, where they stay
/// with it: their cluster is matched once, not again at every mark, which made the layout quadratic
/// in their number.
#[test]
fn a_long_run_of_marks_after_one_letter_lays_out_in_linear_time() {
    let serif = serif();
    let (letter, mark) = ('ǹ', '\u{336}');
    assert!(FontDb::global().fallback_for_run(&[letter, mark], serif.id(), false, false).is_none(), "no face has both");
    let text = |n: usize| std::iter::once(letter).chain(std::iter::repeat_n(mark, n)).collect::<String>();
    assert_linear_steps("marks after one letter", 200, |n| doc(&text(n), true), |d| drop(std::hint::black_box(lay(d))));
}

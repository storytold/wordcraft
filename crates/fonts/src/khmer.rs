//! Khmer register shifter fallback: triisap (U+17CA) over a consonant with an upper vowel.
//!
//! Khmer OS System draws the triisap of `អ៊ិ` `ស៊ី` `ហ៊ឹ` as the U-form below the base, so it
//! doesn't collide with the upper vowel. Some fonts (Khmer Sangam MN for `អ`) leave it above and
//! stack both marks over the consonant. A face that doesn't do the U-form for one of these bases
//! gets it from here: the triisap is shaped as U+17BB (vowel sign U), which every Khmer font
//! draws below the base. Only the shaped character changes; the document text and cluster
//! indices don't.

use crate::FontFace;

const TRIISAP: char = '\u{17CA}';
const SIGN_U: char = '\u{17BB}';
/// Invisible subscript marker: the consonant after it is a subscript.
const COENG: char = '\u{17D2}';
/// The consonants whose triisap goes below when an upper vowel follows: អ ស ហ.
const BASES: [char; 3] = ['\u{17A2}', '\u{179F}', '\u{17A0}'];
/// A base whose triisap stays above: ប.
const NEVER_U: char = '\u{1794}';
/// The vowel the probes use: ិ.
const PROBE_VOWEL: char = '\u{17B7}';

fn is_upper_vowel(c: char) -> bool {
    matches!(c, '\u{17B7}'..='\u{17BA}' | '\u{17BE}')
}

/// Is `c` the triisap?
pub(crate) fn is_triisap(c: char) -> bool {
    c == TRIISAP
}

/// Is a triisap after `prev` (itself after `before`) and before `next` drawn as U+17BB (vowel sign
/// U) instead? It is when `prev` is a base in `needs` and an upper vowel follows. A subscript base
/// (`before` is the coeng) is left to the font: the probe only checks plain bases. `needs` (the
/// face's per-base flags) is only asked for once the characters fit, so a face is probed on its
/// first such triisap.
pub(crate) fn takes_u_form<'a>(before: char, prev: char, next: char, needs: impl FnOnce() -> &'a [bool; 3]) -> bool {
    if !is_upper_vowel(next) || before == COENG {
        return false;
    }
    let Some(k) = BASES.iter().position(|b| *b == prev) else { return false };
    needs().get(k).copied().unwrap_or(false)
}

/// U+17BB, the character a triisap is shaped as when [`takes_u_form`].
pub(crate) const U_FORM: char = SIGN_U;

/// U+25CC, the dotted circle a shaper draws under a mark that has no valid base.
const DOTTED_CIRCLE: char = '\u{25CC}';

/// The glyph the triisap became in `with` (a base, triisap and vowel): the first glyph that the same
/// base and vowel without it (`without`) don't use.
fn triisap_glyph(with: &[(u32, i32)], without: &[(u32, i32)]) -> Option<u32> {
    with.iter().map(|g| g.0).find(|g| !without.iter().any(|w| w.0 == *g))
}

/// Does a face need the rewrite for one base, given the triisap glyph over it (`tri_glyph`) and over a
/// base that never takes the U-form (`above`), its shaping of the base with a triisap (`tri`) and
/// with U+17BB (`u`), and its dotted-circle glyph (`circle`)? It does when the triisap stays as it is
/// over that base, and the rewritten sequence neither adds width nor holds the dotted circle (the
/// face rejects the invalid U + vowel pair). Each shaping entry is (glyph, advance).
fn needs_rewrite(tri_glyph: Option<u32>, above: Option<u32>, tri: &[(u32, i32)], u: &[(u32, i32)], circle: Option<u32>) -> bool {
    let width = |g: &[(u32, i32)]| g.iter().map(|x| x.1).sum::<i32>();
    above.is_some() && tri_glyph == above && width(tri) == width(u) && !circle.is_some_and(|c| u.iter().any(|g| g.0 == c))
}

/// For each base, whether `face` needs the rewrite (see [`needs_rewrite`]). A face that already
/// does the U-form shapes the triisap differently over these bases, so it is left alone. `shape`
/// gives (glyph, advance).
pub(crate) fn probe(face: &FontFace, shape: impl Fn(&str) -> Vec<(u32, i32)>) -> [bool; 3] {
    probe_with(|c| face.glyph_for(c), shape)
}

/// [`probe`] over any glyph lookup and shaping, so it needs no font to test.
fn probe_with(glyph_for: impl Fn(char) -> u32, shape: impl Fn(&str) -> Vec<(u32, i32)>) -> [bool; 3] {
    if glyph_for(SIGN_U) == 0 || glyph_for(TRIISAP) == 0 || glyph_for(NEVER_U) == 0 {
        return [false; 3];
    }
    let seq = |b: char, m: Option<char>| -> String { b.to_string() + &m.map(String::from).unwrap_or_default() + &PROBE_VOWEL.to_string() };
    let triisap_over = |b: char| triisap_glyph(&shape(&seq(b, Some(TRIISAP))), &shape(&seq(b, None)));
    let above = triisap_over(NEVER_U);
    let circle = Some(glyph_for(DOTTED_CIRCLE)).filter(|g| *g != 0);
    BASES.map(|b| glyph_for(b) != 0 && needs_rewrite(triisap_over(b), above, &shape(&seq(b, Some(TRIISAP))), &shape(&seq(b, Some(SIGN_U))), circle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `s` as the shaper would see it: the loop in `shape_dir`, over a face that needs `needs`.
    fn run(s: &str, needs: [bool; 3]) -> String {
        let cs: Vec<char> = s.chars().collect();
        (0..cs.len())
            .map(|i| {
                let before = if i > 1 { cs[i - 2] } else { '\0' };
                let prev = if i > 0 { cs[i - 1] } else { '\0' };
                match cs.get(i + 1) {
                    Some(&next) if is_triisap(cs[i]) && takes_u_form(before, prev, next, || &needs) => U_FORM,
                    _ => cs[i],
                }
            })
            .collect()
    }

    #[test]
    fn rewrites_only_listed_bases_before_upper_vowels() {
        assert_eq!(run("អ៊ិ", [true, false, false]), "អុិ");
        assert_eq!(run("ស៊ី", [true, false, false]), "ស៊ី");
        assert_eq!(run("ស៊ី", [false, true, false]), "សុី");
        assert_eq!(run("ហ៊ឹ", [false, false, true]), "ហុឹ");
        // After another letter, and at the start of the text.
        assert_eq!(run("កអ៊ិ", [true, false, false]), "កអុិ");
    }

    #[test]
    fn leaves_other_contexts() {
        assert_eq!(run("អ៊ា", [true; 3]), "អ៊ា");
        assert_eq!(run("ប៊ិ", [true; 3]), "ប៊ិ");
        assert_eq!(run("អ៊", [true; 3]), "អ៊");
        assert_eq!(run("៊ិ", [true; 3]), "៊ិ");
        // A subscript base is left to the font.
        assert_eq!(run("ក្អ៊ិ", [true; 3]), "ក្អ៊ិ");
        assert_eq!(run("ក្ស៊ី", [true; 3]), "ក្ស៊ី");
    }

    /// A face for `probe_with`: the glyph for a character is its code point (0 for `missing`, 40 for
    /// the dotted circle). Shaping gives (base, triisap glyph, vowel): the triisap stays glyph 900
    /// above a base, 901 when the face draws it below for the bases in `below`; U + vowel gives
    /// glyph 901, or the dotted circle (40) when `circle_in_u`.
    fn fake(missing: &'static [char], below: &'static [char], circle_in_u: bool) -> (impl Fn(char) -> u32, impl Fn(&str) -> Vec<(u32, i32)>) {
        let glyph_for = move |c: char| {
            if missing.contains(&c) {
                0
            } else if c == DOTTED_CIRCLE {
                40
            } else {
                c as u32
            }
        };
        let shape = move |s: &str| {
            let c: Vec<char> = s.chars().collect();
            match c.as_slice() {
                [b, v] => vec![(*b as u32, 1500), (*v as u32, 0)],
                [b, m, v] if *m == TRIISAP => vec![(*b as u32, 1500), (if below.contains(b) { 901 } else { 900 }, 0), (*v as u32 + 1, 0)],
                [b, _, v] => vec![(*b as u32, 1500), (if circle_in_u { 40 } else { 901 }, 0), (*v as u32, 0)],
                _ => vec![],
            }
        };
        (glyph_for, shape)
    }

    #[test]
    fn probe_flags_the_bases_a_face_leaves_above() {
        let (g, s) = fake(&[], &[], false);
        assert_eq!(probe_with(g, s), [true; 3]);
        // A face that already draws the U-form for one base is left alone for it (U+179F here).
        let (g, s) = fake(&[], &['\u{179F}'], false);
        assert_eq!(probe_with(g, s), [true, false, true]);
        let (g, s) = fake(&[], &['\u{17A2}', '\u{179F}', '\u{17A0}'], false);
        assert_eq!(probe_with(g, s), [false; 3]);
    }

    #[test]
    fn probe_needs_the_glyphs_it_compares() {
        // No glyph for the U sign, the triisap or the reference base: the fix is off.
        for missing in [SIGN_U, TRIISAP, NEVER_U] {
            let m: &'static [char] = Box::leak(vec![missing].into_boxed_slice());
            let (g, s) = fake(m, &[], false);
            assert_eq!(probe_with(g, s), [false; 3], "{missing:?}");
        }
        // A base the face has no glyph for is skipped on its own.
        let (g, s) = fake(&['\u{17A0}'], &[], false);
        assert_eq!(probe_with(g, s), [true, true, false]);
    }

    #[test]
    fn probe_refuses_a_face_that_dotted_circles_the_replacement() {
        let (g, s) = fake(&[], &[], true);
        assert_eq!(probe_with(g, s), [false; 3]);
    }

    #[test]
    fn rewrite_is_refused_when_the_replacement_has_the_dotted_circle() {
        // (glyph, advance): base, triisap left above, vowel; the same base with U + vowel.
        let tri = [(1, 1500), (7, 0), (9, 0)];
        let above = Some(7);
        assert!(needs_rewrite(Some(7), above, &tri, &[(1, 1500), (8, 0), (9, 0)], Some(40)));
        // No dotted-circle glyph in the face: as before.
        assert!(needs_rewrite(Some(7), above, &tri, &[(1, 1500), (8, 0), (9, 0)], None));
        // A zero-width dotted circle among the glyphs: refused although the width matches.
        assert!(!needs_rewrite(Some(7), above, &tri, &[(1, 1500), (40, 0), (9, 0)], Some(40)));
        // A wider result is still refused, and a triisap that is already below is left alone.
        assert!(!needs_rewrite(Some(7), above, &tri, &[(1, 1500), (8, 0), (9, 1393)], Some(40)));
        assert!(!needs_rewrite(Some(8), above, &tri, &[(1, 1500), (8, 0), (9, 0)], Some(40)));
        // No triisap glyph found: nothing to rewrite.
        assert!(!needs_rewrite(None, None, &tri, &tri, None));
    }

    #[test]
    fn triisap_glyph_is_the_one_the_triisap_added() {
        assert_eq!(triisap_glyph(&[(1, 0), (7, 0), (9, 0)], &[(1, 0), (9, 0)]), Some(7));
        // Whatever its position, and a vowel variant after it is not mistaken for it.
        assert_eq!(triisap_glyph(&[(3, 0), (1, 0), (7, 0), (10, 0)], &[(1, 0), (9, 0)]), Some(3));
        assert_eq!(triisap_glyph(&[(1, 0), (9, 0)], &[(1, 0), (9, 0)]), None);
    }
}

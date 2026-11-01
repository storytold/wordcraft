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

/// Is a triisap after `prev` and before `next` drawn as U+17BB (vowel sign U) instead? It is when
/// `prev` is a base in `needs` and an upper vowel follows. `needs` (the face's per-base flags) is
/// only asked for once the characters fit, so a face is probed on its first such triisap.
pub(crate) fn takes_u_form<'a>(prev: char, next: char, needs: impl FnOnce() -> &'a [bool; 3]) -> bool {
    if !is_upper_vowel(next) {
        return false;
    }
    let Some(k) = BASES.iter().position(|b| *b == prev) else { return false };
    needs().get(k).copied().unwrap_or(false)
}

/// U+17BB, the character a triisap is shaped as when [`takes_u_form`].
pub(crate) const U_FORM: char = SIGN_U;

/// U+25CC, the dotted circle a shaper draws under a mark that has no valid base.
const DOTTED_CIRCLE: char = '\u{25CC}';

/// Does a face need the rewrite for one base, given its shaping of the base with a triisap (`tri`)
/// and with U+17BB (`u`), the triisap glyph over a base that never takes the U-form (`above`) and
/// its dotted-circle glyph (`circle`)? It does when the triisap stays as it is over that base, and
/// the rewritten sequence neither adds width nor holds the dotted circle (the face rejects the
/// invalid U + vowel pair). Each entry is (glyph, advance).
fn needs_rewrite(tri: &[(u32, i32)], u: &[(u32, i32)], above: Option<u32>, circle: Option<u32>) -> bool {
    let width = |g: &[(u32, i32)]| g.iter().map(|x| x.1).sum::<i32>();
    above.is_some() && tri.get(1).map(|g| g.0) == above && width(tri) == width(u) && !circle.is_some_and(|c| u.iter().any(|g| g.0 == c))
}

/// For each base, whether `face` needs the rewrite (see [`needs_rewrite`]). A face that already
/// does the U-form shapes the triisap differently over these bases, so it is left alone. `shape`
/// gives (glyph, advance).
pub(crate) fn probe(face: &FontFace, shape: impl Fn(&str) -> Vec<(u32, i32)>) -> [bool; 3] {
    if face.glyph_for(SIGN_U) == 0 || face.glyph_for(TRIISAP) == 0 || face.glyph_for(NEVER_U) == 0 {
        return [false; 3];
    }
    let seq = |b: char, m: char| -> String { [b, m, PROBE_VOWEL].iter().collect() };
    let above = shape(&seq(NEVER_U, TRIISAP)).get(1).map(|g| g.0);
    let circle = Some(face.glyph_for(DOTTED_CIRCLE)).filter(|g| *g != 0);
    BASES.map(|b| face.glyph_for(b) != 0 && needs_rewrite(&shape(&seq(b, TRIISAP)), &shape(&seq(b, SIGN_U)), above, circle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `s` as the shaper would see it: the loop in `shape_dir`, over a face that needs `needs`.
    fn run(s: &str, needs: [bool; 3]) -> String {
        let cs: Vec<char> = s.chars().collect();
        (0..cs.len())
            .map(|i| {
                let prev = if i > 0 { cs[i - 1] } else { '\0' };
                match cs.get(i + 1) {
                    Some(&next) if is_triisap(cs[i]) && takes_u_form(prev, next, || &needs) => U_FORM,
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
        assert_eq!(run("ក្អ៊ិ", [true, false, false]), "ក្អុិ");
    }

    #[test]
    fn leaves_other_contexts() {
        assert_eq!(run("អ៊ា", [true; 3]), "អ៊ា");
        assert_eq!(run("ប៊ិ", [true; 3]), "ប៊ិ");
        assert_eq!(run("អ៊", [true; 3]), "អ៊");
        assert_eq!(run("៊ិ", [true; 3]), "៊ិ");
    }

    #[test]
    fn rewrite_is_refused_when_the_replacement_has_the_dotted_circle() {
        // (glyph, advance): base, triisap left above, vowel; the same base with U + vowel.
        let tri = [(1, 1500), (7, 0), (9, 0)];
        let above = Some(7);
        assert!(needs_rewrite(&tri, &[(1, 1500), (8, 0), (9, 0)], above, Some(40)));
        // No dotted-circle glyph in the face: as before.
        assert!(needs_rewrite(&tri, &[(1, 1500), (8, 0), (9, 0)], above, None));
        // A zero-width dotted circle among the glyphs: refused although the width matches.
        assert!(!needs_rewrite(&tri, &[(1, 1500), (40, 0), (9, 0)], above, Some(40)));
        // A wider result is still refused, and a triisap that is already below is left alone.
        assert!(!needs_rewrite(&tri, &[(1, 1500), (8, 0), (9, 1393)], above, Some(40)));
        assert!(!needs_rewrite(&[(1, 1500), (8, 0), (9, 0)], &[(1, 1500), (8, 0), (9, 0)], above, Some(40)));
    }
}

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

/// Does `text` hold a triisap?
pub(crate) fn has_triisap(text: &str) -> bool {
    text.contains(TRIISAP)
}

/// Rewrite the triisaps in `chars` (shaped character, source index) that follow a base in
/// `needs` and precede an upper vowel into U+17BB.
pub(crate) fn apply(chars: &mut [(usize, char)], needs: &[bool; 3]) {
    for i in 1..chars.len().saturating_sub(1) {
        if chars[i].1 != TRIISAP || !is_upper_vowel(chars[i + 1].1) {
            continue;
        }
        if BASES.iter().zip(needs).any(|(b, n)| *n && *b == chars[i - 1].1) {
            chars[i].1 = SIGN_U;
        }
    }
}

/// For each base, whether `face` needs the rewrite. It does when the face leaves the triisap
/// as it draws it over a base that never takes the U-form (ប) *and* the rewritten sequence adds
/// no width (a face that dotted-circles the invalid U + ិ pair, as Khmer Busra does, is not
/// rewritten). A face that already does the U-form (Khmer OS System, Khmer Busra) shapes the
/// triisap differently over these bases, so it is left alone. `shape` gives (glyph, advance).
pub(crate) fn probe(face: &FontFace, shape: impl Fn(&str) -> Vec<(u32, i32)>) -> [bool; 3] {
    if face.glyph_for(SIGN_U) == 0 || face.glyph_for(TRIISAP) == 0 || face.glyph_for(NEVER_U) == 0 {
        return [false; 3];
    }
    let seq = |b: char, m: char| -> String { [b, m, PROBE_VOWEL].iter().collect() };
    let width = |g: &[(u32, i32)]| g.iter().map(|x| x.1).sum::<i32>();
    let above = shape(&seq(NEVER_U, TRIISAP)).get(1).map(|g| g.0);
    BASES.map(|b| {
        if face.glyph_for(b) == 0 {
            return false;
        }
        let tri = shape(&seq(b, TRIISAP));
        let u = shape(&seq(b, SIGN_U));
        above.is_some() && tri.get(1).map(|g| g.0) == above && width(&tri) == width(&u)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str, needs: [bool; 3]) -> String {
        let mut v: Vec<(usize, char)> = s.char_indices().collect();
        apply(&mut v, &needs);
        v.into_iter().map(|(_, c)| c).collect()
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
}

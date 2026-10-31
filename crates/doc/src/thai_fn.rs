//! Thai and Lao text utilities shared across layout and input handling.
//!
//! These helpers operate on raw text and character classification; they have no
//! dependency on layout geometry and are safe to call from any crate that depends
//! on `wordcraft-doc`.

use crate::thai_char::{MAICHATTAWA, MAIEK, MAITAIKHU, MAIMALAI, NIKHAHIT, PHINTHU, SARA_AM, SARA_E, SARA_I, THANTHAKHAT, YAMAKKAN};

/// Returns `true` if `c` is a Thai above/below vowel, tone mark, or other TCB follower.
///
/// These are the combining marks that attach to a Thai base consonant and together
/// form one Thai Character Block (TCB).
pub fn is_thai_following(c: char) -> bool {
    matches!(c,
        SARA_AM
        | SARA_I..=PHINTHU
        | MAITAIKHU
        | MAIEK..=MAICHATTAWA
        | THANTHAKHAT
        | NIKHAHIT
        | YAMAKKAN
    )
}

/// Returns `true` if `c` is a Lao above/below vowel or tone mark (Lao TCB follower).
pub fn is_lao_following(c: char) -> bool {
    // Below vowels: ຸ ູ (U+0EB8–U+0EB9)
    // Above vowels: ິ ີ ຶ ື ົ ຺ ຼ (U+0EB4–U+0EB7, U+0EBA–U+0EBC)
    // Tone marks: ່ ້ ໊ ໋ ໌ (U+0EC8–U+0ECC)
    matches!(c, '\u{0EB4}'..='\u{0EB9}' | '\u{0EBA}'..='\u{0EBC}' | '\u{0EC8}'..='\u{0ECC}')
}

/// Byte length of the Thai/Lao TCB starting at the beginning of `text`.
///
/// A Thai or Lao leading vowel counts as a single-character TCB (it stands alone
/// as a drop cap and as a caret stop).  Otherwise the first character must be a
/// base consonant; any immediately following [`is_thai_following`] /
/// [`is_lao_following`] characters are included.
///
/// Returns `0` if `text` does not start with a Thai or Lao character.
pub fn thai_lao_tcb_byte_len(text: &str) -> usize {
    use crate::cluster::is_thai_lao;
    let mut chars = text.chars();
    let first = match chars.next() {
        Some(c) if is_thai_lao(c) => c,
        _ => return 0,
    };
    let len = first.len_utf8();
    // Thai leading vowels (เ แ โ ใ ไ, U+0E40–U+0E44) stand alone.
    if matches!(first, SARA_E..=MAIMALAI) {
        return len;
    }
    // Lao leading vowels (ເ ແ ໂ ໃ ໄ, U+0EC0–U+0EC4) stand alone.
    if matches!(first, '\u{0EC0}'..='\u{0EC4}') {
        return len;
    }
    // Base consonant: consume any following vowels/tones.
    let mut total = len;
    for c in chars {
        if is_thai_following(c) || is_lao_following(c) {
            total += c.len_utf8();
        } else {
            break;
        }
    }
    total
}

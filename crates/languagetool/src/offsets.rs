//! LanguageTool counts offsets in Java `char`s: UTF-16 code units. The document counts bytes of
//! UTF-8. These map one onto the other, safely for any number the server sends.

/// The byte offset in `text` of UTF-16 offset `units`. Past the end clamps to `text.len()`. An
/// offset inside a character (between the two halves of a surrogate pair) moves to that
/// character's start, or with `round_up` to its end, so the result is always a char boundary.
pub fn utf16_to_byte(text: &str, units: u64, round_up: bool) -> usize {
    let mut seen: u64 = 0;
    for (i, c) in text.char_indices() {
        if seen >= units {
            return i;
        }
        let n = c.len_utf16() as u64;
        if seen.saturating_add(n) > units {
            return if round_up { i.saturating_add(c.len_utf8()) } else { i };
        }
        seen = seen.saturating_add(n);
    }
    text.len()
}

/// The UTF-16 length of `text` (what LanguageTool counts).
pub fn utf16_len(text: &str) -> u64 {
    text.chars().map(|c| c.len_utf16() as u64).sum()
}

/// The byte range of a UTF-16 `(offset, length)` pair, `None` when it's empty after mapping.
pub fn utf16_range(text: &str, offset: u64, length: u64) -> Option<(usize, usize)> {
    let start = utf16_to_byte(text, offset, false);
    let end = utf16_to_byte(text, offset.saturating_add(length), true);
    (start < end).then_some((start, end))
}

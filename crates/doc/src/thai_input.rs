//! Thai input sequence checking — Thai Character Block (TCB) rules.
//!
//! A TCB is the unit of Thai composition: one base consonant, optionally followed by one
//! above or below vowel, optionally followed by one tone mark or diacritic. Maximum 3
//! characters. These rules mirror the behaviour of Windows Thai IMEs and the original
//! Thai edition of Internet Explorer / Word from 1996-2001 (Nusorn Photpipat, Algorithms Co.).
//!
//! ## TCB structure
//! ```text
//! [consonant]  [upper_vowel | lower_vowel]?  [tone | diacritic]?
//!      ↑               ↑                              ↑
//!   required       slot 1 (optional)            slot 2 (optional)
//! ```
//!
//! ## Replacement rules
//! - Typing an upper/lower vowel when slot 1 is already filled → replaces it.
//! - Typing a tone/diacritic when slot 2 is already filled → replaces it.
//! - Typing a consonant always starts a new TCB.
//! - Typing a combining character with no preceding consonant → rejected.

/// Classification of a Unicode Thai character for input-sequence purposes.
/// Derived from `TBL_CHTYPE` in the original THAIFUNC.CPP (Algorithms Co., 2001).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThaiCharClass {
    /// Consonant (TCT_THAICONS / THAICONSLOWER / THAICONSUPPER): the mandatory TCB base.
    Consonant,
    /// Above vowel (TCT_THAIUPPERVOWEL): ◌ั ◌ิ ◌ี ◌ึ ◌ื — occupies slot 1.
    UpperVowel,
    /// Below vowel (TCT_THAILOWERVOWEL): ◌ุ ◌ู ◌ฺ — occupies slot 1.
    LowerVowel,
    /// Tone mark (TCT_THAITONE): ◌้ ◌่ ◌๊ ◌๋ ◌์ — occupies slot 2.
    Tone,
    /// Diacritic (TCT_THAIDIAC): ◌็ — occupies slot 2.
    Diacritic,
    /// Leading vowel (TCT_LEADINGVOWEL): เ แ โ ใ ไ — written before the consonant visually
    /// but typed/stored after; handled outside TCB logic.
    LeadingVowel,
    /// Anything else (digits, punctuation, non-Thai): pass through unchanged.
    Other,
}

/// What the engine must do when the user types `new_char` into a Thai context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThaiInputAction {
    /// Accept and append the character as-is.
    Accept,
    /// Delete `bytes` bytes immediately before the caret, then insert the new character.
    /// Used to replace an existing vowel or tone mark with the newly typed one.
    Replace { bytes: usize },
    /// The character is structurally invalid here; reject it (no insertion, optional beep).
    Reject,
}

/// Classify a Unicode Thai character according to `TBL_CHTYPE` from THAIFUNC.CPP.
pub fn thai_char_class(c: char) -> ThaiCharClass {
    match u32::from(c) {
        // Consonants — normal (TCT_THAICONS)
        0x0E01..=0x0E0D  // ก–ญ
        | 0x0E10..=0x0E1A  // ฐ–น
        | 0x0E1C | 0x0E1E  // ผ พ
        | 0x0E20..=0x0E23  // ภ–ร
        | 0x0E25  // ล
        | 0x0E27..=0x0E2E  // ว–ฮ
        => ThaiCharClass::Consonant,

        // Consonants — lower tail (TCT_THAICONSLOWER): ฎ ฏ ฤ ฦ
        0x0E0E | 0x0E0F | 0x0E24 | 0x0E26
        => ThaiCharClass::Consonant,

        // Consonants — upper tail (TCT_THAICONSUPPER): ป ฝ ฟ
        0x0E1B | 0x0E1D | 0x0E1F
        => ThaiCharClass::Consonant,

        // Above vowels (TCT_THAIUPPERVOWEL): ◌ั ◌ิ ◌ี ◌ึ ◌ื
        0x0E31 | 0x0E34..=0x0E37
        => ThaiCharClass::UpperVowel,

        // Below vowels (TCT_THAILOWERVOWEL): ◌ุ ◌ู ◌ฺ (phinthu)
        0x0E38..=0x0E3A
        => ThaiCharClass::LowerVowel,

        // Diacritic (TCT_THAIDIAC): ◌็ (maitaikhu)
        0x0E47
        => ThaiCharClass::Diacritic,

        // Tone marks (TCT_THAITONE): ◌้ ◌่ ◌๊ ◌๋ ◌์
        0x0E48..=0x0E4C
        => ThaiCharClass::Tone,

        // Leading vowels (TCT_LEADINGVOWEL): เ แ โ ใ ไ
        0x0E40..=0x0E44
        => ThaiCharClass::LeadingVowel,

        _ => ThaiCharClass::Other,
    }
}

/// Extract the current TCB — up to 3 chars immediately before `caret_off` in `text`
/// that form a valid Thai Character Block (consonant + optional vowel + optional tone/diac).
/// Returns a slice of 0–3 `(byte_offset, char)` pairs in logical order.
pub fn current_tcb(text: &str, caret_off: usize) -> Vec<(usize, char)> {
    let before = match text.get(..caret_off) {
        Some(s) => s,
        None => return Vec::new(),
    };
    // Collect up to 3 chars before the caret, newest first.
    let mut chars_rev: Vec<(usize, char)> = before
        .char_indices()
        .rev()
        .take(3)
        .collect();
    chars_rev.reverse(); // back to logical order

    // Walk backwards to find a valid TCB ending at caret_off.
    // A TCB must start with a consonant.  Scan from the rightmost candidate.
    let classes: Vec<ThaiCharClass> = chars_rev.iter().map(|(_, c)| thai_char_class(*c)).collect();

    // Find where the consonant that anchors this TCB is.
    // Work right-to-left: skip at most one tone/diac, then at most one vowel, then consonant.
    let len = chars_rev.len();
    if len == 0 {
        return Vec::new();
    }

    let mut end = len; // exclusive index into chars_rev
    // Try to peel off slot 2 (tone/diac).
    let slot2 = if end > 0
        && matches!(classes[end - 1], ThaiCharClass::Tone | ThaiCharClass::Diacritic)
    {
        end -= 1;
        true
    } else {
        false
    };
    // Try to peel off slot 1 (vowel).
    let slot1 = if end > 0
        && matches!(classes[end - 1], ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel)
    {
        end -= 1;
        true
    } else {
        false
    };
    let _ = slot2; // used structurally above
    // The remaining head must be a consonant.
    if end == 0 || classes[end - 1] != ThaiCharClass::Consonant {
        if slot1 {
            // vowel without a preceding consonant in the window — not a valid TCB
            return Vec::new();
        }
        return Vec::new();
    }
    let consonant_idx = end - 1;
    chars_rev[consonant_idx..].to_vec()
}

/// Determine what action to take when the user types `new_char` given the current TCB
/// (0–3 chars immediately before the caret, as returned by [`current_tcb`]).
///
/// `tcb` should only contain the current TCB chars — not the whole paragraph text.
/// Pass the result of `current_tcb(text, caret_off)` directly.
pub fn thai_input_action(tcb: &[(usize, char)], new_char: char) -> ThaiInputAction {
    let new_class = thai_char_class(new_char);

    match new_class {
        // Non-Thai and leading vowels pass through; the engine handles them normally.
        ThaiCharClass::Other | ThaiCharClass::LeadingVowel => ThaiInputAction::Accept,

        // A consonant always starts a fresh TCB.
        ThaiCharClass::Consonant => ThaiInputAction::Accept,

        // A vowel or tone typed with no TCB context is invalid.
        _ if tcb.is_empty() => ThaiInputAction::Reject,

        ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel => {
            // Find an existing vowel in the TCB (slot 1).
            if let Some(&(_off, existing)) = tcb.iter().find(|(_, c)| {
                matches!(thai_char_class(*c), ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel)
            }) {
                // Replace it.  Byte size of the existing vowel.
                ThaiInputAction::Replace { bytes: existing.len_utf8() }
            } else {
                // No vowel yet — only accept if the TCB is not already full (3 chars).
                if tcb.len() < 3 {
                    ThaiInputAction::Accept
                } else {
                    // TCB is full with consonant + tone/diac (no vowel slot free in 3-char limit).
                    // Per spec: replace is not defined here, so reject.
                    ThaiInputAction::Reject
                }
            }
        }

        ThaiCharClass::Tone | ThaiCharClass::Diacritic => {
            // Find an existing tone/diacritic in the TCB (slot 2).
            if let Some(&(_off, existing)) = tcb.iter().find(|(_, c)| {
                matches!(thai_char_class(*c), ThaiCharClass::Tone | ThaiCharClass::Diacritic)
            }) {
                ThaiInputAction::Replace { bytes: existing.len_utf8() }
            } else {
                // No tone/diac yet — accept if TCB not full.
                if tcb.len() < 3 {
                    ThaiInputAction::Accept
                } else {
                    // 3-char TCB [consonant, vowel, tone] and user types another vowel-class char.
                    // Shouldn't reach here for tone (covered above), but guard it.
                    ThaiInputAction::Reject
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cls(c: char) -> ThaiCharClass { thai_char_class(c) }

    #[test]
    fn thai_char_classification() {
        // Consonants
        assert_eq!(cls('ก'), ThaiCharClass::Consonant); // U+0E01
        assert_eq!(cls('ป'), ThaiCharClass::Consonant); // U+0E1B upper-tail
        assert_eq!(cls('ฎ'), ThaiCharClass::Consonant); // U+0E0E lower-tail
        // Upper vowels
        assert_eq!(cls('\u{0E31}'), ThaiCharClass::UpperVowel); // mai han akat
        assert_eq!(cls('\u{0E35}'), ThaiCharClass::UpperVowel); // sara ii
        // Lower vowels
        assert_eq!(cls('\u{0E38}'), ThaiCharClass::LowerVowel); // sara u
        assert_eq!(cls('\u{0E3A}'), ThaiCharClass::LowerVowel); // phinthu
        // Tone marks
        assert_eq!(cls('\u{0E48}'), ThaiCharClass::Tone); // mai ek
        assert_eq!(cls('\u{0E4C}'), ThaiCharClass::Tone); // thanthakhat
        // Diacritic
        assert_eq!(cls('\u{0E47}'), ThaiCharClass::Diacritic); // maitaikhu
        // Leading vowels
        assert_eq!(cls('เ'), ThaiCharClass::LeadingVowel); // U+0E40
        assert_eq!(cls('ไ'), ThaiCharClass::LeadingVowel); // U+0E44
        // Other
        assert_eq!(cls('a'), ThaiCharClass::Other);
        assert_eq!(cls('๑'), ThaiCharClass::Other); // Thai digit
    }

    /// Helper: build a text string and call current_tcb at its end.
    fn tcb(s: &str) -> Vec<char> {
        current_tcb(s, s.len()).iter().map(|(_, c)| *c).collect::<Vec<char>>()
    }

    #[test]
    fn current_tcb_extraction() {
        // Single consonant.
        assert_eq!(tcb("ก"), vec!['ก']);
        // Consonant + upper vowel.
        assert_eq!(tcb("กิ"), vec!['ก', '\u{0E34}']);
        // Full TCB: consonant + vowel + tone.
        assert_eq!(tcb("กี้"), vec!['ก', '\u{0E35}', '\u{0E49}']);
        // After a space the TCB resets.
        assert_eq!(tcb("ข ก"), vec!['ก']);
        // Latin text: no TCB.
        assert_eq!(tcb("hello"), Vec::<char>::new());
        // Empty.
        assert_eq!(tcb(""), Vec::<char>::new());
    }

    fn action(text: &str, new_char: char) -> ThaiInputAction {
        let tcb = current_tcb(text, text.len());
        thai_input_action(&tcb, new_char)
    }

    #[test]
    fn tcb_input_rules() {
        let kon = 'ก';          // consonant
        let sara_i = '\u{0E34}'; // sara i  (upper vowel)
        let sara_ii = '\u{0E35}'; // sara ii (upper vowel)
        let sara_u = '\u{0E38}'; // sara u  (lower vowel)
        let mai_ek = '\u{0E48}'; // mai ek  (tone)
        let mai_tho = '\u{0E49}'; // mai tho (tone)
        let maitaikhu = '\u{0E47}'; // maitaikhu (diacritic)

        // Consonant after consonant: start new TCB (Accept).
        assert_eq!(action("ก", kon), ThaiInputAction::Accept);

        // Vowel after consonant: Accept (append to slot 1).
        assert_eq!(action("ก", sara_i), ThaiInputAction::Accept);

        // Second vowel after consonant+vowel: Replace existing vowel.
        let s = "กิ"; // ก + sara i
        assert_eq!(action(s, sara_ii), ThaiInputAction::Replace { bytes: sara_i.len_utf8() });

        // Replacing upper vowel with a lower vowel.
        assert_eq!(action(s, sara_u), ThaiInputAction::Replace { bytes: sara_i.len_utf8() });

        // Tone after consonant: Accept (slot 2 free).
        assert_eq!(action("ก", mai_ek), ThaiInputAction::Accept);

        // Tone after consonant+vowel: Accept (slot 2 free).
        assert_eq!(action("กิ", mai_ek), ThaiInputAction::Accept);

        // Second tone after consonant+vowel+tone: Replace existing tone.
        let full = "กิ้"; // ก + sara i + mai ek
        assert_eq!(action(full, mai_tho), ThaiInputAction::Replace { bytes: mai_ek.len_utf8() });

        // Diacritic replacing existing tone.
        assert_eq!(action(full, maitaikhu), ThaiInputAction::Replace { bytes: mai_ek.len_utf8() });

        // Tone replacing existing diacritic.
        let diac = "กิ็"; // ก + sara i + maitaikhu
        assert_eq!(action(diac, mai_ek), ThaiInputAction::Replace { bytes: maitaikhu.len_utf8() });

        // Vowel with no preceding consonant: Reject.
        assert_eq!(action("", sara_i), ThaiInputAction::Reject);
        assert_eq!(action("hello", sara_i), ThaiInputAction::Reject);

        // Tone with no preceding consonant: Reject.
        assert_eq!(action("", mai_ek), ThaiInputAction::Reject);

        // Non-Thai passes through.
        assert_eq!(action("ก", 'a'), ThaiInputAction::Accept);
        assert_eq!(action("ก", ' '), ThaiInputAction::Accept);
    }
}

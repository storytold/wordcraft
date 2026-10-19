//! Thai input sequence checking — Thai Character Block (TCB) rules.
//!
//! A TCB is the unit of Thai composition: one base consonant, optionally followed by one
//! above or below vowel, optionally followed by one tone mark or diacritic. Maximum 3
//! characters.
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
//!

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

/// Describes an in-place consonant form swap (reserved for future use).
/// Both the old and new character must have the same UTF-8 byte length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsSwap {
    /// Absolute byte offset of the consonant in the paragraph text.
    pub offset: usize,
    /// Byte length of the existing consonant.
    pub old_bytes: usize,
    /// Replacement consonant character.
    pub new_char: char,
}

/// What the engine must do when the user types `new_char` into a Thai context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThaiInputAction {
    /// Accept and append the character at the caret as-is.
    Accept,
    /// Perform a slot operation at an absolute byte offset, then insert a character there.
    ///
    /// - `slot_offset`: absolute byte offset in the paragraph text where the char goes.
    /// - `slot_bytes`: bytes to delete at `slot_offset` before inserting (0 = pure insert).
    /// - `cons_swap`: optional same-size consonant swap applied *before* the slot operation.
    /// - `replacement`: if `Some(c)`, insert `c` instead of the originally typed character.
    ///   Used for character substitutions such as เ+เ→แ.
    ///
    /// Execution order in the engine:
    /// 1. If `cons_swap` is `Some`, replace the consonant in-place (same byte length).
    /// 2. Delete `slot_bytes` bytes at `slot_offset` (if > 0), moving caret there.
    ///    Otherwise move the caret to `slot_offset`.
    /// 3. Insert `replacement` (or the typed character if `None`) at the caret.
    Replace {
        slot_offset: usize,
        slot_bytes: usize,
        cons_swap: Option<ConsSwap>,
        /// Character to insert instead of the typed character, or `None` to insert as typed.
        replacement: Option<char>,
    },
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
    let mut chars_rev: Vec<(usize, char)> = before.char_indices().rev().take(3).collect();
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
    let slot2 = if end > 0 && matches!(classes[end - 1], ThaiCharClass::Tone | ThaiCharClass::Diacritic) {
        end -= 1;
        true
    } else {
        false
    };
    // Try to peel off slot 1 (vowel).
    let slot1 = if end > 0 && matches!(classes[end - 1], ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel) {
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
/// (0–3 chars immediately before the caret, as returned by [`current_tcb`]) and the
/// character immediately before the caret (which may differ from the TCB when the caret
/// sits after a leading vowel).
///
/// - `tcb`: result of `current_tcb(text, caret_off)` — may be empty.
/// - `prev_char`: `Some((byte_offset, char))` for the char immediately before the caret,
///   or `None` if the caret is at the start of the paragraph.
pub fn thai_input_action(tcb: &[(usize, char)], new_char: char, prev_char: Option<(usize, char)>) -> ThaiInputAction {
    let new_class = thai_char_class(new_char);

    match new_class {
        // Leading vowel: check for เ+เ=แ substitution; otherwise pass through.
        ThaiCharClass::LeadingVowel => {
            // เ (U+0E40) typed immediately after another เ → replace with แ (U+0E41).
            if new_char == '\u{0E40}' {
                if let Some((off, '\u{0E40}')) = prev_char {
                    return ThaiInputAction::Replace {
                        slot_offset: off,
                        slot_bytes: '\u{0E40}'.len_utf8(),
                        cons_swap: None,
                        replacement: Some('\u{0E41}'),
                    };
                }
            }
            ThaiInputAction::Accept
        }

        // Other non-Thai passes through unchanged.
        ThaiCharClass::Other => ThaiInputAction::Accept,

        // A consonant always starts a fresh TCB.
        ThaiCharClass::Consonant => ThaiInputAction::Accept,

        // A vowel or tone typed with no TCB context is invalid.
        _ if tcb.is_empty() => ThaiInputAction::Reject,

        ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel => {
            // Find an existing vowel in the TCB (slot 1).
            if let Some(&(vowel_off, existing_vowel)) =
                tcb.iter().find(|(_, c)| matches!(thai_char_class(*c), ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel))
            {
                // Replace the existing vowel in-place at its absolute offset.
                // The new char lands at vowel_off; any tone/diac after it shifts right.
                ThaiInputAction::Replace { slot_offset: vowel_off, slot_bytes: existing_vowel.len_utf8(), cons_swap: None, replacement: None }
            } else {
                // No existing vowel.  Insert before a tone/diac if present, else append.
                let caret_pos = tcb.last().map(|&(o, c)| o + c.len_utf8()).unwrap_or(0);
                let slot_offset = tcb
                    .iter()
                    .find(|(_, c)| matches!(thai_char_class(*c), ThaiCharClass::Tone | ThaiCharClass::Diacritic))
                    .map(|&(o, _)| o)
                    .unwrap_or(caret_pos);
                if slot_offset == caret_pos {
                    ThaiInputAction::Accept
                } else {
                    ThaiInputAction::Replace { slot_offset, slot_bytes: 0, cons_swap: None, replacement: None }
                }
            }
        }

        ThaiCharClass::Tone | ThaiCharClass::Diacritic => {
            // Find an existing tone/diacritic in the TCB (slot 2).
            if let Some(&(tone_off, existing)) =
                tcb.iter().find(|(_, c)| matches!(thai_char_class(*c), ThaiCharClass::Tone | ThaiCharClass::Diacritic))
            {
                ThaiInputAction::Replace { slot_offset: tone_off, slot_bytes: existing.len_utf8(), cons_swap: None, replacement: None }
            } else {
                ThaiInputAction::Accept
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cls(c: char) -> ThaiCharClass {
        thai_char_class(c)
    }

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
        let prev = text.char_indices().next_back().map(|(o, c)| (o, c));
        thai_input_action(&tcb, new_char, prev)
    }

    #[test]
    fn tcb_input_rules() {
        let kon = 'ก'; // consonant
        let yor_ying = 'ญ';
        let tho_than = 'ฐ';
        let sara_i = '\u{0E34}'; // sara i  (upper vowel)
        let sara_ii = '\u{0E35}'; // sara ii (upper vowel)
        let sara_u = '\u{0E38}'; // sara u  (lower vowel)
        let _sara_uu = '\u{0E39}'; // sara uu (lower vowel)
        let mai_ek = '\u{0E48}'; // mai ek  (tone)
        let mai_tho = '\u{0E49}'; // mai tho (tone)
        let maitaikhu = '\u{0E47}'; // maitaikhu (diacritic)

        // ── Basic rules ─────────────────────────────────────────────────────────

        // Consonant after consonant: start new TCB (Accept).
        assert_eq!(action("ก", kon), ThaiInputAction::Accept);

        // Vowel after consonant: Accept (append to slot 1).
        assert_eq!(action("ก", sara_i), ThaiInputAction::Accept);

        // Second vowel after consonant+vowel: Replace existing vowel in-place.
        let s = "กิ"; // ก(0) + sara_i(3)
        match action(s, sara_ii) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("expected Replace at 3, got {other:?}"),
        }

        // Replacing upper vowel with a lower vowel: same slot, no cons swap (ก has none).
        match action(s, sara_u) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("expected Replace at 3, got {other:?}"),
        }

        // Tone after consonant: Accept (slot 2 free).
        assert_eq!(action("ก", mai_ek), ThaiInputAction::Accept);

        // Tone after consonant+vowel: Accept (slot 2 free).
        assert_eq!(action("กิ", mai_ek), ThaiInputAction::Accept);

        // Second tone after consonant+vowel+tone: Replace existing tone.
        // กิ้ = ก(0) + sara_i(3) + mai_ek(6); caret at 9.
        let full = "กิ้"; // mai ek U+0E49
        match action(full, mai_tho) {
            ThaiInputAction::Replace { slot_offset: 6, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("expected Replace at 6, got {other:?}"),
        }

        // Diacritic replacing existing tone.
        match action(full, maitaikhu) {
            ThaiInputAction::Replace { slot_offset: 6, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("expected Replace at 6, got {other:?}"),
        }

        // Tone replacing existing diacritic.
        // กิ็ = ก(0) + sara_i(3) + maitaikhu(6)
        let diac = "กิ็";
        match action(diac, mai_ek) {
            ThaiInputAction::Replace { slot_offset: 6, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("expected Replace at 6, got {other:?}"),
        }

        // ── Bug: lower vowel must replace vowel, not tone mark ───────────────────
        // กี้ = ก(0) + sara_ii(3) + mai_tho(6), caret at 9.
        // Typing sara_u should replace sara_ii at offset 3 (not mai_tho at 6).
        let cons_upper_vowel_tone = "กี้";
        match action(cons_upper_vowel_tone, sara_u) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("lower vowel must replace vowel at 3, got {other:?}"),
        }

        // Symmetry: upper vowel also replaces the vowel slot, not the tone.
        match action(cons_upper_vowel_tone, sara_i) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("upper vowel must replace vowel at 3, got {other:?}"),
        }

        // ── No-consonant rejection ───────────────────────────────────────────────

        // Vowel with no preceding consonant: Reject.
        assert_eq!(action("", sara_i), ThaiInputAction::Reject);
        assert_eq!(action("hello", sara_i), ThaiInputAction::Reject);

        // Tone with no preceding consonant: Reject.
        assert_eq!(action("", mai_ek), ThaiInputAction::Reject);

        // Non-Thai passes through.
        assert_eq!(action("ก", 'a'), ThaiInputAction::Accept);
        assert_eq!(action("ก", ' '), ThaiInputAction::Accept);

        // ฐ and ญ with lower vowel: Accept (no short-tail swap).
        assert_eq!(action(&yor_ying.to_string(), sara_u), ThaiInputAction::Accept);
        assert_eq!(action(&tho_than.to_string(), sara_u), ThaiInputAction::Accept);
        // ฐ and ญ with upper vowel: Accept.
        assert_eq!(action(&yor_ying.to_string(), sara_i), ThaiInputAction::Accept);
        assert_eq!(action(&tho_than.to_string(), sara_i), ThaiInputAction::Accept);
    }

    #[test]
    fn leading_vowel_combinations() {
        let sara_e = '\u{0E40}'; // เ
        let sara_ae = '\u{0E41}'; // แ (U+0E41)

        // First เ: no previous leading vowel → Accept normally.
        assert_eq!(action("", sara_e), ThaiInputAction::Accept);
        assert_eq!(action("ก", sara_e), ThaiInputAction::Accept);

        // Second เ immediately after first เ → Replace the เ with แ.
        // "เ" = U+0E40 at byte offset 0, length 3.
        match action("\u{0E40}", sara_e) {
            ThaiInputAction::Replace { slot_offset: 0, slot_bytes: 3, cons_swap: None, replacement: Some('\u{0E41}') } => {}
            other => panic!("เ+เ must Replace to produce แ, got {other:?}"),
        }

        // Other leading vowels do not trigger the combination.
        assert_eq!(action("\u{0E41}", sara_e), ThaiInputAction::Accept); // แ+เ → Accept
        assert_eq!(action("\u{0E40}", '\u{0E41}'), ThaiInputAction::Accept); // เ+แ → Accept

        let _ = sara_ae;
    }
}

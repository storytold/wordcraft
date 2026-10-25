//! Thai input sequence checking — Thai Character Block (TCB) rules.
//!
//! A TCB is the unit of Thai composition: one base consonant, optionally followed by
//! combining marks.  The valid forms are:
//!
//! ## TCB structure
//! ```text
//! Form A (normal):   [consonant]  [upper_vowel | lower_vowel]?  [tone | diacritic]?
//! Form B (pali):     [consonant]  [pali_mark]
//! Form C (sara_am):  [consonant]  [upper_vowel | lower_vowel]?  [sara_am]
//! ```
//!
//! Pali marks (◌ํ nikhahit U+0E4D and ◌ฺ phinthu U+0E3A) cannot share a TCB with
//! upper/lower vowels, tone marks, diacritics, or sara_am (ำ).  They occupy the consonant
//! alone.
//!
//! ## Replacement rules
//! - Typing an upper/lower vowel when slot 1 is already filled → replaces it.
//! - Typing a tone/diacritic when slot 2 is already filled → replaces it.
//! - Typing a pali mark when any vowel/tone/diac/sara_am is present → replaces them all.
//! - Typing a vowel/tone/diac when a pali mark is present → replaces the pali mark.
//! - Typing an upper/lower vowel when sara_am (ำ) is in the TCB → inserts between
//!   the consonant and ำ, replacing an existing vowel if present.
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
    /// Below vowel (TCT_THAILOWERVOWEL): ◌ุ ◌ู — occupies slot 1.
    LowerVowel,
    /// Tone mark (TCT_THAITONE): ◌้ ◌่ ◌๊ ◌๋ ◌์ — occupies slot 2.
    Tone,
    /// Diacritic (TCT_THAIDIAC): ◌็ (maitaikhu U+0E47) — occupies slot 2.
    Diacritic,
    /// Pali mark: ◌ํ (nikhahit U+0E4D) and ◌ฺ (phinthu U+0E3A).
    ///
    /// Pali marks are used in Sanskrit/Pali loanwords.  They can only appear directly
    /// after a consonant — never alongside upper/lower vowels, tone marks, diacritics,
    /// or sara_am (ำ) in the same TCB.
    ///
    /// Nikhahit (U+0E4D) additionally combines with a following sara_aa (า) to produce
    /// sara_am (ำ U+0E33) — see the `Other` arm in [`thai_input_action`].
    PaliMark,
    /// Sara am ำ (U+0E33): produced by the nikhahit + sara_aa combination.
    ///
    /// In the TCB it acts as a terminal vowel that follows the consonant (and any
    /// upper/lower vowel inserted between them).  Typing an upper/lower vowel when sara_am
    /// is present inserts it before ำ; typing a pali mark when sara_am is present
    /// replaces ำ.
    SaraAm,
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

        // Below vowels (TCT_THAILOWERVOWEL): ◌ุ ◌ู
        // Note: phinthu (U+0E3A) is a Pali mark, not a vowel.
        0x0E38 | 0x0E39
        => ThaiCharClass::LowerVowel,

        // Pali marks: phinthu ◌ฺ (U+0E3A) and nikhahit ◌ํ (U+0E4D).
        // These are used in Sanskrit/Pali loanwords and cannot co-exist with vowels,
        // tone marks, or diacritics in the same TCB.
        0x0E3A | 0x0E4D
        => ThaiCharClass::PaliMark,

        // Sara am ำ (U+0E33): produced when nikhahit (U+0E4D) is followed by sara_aa (า).
        0x0E33
        => ThaiCharClass::SaraAm,

        // Diacritic (TCT_THAIDIAC): ◌็ (maitaikhu U+0E47) — occupies slot 2.
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
/// that form a valid Thai Character Block (consonant + optional combining marks).
/// Returns a slice of 0–3 `(byte_offset, char)` pairs in logical order.
///
/// Recognises all three TCB forms:
/// - Form A (normal):  `[consonant][vowel?][tone|diac?]`
/// - Form B (pali):    `[consonant][pali_mark]`
/// - Form C (sara_am): `[consonant][vowel?][sara_am]`
pub fn current_tcb(text: &str, caret_off: usize) -> Vec<(usize, char)> {
    let before = match text.get(..caret_off) {
        Some(s) => s,
        None => return Vec::new(),
    };
    // Collect up to 3 chars before the caret, newest first, then restore logical order.
    let mut chars: Vec<(usize, char)> = before.char_indices().rev().take(3).collect();
    chars.reverse();

    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }

    let cls_of = |i: usize| thai_char_class(chars[i].1);
    let mut end = n; // exclusive; we peel right-to-left, then check the head is a consonant

    // Step 1: peel at most one rightmost slot occupant.
    // SaraAm, Tone/Diacritic, and PaliMark are mutually exclusive in this slot.
    match cls_of(end - 1) {
        ThaiCharClass::SaraAm
        | ThaiCharClass::Tone
        | ThaiCharClass::Diacritic
        | ThaiCharClass::PaliMark => {
            end -= 1;
        }
        _ => {}
    }

    // Step 2: if what we peeled was NOT a PaliMark, also peel an optional vowel.
    // (PaliMark is incompatible with vowels — no vowel to peel in Form B.)
    let had_pali = end < n && cls_of(end) == ThaiCharClass::PaliMark;
    if !had_pali
        && end > 0
        && matches!(cls_of(end - 1), ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel)
    {
        end -= 1;
    }

    // Step 3: the remaining head must be a consonant.
    if end == 0 || cls_of(end - 1) != ThaiCharClass::Consonant {
        return Vec::new();
    }

    chars[end - 1..].to_vec()
}

/// Determine what action to take when the user types `new_char` given the current TCB
/// (0–3 chars immediately before the caret, as returned by [`current_tcb`]) and the
/// character immediately before the caret (which may differ from the TCB when the caret
/// sits after a leading vowel).
///
/// - `tcb`: result of `current_tcb(text, caret_off)` — may be empty.
/// - `prev_char`: `Some((byte_offset, char))` for the char immediately before the caret,
///   or `None` if the caret is at the start of the paragraph.
pub fn thai_input_action(
    tcb: &[(usize, char)],
    new_char: char,
    prev_char: Option<(usize, char)>,
) -> ThaiInputAction {
    let new_class = thai_char_class(new_char);

    match new_class {
        // Leading vowel: check for เ+เ=แ substitution; otherwise pass through.
        ThaiCharClass::LeadingVowel => {
            // เ (U+0E40) typed immediately after another เ → replace with แ (U+0E41).
            if new_char == '\u{0E40}'
                && let Some((off, '\u{0E40}')) = prev_char
            {
                return ThaiInputAction::Replace {
                    slot_offset: off,
                    slot_bytes: '\u{0E40}'.len_utf8(),
                    cons_swap: None,
                    replacement: Some('\u{0E41}'),
                };
            }
            ThaiInputAction::Accept
        }

        // Other characters (and sara_am typed directly): pass through, with one exception —
        // SARA_AA (า U+0E32) typed after a consonant+NIKHAHIT (ํ U+0E4D) combines
        // the two characters into SARA_AM (ำ U+0E33).
        ThaiCharClass::Other | ThaiCharClass::SaraAm => {
            if new_char == '\u{0E32}'
                && let Some(&(off, '\u{0E4D}')) = tcb.iter().find(|(_, c)| *c == '\u{0E4D}')
            {
                return ThaiInputAction::Replace {
                    slot_offset: off,
                    slot_bytes: '\u{0E4D}'.len_utf8(),
                    cons_swap: None,
                    replacement: Some('\u{0E33}'),
                };
            }
            ThaiInputAction::Accept
        }

        // A consonant always starts a fresh TCB.
        ThaiCharClass::Consonant => ThaiInputAction::Accept,

        // A combining character with no TCB context (no preceding consonant) is invalid.
        _ if tcb.is_empty() => ThaiInputAction::Reject,

        // ── Pali marks (◌ํ nikhahit, ◌ฺ phinthu) ─────────────────────────────────────
        ThaiCharClass::PaliMark => {
            // Replace an existing pali mark with this one.
            if let Some(&(pali_off, existing_pali)) =
                tcb.iter().find(|(_, c)| thai_char_class(*c) == ThaiCharClass::PaliMark)
            {
                return ThaiInputAction::Replace {
                    slot_offset: pali_off,
                    slot_bytes: existing_pali.len_utf8(),
                    cons_swap: None,
                    replacement: None,
                };
            }
            // Pali marks cannot co-exist with vowels, tone, diacritics, or sara_am.
            // If any are present, delete them all and place the pali mark in their stead.
            let first_incompat = tcb.iter().find(|(_, c)| {
                matches!(
                    thai_char_class(*c),
                    ThaiCharClass::UpperVowel
                        | ThaiCharClass::LowerVowel
                        | ThaiCharClass::Tone
                        | ThaiCharClass::Diacritic
                        | ThaiCharClass::SaraAm
                )
            });
            if let Some(&(incompat_off, _)) = first_incompat {
                let last = tcb.last().unwrap(); // safe: tcb non-empty (checked above)
                let delete_bytes = last.0 + last.1.len_utf8() - incompat_off;
                return ThaiInputAction::Replace {
                    slot_offset: incompat_off,
                    slot_bytes: delete_bytes,
                    cons_swap: None,
                    replacement: None,
                };
            }
            // TCB has only a consonant: accept.
            ThaiInputAction::Accept
        }

        // ── Upper and lower vowels ─────────────────────────────────────────────────────
        ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel => {
            // Typing a vowel while a pali mark is present replaces the pali mark.
            if let Some(&(pali_off, existing_pali)) =
                tcb.iter().find(|(_, c)| thai_char_class(*c) == ThaiCharClass::PaliMark)
            {
                return ThaiInputAction::Replace {
                    slot_offset: pali_off,
                    slot_bytes: existing_pali.len_utf8(),
                    cons_swap: None,
                    replacement: None,
                };
            }
            // Sara_am (ำ) in the TCB: insert the vowel between the consonant and ำ,
            // replacing any vowel already there.
            if let Some(&(sara_am_off, _)) =
                tcb.iter().find(|(_, c)| thai_char_class(*c) == ThaiCharClass::SaraAm)
            {
                if let Some(&(vowel_off, existing_vowel)) = tcb.iter().find(|(_, c)| {
                    matches!(
                        thai_char_class(*c),
                        ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel
                    )
                }) {
                    return ThaiInputAction::Replace {
                        slot_offset: vowel_off,
                        slot_bytes: existing_vowel.len_utf8(),
                        cons_swap: None,
                        replacement: None,
                    };
                }
                // No vowel yet: insert before sara_am.
                return ThaiInputAction::Replace {
                    slot_offset: sara_am_off,
                    slot_bytes: 0,
                    cons_swap: None,
                    replacement: None,
                };
            }
            // Normal case: replace an existing vowel in-place.
            if let Some(&(vowel_off, existing_vowel)) = tcb.iter().find(|(_, c)| {
                matches!(
                    thai_char_class(*c),
                    ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel
                )
            }) {
                ThaiInputAction::Replace {
                    slot_offset: vowel_off,
                    slot_bytes: existing_vowel.len_utf8(),
                    cons_swap: None,
                    replacement: None,
                }
            } else {
                // No existing vowel: insert before a tone/diac if present, else append.
                let caret_pos = tcb.last().map(|&(o, c)| o + c.len_utf8()).unwrap_or(0);
                let slot_offset = tcb
                    .iter()
                    .find(|(_, c)| {
                        matches!(
                            thai_char_class(*c),
                            ThaiCharClass::Tone | ThaiCharClass::Diacritic
                        )
                    })
                    .map(|&(o, _)| o)
                    .unwrap_or(caret_pos);
                if slot_offset == caret_pos {
                    ThaiInputAction::Accept
                } else {
                    ThaiInputAction::Replace {
                        slot_offset,
                        slot_bytes: 0,
                        cons_swap: None,
                        replacement: None,
                    }
                }
            }
        }

        // ── Tone marks and diacritics ─────────────────────────────────────────────────
        ThaiCharClass::Tone | ThaiCharClass::Diacritic => {
            // Typing a tone/diac while a pali mark is present replaces the pali mark.
            if let Some(&(pali_off, existing_pali)) =
                tcb.iter().find(|(_, c)| thai_char_class(*c) == ThaiCharClass::PaliMark)
            {
                return ThaiInputAction::Replace {
                    slot_offset: pali_off,
                    slot_bytes: existing_pali.len_utf8(),
                    cons_swap: None,
                    replacement: None,
                };
            }
            // Replace an existing tone/diac, otherwise accept.
            if let Some(&(tone_off, existing)) = tcb
                .iter()
                .find(|(_, c)| matches!(thai_char_class(*c), ThaiCharClass::Tone | ThaiCharClass::Diacritic))
            {
                ThaiInputAction::Replace {
                    slot_offset: tone_off,
                    slot_bytes: existing.len_utf8(),
                    cons_swap: None,
                    replacement: None,
                }
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
        // Lower vowels (sara u, sara uu only — phinthu is now PaliMark)
        assert_eq!(cls('\u{0E38}'), ThaiCharClass::LowerVowel); // sara u
        assert_eq!(cls('\u{0E39}'), ThaiCharClass::LowerVowel); // sara uu
        // Pali marks
        assert_eq!(cls('\u{0E3A}'), ThaiCharClass::PaliMark); // phinthu ◌ฺ
        assert_eq!(cls('\u{0E4D}'), ThaiCharClass::PaliMark); // nikhahit ◌ํ
        // Sara am
        assert_eq!(cls('\u{0E33}'), ThaiCharClass::SaraAm); // ำ
        // Tone marks
        assert_eq!(cls('\u{0E48}'), ThaiCharClass::Tone); // mai ek
        assert_eq!(cls('\u{0E4C}'), ThaiCharClass::Tone); // thanthakhat
        // Diacritic (maitaikhu only)
        assert_eq!(cls('\u{0E47}'), ThaiCharClass::Diacritic); // maitaikhu ◌็
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
        // Consonant + pali mark (Form B).
        assert_eq!(tcb("กํ"), vec!['ก', '\u{0E4D}']); // nikhahit
        assert_eq!(tcb("กฺ"), vec!['ก', '\u{0E3A}']); // phinthu
        // Consonant + sara_am (Form C).
        assert_eq!(tcb("กำ"), vec!['ก', '\u{0E33}']);
        // Consonant + vowel + sara_am (Form C with vowel).
        assert_eq!(tcb("กิำ"), vec!['ก', '\u{0E34}', '\u{0E33}']);
        // After a space the TCB resets.
        assert_eq!(tcb("ข ก"), vec!['ก']);
        // Latin text: no TCB.
        assert_eq!(tcb("hello"), Vec::<char>::new());
        // Empty.
        assert_eq!(tcb(""), Vec::<char>::new());
    }

    fn action(text: &str, new_char: char) -> ThaiInputAction {
        let tcb = current_tcb(text, text.len());
        let prev = text.char_indices().next_back();
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

    #[test]
    fn nikhahit_sara_aa_combines_to_sara_am() {
        let nikhahit = '\u{0E4D}'; // ◌ํ
        let sara_aa = '\u{0E32}'; // า
        let sara_am = '\u{0E33}'; // ำ

        // NIKHAHIT is now classified as PaliMark.
        assert_eq!(cls(nikhahit), ThaiCharClass::PaliMark);

        // Typing SARA_AA after consonant alone: plain Accept (no NIKHAHIT present).
        assert_eq!(action("ก", sara_aa), ThaiInputAction::Accept);

        // Typing SARA_AA after consonant+NIKHAHIT: Replace NIKHAHIT with SARA_AM.
        // "กํ" = ก(0,3) + ํ(3,3); caret at 6.
        let s = "กํ";
        match action(s, sara_aa) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: Some(r) } if r == sara_am => {}
            other => panic!("expected Replace(NIKHAHIT→SARA_AM), got {other:?}"),
        }

        // SARA_AA on its own (no preceding Thai): Accept.
        assert_eq!(action("hello", sara_aa), ThaiInputAction::Accept);
        assert_eq!(action("", sara_aa), ThaiInputAction::Accept);
    }

    #[test]
    fn pali_mark_input_rules() {
        let kon = 'ก'; // consonant
        let nikhahit = '\u{0E4D}'; // ◌ํ PaliMark
        let phinthu = '\u{0E3A}'; // ◌ฺ PaliMark
        let sara_i = '\u{0E34}'; // sara i  (upper vowel)
        let sara_u = '\u{0E38}'; // sara u  (lower vowel)
        let mai_ek = '\u{0E48}'; // mai ek  (tone)
        let maitaikhu = '\u{0E47}'; // maitaikhu (diacritic)

        // ── Pali mark after consonant only → Accept ──────────────────────────────
        assert_eq!(action("ก", nikhahit), ThaiInputAction::Accept);
        assert_eq!(action("ก", phinthu), ThaiInputAction::Accept);

        // ── Pali mark after consonant+vowel → replaces vowel (and any tone/diac) ─
        // กิ = ก(0) + ิ(3); nikhahit should delete ิ and go to offset 3
        match action("กิ", nikhahit) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("pali mark must replace vowel at 3, got {other:?}"),
        }

        // กิ้ = ก(0) + ิ(3) + ้(6); nikhahit must delete ิ+้ (6 bytes) starting at 3
        match action("กิ้", nikhahit) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 6, cons_swap: None, replacement: None } => {}
            other => panic!("pali mark must replace vowel+tone (6 B) at 3, got {other:?}"),
        }

        // กิ็ = ก(0) + ิ(3) + ็(6); phinthu must delete ิ+็ (6 bytes) starting at 3
        match action("กิ็", phinthu) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 6, cons_swap: None, replacement: None } => {}
            other => panic!("phinthu must replace vowel+diac (6 B) at 3, got {other:?}"),
        }

        // ก้ = ก(0) + ้(3) (tone with no vowel); pali mark replaces tone
        match action("ก้", nikhahit) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("pali mark must replace tone at 3, got {other:?}"),
        }

        // ก็ = ก(0) + ็(3) (diac with no vowel); pali mark replaces diac
        match action("ก็", phinthu) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("phinthu must replace diac at 3, got {other:?}"),
        }

        // ── Second pali mark → replaces existing pali mark ───────────────────────
        // กํ = ก(0) + ํ(3); typing phinthu replaces nikhahit
        match action("กํ", phinthu) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("second pali mark must replace first at 3, got {other:?}"),
        }

        // ── No-consonant rejection ────────────────────────────────────────────────
        assert_eq!(action("", nikhahit), ThaiInputAction::Reject);
        assert_eq!(action("hello", phinthu), ThaiInputAction::Reject);

        // ── Vowel replaces pali mark ──────────────────────────────────────────────
        // กํ = ก(0) + ํ(3); typing sara_i should replace nikhahit at 3
        match action("กํ", sara_i) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("vowel must replace pali mark at 3, got {other:?}"),
        }
        match action("กฺ", sara_u) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("lower vowel must replace phinthu at 3, got {other:?}"),
        }

        // ── Tone/diac replaces pali mark ──────────────────────────────────────────
        match action("กํ", mai_ek) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("tone must replace pali mark at 3, got {other:?}"),
        }
        match action("กฺ", maitaikhu) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("diacritic must replace phinthu at 3, got {other:?}"),
        }

        // Consonant after pali mark: new TCB (Accept).
        assert_eq!(action("กํ", kon), ThaiInputAction::Accept);
    }

    #[test]
    fn sara_am_vowel_insertion() {
        let sara_i = '\u{0E34}'; // sara i  (upper vowel)
        let sara_u = '\u{0E38}'; // sara u  (lower vowel)
        let sara_ii = '\u{0E35}'; // sara ii (upper vowel)
        let sara_am = '\u{0E33}'; // ำ

        // กำ = ก(0,3) + ำ(3,3); caret at 6.
        // Typing sara_i inserts it BEFORE ำ at offset 3 (slot_bytes=0).
        match action("กำ", sara_i) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 0, cons_swap: None, replacement: None } => {}
            other => panic!("vowel must insert before sara_am at 3, got {other:?}"),
        }
        match action("กำ", sara_u) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 0, cons_swap: None, replacement: None } => {}
            other => panic!("lower vowel must insert before sara_am at 3, got {other:?}"),
        }

        // กิำ = ก(0,3) + ิ(3,3) + ำ(6,3); caret at 9.
        // Typing sara_ii replaces existing sara_i at offset 3.
        match action("กิำ", sara_ii) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("second vowel must replace first before sara_am, got {other:?}"),
        }
        match action("กิำ", sara_u) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("lower vowel must replace upper vowel before sara_am, got {other:?}"),
        }

        // sara_am in TCB recognised correctly.
        assert_eq!(tcb("กำ"), vec!['ก', sara_am]);
        assert_eq!(tcb("กิำ"), vec!['ก', sara_i, sara_am]);
    }
}

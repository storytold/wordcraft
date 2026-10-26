//! Thai input sequence checking — Thai Character Block (TCB) rules.
//!
//! Character name constants (e.g. [`crate::thai_char::NIKHAHIT`]) and type-class flags
//! are defined in [`crate::thai_char`].
//!
//! A TCB is the unit of Thai composition: one base consonant, optionally followed by
//! combining marks.  The valid forms are:
//!
//! ## TCB structure
//! ```text
//! Form A (normal):  [consonant]  [upper_vowel | lower_vowel | sara_am]?  [tone | karan | diacritic]?
//! Form A2 (tone-am):[consonant]  [tone]  [sara_am]   — canonical Thai order
//! Form B (pali):    [consonant]  [pali_mark]
//! ```
//!
//! Sara am (ำ U+0E33) occupies the same vowel slot as upper/lower vowels, but is its own
//! class (`SaraAm`) in the input sequencer.
//!
//! Pali marks (◌ํ nikhahit U+0E4D and ◌ฺ phinthu U+0E3A) cannot share a TCB with
//! upper/lower vowels, tone marks, diacritics, karan, or sara_am (ำ).  They occupy the
//! consonant alone.
//!
//! ## Replacement rules
//! - Typing an upper/lower vowel or sara_am when slot 1 is already filled → replaces it.
//! - Typing a tone/karan when slot 2 is already filled → replaces it.
//! - Typing maitaikhu (◌็) when a vowel occupies slot 1 (and slot 2 is free) → replaces it.
//! - Typing a pali mark when any vowel/tone/diac/karan is present → replaces them all.
//! - Typing a vowel/tone/diac when a pali mark is present → replaces the pali mark.
//! - Typing a consonant always starts a new TCB.
//! - Typing a combining character with no preceding consonant → rejected.

use crate::thai_char::{
    MAIHUNAKAT, MAITAIKHU, MAIEK, MAITHO, MAITRI, MAICHATTAWA, THANTHAKHAT,
    NIKHAHIT, PHINTHU, SARA_AM, SARA_I, SARA_II, SARA_UE, SARA_UEE, SARA_U, SARA_UU,
    SARA_E, SARA_AE, SARA_O, MAIMUAN, MAIMALAI,
};

/// Classification of a Unicode Thai character for input-sequence purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThaiCharClass {
    /// Consonant (TCT_THAICONS / THAICONSLOWER / THAICONSUPPER): the mandatory TCB base.
    Consonant,
    /// Above vowel (TCT_THAIUPPERVOWEL): ◌ั ◌ิ ◌ี ◌ึ ◌ื — occupies slot 1.
    UpperVowel,
    /// Below vowel (TCT_THAILOWERVOWEL): ◌ุ ◌ู — occupies slot 1.
    LowerVowel,
    /// Tone mark (ctMAI1234): ◌่ ◌้ ◌๊ ◌๋ — occupies slot 2.
    /// Note: ์ (thanthakhat U+0E4C) is `Karan`, not `Tone`.
    Tone,
    /// Diacritic (ctTYKOU): ◌็ (maitaikhu U+0E47) — occupies slot 2.
    Diacritic,
    /// Karan (ctKARAN): ์ (thanthakhat U+0E4C) — occupies slot 2.
    /// Silent-consonant marker; treated separately from tone marks.
    Karan,
    /// Pali mark: ◌ํ (nikhahit U+0E4D) and ◌ฺ (phinthu U+0E3A).
    ///
    /// Pali marks are used in Sanskrit/Pali loanwords.  They can only appear directly
    /// after a consonant — never alongside upper/lower vowels, tone marks, diacritics,
    /// karan, or sara_am (ำ) in the same TCB.
    ///
    /// Nikhahit (U+0E4D) additionally combines with a following sara_aa (า) to produce
    /// sara_am (ำ U+0E33) — see the `Other` arm in [`thai_input_action`].
    PaliMark,
    /// Sara am ำ (U+0E33, ctUM): occupies the same vowel slot as
    /// `UpperVowel`/`LowerVowel`, but is its own distinct class in the input sequencer.
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
    /// Delete `slot_bytes` bytes at `slot_offset`, then insert the (possibly replaced) character
    /// at the **original caret position adjusted for the deletion** rather than at `slot_offset`.
    ///
    /// Used when the deletion point and the insertion point differ — specifically for
    /// `[cons, vowel, tone] + ำ → [cons, tone, ำ]`: the vowel is deleted but ำ is appended
    /// after the tone mark that stays in place.
    ///
    /// Execution order in the engine:
    /// 1. If `cons_swap` is `Some`, replace the consonant in-place.
    /// 2. Delete `slot_bytes` bytes at `slot_offset`.
    /// 3. Move caret to `original_caret_offset − slot_bytes`.
    /// 4. Insert `replacement` (or the typed character if `None`) at the caret.
    DeleteThenAppend {
        slot_offset: usize,
        slot_bytes: usize,
        cons_swap: Option<ConsSwap>,
        /// Character to insert instead of the typed character, or `None` to insert as typed.
        replacement: Option<char>,
    },
    /// The character is structurally invalid here; reject it (no insertion, optional beep).
    Reject,
}

/// Classify a Unicode Thai character for input-sequence purposes.
pub fn thai_char_class(c: char) -> ThaiCharClass {
    match u32::from(c) {
        // Consonants — normal (TCT_THAICONS): ก–ญ ฐ–น ผ พ ภ–ร ล ว–ฮ
        0x0E01..=0x0E0D
        | 0x0E10..=0x0E1A
        | 0x0E1C | 0x0E1E
        | 0x0E20..=0x0E23
        | 0x0E25
        | 0x0E27..=0x0E2E
        => ThaiCharClass::Consonant,

        // Consonants — lower tail (TCT_THAICONSLOWER): ฎ ฏ ฤ ฦ  (DOCHADA TOPATAK RU LU)
        0x0E0E | 0x0E0F | 0x0E24 | 0x0E26
        => ThaiCharClass::Consonant,

        // Consonants — upper tail (TCT_THAICONSUPPER): ป ฝ ฟ  (POPLA FOFA FOFAN)
        0x0E1B | 0x0E1D | 0x0E1F
        => ThaiCharClass::Consonant,

        // Above vowels (TCT_THAIUPPERVOWEL): ◌ั ◌ิ ◌ี ◌ึ ◌ื
        // MAIHUNAKAT | SARA_I SARA_II SARA_UE SARA_UEE
        _ if c == MAIHUNAKAT
          || c == SARA_I || c == SARA_II || c == SARA_UE || c == SARA_UEE
        => ThaiCharClass::UpperVowel,

        // Below vowels (TCT_THAILOWERVOWEL): ◌ุ ◌ู  (SARA_U SARA_UU)
        _ if c == SARA_U || c == SARA_UU
        => ThaiCharClass::LowerVowel,

        // Pali marks: PHINTHU ◌ฺ and NIKHAHIT ◌ํ — Sanskrit/Pali loanword markers.
        _ if c == PHINTHU || c == NIKHAHIT
        => ThaiCharClass::PaliMark,

        // SARA_AM ำ (ctUM): occupies the vowel slot; own distinct class.
        _ if c == SARA_AM
        => ThaiCharClass::SaraAm,

        // Diacritic (ctTYKOU): MAITAIKHU ◌็ — occupies slot 2.
        _ if c == MAITAIKHU
        => ThaiCharClass::Diacritic,

        // Karan (ctKARAN): THANTHAKHAT ์ — silent-consonant marker, slot 2.
        _ if c == THANTHAKHAT
        => ThaiCharClass::Karan,

        // Tone marks (ctMAI1234): ◌่ ◌้ ◌๊ ◌๋  (NOT ์ — that is Karan)
        _ if c == MAIEK || c == MAITHO || c == MAITRI || c == MAICHATTAWA
        => ThaiCharClass::Tone,

        // Leading vowels (TCT_LEADINGVOWEL): เ แ โ ใ ไ
        _ if c == SARA_E || c == SARA_AE || c == SARA_O || c == MAIMUAN || c == MAIMALAI
        => ThaiCharClass::LeadingVowel,

        _ => ThaiCharClass::Other,
    }
}


/// Extract the current TCB — up to 3 chars immediately before `caret_off` in `text`
/// that form a valid Thai Character Block (consonant + optional combining marks).
/// Returns a slice of 0–3 `(byte_offset, char)` pairs in logical order.
///
/// Valid TCB patterns (pattern matched explicitly):
/// - 3-char: `(Consonant, UpperVowel|LowerVowel, Tone|Karan|Diacritic)`
/// - 3-char: `(Consonant, Tone, SaraAm)` — canonical Thai order
/// - 3-char: `(Consonant, SaraAm, Tone)` — alternative Unicode order
/// - 2-char: `(Consonant, UpperVowel|LowerVowel|Tone|Karan|Diacritic|PaliMark|SaraAm)`
/// - 1-char: `Consonant`
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

    let cls = |i: usize| thai_char_class(chars[i].1);

    // Pattern match on the last 1–3 chars explicitly.
    match n {
        3 => {
            let (c0, c1, c2) = (cls(0), cls(1), cls(2));
            let valid = match (c0, c1, c2) {
                // Form A full TCB: consonant + vowel + slot-2
                (ThaiCharClass::Consonant,
                 ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel,
                 ThaiCharClass::Tone | ThaiCharClass::Karan | ThaiCharClass::Diacritic) => true,
                // Canonical: consonant + tone + sara_am
                (ThaiCharClass::Consonant, ThaiCharClass::Tone, ThaiCharClass::SaraAm) => true,
                // Alternative Unicode order: consonant + sara_am + tone
                (ThaiCharClass::Consonant, ThaiCharClass::SaraAm, ThaiCharClass::Tone) => true,
                _ => false,
            };
            if valid {
                chars.to_vec()
            } else {
                // Fall back: try 2-char suffix (last 2 of the 3 collected)
                let (c1, c2) = (cls(1), cls(2));
                let valid2 = matches!(
                    (c1, c2),
                    (ThaiCharClass::Consonant,
                     ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel
                     | ThaiCharClass::Tone | ThaiCharClass::Karan
                     | ThaiCharClass::Diacritic | ThaiCharClass::PaliMark
                     | ThaiCharClass::SaraAm)
                );
                if valid2 {
                    chars[1..].to_vec()
                } else if c2 == ThaiCharClass::Consonant {
                    chars[2..].to_vec()
                } else {
                    Vec::new()
                }
            }
        }
        2 => {
            let (c0, c1) = (cls(0), cls(1));
            let valid = matches!(
                (c0, c1),
                (ThaiCharClass::Consonant,
                 ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel
                 | ThaiCharClass::Tone | ThaiCharClass::Karan
                 | ThaiCharClass::Diacritic | ThaiCharClass::PaliMark
                 | ThaiCharClass::SaraAm)
            );
            if valid {
                chars.to_vec()
            } else if c1 == ThaiCharClass::Consonant {
                chars[1..].to_vec()
            } else {
                Vec::new()
            }
        }
        1 => {
            if cls(0) == ThaiCharClass::Consonant {
                chars.to_vec()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// Determine what action to take when the user types `new_char` given the current TCB
/// (0–3 chars immediately before the caret, as returned by [`current_tcb`]) and the
/// character immediately before the caret (which may differ from the TCB when the caret
/// sits after a leading vowel).
///
/// Logic ported from the Thai input sequence algorithm.
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

        // Other characters: pass through, with one exception —
        // SARA_AA (า U+0E32) typed after a consonant+NIKHAHIT (ํ U+0E4D) combines
        // the two characters into SARA_AM (ำ U+0E33).
        ThaiCharClass::Other => {
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

            // Vowel upgrade: Nikhahit (◌ํ) after Sara_I → Sara_UE (◌ึ).
            // Sara_I is replaced in the document; Nikhahit is consumed (not stored).
            // Any slot-2 mark (tone/karan/diac) that follows Sara_I stays in place.
            if new_char == NIKHAHIT {
                if let Some(&(sara_off, sara_ch)) = tcb.iter().find(|(_, c)| *c == SARA_I) {
                    return ThaiInputAction::Replace {
                        slot_offset: sara_off,
                        slot_bytes: sara_ch.len_utf8(),
                        cons_swap: None,
                        replacement: Some(SARA_UE),
                    };
                }
            }

            // Pali marks cannot co-exist with vowels, tone, diacritics, karan, or sara_am.
            // If any are present, delete them all and place the pali mark in their stead.
            let first_incompat = tcb.iter().find(|(_, c)| {
                matches!(
                    thai_char_class(*c),
                    ThaiCharClass::UpperVowel
                        | ThaiCharClass::LowerVowel
                        | ThaiCharClass::Tone
                        | ThaiCharClass::Diacritic
                        | ThaiCharClass::Karan
                        | ThaiCharClass::SaraAm
                )
            });
            if let Some(&(incompat_off, _)) = first_incompat {
                let last = match tcb.last() {
                    Some(v) => v,
                    None => return ThaiInputAction::Accept,
                };
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

        // ── Sara am ำ (ctUM) ──────────────────────────────────────────────────────────
        // ำ typed directly; uses its own class now.
        ThaiCharClass::SaraAm => {
            // Get the last element of the TCB (the character immediately before caret).
            let &(prev_off, prev_ch) = match tcb.last() {
                Some(v) => v,
                None => return ThaiInputAction::Accept,
            };
            let prev_cls = thai_char_class(prev_ch);

            match prev_cls {
                // After consonant alone: Accept (append).
                ThaiCharClass::Consonant => ThaiInputAction::Accept,

                // After vowel or existing sara_am: replace it (vowel slot taken by ำ).
                ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel | ThaiCharClass::SaraAm => {
                    ThaiInputAction::Replace {
                        slot_offset: prev_off,
                        slot_bytes: prev_ch.len_utf8(),
                        cons_swap: None,
                        replacement: None,
                    }
                }

                // After tone mark: look at prev-prev.
                ThaiCharClass::Tone => {
                    // Find the element before this tone in the TCB.
                    let prev_prev_cls = if tcb.len() >= 2 {
                        thai_char_class(tcb[tcb.len() - 2].1)
                    } else {
                        ThaiCharClass::Consonant // only one element means consonant
                    };
                    match prev_prev_cls {
                        // [cons, vowel, tone] + type ำ → delete vowel, append ำ after tone.
                        // Result: [cons, tone, ำ] — the canonical tone-am order.
                        // We use DeleteThenAppend because deletion (vowel) and insertion (ำ)
                        // happen at different positions.
                        ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel => {
                            let (vowel_off, vowel_ch) = tcb[tcb.len() - 2];
                            ThaiInputAction::DeleteThenAppend {
                                slot_offset: vowel_off,
                                slot_bytes: vowel_ch.len_utf8(),
                                cons_swap: None,
                                replacement: None,
                            }
                        }
                        // [cons, tone] + type ำ → Accept (append; result: cons+tone+ำ).
                        _ => ThaiInputAction::Accept,
                    }
                }

                // After diacritic, karan, or pali mark: replace it.
                ThaiCharClass::Diacritic | ThaiCharClass::Karan | ThaiCharClass::PaliMark => {
                    ThaiInputAction::Replace {
                        slot_offset: prev_off,
                        slot_bytes: prev_ch.len_utf8(),
                        cons_swap: None,
                        replacement: None,
                    }
                }

                // Anything else: accept.
                _ => ThaiInputAction::Accept,
            }
        }

        // ── Tone marks (ctMAI1234 — ่ ้ ๊ ๋ only) ───────────────────────────────────
        ThaiCharClass::Tone => {
            let &(prev_off, prev_ch) = match tcb.last() {
                Some(v) => v,
                None => return ThaiInputAction::Accept,
            };
            let prev_cls = thai_char_class(prev_ch);

            // Typing a tone while a pali mark is present replaces the pali mark.
            if prev_cls == ThaiCharClass::PaliMark {
                return ThaiInputAction::Replace {
                    slot_offset: prev_off,
                    slot_bytes: prev_ch.len_utf8(),
                    cons_swap: None,
                    replacement: None,
                };
            }

            match prev_cls {
                // After consonant or vowel: Accept (slot 2 free).
                ThaiCharClass::Consonant
                | ThaiCharClass::UpperVowel
                | ThaiCharClass::LowerVowel => ThaiInputAction::Accept,

                // After existing tone, diacritic, or karan: replace it.
                ThaiCharClass::Tone | ThaiCharClass::Diacritic | ThaiCharClass::Karan => {
                    ThaiInputAction::Replace {
                        slot_offset: prev_off,
                        slot_bytes: prev_ch.len_utf8(),
                        cons_swap: None,
                        replacement: None,
                    }
                }

                // After sara_am: depends on what came before it.
                ThaiCharClass::SaraAm => {
                    // Check if there is already a tone before the sara_am.
                    if tcb.len() >= 2 {
                        let prev_prev_cls = thai_char_class(tcb[tcb.len() - 2].1);
                        if prev_prev_cls == ThaiCharClass::Tone {
                            // [cons, tone, sara_am] + new tone → replace the old tone (2 back).
                            let (tone_off, tone_ch) = tcb[tcb.len() - 2];
                            return ThaiInputAction::Replace {
                                slot_offset: tone_off,
                                slot_bytes: tone_ch.len_utf8(),
                                cons_swap: None,
                                replacement: None,
                            };
                        }
                    }
                    // [cons, sara_am] + tone → insert before sara_am.
                    ThaiInputAction::Replace {
                        slot_offset: prev_off,
                        slot_bytes: 0,
                        cons_swap: None,
                        replacement: None,
                    }
                }

                // After pali mark: already handled above; anything else: accept.
                _ => ThaiInputAction::Accept,
            }
        }

        // ── Karan (ctKARAN — ์ only) ──────────────────────────────────────────────────
        ThaiCharClass::Karan => {
            let &(prev_off, prev_ch) = match tcb.last() {
                Some(v) => v,
                None => return ThaiInputAction::Reject,
            };
            let prev_cls = thai_char_class(prev_ch);

            match prev_cls {
                // After consonant alone: Accept.
                ThaiCharClass::Consonant => ThaiInputAction::Accept,

                // After upper/lower vowel: special case for กิ์ and กุ์.
                ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel => {
                    if prev_ch == SARA_I || prev_ch == SARA_U {
                        // กิ์ and กุ์ are valid; append.
                        ThaiInputAction::Accept
                    } else {
                        // Other vowels: karan replaces them.
                        ThaiInputAction::Replace {
                            slot_offset: prev_off,
                            slot_bytes: prev_ch.len_utf8(),
                            cons_swap: None,
                            replacement: None,
                        }
                    }
                }

                // After tone, diacritic, or another karan: look at what precedes in the TCB.
                // - If the preceding vowel is Sara_I or Sara_U, กิ์/กุ์ pattern with a slot-2
                //   mark — replace only the slot-2 mark (keep Sara_I/Sara_U).
                // - Otherwise delete both the vowel and the slot-2 mark together.
                // - If there is no vowel before (e.g. [cons, tone]), just replace the tone.
                ThaiCharClass::Tone | ThaiCharClass::Diacritic | ThaiCharClass::Karan => {
                    if tcb.len() >= 2 {
                        let (vowel_cand_off, vowel_cand_ch) = tcb[tcb.len() - 2];
                        match thai_char_class(vowel_cand_ch) {
                            ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel => {
                                if vowel_cand_ch == SARA_I || vowel_cand_ch == SARA_U {
                                    // กิ้์ pattern: keep Sara_I/Sara_U, replace slot-2 mark.
                                    ThaiInputAction::Replace {
                                        slot_offset: prev_off,
                                        slot_bytes: prev_ch.len_utf8(),
                                        cons_swap: None,
                                        replacement: None,
                                    }
                                } else {
                                    // Other vowel + slot-2 mark: delete both, insert Karan.
                                    let delete_bytes =
                                        prev_off + prev_ch.len_utf8() - vowel_cand_off;
                                    ThaiInputAction::Replace {
                                        slot_offset: vowel_cand_off,
                                        slot_bytes: delete_bytes,
                                        cons_swap: None,
                                        replacement: None,
                                    }
                                }
                            }
                            // [cons, tone/diac/karan] without a vowel: replace slot-2 mark.
                            _ => ThaiInputAction::Replace {
                                slot_offset: prev_off,
                                slot_bytes: prev_ch.len_utf8(),
                                cons_swap: None,
                                replacement: None,
                            },
                        }
                    } else {
                        ThaiInputAction::Replace {
                            slot_offset: prev_off,
                            slot_bytes: prev_ch.len_utf8(),
                            cons_swap: None,
                            replacement: None,
                        }
                    }
                }

                // After sara_am or pali mark: replace.
                ThaiCharClass::SaraAm | ThaiCharClass::PaliMark => ThaiInputAction::Replace {
                    slot_offset: prev_off,
                    slot_bytes: prev_ch.len_utf8(),
                    cons_swap: None,
                    replacement: None,
                },

                _ => ThaiInputAction::Reject,
            }
        }

        // ── Diacritic (ctTYKOU — ็ only) ─────────────────────────────────────────────
        ThaiCharClass::Diacritic => {
            let &(prev_off, prev_ch) = match tcb.last() {
                Some(v) => v,
                None => return ThaiInputAction::Accept,
            };
            let prev_cls = thai_char_class(prev_ch);

            // Typing a diac while a pali mark is present replaces the pali mark.
            if prev_cls == ThaiCharClass::PaliMark {
                return ThaiInputAction::Replace {
                    slot_offset: prev_off,
                    slot_bytes: prev_ch.len_utf8(),
                    cons_swap: None,
                    replacement: None,
                };
            }

            match prev_cls {
                // After consonant alone: Accept.
                ThaiCharClass::Consonant => ThaiInputAction::Accept,

                // After any other class: check prev-prev.
                _ => {
                    // If the TCB has prev-prev = Consonant, replace the slot-2 occupant.
                    if tcb.len() >= 2 {
                        let prev_prev_cls = thai_char_class(tcb[tcb.len() - 2].1);
                        if prev_prev_cls == ThaiCharClass::Consonant {
                            return ThaiInputAction::Replace {
                                slot_offset: prev_off,
                                slot_bytes: prev_ch.len_utf8(),
                                cons_swap: None,
                                replacement: None,
                            };
                        }
                    }
                    // [cons, vowel, tone/karan] + type ็ → Reject (structurally invalid).
                    ThaiInputAction::Reject
                }
            }
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
            // Look for an existing vowel (UpperVowel, LowerVowel, or SaraAm) in the TCB.
            if let Some(&(vowel_off, existing_vowel)) = tcb.iter().find(|(_, c)| {
                matches!(
                    thai_char_class(*c),
                    ThaiCharClass::UpperVowel | ThaiCharClass::LowerVowel | ThaiCharClass::SaraAm
                )
            }) {
                // Special case: if the vowel is SaraAm and there's a Tone before it
                // (pattern: [cons, tone, sara_am]), reject — that slot combination is locked.
                if thai_char_class(existing_vowel) == ThaiCharClass::SaraAm && tcb.len() >= 2 {
                    let prev_of_sara_am_cls = thai_char_class(tcb[tcb.len() - 2].1);
                    if prev_of_sara_am_cls == ThaiCharClass::Tone {
                        return ThaiInputAction::Reject;
                    }
                }
                ThaiInputAction::Replace {
                    slot_offset: vowel_off,
                    slot_bytes: existing_vowel.len_utf8(),
                    cons_swap: None,
                    replacement: None,
                }
            } else {
                // No existing vowel: check for a slot-2 character (tone/karan/diac).
                // - MAITAIKHU (◌็): the vowel replaces it entirely (slot_bytes = its length).
                // - Tone / Karan: insert before it, preserving the slot-2 mark.
                let caret_pos = tcb.last().map(|&(o, c)| o + c.len_utf8()).unwrap_or(0);
                if let Some(&(slot_off, slot_ch)) = tcb.iter().find(|(_, c)| {
                    matches!(
                        thai_char_class(*c),
                        ThaiCharClass::Tone | ThaiCharClass::Karan | ThaiCharClass::Diacritic
                    )
                }) {
                    let slot_bytes = if slot_ch == MAITAIKHU {
                        slot_ch.len_utf8() // replace MAITAIKHU
                    } else {
                        0 // insert before tone/karan
                    };
                    ThaiInputAction::Replace {
                        slot_offset: slot_off,
                        slot_bytes,
                        cons_swap: None,
                        replacement: None,
                    }
                } else {
                    let _ = caret_pos;
                    ThaiInputAction::Accept
                }
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
        // Sara am is its own class now (not UpperVowel)
        assert_eq!(cls('\u{0E33}'), ThaiCharClass::SaraAm); // ำ
        // Tone marks (only the 4 mai marks, NOT thanthakhat)
        assert_eq!(cls('\u{0E48}'), ThaiCharClass::Tone); // mai ek
        assert_eq!(cls('\u{0E49}'), ThaiCharClass::Tone); // mai tho
        assert_eq!(cls('\u{0E4A}'), ThaiCharClass::Tone); // mai tri
        assert_eq!(cls('\u{0E4B}'), ThaiCharClass::Tone); // mai chattawa
        // Karan (thanthakhat — separate from Tone)
        assert_eq!(cls('\u{0E4C}'), ThaiCharClass::Karan); // thanthakhat ์
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
        // Consonant + sara_am: ำ occupies the vowel slot (Form A).
        assert_eq!(tcb("กำ"), vec!['ก', '\u{0E33}']);
        // Consonant + sara_i + sara_am: two vowels — invalid TCB, returns empty.
        assert_eq!(tcb("กิำ"), Vec::<char>::new());
        // Consonant + tone + sara_am: valid 3-char TCB (canonical Thai order).
        assert_eq!(tcb("ก้ำ"), vec!['ก', '\u{0E49}', '\u{0E33}']);
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

        // Diacritic after cons+vowel+tone: rejected (prev-prev = UpperVowel).
        assert_eq!(action(full, maitaikhu), ThaiInputAction::Reject);

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

        // ── Typing diacritic after cons+vowel+tone: structurally invalid → Reject ──
        // กี้ = ก(0) + sara_ii(3) + mai_tho(6); prev-prev of TCB tail = UpperVowel → Reject.
        assert_eq!(action("กี้", maitaikhu), ThaiInputAction::Reject);

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

        // ── Vowel upgrade via Nikhahit: cons + Sara_I + Nikhahit = cons + Sara_UE ────
        let sara_ue  = '\u{0E36}'; // ึ  Sara_UE

        // กิ + ํ → replace ิ(3,3) with ึ  (Sara_I → Sara_UE; Nikhahit consumed)
        match action("กิ", nikhahit) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None,
                                       replacement: Some(r) } if r == sara_ue => {}
            other => panic!("nikhahit after sara_i must upgrade to sara_ue, got {other:?}"),
        }

        // กิ้ + ํ → replace ิ(3,3) with ึ; tone ้ at offset 6 stays
        match action("กิ้", nikhahit) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None,
                                       replacement: Some(r) } if r == sara_ue => {}
            other => panic!("nikhahit after sara_i+tone must upgrade sara_i→sara_ue, keep tone; got {other:?}"),
        }

        let _ = sara_ue;

        // ── Pali mark after other upper/lower vowels → replaces vowel (and any tone/diac) ─
        // กั = ก(0) + ั(3); nikhahit must delete ั (3 bytes) and insert nikhahit at 3
        match action("กั", nikhahit) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("nikhahit must replace maihunakat at 3, got {other:?}"),
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
        let sara_am = '\u{0E33}'; // ำ

        // ำ is now its own class (SaraAm), not UpperVowel.
        assert_eq!(cls(sara_am), ThaiCharClass::SaraAm);

        // กำ = ก(0,3) + ำ(3,3); caret at 6.
        // Typing sara_i REPLACES ำ at offset 3 (sara_am is the vowel occupant).
        match action("กำ", sara_i) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("vowel must replace sara_am at 3, got {other:?}"),
        }
        match action("กำ", sara_u) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("lower vowel must replace sara_am at 3, got {other:?}"),
        }

        // Typing sara_am directly after consonant+vowel replaces the existing vowel.
        // กิ = ก(0,3) + ิ(3,3); typing ำ replaces ิ at offset 3.
        match action("กิ", sara_am) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("sara_am must replace existing vowel at 3, got {other:?}"),
        }
        // Typing sara_am after consonant alone: Accept.
        assert_eq!(action("ก", sara_am), ThaiInputAction::Accept);

        // sara_am in TCB.
        assert_eq!(tcb("กำ"), vec!['ก', sara_am]);
        // กิำ = two vowels → not a valid TCB.
        assert_eq!(tcb("กิำ"), Vec::<char>::new());
    }

    #[test]
    fn maitaikhu_replaces_vowel() {
        let sara_i = '\u{0E34}'; // sara i  (upper vowel)
        let sara_u = '\u{0E38}'; // sara u  (lower vowel)
        let sara_am = '\u{0E33}'; // ำ
        let mai_ek = '\u{0E48}'; // mai ek  (tone)
        let maitaikhu = '\u{0E47}'; // ◌็

        // Maitaikhu after consonant only: Accept (nothing to replace).
        assert_eq!(action("ก", maitaikhu), ThaiInputAction::Accept);

        // Maitaikhu after consonant+upper vowel: replaces the vowel.
        // กิ = ก(0,3) + ิ(3,3); caret at 6.
        match action("กิ", maitaikhu) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("maitaikhu must replace sara_i at 3, got {other:?}"),
        }

        // Maitaikhu after consonant+lower vowel: replaces the vowel.
        match action("กุ", maitaikhu) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("maitaikhu must replace sara_u at 3, got {other:?}"),
        }

        // Maitaikhu after consonant+sara_am: replaces sara_am (vowel slot).
        match action("กำ", maitaikhu) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("maitaikhu must replace sara_am at 3, got {other:?}"),
        }

        // Maitaikhu after consonant+vowel+tone: rejected (prev-prev = UpperVowel).
        // กิ้ = ก(0,3) + ิ(3,3) + ้(6,3); caret at 9.
        assert_eq!(action("กิ้", maitaikhu), ThaiInputAction::Reject);

        // Maitaikhu after consonant+vowel+diac: rejected too.
        assert_eq!(action("กิ็", maitaikhu), ThaiInputAction::Reject);

        // MAIEK after Sara_I: tone appended normally (no vowel-upgrade).
        assert_eq!(action("กิ", mai_ek), ThaiInputAction::Accept);

        // ── Special: MAITAIKHU ↔ vowel mutual replacement ────────────────────────

        // Case 1: cons + MAITAIKHU, then type upper/lower vowel → vowel replaces MAITAIKHU.
        // ก็ = ก(0,3) + ็(3,3); type ิ → Replace ็ at 3 with ิ (slot_bytes=3, not 0).
        match action("ก็", sara_i) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("upper vowel must replace maitaikhu at 3, got {other:?}"),
        }
        match action("ก็", sara_u) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("lower vowel must replace maitaikhu at 3, got {other:?}"),
        }

        // Case 2: cons + upper/lower vowel, then type MAITAIKHU → MAITAIKHU replaces vowel.
        // กิ = ก(0,3) + ิ(3,3); type ็ → Replace ิ at 3. (already tested above, verify again)
        match action("กิ", maitaikhu) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("maitaikhu must replace sara_i at 3, got {other:?}"),
        }
        match action("กุ", maitaikhu) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("maitaikhu must replace sara_u at 3, got {other:?}"),
        }

        // Tone/Karan before caret: vowel still inserts BEFORE them (not replacing).
        // ก้ = ก(0,3) + ้(3,3); type ิ → pure insert before ้ (slot_bytes=0).
        match action("ก้", sara_i) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 0, cons_swap: None, replacement: None } => {}
            other => panic!("upper vowel must insert before tone (not replace), got {other:?}"),
        }

        let _ = (sara_i, sara_u, sara_am);
    }

    #[test]
    fn tone_and_sara_am_interaction() {
        let maiek = '\u{0E48}'; // ่ mai ek
        let maitho = '\u{0E49}'; // ้ mai tho
        let sara_am = '\u{0E33}'; // ำ

        // cons + sara_am + type tone: insert before sara_am (slot_bytes = 0).
        // "กำ" = ก(0,3) + ำ(3,3); caret at 6.
        match action("กำ", maiek) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 0, cons_swap: None, replacement: None } => {}
            other => panic!("expected insert before sara_am at 3, got {other:?}"),
        }

        // cons + tone + sara_am + type new tone: replace old tone (2 back).
        // "ก้ำ" = ก(0,3) + ้(3,3) + ำ(6,3); caret at 9.
        match action("ก้ำ", maitho) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("expected Replace ้ at 3, got {other:?}"),
        }

        // [cons, vowel, tone] + type ำ → DeleteThenAppend: delete vowel, append ำ after tone.
        // Result order: [cons, tone, ำ]  (not [cons, ำ, tone]).
        // กิ้ = ก(0,3) + ิ(3,3) + ้(6,3); caret at 9 → delete ิ at 3, insert ำ at 6.
        match action("กิ้", sara_am) {
            ThaiInputAction::DeleteThenAppend { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("expected DeleteThenAppend(vowel=3,3) for [cons,vowel,tone]+ำ, got {other:?}"),
        }

        // Same for a lower vowel + tone.
        // กุ้ = ก(0,3) + ุ(3,3) + ้(6,3); caret at 9.
        match action("กุ้", sara_am) {
            ThaiInputAction::DeleteThenAppend { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("expected DeleteThenAppend for [cons,lower_vowel,tone]+ำ, got {other:?}"),
        }

        // The full upgraded sequence: cons+Sara_I+MAIEK+MAIEK+MAIEK → cons+Sara_UEE+MAIEK, then +ำ.
        // กืื้ shorthand: กื = ก+Sara_UEE, กื้ = ก+Sara_UEE+้ (offset: ก=0, ื=3, ้=6).
        match action("กื้", sara_am) {
            ThaiInputAction::DeleteThenAppend { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("expected DeleteThenAppend for [cons,Sara_UEE,tone]+ำ, got {other:?}"),
        }

        let _ = sara_am;
    }

    #[test]
    fn karan_rules() {
        let karan = '\u{0E4C}'; // ์ thanthakhat
        let sara_i = '\u{0E34}'; // ิ sara i
        let sara_u = '\u{0E38}'; // ุ sara u
        let sara_ii = '\u{0E35}'; // ี sara ii

        // consonant alone: accept
        assert_eq!(action("ก", karan), ThaiInputAction::Accept);

        // กิ์ and กุ์: karan appends after SARA_I and SARA_U
        assert_eq!(action("กิ", karan), ThaiInputAction::Accept);
        assert_eq!(action("กุ", karan), ThaiInputAction::Accept);

        // other vowels: karan replaces
        // กี + type ์ → replace ี at offset 3
        match action("กี", karan) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("karan must replace sara_ii at 3, got {other:?}"),
        }

        // karan replaces tone
        // ก้ = ก(0,3) + ้(3,3); type ์ → replace ้ at 3
        match action("ก้", karan) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("karan must replace tone at 3, got {other:?}"),
        }

        // กิ้ + type ์ → replace ้ at 6, keep ิ  (Sara_I is special: กิ์ is valid)
        // กิ้ = ก(0,3) + ิ(3,3) + ้(6,3); caret at 9
        match action("กิ้", karan) {
            ThaiInputAction::Replace { slot_offset: 6, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("karan must replace tone at 6 (keep sara_i), got {other:?}"),
        }

        // กุ้ + type ์ → replace ้ at 6, keep ุ  (Sara_U is special: กุ์ is valid)
        // กุ้ = ก(0,3) + ุ(3,3) + ้(6,3)
        match action("กุ้", karan) {
            ThaiInputAction::Replace { slot_offset: 6, slot_bytes: 3, cons_swap: None, replacement: None } => {}
            other => panic!("karan must replace tone at 6 (keep sara_u), got {other:?}"),
        }

        // กั่ + type ์ → delete ั+่ (6 bytes) at offset 3, insert ์
        // MAIHUNAKAT is not sara_i/sara_u, so both vowel and tone are removed.
        // กั่ = ก(0,3) + ั(3,3) + ่(6,3)
        match action("กั่", karan) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 6, cons_swap: None, replacement: None } => {}
            other => panic!("karan must delete maihunakat+tone (6 B) at 3, got {other:?}"),
        }

        // กี้ + type ์ → delete ี+้ (6 bytes) at offset 3, insert ์
        // กี้ = ก(0,3) + ี(3,3) + ้(6,3)
        match action("กี้", karan) {
            ThaiInputAction::Replace { slot_offset: 3, slot_bytes: 6, cons_swap: None, replacement: None } => {}
            other => panic!("karan must delete sara_ii+tone (6 B) at 3, got {other:?}"),
        }

        // no TCB: reject
        assert_eq!(action("", karan), ThaiInputAction::Reject);
        assert_eq!(action("hello", karan), ThaiInputAction::Reject);

        let _ = (sara_i, sara_u, sara_ii);
    }
}

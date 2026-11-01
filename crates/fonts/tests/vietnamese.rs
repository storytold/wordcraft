//! Vietnamese diacritic shaping tests.
//!
//! Verifies OpenType shaping of Vietnamese precomposed (NFC) and decomposed (NFD) text
//! across all bundled fonts (`Source Serif 4`, `Source Sans 3`, `Inter`, `JetBrains Mono`).
//! Independent of legacy 8-bit encodings (TCVN3/VNI), which are transcoded before text reaches
//! the shaper.

use unicode_normalization::UnicodeNormalization;
use wordcraft_fonts::{FontDb, shape};

/// The 4 fonts bundled in `assets/fonts/` (OFL).
const BUNDLED_FAMILIES: &[&str] = &["Source Serif 4", "Source Sans 3", "Inter", "JetBrains Mono"];

/// Vietnamese vowels (base vowels + vowels with inherent diacritics: circumflex, breve, horn).
const VOWELS: &[&str] = &["a", "ă", "â", "e", "ê", "i", "o", "ô", "ơ", "u", "ư", "y", "A", "Ă", "Â", "E", "Ê", "I", "O", "Ô", "Ơ", "U", "Ư", "Y"];

/// 5 Vietnamese tone marks as combining characters (NFD form).
const TONES_NFD: &[&str] = &[
    "\u{0301}", // sắc (acute)
    "\u{0300}", // huyền (grave)
    "\u{0309}", // hỏi (hook above)
    "\u{0303}", // ngã (tilde)
    "\u{0323}", // nặng (dot below)
];

// ---------------------------------------------------------------------
// Test 1: Every precomposed Vietnamese char shapes to exactly one glyph
// ---------------------------------------------------------------------
#[test]
fn precomposed_latin_extended_additional_shapes_as_one_glyph() {
    let db = FontDb::global();
    for &family in BUNDLED_FAMILIES {
        let face = db.face(family, "Regular");
        // Latin Extended Additional block (U+1EA0..=U+1EF9 contains almost all Vietnamese accented letters)
        for cp in 0x1EA0u32..=0x1EF9 {
            let Some(ch) = char::from_u32(cp) else { continue };
            if !ch.is_alphabetic() {
                continue;
            }
            let text = ch.to_string();
            let glyphs = shape(&face, &text, &[], |c| c);
            assert_eq!(glyphs.len(), 1, "{family} U+{cp:04X} ('{ch}') should shape to exactly 1 glyph, got {}", glyphs.len());
            assert_ne!(glyphs[0].gid, 0, "{family} U+{cp:04X} ('{ch}') shaped to .notdef (gid 0)");
            assert!(glyphs[0].x_advance > 0, "{family} U+{cp:04X} ('{ch}') advance should be positive, got {}", glyphs[0].x_advance);
        }

        // Also verify Vietnamese letters in Latin-1 / Latin Extended-A (đ, Đ, and base accented vowels)
        for ch in "àáảãạèéẻẽẹìíỉĩịòóỏõọùúủũụỳýỷỹỵđĐ".chars() {
            let text = ch.to_string();
            let glyphs = shape(&face, &text, &[], |c| c);
            assert_eq!(glyphs.len(), 1, "{family} ('{ch}') should shape to exactly 1 glyph, got {}", glyphs.len());
            assert_ne!(glyphs[0].gid, 0, "{family} ('{ch}') shaped to .notdef (gid 0)");
        }
    }
}

// ---------------------------------------------------------------------
// Test 2: NFC and NFD produce identical glyph sequences (ccmp / normalization)
// ---------------------------------------------------------------------
#[test]
fn nfc_nfd_equivalence_across_bundled_fonts() {
    let db = FontDb::global();
    for &family in BUNDLED_FAMILIES {
        let face = db.face(family, "Regular");
        for &vowel in VOWELS {
            for &tone in TONES_NFD {
                let nfd = format!("{vowel}{tone}");
                let nfc: String = nfd.nfc().collect();
                let glyphs_nfd = shape(&face, &nfd, &[], |c| c);
                let glyphs_nfc = shape(&face, &nfc, &[], |c| c);

                let gids_nfd: Vec<u32> = glyphs_nfd.iter().map(|g| g.gid).collect();
                let gids_nfc: Vec<u32> = glyphs_nfc.iter().map(|g| g.gid).collect();

                assert_eq!(gids_nfd, gids_nfc, "{family}: NFD {nfd:?} vs NFC {nfc:?} produced different glyph IDs: {gids_nfd:?} vs {gids_nfc:?}");

                let total_adv_nfd: i32 = glyphs_nfd.iter().map(|g| g.x_advance).sum();
                let total_adv_nfc: i32 = glyphs_nfc.iter().map(|g| g.x_advance).sum();
                assert_eq!(total_adv_nfd, total_adv_nfc, "{family}: NFD {nfd:?} vs NFC {nfc:?} advance mismatch: {total_adv_nfd} vs {total_adv_nfc}");
            }
        }
    }
}

// ---------------------------------------------------------------------
// Test 3: Double diacritics without precomposed base (ccmp ligature or mkmk lifting)
// ---------------------------------------------------------------------
#[test]
fn double_diacritics_stacking_without_precomposed_base() {
    // 'c' + circumflex + acute has no precomposed glyph in Unicode.
    // OpenType handles this via either:
    // (a) `ccmp` ligature composing the two marks into a dedicated double-mark glyph, OR
    // (b) GPOS `mkmk` lifting the second mark above the first (y_offset > 0).
    let text = "c\u{0302}\u{0301}";
    let db = FontDb::global();
    for &family in BUNDLED_FAMILIES {
        let face = db.face(family, "Regular");
        let glyphs = shape(&face, text, &[], |c| c);

        // Expect: base glyph + at least one mark glyph
        assert!(glyphs.len() >= 2, "{family}: expected base + mark for {text:?}, got {}", glyphs.len());

        // Base glyph advances; combining mark(s) have zero x_advance.
        let base = &glyphs[0];
        assert!(base.x_advance > 0, "{family}: base glyph should advance");

        let marks = &glyphs[1..];
        for (i, m) in marks.iter().enumerate() {
            assert_eq!(m.x_advance, 0, "{family}: mark {i} should have 0 x_advance, got {}", m.x_advance);
            assert_ne!(m.gid, 0, "{family}: mark {i} should not be .notdef");
        }

        if marks.len() == 1 {
            // ccmp ligature: the font composed the two marks into a single combined accent glyph.
            assert_ne!(marks[0].gid, 0);
        } else {
            // Separate marks: GPOS mkmk must lift the second mark above the first (y_offset > 0).
            let lifted = marks.iter().any(|g| g.y_offset > 0);
            assert!(lifted, "{family}: separate marks must use mkmk to lift the upper mark (y_offset > 0); marks: {marks:?}");
        }
    }
}

// ---------------------------------------------------------------------
// Test 4: Vietnamese phrases shape identically in NFC and NFD
// ---------------------------------------------------------------------
#[test]
fn vietnamese_phrases_shaping_consistency() {
    let phrases =
        ["Tiếng Việt", "Cộng hòa Xã hội Chủ nghĩa Việt Nam", "Độc lập - Tự do - Hạnh phúc", "Thủ đô Hà Nội nghìn năm văn hiến", "Chữ Quốc ngữ"];

    let db = FontDb::global();
    for &family in BUNDLED_FAMILIES {
        let face = db.face(family, "Regular");
        for phrase in phrases {
            let nfc_text: String = phrase.nfc().collect();
            let nfd_text: String = phrase.nfd().collect();

            let glyphs_nfc = shape(&face, &nfc_text, &[], |c| c);
            let glyphs_nfd = shape(&face, &nfd_text, &[], |c| c);

            assert!(!glyphs_nfc.is_empty(), "{family}: empty glyphs for {phrase}");
            assert!(glyphs_nfc.iter().all(|g| g.gid != 0), "{family}: missing glyph (.notdef) in phrase {phrase:?}");

            let gids_nfc: Vec<u32> = glyphs_nfc.iter().map(|g| g.gid).collect();
            let gids_nfd: Vec<u32> = glyphs_nfd.iter().map(|g| g.gid).collect();
            assert_eq!(gids_nfc, gids_nfd, "{family}: glyph IDs diverge between NFC and NFD for {phrase:?}");

            let adv_nfc: i32 = glyphs_nfc.iter().map(|g| g.x_advance).sum();
            let adv_nfd: i32 = glyphs_nfd.iter().map(|g| g.x_advance).sum();
            assert_eq!(adv_nfc, adv_nfd, "{family}: total advance mismatch for {phrase:?}: {adv_nfc} vs {adv_nfd}");
        }
    }
}

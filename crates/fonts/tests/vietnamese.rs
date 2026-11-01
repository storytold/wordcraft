//! Vietnamese diacritic shaping tests.
//!
//! Verifies OpenType shaping of Vietnamese precomposed (NFC) and decomposed (NFD) text
//! across all bundled fonts (`Source Serif 4`, `Source Sans 3`, `Inter`, `JetBrains Mono`).
//! Independent of legacy 8-bit encodings (TCVN3/VNI), which are transcoded before text reaches
//! the shaper.

use kurbo::Shape;
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
        assert_eq!(&face.family, family, "FontDb fell back to a different face for {family}");

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

        // Also verify Vietnamese letters in Latin-1 / Latin Extended-A (đ, Đ, and base accented vowels in both cases)
        for ch in "àáảãạèéẻẽẹìíỉĩịòóỏõọùúủũụỳýỷỹỵđĐÀÁẢÃẠÈÉẺẼẸÌÍỈĨỊÒÓỎÕỌÙÚỦŨỤỲÝỶỸỴ".chars()
        {
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
        assert_eq!(&face.family, family, "FontDb fell back to a different face for {family}");

        for &vowel in VOWELS {
            for &tone in TONES_NFD {
                let nfd = format!("{vowel}{tone}");
                let nfc: String = nfd.nfc().collect();
                let glyphs_nfd = shape(&face, &nfd, &[], |c| c);
                let glyphs_nfc = shape(&face, &nfc, &[], |c| c);

                let gids_nfd: Vec<u32> = glyphs_nfd.iter().map(|g| g.gid).collect();
                let gids_nfc: Vec<u32> = glyphs_nfc.iter().map(|g| g.gid).collect();
                assert_eq!(gids_nfd, gids_nfc, "{family}: NFD {nfd:?} vs NFC {nfc:?} produced different glyph IDs: {gids_nfd:?} vs {gids_nfc:?}");

                let advs_nfd: Vec<i32> = glyphs_nfd.iter().map(|g| g.x_advance).collect();
                let advs_nfc: Vec<i32> = glyphs_nfc.iter().map(|g| g.x_advance).collect();
                assert_eq!(advs_nfd, advs_nfc, "{family}: per-glyph advance mismatch between NFD {nfd:?} and NFC {nfc:?}");

                let offsets_nfd: Vec<(i32, i32)> = glyphs_nfd.iter().map(|g| (g.x_offset, g.y_offset)).collect();
                let offsets_nfc: Vec<(i32, i32)> = glyphs_nfc.iter().map(|g| (g.x_offset, g.y_offset)).collect();
                assert_eq!(offsets_nfd, offsets_nfc, "{family}: per-glyph offset mismatch between NFD {nfd:?} and NFC {nfc:?}");
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
        assert_eq!(&face.family, family, "FontDb fell back to a different face for {family}");

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
            let outline = face.glyph_outline(marks[0].gid);
            assert!(!outline.elements().is_empty(), "{family}: ccmp ligature outline is empty");
        } else {
            // Separate marks: GPOS mkmk must lift the second mark above the first.
            let outline0 = face.glyph_outline(marks[0].gid);
            let outline1 = face.glyph_outline(marks[1].gid);
            let bbox0 = outline0.bounding_box();
            let bbox1 = outline1.bounding_box();

            let top0 = bbox0.y1 + marks[0].y_offset as f64;
            let bottom0 = bbox0.y0 + marks[0].y_offset as f64;
            let top1 = bbox1.y1 + marks[1].y_offset as f64;
            let bottom1 = bbox1.y0 + marks[1].y_offset as f64;

            assert!(top1 > top0, "{family}: upper mark top ({top1}) should be strictly above lower mark top ({top0})");
            assert!(bottom1 > bottom0, "{family}: upper mark bottom ({bottom1}) should be strictly above lower mark bottom ({bottom0})");
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
        assert_eq!(&face.family, family, "FontDb fell back to a different face for {family}");

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

            let advs_nfc: Vec<i32> = glyphs_nfc.iter().map(|g| g.x_advance).collect();
            let advs_nfd: Vec<i32> = glyphs_nfd.iter().map(|g| g.x_advance).collect();
            assert_eq!(advs_nfc, advs_nfd, "{family}: per-glyph advance mismatch between NFC and NFD for {phrase:?}");

            let offsets_nfc: Vec<(i32, i32)> = glyphs_nfc.iter().map(|g| (g.x_offset, g.y_offset)).collect();
            let offsets_nfd: Vec<(i32, i32)> = glyphs_nfd.iter().map(|g| (g.x_offset, g.y_offset)).collect();
            assert_eq!(offsets_nfc, offsets_nfd, "{family}: per-glyph offset mismatch between NFC and NFD for {phrase:?}");
        }
    }
}

// ---------------------------------------------------------------------
// Test 5: Fully decomposed triple-codepoints (ASCII + inherent mark + tone)
// ---------------------------------------------------------------------
#[test]
fn fully_decomposed_triple_codepoint_equivalence() {
    // Letters with two marks: base ASCII + inherent diacritic (circumflex, breve, horn) + tone mark.
    // In canonical Unicode order:
    // (a) For tone mark ABOVE: base ASCII + inherent mark (\u{0302}/\u{0306}/\u{031B}) + tone mark (\u{0301}/\u{0300}/\u{0309}/\u{0303}).
    // (b) For dot BELOW (\u{0323}, ccc 220): because it has a different Canonical Combining Class
    //     than above marks (ccc 230), Unicode normalization ensures dot-below can be ordered
    //     either before or after the above mark, and both compose identically to the precomposed glyph.
    let triply_decomposed = [
        ("ệ", "e\u{0302}\u{0323}", Some("e\u{0323}\u{0302}")), // 'ệ': circumflex + dot below (both CCC orders)
        ("ế", "e\u{0302}\u{0301}", None),                      // 'ế': e + circumflex + acute
        ("ề", "e\u{0302}\u{0300}", None),                      // 'ề': e + circumflex + grave
        ("ể", "e\u{0302}\u{0309}", None),                      // 'ể': e + circumflex + hook
        ("ễ", "e\u{0302}\u{0303}", None),                      // 'ễ': e + circumflex + tilde
        ("ắ", "a\u{0306}\u{0301}", None),                      // 'ắ': a + breve + acute
        ("ằ", "a\u{0306}\u{0300}", None),                      // 'ằ': a + breve + grave
        ("ặ", "a\u{0306}\u{0323}", Some("a\u{0323}\u{0306}")), // 'ặ': a + breve + dot below (both CCC orders)
        ("ở", "o\u{031b}\u{0309}", None),                      // 'ở': o + horn + hook above
        ("ợ", "o\u{031b}\u{0323}", Some("o\u{0323}\u{031b}")), // 'ợ': o + horn + dot below (both CCC orders)
        ("ử", "u\u{031b}\u{0309}", None),                      // 'ử': u + horn + hook above
        ("ự", "u\u{031b}\u{0323}", Some("u\u{0323}\u{031b}")), // 'ự': u + horn + dot below (both CCC orders)
        ("Ệ", "E\u{0302}\u{0323}", Some("E\u{0323}\u{0302}")), // uppercase 'Ệ' (both CCC orders)
        ("Ế", "E\u{0302}\u{0301}", None),                      // uppercase 'Ế'
        ("Ắ", "A\u{0306}\u{0301}", None),                      // uppercase 'Ắ'
        ("Ở", "O\u{031b}\u{0309}", None),                      // uppercase 'Ở'
        ("Ự", "U\u{031b}\u{0323}", Some("U\u{0323}\u{031b}")), // uppercase 'Ự' (both CCC orders)
    ];

    let db = FontDb::global();
    for &family in BUNDLED_FAMILIES {
        let face = db.face(family, "Regular");
        assert_eq!(&face.family, family, "FontDb fell back to a different face for {family}");

        for (precomposed, nfd_canonical, nfd_alt) in triply_decomposed {
            let g_pre = shape(&face, precomposed, &[], |c| c);
            let g_can = shape(&face, nfd_canonical, &[], |c| c);

            assert_eq!(g_pre.len(), 1, "{family}: {precomposed} should shape to 1 glyph");
            let target_gid = g_pre[0].gid;
            let target_adv = g_pre[0].x_advance;
            let target_offset = (g_pre[0].x_offset, g_pre[0].y_offset);
            assert_ne!(target_gid, 0, "{family}: {precomposed} has .notdef gid 0");

            // Canonical NFD must compose to the exact precomposed glyph
            assert_eq!(
                g_can.iter().map(|g| g.gid).collect::<Vec<_>>(),
                vec![target_gid],
                "{family}: full NFD {nfd_canonical:?} did not compose to precomposed glyph {target_gid}"
            );
            assert_eq!(g_can[0].x_advance, target_adv, "{family}: advance mismatch for {nfd_canonical:?} vs {precomposed}");
            assert_eq!((g_can[0].x_offset, g_can[0].y_offset), target_offset, "{family}: offset mismatch for {nfd_canonical:?} vs {precomposed}");

            // If an alternative valid CCC order exists (e.g. dot below + circumflex), verify it too
            if let Some(alt) = nfd_alt {
                let g_alt = shape(&face, alt, &[], |c| c);
                assert_eq!(
                    g_alt.iter().map(|g| g.gid).collect::<Vec<_>>(),
                    vec![target_gid],
                    "{family}: alternative CCC order {alt:?} did not compose to precomposed glyph {target_gid}"
                );
                assert_eq!(g_alt[0].x_advance, target_adv, "{family}: advance mismatch for {alt:?} vs {precomposed}");
                assert_eq!((g_alt[0].x_offset, g_alt[0].y_offset), target_offset, "{family}: offset mismatch for {alt:?} vs {precomposed}");
            }
        }
    }
}

// ---------------------------------------------------------------------
// Test 6: Orthography styles (new-style vs old-style tone placement)
// ---------------------------------------------------------------------
#[test]
fn vietnamese_orthography_new_and_old_styles() {
    // Both new-style (hòa, thủy) and old-style (hoà, thuỷ) orthography are widely used.
    // Both must shape without .notdef or font anomalies, with positive advances.
    let pairs = [("hòa", "hoà"), ("thủy", "thuỷ"), ("họa", "hoạ"), ("khỏe", "khoẻ"), ("tòa", "toà"), ("quế", "quế")];

    let db = FontDb::global();
    for &family in BUNDLED_FAMILIES {
        let face = db.face(family, "Regular");
        assert_eq!(&face.family, family, "FontDb fell back to a different face for {family}");

        for (new_style, old_style) in pairs {
            let g_new = shape(&face, new_style, &[], |c| c);
            let g_old = shape(&face, old_style, &[], |c| c);

            assert!(!g_new.is_empty() && g_new.iter().all(|g| g.gid != 0), "{family}: .notdef in {new_style}");
            assert!(!g_old.is_empty() && g_old.iter().all(|g| g.gid != 0), "{family}: .notdef in {old_style}");

            let adv_new: i32 = g_new.iter().map(|g| g.x_advance).sum();
            let adv_old: i32 = g_old.iter().map(|g| g.x_advance).sum();
            assert!(adv_new > 0 && adv_old > 0, "{family}: non-positive advance in {new_style}/{old_style}");
        }
    }
}

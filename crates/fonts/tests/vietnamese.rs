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

/// 1. Precomposed Vietnamese glyph shaping & Cluster integrity.
///
/// # WHY:
/// Verifies that every precomposed Vietnamese character in the Latin Extended Additional
/// block (U+1EA0..=U+1EF9) and Latin-1/Latin Extended-A shapes to exactly one glyph
/// with a positive advance width, non-zero glyph ID, and valid cluster index 0.
/// Cluster index 0 is vital for caret placement and hit-testing in the text editor.
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
            assert_eq!(glyphs[0].cluster, 0, "{family} U+{cp:04X} ('{ch}') cluster index must be 0, got {}", glyphs[0].cluster);
        }

        // Also verify Vietnamese letters in Latin-1 / Latin Extended-A (đ, Đ, and base accented vowels in both cases)
        for ch in "àáảãạèéẻẽẹìíỉĩịòóỏõọùúủũụỳýỷỹỵđĐÀÁẢÃẠÈÉẺẼẸÌÍỈĨỊÒÓỎÕỌÙÚỦŨỤỲÝỶỸỴ".chars()
        {
            let text = ch.to_string();
            let glyphs = shape(&face, &text, &[], |c| c);
            assert_eq!(glyphs.len(), 1, "{family} ('{ch}') should shape to exactly 1 glyph, got {}", glyphs.len());
            assert_ne!(glyphs[0].gid, 0, "{family} ('{ch}') shaped to .notdef (gid 0)");
            assert!(glyphs[0].x_advance > 0, "{family} ('{ch}') advance should be positive, got {}", glyphs[0].x_advance);
            assert_eq!(glyphs[0].cluster, 0, "{family} ('{ch}') cluster index must be 0, got {}", glyphs[0].cluster);
        }
    }
}

/// 2. Canonical equivalence & Composition Guard.
///
/// # WHY:
/// - Verifies that decomposed (NFD) text normalizes to standard precomposed (NFC) forms
///   across all 24 vowels × 5 tones (120 combinations).
/// - We explicitly assert `glyphs_nfc.len() == 1` and `gid != 0`. Without this guard, if
///   the OpenType shaper or font `ccmp` table fails to compose diacritics, both NFC and NFD
///   could decompose into identical multi-glyph sequences, causing a dangerous VACUOUS PASS!
/// - We assert ordered per-glyph advance vectors (`Vec<i32>`) and offsets (`Vec<(i32, i32)>`)
///   rather than scalar `.sum()`, preventing positioning errors from masking each other.
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

                // Anti-vacuous-pass guard: precomposed NFC vowel+tone MUST shape to exactly 1 glyph
                assert_eq!(glyphs_nfc.len(), 1, "{family}: NFC {nfc:?} must shape to exactly 1 glyph, got {}", glyphs_nfc.len());
                assert_ne!(glyphs_nfc[0].gid, 0, "{family}: NFC {nfc:?} shaped to .notdef (gid 0)");
                assert!(glyphs_nfc[0].x_advance > 0, "{family}: NFC {nfc:?} advance must be positive, got {}", glyphs_nfc[0].x_advance);
                assert_eq!(glyphs_nfc[0].cluster, 0, "{family}: NFC {nfc:?} cluster index must be 0, got {}", glyphs_nfc[0].cluster);

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

/// 3. Stacked double diacritics without a precomposed base.
///
/// # WHY:
/// Tests mark positioning when 'c' is followed by circumflex (U+0302) and acute (U+0301).
/// There is no precomposed 'c' with circumflex and acute in Unicode.
/// Fonts handle this via two legitimate OpenType strategies:
/// (a) Composition with base + mark: The font composes 'c' + circumflex into 'ccircumflex' (ĉ),
///     leaving the acute mark attached on top. We verify that the base is 'ccircumflex'
///     (differs from plain 'c') and the remaining mark is acute with non-empty outline.
///     If the base remained plain 'c', the single mark must be a dedicated combined accent
///     ligature representing both marks (neither mark was lost).
/// (b) GPOS `mkmk` separate marks: When separate marks are emitted, we convert shaping
///     `y_offset` into y-down coordinates and verify that the upper mark's bottom edge
///     is positioned at or above the lower mark's top edge without vertical collision.
#[test]
fn double_diacritics_stacking_without_precomposed_base() {
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
            // Either the base composed with the first mark (leaving the second mark attached),
            // or the two marks were composed via ccmp into a combined accent glyph.
            // Verify that the mark glyph is non-empty and not .notdef, and that the base
            // character absorbed the circumflex diacritic if only 1 mark remains.
            let single_mark = &marks[0];
            assert_ne!(single_mark.gid, 0, "{family}: mark glyph is .notdef");
            let outline = face.glyph_outline(single_mark.gid);
            assert!(!outline.elements().is_empty(), "{family}: mark glyph outline is empty");

            let gid_plain_c = face.glyph_for('c');
            if glyphs[0].gid != gid_plain_c {
                // Base composed with circumflex; remaining mark is the acute accent.
                let gid_acute = face.glyph_for('\u{0301}');
                assert_eq!(single_mark.gid, gid_acute, "{family}: expected acute mark above composed base");
            } else {
                // If base is plain 'c', the single mark must be a combined ligature of both marks.
                let gid_circ = face.glyph_for('\u{0302}');
                let gid_acute = face.glyph_for('\u{0301}');
                assert_ne!(single_mark.gid, gid_circ, "{family}: second mark (acute) was dropped");
                assert_ne!(single_mark.gid, gid_acute, "{family}: first mark (circumflex) was dropped");
            }
        } else {
            // Separate marks: convert shaping y_offset to y-down coordinates.
            // In y-down coordinates, visually higher ink corresponds to smaller y values.
            let ink_spans: Vec<(f64, f64)> = marks
                .iter()
                .map(|m| {
                    let bbox = face.glyph_outline(m.gid).bounding_box();
                    // In y-down coordinates, positive shaping y_offset lifts glyph up (subtract y_offset)
                    let y_top = bbox.y0 - m.y_offset as f64;
                    let y_bottom = bbox.y1 - m.y_offset as f64;
                    (y_top, y_bottom)
                })
                .collect();

            // Identify which mark is upper (smaller y_top) and which is lower (larger y_top)
            let (upper, lower) = if ink_spans[0].0 < ink_spans[1].0 { (ink_spans[0], ink_spans[1]) } else { (ink_spans[1], ink_spans[0]) };

            // Assert that the upper mark has smaller bounds and does not vertically collide with lower mark
            assert!(
                upper.1 <= lower.0,
                "{family}: upper mark bottom ({}) must be at or above lower mark top ({}); ink: {ink_spans:?}",
                upper.1,
                lower.0
            );
        }
    }
}

/// 4. Sentence-level shaping consistency across canonical forms.
///
/// # WHY:
/// Verifies natural Vietnamese phrase runs across both NFC and NFD forms, ensuring that
/// common national mottos and phrases produce identical glyph ID sequences, per-glyph
/// advances, and offsets without `.notdef` (missing glyphs).
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

/// 5. Fully decomposed triple-codepoints (ASCII base + inherent diacritic + tone mark).
///
/// # WHY:
/// Verifies deep NFD decomposition where letters with two marks (e.g. 'ệ', 'ế', 'ở', 'ự')
/// are broken into 3 codepoints: base ASCII + inherent mark + tone mark.
/// Tests both:
/// (a) Canonical NFD order (above marks share CCC 230; tone follows inherent mark).
/// (b) Alternative Canonical Combining Class (CCC) orderings for dot-below (CCC 220) vs
///     above marks (CCC 230), proving that Unicode normalization canonically composes
///     both permutations to the identical precomposed glyph.
#[test]
fn fully_decomposed_triple_codepoint_equivalence() {
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

/// 6. Orthography styles: Modern (new-style) vs Traditional (old-style) tone placement.
///
/// # WHY:
/// In Vietnamese orthography, vowels in diphthongs have two widely accepted tone placement rules:
/// - Modern style: tone mark placed on the nuclear vowel (e.g. "hòa", "thủy", "tòa", "hóa", "khỏe").
/// - Traditional style: tone mark placed on the first vowel (e.g. "hoà", "thuỷ", "toà", "hoá", "khoẻ").
///
/// Note: Words with initial digraph "qu" (e.g. "quế") are excluded because "qu" functions as an
/// initial consonant cluster /kʷ/, so the syllable nucleus is invariant ("ê") in both styles.
///
/// Both styles must shape cleanly with valid non-zero glyphs and positive advances.
#[test]
fn vietnamese_orthography_new_and_old_styles() {
    let pairs = [("hòa", "hoà"), ("thủy", "thuỷ"), ("họa", "hoạ"), ("khỏe", "khoẻ"), ("tòa", "toà"), ("hóa", "hoá")];

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

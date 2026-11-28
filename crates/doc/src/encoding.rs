//! Legacy character decoding delegates to craft-codepage; document fallback policy stays here.
pub use craft_codepage::{
    MAC_ROMAN_CODE_PAGE, MULTIBYTE_CODE_PAGES, OEM_CODE_PAGES, SINGLE_BYTE_CODE_PAGES, cp1252, is_multibyte_code_page, multibyte_encoding_name,
    oem_byte, windows_byte,
};

/// Word's supported single-byte document code pages (symbol-font policy is separate).
pub fn is_byte_code_page(cp: u16) -> bool {
    SINGLE_BYTE_CODE_PAGES.contains(&cp) || OEM_CODE_PAGES.contains(&cp) || cp == MAC_ROMAN_CODE_PAGE
}
/// Decode document text without applying OEM control glyphs or symbol-font remapping.
pub fn code_page_byte(cp: u16, b: u8) -> char {
    if b >= 0x80 && OEM_CODE_PAGES.contains(&cp) { oem_byte(cp, b) } else { windows_byte(cp, b) }
}
/// Whether Word's document import accepts this encoding.
pub fn is_text_code_page(cp: u16) -> bool {
    is_byte_code_page(cp) || is_multibyte_code_page(cp)
}
/// Preserve Word's existing symbol-font selection policy, not the shared symbol extension.
pub fn charset_code_page(charset: i32) -> Option<u16> {
    if charset == 2 { None } else { craft_codepage::charset_code_page(charset) }
}
#[cfg(test)]
#[allow(dead_code)]
#[path = "encoding_legacy_tests.rs"]
mod legacy;
#[cfg(test)]
mod parity {
    use super::*;
    #[test]
    fn every_supported_byte_and_fallback_matches_original() {
        let pages = [0, 42, 437, 737, 850, 852, 857, 866, 874, 1250, 1251, 1252, 1253, 1254, 1255, 1256, 1257, 1258, 10000, 9999, u16::MAX];
        for cp in pages {
            for b in 0..=u8::MAX {
                assert_eq!(windows_byte(cp, b), legacy::windows_byte(cp, b), "windows {cp}/{b}");
                assert_eq!(oem_byte(cp, b), legacy::oem_byte(cp, b), "oem {cp}/{b}");
                assert_eq!(code_page_byte(cp, b), legacy::code_page_byte(cp, b), "text {cp}/{b}");
                assert_eq!(cp1252(b), legacy::cp1252(b));
            }
        }
        for cp in 0..=u16::MAX {
            assert_eq!(is_byte_code_page(cp), legacy::is_byte_code_page(cp));
            assert_eq!(is_text_code_page(cp), legacy::is_text_code_page(cp));
            assert_eq!(multibyte_encoding_name(cp), legacy::multibyte_encoding_name(cp));
        }
        for charset in -1..=256 {
            assert_eq!(charset_code_page(charset), legacy::charset_code_page(charset));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_byte_code_pages() {
        let decode = |cp: u16, bytes: &[u8]| bytes.iter().map(|&b| windows_byte(cp, b)).collect::<String>();
        assert_eq!(decode(1251, &[0xD1, 0xF7, 0xE5, 0xF2, 0x20, 0xB9]), "Счет №");
        assert_eq!(decode(1251, &[0xA8, 0xB8]), "Ёё");
        assert_eq!(decode(1250, &[0xA3, 0xF3, 0x9F]), "Łóź");
        assert_eq!(decode(1253, &[0xC1, 0xE2]), "Αβ");
        assert_eq!(decode(1252, &[0x80, 0xE9]), "€é");
        assert_eq!(decode(9999, &[0xE9]), "é");
        // Mac OS Roman (#1653): accents, the euro sign at 0xDB, typographic quotes.
        assert_eq!(decode(10000, &[0x80, 0x8E, 0xDB, 0xF1, 0xF2, 0xF3, 0xD2, 0xD3, 0x41]), "Äé€ÒÚÛ“”A");
        assert_eq!(charset_code_page(77), Some(10000));
        assert_eq!(charset_code_page(254), Some(437));
        assert_eq!(charset_code_page(255), Some(437));
        assert_eq!(charset_code_page(204), Some(1251));
        assert_eq!(charset_code_page(0), None);
        for cp in SINGLE_BYTE_CODE_PAGES {
            for b in 0..=255u8 {
                let _ = windows_byte(cp, b);
            }
        }
    }

    #[test]
    fn windows_1255_holam_haser_for_vav() {
        assert_eq!(windows_byte(1255, 0xCA), '\u{05BA}');
    }

    /// #1653: one decoder for document text in any code page RTF names: Windows, DOS or Mac.
    #[test]
    fn document_text_in_windows_dos_and_mac_code_pages() {
        let decode = |cp: u16, bytes: &[u8]| bytes.iter().map(|&b| code_page_byte(cp, b)).collect::<String>();
        assert_eq!(decode(437, &[0x82, 0xA0, 0x41, 0x01]), "éáA\u{1}", "DOS text; ASCII stays ASCII");
        assert_eq!(decode(850, &[0x9D, 0xE9]), "ØÚ");
        assert_eq!(decode(10000, &[0x8E]), "é");
        assert_eq!(decode(1251, &[0xC4]), "Д");
        for cp in [437, 850, 866, 1250, 1252, 10000] {
            assert!(is_byte_code_page(cp), "{cp}");
        }
        assert!(!is_byte_code_page(932) && !is_byte_code_page(0) && !SINGLE_BYTE_CODE_PAGES.contains(&10000));
    }

    /// #1653: the East Asian charsets and code pages RTF names are multibyte: their bytes decode
    /// as runs (a lead byte and its trail byte together), not one at a time.
    #[test]
    fn east_asian_code_pages_are_multibyte() {
        assert_eq!(charset_code_page(128), Some(932), "Japanese");
        assert_eq!(charset_code_page(129), Some(949), "Korean");
        assert_eq!(charset_code_page(134), Some(936), "Simplified Chinese");
        assert_eq!(charset_code_page(136), Some(950), "Traditional Chinese");
        assert_eq!(charset_code_page(255), Some(437), "OEM is single-byte");
        assert_eq!(MULTIBYTE_CODE_PAGES, [932, 936, 949, 950]);
        for cp in MULTIBYTE_CODE_PAGES {
            assert!(is_multibyte_code_page(cp) && is_text_code_page(cp), "{cp}");
            assert!(!is_byte_code_page(cp) && !SINGLE_BYTE_CODE_PAGES.contains(&cp), "{cp}");
            assert!(multibyte_encoding_name(cp).is_some(), "{cp}");
        }
        assert!(is_text_code_page(1251) && is_text_code_page(437) && is_text_code_page(10000));
        assert!(!is_text_code_page(0) && !is_text_code_page(9999));
    }

    #[test]
    fn oem_code_pages() {
        let decode = |cp: u16, bytes: &[u8]| bytes.iter().map(|&b| oem_byte(cp, b)).collect::<String>();
        assert_eq!(decode(437, &[0x82, 0xE9, 0x01, 0x7F]), "éΘ☺⌂");
        assert_eq!(decode(850, &[0x9B, 0xE9]), "øÚ");
        assert_eq!(decode(852, &[0xA5, 0x88, 0xBE, 0xA9]), "ąłżę");
        assert_eq!(decode(866, &[0x8F, 0xE0, 0xA8, 0xA2, 0xA5, 0xE2, 0xF0, 0xFC]), "ПриветЁ№");
        assert_eq!(decode(737, &[0x80, 0x99, 0xE0]), "Αβω");
        assert_eq!(decode(857, &[0x8D, 0x98, 0xA6, 0x9F]), "ıİĞş");
        assert_eq!(oem_byte(857, 0xD5), '\u{FFFD}', "unassigned in 857");
        assert_eq!(decode(9999, &[0xE9]), "Θ", "unknown pages read as 437");
        for cp in OEM_CODE_PAGES {
            for b in 0..=255u8 {
                let _ = oem_byte(cp, b);
            }
        }
    }
}

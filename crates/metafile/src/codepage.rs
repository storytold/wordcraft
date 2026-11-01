//! WMF text code pages: the bytes of a Windows metafile text record are in the code page of the
//! font selected when it was drawn (the LOGFONT's `lfCharSet`), and a symbol font's bytes are the
//! font's own character codes (the Adobe Symbol encoding). Decoded here so the rest of the
//! metafile model — and both renderers — only ever see Unicode.

use encoding_rs::{
    BIG5, EUC_KR, GBK, IBM866, SHIFT_JIS, WINDOWS_874, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252, WINDOWS_1253, WINDOWS_1254, WINDOWS_1255,
    WINDOWS_1256, WINDOWS_1257,
};

/// Decoded text and the number of source bytes each character came from. A DBCS character takes
/// two bytes but one `dx` entry in some writers (MathType writes one per byte, with a zero
/// second), so the caller adds the advances per character with these lengths.
pub(crate) fn decode(bytes: &[u8], charset: u8) -> (String, Vec<usize>) {
    if is_dbcs(charset) {
        decode_dbcs(bytes, encoding(charset))
    } else {
        let (text, _, _) = encoding(charset).decode(bytes);
        (text.into_owned(), vec![1; bytes.len()])
    }
}

/// A symbol font's character code as the Unicode character that means the same thing. `None` for
/// codes with no Unicode equivalent.
pub(crate) fn symbol_char(code: u8) -> Option<char> {
    let i = usize::from(code.checked_sub(0x20)?);
    SYMBOL.get(i).copied().filter(|v| *v != 0).and_then(char::from_u32)
}

/// Whether a font with this `lfCharSet` is one of the East Asian double-byte code pages.
fn is_dbcs(charset: u8) -> bool {
    matches!(charset, 128 | 129 | 134 | 136)
}

fn encoding(charset: u8) -> &'static encoding_rs::Encoding {
    match charset {
        // SHIFTJIS_CHARSET, HANGEUL_CHARSET, GB2312_CHARSET, CHINESEBIG5_CHARSET.
        128 => SHIFT_JIS,
        129 => EUC_KR,
        134 => GBK,
        136 => BIG5,
        161 => WINDOWS_1253,
        162 => WINDOWS_1254,
        177 => WINDOWS_1255,
        178 => WINDOWS_1256,
        186 => WINDOWS_1257,
        204 => WINDOWS_1251,
        222 => WINDOWS_874,
        238 => WINDOWS_1250,
        255 => IBM866,
        // ANSI_CHARSET, DEFAULT_CHARSET and anything unknown.
        _ => WINDOWS_1252,
    }
}

fn decode_dbcs(bytes: &[u8], enc: &'static encoding_rs::Encoding) -> (String, Vec<usize>) {
    let mut text = String::with_capacity(bytes.len());
    let mut lengths = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let n = if is_lead(bytes[i]) && bytes.get(i + 1).is_some_and(|b| is_trail(*b)) { 2 } else { 1 };
        let (decoded, _, _) = enc.decode(&bytes[i..i + n]);
        let chars: Vec<char> = decoded.chars().collect();
        if chars.is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
            lengths.push(n);
        } else {
            // One Unicode character normally; a malformed pair can decode to more.
            for c in chars {
                text.push(c);
                lengths.push(n);
            }
        }
        i += n;
    }
    (text, lengths)
}

/// The lead and trail byte ranges of Big5, GBK, Shift_JIS and UHC. A pair outside them is read as
/// single bytes, and the decoder turns an invalid one into U+FFFD.
fn is_lead(b: u8) -> bool {
    (0x81..=0xFE).contains(&b)
}

fn is_trail(b: u8) -> bool {
    (0x40..=0xFE).contains(&b) && b != 0x7F
}

/// Adobe Symbol encoding (Unicode's published `MAPPINGS/VENDORS/ADOBE/symbol.txt`), indexed by
/// code − 0x20. Codes the published table leaves in the private use area are mapped to the
/// standard characters with the same meaning, so they draw in fallback fonts: the big
/// parenthesis, bracket and brace pieces (U+239B–U+23AD), the integral pieces, the radical
/// extension and the arrow pieces. 0 = no Unicode equivalent.
const SYMBOL: [u32; 224] = [
    0x0020, 0x0021, 0x2200, 0x0023, 0x2203, 0x0025, 0x0026, 0x220B, // 0x20
    0x0028, 0x0029, 0x2217, 0x002B, 0x002C, 0x2212, 0x002E, 0x002F, // 0x28
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037, // 0x30
    0x0038, 0x0039, 0x003A, 0x003B, 0x003C, 0x003D, 0x003E, 0x003F, // 0x38
    0x2245, 0x0391, 0x0392, 0x03A7, 0x0394, 0x0395, 0x03A6, 0x0393, // 0x40
    0x0397, 0x0399, 0x03D1, 0x039A, 0x039B, 0x039C, 0x039D, 0x039F, // 0x48
    0x03A0, 0x0398, 0x03A1, 0x03A3, 0x03A4, 0x03A5, 0x03C2, 0x03A9, // 0x50
    0x039E, 0x03A8, 0x0396, 0x005B, 0x2234, 0x005D, 0x22A5, 0x005F, // 0x58
    0x203E, 0x03B1, 0x03B2, 0x03C7, 0x03B4, 0x03B5, 0x03C6, 0x03B3, // 0x60
    0x03B7, 0x03B9, 0x03D5, 0x03BA, 0x03BB, 0x03BC, 0x03BD, 0x03BF, // 0x68
    0x03C0, 0x03B8, 0x03C1, 0x03C3, 0x03C4, 0x03C5, 0x03D6, 0x03C9, // 0x70
    0x03BE, 0x03C8, 0x03B6, 0x007B, 0x007C, 0x007D, 0x223C, 0x0000, // 0x78
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, // 0x80
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, // 0x88
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, // 0x90
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, // 0x98
    0x20AC, 0x03D2, 0x2032, 0x2264, 0x2044, 0x221E, 0x0192, 0x2663, // 0xA0
    0x2666, 0x2665, 0x2660, 0x2194, 0x2190, 0x2191, 0x2192, 0x2193, // 0xA8
    0x00B0, 0x00B1, 0x2033, 0x2265, 0x00D7, 0x221D, 0x2202, 0x2022, // 0xB0
    0x00F7, 0x2260, 0x2261, 0x2248, 0x2026, 0x23D0, 0x23AF, 0x21B5, // 0xB8
    0x2135, 0x2111, 0x211C, 0x2118, 0x2297, 0x2295, 0x2205, 0x2229, // 0xC0
    0x222A, 0x2283, 0x2287, 0x2284, 0x2282, 0x2286, 0x2208, 0x2209, // 0xC8
    0x2220, 0x2207, 0x00AE, 0x00A9, 0x2122, 0x220F, 0x221A, 0x22C5, // 0xD0
    0x00AC, 0x2227, 0x2228, 0x21D4, 0x21D0, 0x21D1, 0x21D2, 0x21D3, // 0xD8
    0x25CA, 0x2329, 0x00AE, 0x00A9, 0x2122, 0x2211, 0x239B, 0x239C, // 0xE0
    0x239D, 0x23A1, 0x23A2, 0x23A3, 0x23A7, 0x23A8, 0x23A9, 0x23AA, // 0xE8
    0x0000, 0x232A, 0x222B, 0x2320, 0x23AE, 0x2321, 0x239E, 0x239F, // 0xF0
    0x23A0, 0x23A4, 0x23A5, 0x23A6, 0x23AB, 0x23AC, 0x23AD, 0x0000, // 0xF8
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_and_big5_decode() {
        assert_eq!(decode("Hi".as_bytes(), 0).0, "Hi");
        // Big5 A1D0 is the fullwidth hyphen-minus MathType uses for a minus sign.
        let (text, lens) = decode(&[0xA1, 0xD0], 136);
        assert_eq!(text, "－");
        assert_eq!(lens, vec![2]);
    }

    #[test]
    fn symbol_codes_map_to_standard_unicode() {
        assert_eq!(symbol_char(0x61), Some('α'));
        assert_eq!(symbol_char(0xB4), Some('×'));
        assert_eq!(symbol_char(0xE6), Some('\u{239B}'));
        assert_eq!(symbol_char(0xF8), Some('\u{23A0}'));
    }
}

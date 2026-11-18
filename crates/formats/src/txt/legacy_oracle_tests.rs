//! The pre-cutover local implementations, kept as an oracle: the shared `craft_mime::detect`
//! functions plus the cp1252 wrapper must agree with them byte for byte.

use super::cp1252;

fn utf16(b: &[u8], le: bool) -> String {
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
    String::from_utf16_lossy(&units)
}

/// UTF-16 without a BOM, told by its zero bytes: `Some(true)` little-endian, `Some(false)`
/// big-endian.
fn unmarked_utf16(b: &[u8]) -> Option<bool> {
    if b.len() < 4 || !b.len().is_multiple_of(2) {
        return None;
    }
    let sample = b.get(..b.len().min(512))?;
    let even_zero = sample.iter().step_by(2).filter(|x| **x == 0).count();
    let odd_zero = sample.iter().skip(1).step_by(2).filter(|x| **x == 0).count();
    let half = sample.len() / 2;
    if odd_zero * 10 >= half * 4 && even_zero * 10 < half {
        return Some(true);
    }
    if even_zero * 10 >= half * 4 && odd_zero * 10 < half {
        return Some(false);
    }
    None
}

/// Does `b` look like plain text, so a file with an extension WordCraft doesn't know (a
/// playlist, a log, a config file…) can open as text the way Word's text import opens it?
///
/// Text has a BOM, or reads as UTF-16 by its zero bytes, or has no NUL byte and next to no
/// other control characters in its first 64 KB. PDFs are refused by their signature (they
/// start as text). An empty file is text.
fn is_text(b: &[u8]) -> bool {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) || b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    if b.starts_with(b"%PDF-") {
        return false;
    }
    let sample = b.get(..b.len().min(64 << 10)).unwrap_or(b);
    // Tab, line feed, form feed, carriage return, DOS end-of-file (^Z) and escape appear in text;
    // other controls in more than 1 of 100 characters mean binary.
    let odd = |c: u32| c == 0 || (c < 0x20 && !matches!(c, 0x09 | 0x0A | 0x0C | 0x0D | 0x1A | 0x1B)) || c == 0x7F;
    if let Some(le) = unmarked_utf16(sample) {
        let units = sample.as_chunks::<2>().0;
        let n = units.iter().filter(|c| odd(u32::from(if le { u16::from_le_bytes(**c) } else { u16::from_be_bytes(**c) }))).count();
        return n.saturating_mul(100) <= units.len();
    }
    if sample.contains(&0) {
        return false;
    }
    sample.iter().filter(|c| odd(u32::from(**c))).count().saturating_mul(100) <= sample.len()
}

/// Decode text bytes: BOMs first, then UTF-16 without BOM (by its zero bytes), UTF-8, and the
/// legacy encoding the bytes look like ([`legacy`]) as the last resort.
fn decode(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    if let Some(le) = unmarked_utf16(b) {
        return utf16(b, le);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => legacy(b),
    }
}

/// Most bytes looked at to guess a legacy encoding.
const SNIFF: usize = 1 << 20;

/// Bytes that aren't UTF-8 in the legacy encoding they look like (chardetng, the detector
/// Firefox uses for unlabelled pages): GBK/GB18030 for Simplified Chinese, Big5, Shift_JIS or
/// EUC-JP, EUC-KR, or a single-byte Windows / ISO code page; Windows-1252 as before when the
/// guess is a single-byte Western one.
fn legacy(b: &[u8]) -> String {
    let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    det.feed(b.get(..SNIFF).unwrap_or(b), b.len() <= SNIFF);
    let enc = det.guess(None, chardetng::Utf8Detection::Deny);
    if enc == encoding_rs::WINDOWS_1252 {
        return b.iter().map(|c| cp1252(*c)).collect();
    }
    enc.decode_without_bom_handling(b).0.into_owned()
}

fn samples() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = vec![vec![]];
    for b in 0..=255u8 {
        v.push(vec![b]);
    }
    for b in 0x80..=0x9Fu8 {
        v.push([b"caf".as_slice(), &[b], b" ok text here"].concat());
        v.push(vec![b, b'a', b'b']);
    }
    // Western text with each byte Windows-1252 leaves undefined: the case the cp1252 wrapper is for.
    for b in [0x81u8, 0x8D, 0x8F, 0x90, 0x9D] {
        v.push([b"caf\xE9 au lait ".as_slice(), &[b], b" ok"].concat());
        v.push([b"Le caf\xE9 de la gare ".as_slice(), &[b]].concat());
        v.push([b"r\xE9sum\xE9 ".as_slice(), &[b]].concat());
    }
    v.push("héllo wörld € naïve".as_bytes().to_vec());
    v.push([&[0xEF, 0xBB, 0xBF][..], "héllo".as_bytes()].concat());
    v.push([&[0xEF, 0xBB, 0xBF][..], &[0xE9, 0x80]].concat());
    for (bom, le) in [(&[0xFF, 0xFE][..], true), (&[0xFE, 0xFF][..], false)] {
        let t = "héllo wörld 日本語 😀";
        v.push([bom, &t.encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }).collect::<Vec<u8>>()[..]].concat());
        v.push([bom, b"h\0i\0!"].concat());
        let raw: Vec<u8> = "hello world, plain ascii".encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }).collect();
        v.push(raw);
        let raw: Vec<u8> =
            "héllo wörld, mostly ascii \u{20AC}".encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }).collect();
        v.push(raw);
    }
    v.push(vec![0xC3, 0x28, 0xA0, 0xA1, 0xE2, 0x82]);
    v.push(b"#EXTM3U\r\n#EXTINF:1,x\r\n".to_vec());
    v.push(b"%PDF-1.7\n%\xE2\xE3".to_vec());
    v.push(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec());
    v.push(vec![1, 0, 2, 0, 3]);
    for (enc, text) in [
        (encoding_rs::GBK, "这是一个用简体中文写的文本文件，用来测试编码检测。"),
        (encoding_rs::BIG5, "這是一個用繁體中文寫的文字檔案，用來測試編碼偵測。"),
        (encoding_rs::SHIFT_JIS, "これは日本語で書かれたテキストファイルです。エンコーディングの検出をテストします。"),
        (encoding_rs::EUC_KR, "이것은 한국어로 작성된 텍스트 파일입니다. 인코딩 감지를 테스트합니다."),
        (encoding_rs::WINDOWS_1251, "Это текстовый файл на русском языке для проверки определения кодировки."),
    ] {
        let (b, _, _) = enc.encode(text);
        v.push(b.into_owned());
    }
    // Past the 1 MiB sniff limit, and a 2 MiB input.
    let mut big = vec![b'a'; 1 << 20];
    big.extend_from_slice(&[0xE9; 3]);
    v.push(big);
    let mut big = "héllo wörld ".repeat((2 << 20) / 13).into_bytes();
    big.push(0xE9);
    v.push(big);
    v.push(vec![0x80; 2 << 20]);
    v
}

fn assert_same(b: &[u8]) {
    let head: Vec<u8> = b.iter().take(24).copied().collect();
    assert_eq!(is_text(b), super::is_text(b), "is_text {head:02x?}");
    assert_eq!(legacy(b), super::legacy(b), "legacy {head:02x?}");
    assert_eq!(decode(b), super::decode(b), "decode {head:02x?}");
}

#[test]
fn shared_detect_matches_the_old_local_copies() {
    for b in samples() {
        assert_same(&b);
    }
}

#[test]
fn cp1252_agrees_with_encoding_rs_on_every_byte() {
    // Why a Windows-1252 guess needs no local mapping: both give U+0081, U+008D, U+008F, U+0090
    // and U+009D for the bytes the code page leaves undefined.
    for b in 0..=255u8 {
        let one = [b];
        let (s, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(&one);
        assert_eq!(s.chars().collect::<Vec<_>>(), vec![cp1252(b)], "byte {b:#x}");
    }
}

#[test]
fn random_buffers_never_panic_and_match() {
    let mut s: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (s >> 33) as u32
    };
    for i in 0..3000 {
        let len = (next() % 700) as usize;
        let mode = i % 4;
        let b: Vec<u8> = (0..len)
            .map(|_| {
                let r = next();
                match mode {
                    0 => r as u8,
                    1 => 0x80 | (r as u8 & 0x1F), // 0x80..=0x9F
                    2 => {
                        if r % 3 == 0 {
                            0
                        } else {
                            (r >> 8) as u8 % 0x80
                        }
                    }
                    _ => {
                        if r % 5 == 0 {
                            0x80 + (r >> 8) as u8 % 0x80
                        } else {
                            b'a' + (r >> 8) as u8 % 26
                        }
                    }
                }
            })
            .collect();
        assert_same(&b);
    }
}

//! Word 6.0/95 fixtures, built in code. No public specification describes this version; these
//! build the layout observed in real files and described in `crate::word6`.

use proptest::prelude::*;

use super::{body_texts, cfb_file};
use crate::{DocbinError, read};

/// Where the text starts: right after the FIB, as in every observed file.
const TEXT_AT: u32 = 0x300;

/// What a test Word 6.0/95 file holds. Offsets in `runs` and `pieces` are into `text`.
#[derive(Clone, Default)]
struct Doc6<'a> {
    nfib: u16,
    flags: u16,
    /// The stored 8-bit text.
    text: &'a [u8],
    /// (charset, name) per font index.
    fonts: &'a [(u8, &'a str)],
    /// Contiguous character runs (lo, hi, grpprl), all in one FKP page.
    runs: &'a [(u32, u32, &'a [u8])],
    /// For a fast-saved file: (length, offset) per piece, in CP order.
    pieces: Option<&'a [(u32, u32)]>,
}

fn put16(v: &mut [u8], at: usize, x: u16) {
    v[at..at + 2].copy_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut [u8], at: usize, x: u32) {
    v[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

/// The WordDocument stream: FIB, text, then the FKP page, font table and piece table.
fn word_stream(d: &Doc6) -> Vec<u8> {
    let mut w = vec![0u8; TEXT_AT as usize];
    w.extend_from_slice(d.text);
    let ccp = match d.pieces {
        Some(p) => p.iter().map(|(len, _)| len).sum(),
        None => d.text.len() as u32,
    };
    let mut pairs: Vec<(usize, u32, u32)> = Vec::new();
    if let (Some(first), Some(last)) = (d.runs.first(), d.runs.last()) {
        w.resize(w.len().div_ceil(512) * 512, 0);
        let pn = (w.len() / 512) as u16;
        let crun = d.runs.len();
        let mut page = [0u8; 512];
        for (k, (lo, _, _)) in d.runs.iter().enumerate() {
            put32(&mut page, 4 * k, TEXT_AT + lo);
        }
        put32(&mut page, 4 * crun, TEXT_AT + last.1);
        // CHPXs from the end of the page down, each at an even offset.
        let mut top = 511usize;
        for (k, (_, _, g)) in d.runs.iter().enumerate() {
            if g.is_empty() {
                continue;
            }
            let at = (top - 1 - g.len()) & !1;
            page[at] = g.len() as u8;
            page[at + 1..at + 1 + g.len()].copy_from_slice(g);
            page[4 * (crun + 1) + k] = (at / 2) as u8;
            top = at;
        }
        page[511] = crun as u8;
        w.extend_from_slice(&page);
        let mut bte = Vec::new();
        bte.extend_from_slice(&(TEXT_AT + first.0).to_le_bytes());
        bte.extend_from_slice(&(TEXT_AT + last.1).to_le_bytes());
        bte.extend_from_slice(&pn.to_le_bytes());
        pairs.push((12, w.len() as u32, bte.len() as u32));
        w.extend_from_slice(&bte);
    }
    if !d.fonts.is_empty() {
        let mut ffn = vec![0u8, 0];
        for (charset, name) in d.fonts {
            let mut e = vec![0u8, 0x16, 0x90, 0x01, *charset, 0x00];
            e.extend_from_slice(name.as_bytes());
            e.push(0);
            e[0] = (e.len() - 1) as u8;
            ffn.extend_from_slice(&e);
        }
        let size = ffn.len() as u16;
        put16(&mut ffn, 0, size);
        pairs.push((15, w.len() as u32, ffn.len() as u32));
        w.extend_from_slice(&ffn);
    }
    if let Some(pieces) = d.pieces {
        let mut plc = Vec::new();
        let mut cp = 0u32;
        plc.extend_from_slice(&cp.to_le_bytes());
        for (len, _) in pieces {
            cp += len;
            plc.extend_from_slice(&cp.to_le_bytes());
        }
        for (_, off) in pieces {
            plc.extend_from_slice(&[0, 0]);
            plc.extend_from_slice(&(TEXT_AT + off).to_le_bytes());
            plc.extend_from_slice(&[0, 0]);
        }
        let mut clx = vec![0x02u8];
        clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
        clx.extend_from_slice(&plc);
        pairs.push((33, w.len() as u32, clx.len() as u32));
        w.extend_from_slice(&clx);
    }
    put16(&mut w, 0x00, 0xA5DC);
    put16(&mut w, 0x02, if d.nfib == 0 { 0x0065 } else { d.nfib });
    put16(&mut w, 0x06, 0x0409);
    put16(&mut w, 0x0A, d.flags | if d.pieces.is_some() { 0x0004 } else { 0 });
    put16(&mut w, 0x0C, 0x0065);
    put32(&mut w, 0x18, TEXT_AT);
    put32(&mut w, 0x1C, TEXT_AT + d.text.len() as u32);
    let len = w.len() as u32;
    put32(&mut w, 0x20, len);
    put32(&mut w, 0x34, ccp);
    for (i, fc, lcb) in pairs {
        put32(&mut w, 0x58 + i * 8, fc);
        put32(&mut w, 0x58 + i * 8 + 4, lcb);
    }
    w
}

/// A Word 6.0/95 file holding `d`: the WordDocument stream only, as the real ones have no Table
/// stream.
fn word6_file(d: &Doc6) -> Vec<u8> {
    cfb_file(&[("WordDocument", &word_stream(d))])
}

fn texts(d: &Doc6) -> Vec<String> {
    body_texts(&read(&word6_file(d)).expect("a Word 6.0/95 file opens"))
}

/// Fonts 0 (Western) and 1 (Cyrillic, charset 204), as the Russian files have.
const FONTS: &[(u8, &str)] = &[(0, "Times New Roman"), (204, "Times New Roman Cyr")];

#[test]
fn word_6_and_95_files_open_as_paragraphs() {
    for nfib in [0x0065, 0x0068] {
        let d = Doc6 { nfib, text: b"Hello\rWorld\r", ..Default::default() };
        assert_eq!(texts(&d), ["Hello", "World"], "nFib 0x{nfib:04X}");
    }
}

#[test]
fn fast_saved_text_follows_the_piece_table() {
    // Stored out of order; the pieces' fcs are plain byte offsets (Word 97 would halve them).
    let d = Doc6 { text: b"World\rHello\r", pieces: Some(&[(6, 6), (6, 0)]), ..Default::default() };
    assert_eq!(texts(&d), ["Hello", "World"]);
}

#[test]
fn fast_saved_file_without_a_piece_table_is_malformed() {
    let d = Doc6 { flags: 0x0004, text: b"Hello\r", ..Default::default() };
    assert!(matches!(read(&word6_file(&d)), Err(DocbinError::Malformed(_))));
}

#[test]
fn a_runs_font_charset_chooses_its_code_page() {
    // "Da " in font 0, then 0xC4 0xE0 ("Да" in Windows-1251) in the Cyrillic font 1.
    let text = b"Da \xC4\xE0\r";
    let runs: &[(u32, u32, &[u8])] = &[(0, 3, &[]), (3, 5, &[0x5D, 0x01, 0x00]), (5, 6, &[])];
    let d = Doc6 { text, fonts: FONTS, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Da Да"]);
}

#[test]
fn the_font_sprm_is_found_after_other_sprms() {
    // As in real files: bold-like toggles, a size, a picture anchor's length-prefixed sprm.
    let text = b"\xC4\xE0\r";
    for g in [&[0x55, 0x81, 0x63, 0x18, 0x00, 0x5D, 0x01, 0x00][..], &[0x75, 0x01, 0x44, 0x04, 0, 0, 0, 0, 0x5D, 0x01, 0x00][..]] {
        let runs: &[(u32, u32, &[u8])] = &[(0, 2, g), (2, 3, &[])];
        let d = Doc6 { text, fonts: FONTS, runs, ..Default::default() };
        assert_eq!(texts(&d), ["Да"], "grpprl {g:02X?}");
    }
}

#[test]
fn an_unknown_sprm_leaves_font_0() {
    // 0x99 has no known operand length, so the rest can't be framed: font 0 (Windows-1252).
    let text = b"\xC4\xE0\r";
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, &[0x99, 0x5D, 0x01, 0x00]), (2, 3, &[])];
    let d = Doc6 { text, fonts: FONTS, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Äà"]);
}

#[test]
fn east_asian_font_bytes_read_as_replacement_characters() {
    // A multibyte code page can't be read one byte per CP: ASCII stays, the rest is U+FFFD.
    let text = b"ab\x82\xA0\r";
    let runs: &[(u32, u32, &[u8])] = &[(0, 4, &[0x5D, 0x01, 0x00]), (4, 5, &[])];
    let d = Doc6 { text, fonts: &[(0, "Arial"), (128, "MS Mincho")], runs, ..Default::default() };
    assert_eq!(texts(&d), ["ab\u{FFFD}\u{FFFD}"]);
}

#[test]
fn password_protected_files_are_refused() {
    let d = Doc6 { flags: 0x0100, text: b"secret\r", ..Default::default() };
    assert!(matches!(read(&word6_file(&d)), Err(DocbinError::Encryption(_))));
}

#[test]
fn cell_marks_end_plain_paragraphs() {
    // Rows can't be told apart without paragraph formatting, so there is no table yet.
    let d = Doc6 { text: b"A\x07B\x07\x07after\r", ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    assert!(doc.body.iter().all(|b| b.as_para().is_some()), "no table blocks");
    assert_eq!(body_texts(&doc), ["A", "B", "", "after"]);
}

#[test]
fn a_field_shows_its_cached_result() {
    let d = Doc6 { text: b"Date \x13DATE \x1412/12/97\x15\r", ..Default::default() };
    let text = read(&word6_file(&d)).expect("opens").plain_text(wordcraft_doc::StoryRef::Body);
    assert!(text.contains("Date 12/12/97"), "{text:?}");
    assert!(!text.contains("DATE"), "{text:?}");
}

/// A fixture with every structure read: runs, fonts and a piece table.
fn full_stream() -> Vec<u8> {
    let runs: &[(u32, u32, &[u8])] = &[(0, 6, &[0x55, 0x81]), (6, 9, &[0x5D, 0x01, 0x00]), (9, 12, &[])];
    word_stream(&Doc6 { text: b"Hello\r\xC4\xE0\rx\r\r", fonts: FONTS, runs, pieces: Some(&[(6, 0), (6, 6)]), ..Default::default() })
}

#[test]
fn the_full_fixture_reads() {
    let f = cfb_file(&[("WordDocument", &full_stream())]);
    assert_eq!(body_texts(&read(&f).expect("opens")), ["Hello", "Да", "x", ""]);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn corrupt_word6_streams_never_panic(edits in proptest::collection::vec((0..0x700usize, any::<u8>()), 1..16)) {
        let mut w = full_stream();
        for (at, b) in edits {
            if let Some(x) = w.get_mut(at) {
                *x = b;
            }
        }
        let _ = read(&cfb_file(&[("WordDocument", &w)]));
    }

    #[test]
    fn truncated_word6_streams_never_panic(n in 0..0x700usize) {
        let w = full_stream();
        let _ = read(&cfb_file(&[("WordDocument", &w[..n.min(w.len())])]));
    }
}

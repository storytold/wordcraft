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
    /// The font table's entries as raw bytes instead of `fonts` (the size word is added).
    raw_fonts: Option<&'a [u8]>,
    /// Contiguous character runs (lo, hi, grpprl), all in one FKP page.
    runs: &'a [(u32, u32, &'a [u8])],
    /// Contiguous paragraph runs (lo, hi, grpprl), style 0, all in one FKP page. Each covers the
    /// paragraph whose mark lies in it.
    paras: &'a [(u32, u32, &'a [u8])],
    /// For a fast-saved file: (length, offset) per piece, in CP order.
    pieces: Option<&'a [(u32, u32)]>,
    /// Each piece's Prm (0 when absent or shorter than `pieces`).
    prms: &'a [u16],
    /// The Clx's Prc grpprls, which a Prm1 indexes.
    prcs: &'a [&'a [u8]],
    /// The FIB's character set word at 0x14.
    chse: u16,
    /// The main text's length in characters, when it isn't the stored bytes' (16-bit text).
    ccp: Option<u32>,
}

fn put16(v: &mut [u8], at: usize, x: u16) {
    v[at..at + 2].copy_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut [u8], at: usize, x: u32) {
    v[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

/// Append one FKP page for `runs` and its bin table, and return the bin table's (fc, lcb). A
/// character page has a 1-byte entry per run and CHPXs (byte count + grpprl); a paragraph page
/// has 7-byte entries and PAPXs (word count + style 0 + grpprl padded to whole words).
fn fkp(w: &mut Vec<u8>, runs: &[(u32, u32, &[u8])], papx: bool) -> (u32, u32) {
    let (Some(first), Some(last)) = (runs.first(), runs.last()) else { return (0, 0) };
    w.resize(w.len().div_ceil(512) * 512, 0);
    let pn = (w.len() / 512) as u16;
    let crun = runs.len();
    let entry = if papx { 7 } else { 1 };
    let mut page = [0u8; 512];
    for (k, (lo, _, _)) in runs.iter().enumerate() {
        put32(&mut page, 4 * k, TEXT_AT + lo);
    }
    put32(&mut page, 4 * crun, TEXT_AT + last.1);
    // Records from the end of the page down, each at an even offset.
    let mut top = 511usize;
    for (k, (_, _, g)) in runs.iter().enumerate() {
        let body: Vec<u8> = if papx {
            let mut b = vec![0u8, 0];
            b.extend_from_slice(g);
            if b.len() % 2 == 1 {
                b.push(0);
            }
            b
        } else if g.is_empty() {
            continue;
        } else {
            g.to_vec()
        };
        let at = (top - 1 - body.len()) & !1;
        page[at] = if papx { (body.len() / 2) as u8 } else { body.len() as u8 };
        page[at + 1..at + 1 + body.len()].copy_from_slice(&body);
        page[4 * (crun + 1) + entry * k] = (at / 2) as u8;
        top = at;
    }
    page[511] = crun as u8;
    w.extend_from_slice(&page);
    let mut bte = Vec::new();
    bte.extend_from_slice(&(TEXT_AT + first.0).to_le_bytes());
    bte.extend_from_slice(&(TEXT_AT + last.1).to_le_bytes());
    bte.extend_from_slice(&pn.to_le_bytes());
    let at = w.len() as u32;
    w.extend_from_slice(&bte);
    (at, bte.len() as u32)
}

/// The WordDocument stream: FIB, text, then the FKP pages, font table and piece table.
fn word_stream(d: &Doc6) -> Vec<u8> {
    let mut w = vec![0u8; TEXT_AT as usize];
    w.extend_from_slice(d.text);
    let ccp = match (d.ccp, d.pieces) {
        (Some(ccp), _) => ccp,
        (None, pieces) => match pieces {
            Some(p) => p.iter().map(|(len, _)| len).sum(),
            None => d.text.len() as u32,
        },
    };
    let mut pairs: Vec<(usize, u32, u32)> = Vec::new();
    for (pair, runs, papx) in [(12, d.runs, false), (13, d.paras, true)] {
        let (fc, lcb) = fkp(&mut w, runs, papx);
        if lcb > 0 {
            pairs.push((pair, fc, lcb));
        }
    }
    if !d.fonts.is_empty() || d.raw_fonts.is_some() {
        let mut ffn = vec![0u8, 0];
        ffn.extend_from_slice(d.raw_fonts.unwrap_or_default());
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
        for (i, (_, off)) in pieces.iter().enumerate() {
            plc.extend_from_slice(&[0, 0]);
            plc.extend_from_slice(&(TEXT_AT + off).to_le_bytes());
            plc.extend_from_slice(&d.prms.get(i).copied().unwrap_or(0).to_le_bytes());
        }
        let mut clx = Vec::new();
        for g in d.prcs {
            clx.push(0x01);
            clx.extend_from_slice(&(g.len() as u16).to_le_bytes());
            clx.extend_from_slice(g);
        }
        clx.push(0x02);
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
    put16(&mut w, 0x14, d.chse);
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
fn the_last_font_sprm_of_a_run_chooses_its_code_page() {
    // Font 0, then font 1 (Cyrillic): the later sprm is the one the formatting applies.
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, &[0x5D, 0x00, 0x00, 0x5D, 0x01, 0x00]), (2, 3, &[])];
    let d = Doc6 { text: b"\xC4\xE0\r", fonts: FONTS, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Да"]);
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
fn the_toggle_sprms_before_the_font_are_framed() {
    // 0x57 (strike, a one-byte toggle in [MS-DOC]'s Prm0 isprm table) before the font sprm: the
    // font must still be found, or the Cyrillic text reads as Windows-1252.
    let text = b"\xC4\xE0\r";
    for op in [0x57, 0x58, 0x59, 0x5C] {
        let g = [op, 0x01, 0x5D, 0x01, 0x00];
        let runs: &[(u32, u32, &[u8])] = &[(0, 2, &g), (2, 3, &[])];
        let d = Doc6 { text, fonts: FONTS, runs, ..Default::default() };
        assert_eq!(texts(&d), ["Да"], "opcode 0x{op:02X}");
    }
}

#[test]
fn a_font_entry_too_short_for_its_charset_ends_the_font_table() {
    // Entry 0 says it is 3 bytes long, which can't hold the charset at offset 4. The bytes after
    // it (the next entry, whose flags byte is 0xCC) must not be read as font 0's charset.
    let raw: &[u8] = &[0x02, 0x16, 0x90, 0x05, 0xCC, 0x90, 0x01, 0xCC, 0x00, b'X', 0];
    let d = Doc6 { text: b"\xC4\xE0\r", raw_fonts: Some(raw), runs: &[(0, 3, &[])], ..Default::default() };
    assert_eq!(texts(&d), ["Äà"], "font 0 has no charset: Windows-1252");
}

#[test]
fn a_bin_table_naming_one_page_many_times_reads_it_once() {
    // A hostile bin table: 1,000,000 entries, all naming one page of 50 Cyrillic runs. The text still
    // reads right; that the page is read once is pinned in `word6::tests`.
    let text: Vec<u8> = std::iter::repeat_n(0xC4u8, 50).chain(*b"\r").collect();
    let g: &[u8] = &[0x5D, 0x01, 0x00];
    let runs: Vec<(u32, u32, &[u8])> = (0..50).map(|i| (i, i + 1, g)).chain([(50, 51, &[][..])]).collect();
    let mut w = word_stream(&Doc6 { text: &text, fonts: FONTS, runs: &runs, ..Default::default() });
    let u32_at = |w: &[u8], at: usize| u32::from_le_bytes([w[at], w[at + 1], w[at + 2], w[at + 3]]);
    let bte_fc = u32_at(&w, 0x58 + 12 * 8) as usize;
    let pn = u16::from_le_bytes([w[bte_fc + 8], w[bte_fc + 9]]);
    let n = 1_000_000usize;
    let mut bte = Vec::with_capacity(4 * (n + 1) + 2 * n);
    bte.extend_from_slice(&TEXT_AT.to_le_bytes());
    for _ in 0..n {
        bte.extend_from_slice(&(TEXT_AT + 51).to_le_bytes());
    }
    for _ in 0..n {
        bte.extend_from_slice(&pn.to_le_bytes());
    }
    let at = w.len() as u32;
    w.extend_from_slice(&bte);
    put32(&mut w, 0x58 + 12 * 8, at);
    put32(&mut w, 0x58 + 12 * 8 + 4, bte.len() as u32);
    let doc = read(&cfb_file(&[("WordDocument", &w)])).expect("opens");
    assert_eq!(body_texts(&doc), ["Д".repeat(50)]);
}

/// `s` as UTF-16LE bytes.
fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn a_file_with_extended_characters_holds_utf16_text() {
    // fExtChar (0x1000): Word reads the text as 16-bit units.
    let text = utf16("Да ok\r");
    let d = Doc6 { flags: 0x1000, text: &text, ccp: Some(6), ..Default::default() };
    assert_eq!(texts(&d), ["Да ok"]);
}

#[test]
fn utf16_text_stops_at_the_end_of_its_bytes() {
    // Word reads no more 16-bit characters than the text's byte range (0x18..0x1C) holds, even
    // when the character count says more.
    let text = utf16("Hi\r");
    let d = Doc6 { flags: 0x1000, text: &text, ccp: Some(50), ..Default::default() };
    assert_eq!(texts(&d), ["Hi"]);
}

#[test]
fn fast_saved_utf16_pieces_are_at_plain_byte_offsets() {
    let text = utf16("World\rHello\r");
    let d = Doc6 { flags: 0x1000, text: &text, pieces: Some(&[(6, 12), (6, 0)]), ..Default::default() };
    assert_eq!(texts(&d), ["Hello", "World"]);
}

#[test]
fn a_macintosh_document_reads_as_mac_roman() {
    // FIB 0x14 = 0x0100: byte 0x8E is "é" (Mac Roman), not "Ž" (Windows-1252), even in a run whose
    // font is Cyrillic.
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, &[0x5D, 0x01, 0x00]), (2, 3, &[])];
    let d = Doc6 { chse: 0x0100, text: b"H\x8E\r", fonts: FONTS, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Hé"]);
}

#[test]
fn a_symbol_document_maps_characters_to_private_use() {
    // FIB 0x14 = 0x0002: every character from the space up is U+F000 + byte; marks stay marks.
    let d = Doc6 { chse: 0x0002, text: b"Hi\x8E\rA\r", ..Default::default() };
    assert_eq!(texts(&d), ["\u{F048}\u{F069}\u{F08E}", "\u{F041}"]);
}

#[test]
fn another_character_set_leaves_only_ascii_readable() {
    let d = Doc6 { chse: 0x0001, text: b"H\x8Ei\r", ..Default::default() };
    assert_eq!(texts(&d), ["H\u{FFFD}i"]);
}

#[test]
fn a_font_in_a_pieces_prm_chooses_its_code_page() {
    // As Word 95 fast-saves a font change: piece 0 ("\xC4\xE0") has Prm1 -> Prc 0 = sprm 0x5D font 1
    // (Cyrillic), while its character run names no font. Piece 1 keeps the run's font 0.
    let text = b"\xC4\xE0\xC4\xE0\r";
    let d = Doc6 { text, fonts: FONTS, pieces: Some(&[(2, 0), (3, 2)]), prms: &[0x0001], prcs: &[&[0x5D, 0x01, 0x00]], ..Default::default() };
    assert_eq!(texts(&d), ["ДаÄà"]);
}

#[test]
fn a_prm_index_past_the_prcs_is_ignored() {
    let d = Doc6 { text: b"\xC4\xE0\r", fonts: FONTS, pieces: Some(&[(3, 0)]), prms: &[0x0011], prcs: &[&[0x5D, 0x01, 0x00]], ..Default::default() };
    assert_eq!(texts(&d), ["Äà"]);
}

#[test]
fn password_protected_files_are_refused() {
    let d = Doc6 { flags: 0x0100, text: b"secret\r", ..Default::default() };
    assert!(matches!(read(&word6_file(&d)), Err(DocbinError::Encryption(_))));
}

/// The first paragraph of `doc`'s body.
fn first_para(doc: &wordcraft_doc::Document) -> &wordcraft_doc::para::Paragraph {
    doc.body.iter().find_map(|b| b.as_para()).expect("a paragraph")
}

#[test]
fn character_formatting_is_read() {
    // Bold on, italic as the opposite of the (unread, so plain) style, 14 pt, red, single
    // underline, font 2: the Word 6 opcodes as observed.
    let g: &[u8] = &[0x55, 0x01, 0x56, 0x81, 0x63, 0x1C, 0x00, 0x62, 0x06, 0x5E, 0x01, 0x5D, 0x02, 0x00];
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, g), (2, 3, &[])];
    let fonts = &[(0, "Times New Roman"), (2, "Symbol"), (0, "Arial")];
    let doc = read(&word6_file(&Doc6 { text: b"Hi\r", fonts, runs, ..Default::default() })).expect("opens");
    let p = first_para(&doc);
    let c = &p.runs.first().expect("a run").props;
    assert_eq!((c.bold, c.italic, c.size), (Some(true), Some(true), Some(14.0)));
    assert_eq!(c.color, Some(wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::props::Rgb(0xFF, 0, 0))));
    assert_eq!(c.underline, Some(wordcraft_doc::props::Underline::Single));
    assert_eq!(c.font.as_deref(), Some("Arial"));
}

#[test]
fn unseen_operand_values_are_left_unread() {
    // Underline 3 and "multiple" line spacing were never seen in a real file, so they aren't
    // guessed at.
    let runs: &[(u32, u32, &[u8])] = &[(0, 3, &[0x5E, 0x03])];
    let paras: &[(u32, u32, &[u8])] = &[(0, 3, &[0x14, 0xF0, 0x00, 0x01, 0x00])];
    let doc = read(&word6_file(&Doc6 { text: b"Hi\r", runs, paras, ..Default::default() })).expect("opens");
    let p = first_para(&doc);
    assert_eq!(p.runs.first().and_then(|r| r.props.underline), None);
    assert_eq!(p.props.line_spacing, None);
}

#[test]
fn an_unseen_alignment_value_is_left_unread() {
    // Only 0-3 were seen; 4 would otherwise become "distribute" and 0xFF "left".
    for jc in [0x04u8, 0xFF] {
        let g = [0x05, jc];
        let paras: &[(u32, u32, &[u8])] = &[(0, 3, &g)];
        let doc = read(&word6_file(&Doc6 { text: b"Hi\r", paras, ..Default::default() })).expect("opens");
        assert_eq!(first_para(&doc).props.align, None, "jc {jc:#04x}");
    }
}

#[test]
fn paragraph_formatting_is_read() {
    // Justified, 0.5" left and 0.25" right indents, a 0.5" hanging first line, at least 27 pt.
    let g: &[u8] = &[0x05, 0x03, 0x11, 0xD0, 0x02, 0x10, 0x68, 0x01, 0x13, 0x30, 0xFD, 0x14, 0x1C, 0x02, 0x00, 0x00];
    let paras: &[(u32, u32, &[u8])] = &[(0, 3, g), (3, 6, &[0x05, 0x01])];
    let doc = read(&word6_file(&Doc6 { text: b"Hi\rHo\r", paras, ..Default::default() })).expect("opens");
    let ps: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).collect();
    let p = &ps[0].props;
    assert_eq!(p.align, Some(wordcraft_doc::props::Align::Justify));
    assert_eq!((p.indent_left, p.indent_right, p.indent_first), (Some(36.0), Some(18.0), Some(-36.0)));
    assert_eq!(p.line_spacing, Some(wordcraft_doc::props::LineSpacing::AtLeast(27.0)));
    assert_eq!(ps[1].props.align, Some(wordcraft_doc::props::Align::Center));
}

/// A Word 6 table definition sprm (0xBE): the cell edges, then a 10-byte descriptor per cell
/// whose first word holds the merge flags.
fn def_table(edges: &[i16], rgfs: &[u16]) -> Vec<u8> {
    let mut rest = vec![rgfs.len() as u8];
    for e in edges {
        rest.extend_from_slice(&e.to_le_bytes());
    }
    for r in rgfs {
        rest.extend_from_slice(&r.to_le_bytes());
        rest.extend_from_slice(&[0; 8]);
    }
    let mut v = vec![0xBE];
    v.extend_from_slice(&((rest.len() + 1) as u16).to_le_bytes());
    v.extend_from_slice(&rest);
    v
}

/// The rows of the first table in `doc`'s body, each as its cells' texts.
fn table_rows(doc: &wordcraft_doc::Document) -> Vec<Vec<String>> {
    let t = doc.body.iter().find_map(|b| match b.as_ref() {
        wordcraft_doc::Block::Table(t) => Some(t),
        _ => None,
    });
    let t = t.expect("a table");
    t.rows
        .iter()
        .map(|r| r.cells.iter().map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<Vec<_>>().join("|")).collect())
        .collect()
}

#[test]
fn table_rows_and_merged_cells_are_read() {
    // Row 1: "A", "B". Row 2: "C" merged across both cells. In-table marks carry 0x18, the row
    // ends 0x18, 0x19 and the table definition.
    let text = b"A\x07B\x07\x07C\x07\x07\x07after\r";
    let mut row1 = vec![0x18, 1, 0x19, 1];
    row1.extend(def_table(&[0, 1440, 2880], &[0, 0]));
    let mut row2 = vec![0x18, 1, 0x19, 1];
    row2.extend(def_table(&[0, 1440, 2880], &[0x0001, 0x0002]));
    let cell: &[u8] = &[0x18, 1];
    let paras: &[(u32, u32, &[u8])] = &[(0, 2, cell), (2, 4, cell), (4, 5, &row1), (5, 7, cell), (7, 8, cell), (8, 9, &row2), (9, 15, &[])];
    let doc = read(&word6_file(&Doc6 { text, paras, ..Default::default() })).expect("opens");
    assert_eq!(table_rows(&doc), [vec!["A", "B"], vec!["C"]]);
    assert_eq!(body_texts(&doc), ["after"], "the paragraph after the table stays outside it");
}

#[test]
fn several_merges_in_one_row_each_keep_their_cells() {
    // Five cells merged as [A B] [C] [D E]: the second merge must not be shifted by the first.
    let text = b"A\x07B\x07C\x07D\x07E\x07\x07";
    let mut row = vec![0x18, 1, 0x19, 1];
    row.extend(def_table(&[0, 100, 200, 300, 400, 500], &[0x0001, 0x0002, 0x0000, 0x0001, 0x0002]));
    let cell: &[u8] = &[0x18, 1];
    let paras: &[(u32, u32, &[u8])] = &[(0, 2, cell), (2, 4, cell), (4, 6, cell), (6, 8, cell), (8, 10, cell), (10, 11, &row)];
    let doc = read(&word6_file(&Doc6 { text, paras, ..Default::default() })).expect("opens");
    assert_eq!(table_rows(&doc), [vec!["A|B", "C", "D|E"]]);
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

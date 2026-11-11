//! Word 6.0 and Word 95 `.doc` files (`wIdent` 0xA5DC, `nFib` 0x0065 and 0x0068).
//!
//! No public specification describes this version: [MS-DOC] covers Word 97 and later. Everything
//! here is observed, from the layout of real files checked against what Word shows when it opens
//! them (its text through COM, its formatting in the `.docx` Word saves from them), plus the Word
//! 97 structures in [MS-DOC] that these files visibly share. Nothing else is assumed.
//!
//! What is read: the main text, its paragraphs and fields, the code page of each run, the
//! character and paragraph formatting listed below, and table rows. Styles, sections, headers
//! and footers, notes, pictures, borders, shading and tabs are not read yet.
//!
//! The layout as observed:
//!
//! - A compound file with a `WordDocument` stream and **no Table stream**: the structures that
//!   Word 97 keeps in `0Table`/`1Table` live in the WordDocument stream itself.
//! - The FIB is one fixed block. The flags at 0x0A have the Word 97 meaning for bit 2 (fast-saved,
//!   set exactly in the files that carry a piece table) and, as far as can be told, bit 8
//!   (encrypted). The text starts at the u32 at 0x18, and the main text's length in characters is
//!   the u32 at 0x34.
//! - From 0x58, (u32 fc, u32 lcb) pairs, in the order of Word 97's `FibRgFcLcb97`. Index 12 is the
//!   character bin table, 13 the paragraph bin table, 15 the font table and 33 the piece table.
//! - Text is one byte per character, in the code page of the font of its run. The FIB's language
//!   does not choose it: files marked English hold Russian and Czech text that Word shows as such.
//! - Both bin tables hold 2-byte page numbers (Word 97: 4 bytes). The FKP pages have the Word 97
//!   layout, except that a paragraph page's entries are 7 bytes (an offset byte and 6 more; Word
//!   97: 13), and a PAPX is a count of 2-byte words followed by exactly that many words: the
//!   2-byte style index, then the grpprl (padded with one 0 byte to whole words).
//! - Sprms have **1-byte opcodes**. Those with a 1-byte operand match the opcodes [MS-DOC]'s
//!   `Prm0` table lists for the same properties (bold 0x55, colour 0x62, in-table 0x18…); the
//!   others were identified against Word. See [`chp`] and [`pap`].
//! - A font table entry is a size byte (the entry's length − 1), a flags byte, a 2-byte weight, the
//!   charset byte, one more byte, then the name in 8-bit characters.

use wordcraft_doc::encoding::charset_code_page;

use crate::DocbinError;
use crate::fkp::{Bins, TranslatedRun};
use crate::piece::{CodePageRun, PieceTable, SYMBOL_CODE_PAGE, Word6Text};

/// `wIdent` of a Word 6.0/95 file.
const W_IDENT: u16 = 0xA5DC;
const OFF_FLAGS: usize = 0x0A;
/// Fast-saved: the text is in pieces listed by a piece table.
const F_COMPLEX: u16 = 0x0004;
/// Password-protected (Word 97's bit; no unprotected file sets it).
const F_ENCRYPTED: u16 = 0x0100;
/// Word 97's `fExtChar` bit. Word reads a Word 6.0/95 file that sets it as UTF-16 text (each
/// piece's fc a plain byte offset), and so does this reader.
const F_EXT_CHAR: u16 = 0x1000;
/// The document's character set (u16): 0 Windows, 0x0100 Macintosh (all 8-bit text is Mac Roman,
/// whatever the fonts say), 0x0002 Symbol (every byte a private-use character). Word reads the
/// bytes above ASCII of any other value as U+FFFD.
const OFF_CHSE: usize = 0x14;
const OFF_FC_MAC: usize = 0x1C;
/// Bin table entries read, as for Word 97 (`fkp::MAX_BINS`).
const MAX_BINS: usize = 100_000;
/// FKP runs read (character or paragraph): far more than any real file's, and few enough to stay small.
const MAX_RUNS: usize = 1_000_000;
const OFF_FC_MIN: usize = 0x18;
const OFF_CCP_TEXT: usize = 0x34;
const OFF_PAIRS: usize = 0x58;
/// Pairs read: up to and including the piece table's.
const PAIRS: usize = 34;
const PAIR_BTE_CHPX: usize = 12;
const PAIR_BTE_PAPX: usize = 13;
const PAIR_FONTS: usize = 15;
const PAIR_CLX: usize = 33;
/// More characters than Word's own documents get near: the FIB is corrupt (as for Word 97).
const MAX_CCP: u32 = 100_000_000;
/// The font table's entries: more than any real file has.
const MAX_FONTS: usize = 0x1_0000;
/// FKP page size in bytes.
const PAGE: usize = 512;
/// Size of a paragraph FKP entry (offset byte + 6).
const PAPX_BX: usize = 7;
/// Most cells a row's table definition is read for (the model's own cap is 63).
const MAX_CELLS: usize = 63;

/// Character sprms read, by Word 6.0/95 opcode. Each one was matched against Word's own reading of
/// the corpus files (the `.docx` it saves from them).
mod chp {
    /// Bold, a toggle (0 off, 1 on, 0x80 as the style, 0x81 the opposite of the style).
    pub(super) const BOLD: u8 = 0x55;
    /// Italic, a toggle.
    pub(super) const ITALIC: u8 = 0x56;
    /// Small capitals, a toggle.
    pub(super) const SMALL_CAPS: u8 = 0x5A;
    /// All capitals, a toggle.
    pub(super) const CAPS: u8 = 0x5B;
    /// The font: a 2-byte index into the font table.
    pub(super) const FONT: u8 = 0x5D;
    /// Underline: 0 none, 1 single (the only values seen).
    pub(super) const UNDERLINE: u8 = 0x5E;
    /// Colour: an index into Word's 16-colour palette, as Word 97's `Ico`.
    pub(super) const COLOR: u8 = 0x62;
    /// Size in half-points (2 bytes).
    pub(super) const SIZE: u8 = 0x63;
}

/// Paragraph and table sprms read, by Word 6.0/95 opcode, matched like [`chp`]'s.
mod pap {
    /// Alignment: 0 left, 1 centred, 2 right, 3 justified.
    pub(super) const JC: u8 = 0x05;
    /// Right indent in twips (signed, 2 bytes).
    pub(super) const RIGHT: u8 = 0x10;
    /// Left indent in twips.
    pub(super) const LEFT: u8 = 0x11;
    /// First-line indent in twips, relative to the left one (negative: hanging).
    pub(super) const FIRST: u8 = 0x13;
    /// Line spacing: the 4-byte Word 97 `LSPD` (dyaLine, fMultLinespace).
    pub(super) const LINE: u8 = 0x14;
    /// The paragraph is in a table.
    pub(super) const IN_TABLE: u8 = 0x18;
    /// The mark ends a table row.
    pub(super) const TTP: u8 = 0x19;
    /// Row alignment (2 bytes): 1 centred.
    pub(super) const ROW_JC: u8 = 0xB6;
    /// Row height in twips (signed, 2 bytes): positive at least, negative exactly.
    pub(super) const ROW_HEIGHT: u8 = 0xBD;
    /// The row's cells: u16 count, cell count, (count + 1) i16 cell edges, then a 10-byte
    /// descriptor per cell whose first word's bit 0 starts a horizontal merge and bit 1 continues
    /// it.
    pub(super) const DEF_TABLE: u8 = 0xBE;
}

/// Word 97 opcodes the translations produce ([MS-DOC] §2.6).
mod w97 {
    pub(super) const C_BOLD: u16 = 0x0835;
    pub(super) const C_ITALIC: u16 = 0x0836;
    pub(super) const C_SMALL_CAPS: u16 = 0x083A;
    pub(super) const C_CAPS: u16 = 0x083B;
    pub(super) const C_KUL: u16 = 0x2A3E;
    pub(super) const C_ICO: u16 = 0x2A42;
    pub(super) const C_HPS: u16 = 0x4A43;
    pub(super) const C_FTC0: u16 = 0x4A4F;
    pub(super) const P_JC80: u16 = 0x2403;
    pub(super) const P_DXA_RIGHT80: u16 = 0x840E;
    pub(super) const P_DXA_LEFT80: u16 = 0x840F;
    pub(super) const P_DXA_LEFT1_80: u16 = 0x8411;
    pub(super) const P_DYA_LINE: u16 = 0x6412;
    pub(super) const P_IN_TABLE: u16 = 0x2416;
    pub(super) const P_TTP: u16 = 0x2417;
    pub(super) const T_JC: u16 = 0x5400;
    pub(super) const T_ROW_HEIGHT: u16 = 0x9407;
    pub(super) const T_DEF_TABLE: u16 = 0xD608;
    pub(super) const T_MERGE: u16 = 0x5624;
}

/// Is this WordDocument stream a Word 6.0/95 one?
pub(crate) fn is_word6(word: &[u8]) -> bool {
    word.get(0..2) == Some(&W_IDENT.to_le_bytes()[..])
}

/// The fields of a Word 6.0/95 FIB that are read.
#[derive(Clone, Debug)]
pub(crate) struct Fib6 {
    complex: bool,
    /// Text is UTF-16 (fExtChar).
    wide: bool,
    chse: u16,
    fc_min: u32,
    fc_mac: u32,
    /// Characters in the main text.
    pub(crate) ccp_text: u32,
    pairs: [(u32, u32); PAIRS],
}

impl Fib6 {
    pub(crate) fn parse(word: &[u8]) -> Result<Fib6, DocbinError> {
        let u16_at = |at: usize| word.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
        let u32_at = |at: usize| word.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        if word.len() < OFF_PAIRS + PAIRS * 8 {
            return Err(DocbinError::NotWord(format!("Word 6.0/95 WordDocument stream is {} bytes", word.len())));
        }
        let flags = u16_at(OFF_FLAGS).unwrap_or(0);
        if flags & F_ENCRYPTED != 0 {
            return Err(DocbinError::Encryption("password-protected Word 6.0/95 files can't be opened".into()));
        }
        let ccp_text = u32_at(OFF_CCP_TEXT).unwrap_or(0);
        if ccp_text > MAX_CCP {
            return Err(DocbinError::Limit(format!("document claims {ccp_text} characters")));
        }
        let mut pairs = [(0, 0); PAIRS];
        for (i, p) in pairs.iter_mut().enumerate() {
            let at = OFF_PAIRS + i * 8;
            *p = (u32_at(at).unwrap_or(0), u32_at(at + 4).unwrap_or(0));
        }
        Ok(Fib6 {
            complex: flags & F_COMPLEX != 0,
            wide: flags & F_EXT_CHAR != 0,
            chse: u16_at(OFF_CHSE).unwrap_or(0),
            fc_min: u32_at(OFF_FC_MIN).unwrap_or(0),
            fc_mac: u32_at(OFF_FC_MAC).unwrap_or(0),
            ccp_text,
            pairs,
        })
    }

    /// The (fc, lcb) pair at index `i`, if it points at anything.
    fn pair(&self, i: usize) -> Option<(u32, u32)> {
        self.pairs.get(i).copied().filter(|&(_, lcb)| lcb > 0)
    }
}

/// Everything the walk needs from a Word 6.0/95 file besides its text stream.
pub(crate) struct Content {
    pub(crate) pieces: PieceTable,
    /// Character formatting, translated to Word 97 grpprls.
    pub(crate) chpx: Bins,
    /// Paragraph formatting and table rows, translated to Word 97 grpprls.
    pub(crate) papx: Bins,
    /// Font names by index (the walk resolves translated font sprms against them).
    pub(crate) fonts: Vec<String>,
}

/// Read the text, formatting and fonts of a Word 6.0/95 file.
///
/// 8-bit text decodes in the code page of, in order: the document's character set when it isn't
/// Windows ([`OFF_CHSE`]); else the font a piece's Prm names (as Word 95 writes when a fast save
/// changes a font); else the font of the character run (font 0 when it names none, which matches
/// every real file so far; Word itself then follows the style's language, which isn't read yet);
/// else Windows-1252.
pub(crate) fn content(word: &[u8], fib: &Fib6) -> Result<Content, DocbinError> {
    let fonts = fib.pair(PAIR_FONTS).map(|(fc, lcb)| font_table(word, fc, lcb)).unwrap_or_default();
    let chp_runs = fib.pair(PAIR_BTE_CHPX).map(|(fc, lcb)| fkp_runs(word, fc, lcb, false)).unwrap_or_default();
    let pap_runs = fib.pair(PAIR_BTE_PAPX).map(|(fc, lcb)| fkp_runs(word, fc, lcb, true)).unwrap_or_default();
    let windows = fib.chse == 0;
    let default_code_page = match fib.chse {
        0 => 1252,
        0x0100 => MAC_ROMAN,
        0x0002 => SYMBOL_CODE_PAGE,
        // Not a code page: bytes above ASCII read as U+FFFD.
        _ => 0,
    };
    let code_pages = if windows { code_page_runs(&chp_runs, &fonts) } else { Vec::new() };
    let text = Word6Text { wide: fib.wide, default_code_page, code_pages };
    let pieces = if fib.complex {
        let (fc, lcb) = fib.pair(PAIR_CLX).ok_or_else(|| DocbinError::Malformed("the fast-saved file has no piece table".into()))?;
        // A Prm1's Prc may name the piece's font (sprm 0x5D); a Prm0 can't, its operand being one
        // byte.
        let piece_code_page = |prm: u16, prcs: &[Vec<u8>]| -> Option<u16> {
            if !windows || prm & 1 == 0 {
                return None;
            }
            last_font(prcs.get(usize::from(prm >> 1))?).map(|f| code_page_of(&fonts, f))
        };
        // The Prcs then carry Word 97 sprms, as the CHPXs do, for the walk to apply to the piece.
        PieceTable::parse_word6(word, fc, lcb, text, piece_code_page, translate_chp)?
    } else {
        // Word reads no more characters than the text's byte range (fcMin..fcMac) holds, one byte
        // each or two for UTF-16, whatever the character count says.
        let unit = if fib.wide { 2 } else { 1 };
        let ccp = fib.ccp_text.min(fib.fc_mac.saturating_sub(fib.fc_min) / unit);
        PieceTable::contiguous_word6(fib.fc_min, ccp, text)
    };
    let chpx = Bins::translated(chp_runs.iter().map(|r| TranslatedRun { lo: r.lo, hi: r.hi, istd: 0, grpprl: translate_chp(r.grpprl) }).collect());
    let papx =
        Bins::translated(pap_runs.iter().map(|r| TranslatedRun { lo: r.lo, hi: r.hi, istd: r.istd, grpprl: translate_pap(r.grpprl) }).collect());
    Ok(Content { pieces, chpx, papx, fonts: fonts.into_iter().map(|f| f.name).collect() })
}

/// Mac OS Roman, the 8-bit text of a Macintosh Word document.
const MAC_ROMAN: u16 = wordcraft_doc::encoding::MAC_ROMAN_CODE_PAGE;

/// The code page of font `font`'s charset (Windows-1252 for an ANSI, default or unknown one).
fn code_page_of(fonts: &[Font], font: u16) -> u16 {
    fonts.get(usize::from(font)).and_then(|f| charset_code_page(i32::from(f.charset))).unwrap_or(1252)
}

/// One font table entry.
struct Font {
    name: String,
    charset: u8,
}

/// The fonts of the font table at `word[fc..fc+lcb]`, by index. A truncated entry ends the table.
fn font_table(word: &[u8], fc: u32, lcb: u32) -> Vec<Font> {
    let Some(table) = (fc as usize).checked_add(lcb as usize).and_then(|end| word.get(fc as usize..end)) else { return Vec::new() };
    // The table starts with its own size, which bounds the entries along with `lcb`.
    let size = table.get(0..2).map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])));
    let table = table.get(..size.min(table.len())).unwrap_or_default();
    let mut fonts = Vec::new();
    let mut at = 2usize;
    while let Some(&cb_m1) = table.get(at) {
        let next = at + usize::from(cb_m1) + 1;
        // The entry must lie inside the table and hold its charset (one too short is corrupt).
        let Some(entry) = table.get(at..next) else { break };
        let (Some(&charset), Some(name)) = (entry.get(4), entry.get(6..)) else { break };
        if fonts.len() >= MAX_FONTS {
            break;
        }
        // An 8-bit name; font names are ASCII in every file seen, other bytes read as Latin-1.
        let name = name.split(|&b| b == 0).next().unwrap_or_default().iter().map(|&b| char::from(b)).collect();
        fonts.push(Font { name, charset });
        at = next;
    }
    fonts
}

/// One FKP run as stored: its FC range, the paragraph style (PAPX runs) and its Word 6 grpprl.
struct Run<'w> {
    lo: u32,
    hi: u32,
    istd: u16,
    grpprl: &'w [u8],
}

/// The page numbers of a bin table with `n` entries ((n+1) u32 FCs, then n u16 page numbers),
/// each once, however many entries name it: a page's runs carry their own FCs, so reading it
/// again would only repeat them. The entries read are capped as for Word 97.
fn distinct_pages(plc: &[u8], n: usize) -> Vec<usize> {
    let mut pns: Vec<usize> = (0..n.min(MAX_BINS))
        .filter_map(|i| plc.get(4 * (n + 1) + 2 * i..4 * (n + 1) + 2 * i + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]]))))
        .collect();
    pns.sort_unstable();
    pns.dedup();
    pns
}

/// Every run of the FKP pages a bin table at `word[fc..fc+lcb]` lists, each page read once and
/// at most [`MAX_RUNS`] runs in all. Unreadable pages are skipped.
fn fkp_runs(word: &[u8], fc: u32, lcb: u32, papx: bool) -> Vec<Run<'_>> {
    let Some(plc) = (fc as usize).checked_add(lcb as usize).and_then(|end| word.get(fc as usize..end)) else { return Vec::new() };
    let Some(n) = plc.len().checked_sub(4).filter(|r| r % 6 == 0).map(|r| r / 6) else { return Vec::new() };
    let entry = if papx { PAPX_BX } else { 1 };
    let mut runs = Vec::new();
    for pn in distinct_pages(plc, n) {
        if runs.len() >= MAX_RUNS {
            break;
        }
        let Some(page) = pn.checked_mul(PAGE).and_then(|at| word.get(at..at.checked_add(PAGE)?)) else { continue };
        let crun = usize::from(page.get(PAGE - 1).copied().unwrap_or(0));
        // The FCs and the entries must fit before the count byte.
        if 4 * (crun + 1) + entry * crun >= PAGE {
            continue;
        }
        let fc_at = |k: usize| page.get(4 * k..4 * k + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        for k in 0..crun {
            let (Some(lo), Some(hi)) = (fc_at(k), fc_at(k + 1)) else { break };
            let off = usize::from(page.get(4 * (crun + 1) + entry * k).copied().unwrap_or(0)) * 2;
            let (istd, grpprl) = match (off, papx) {
                (0, _) => (0, &[][..]),
                // A CHPX: a byte count, then the grpprl.
                (_, false) => (0, page.get(off).and_then(|&cb| page.get(off + 1..off + 1 + usize::from(cb))).unwrap_or_default()),
                // A PAPX: a count of 2-byte words, then the style index and the grpprl.
                (_, true) => {
                    let body = page.get(off).and_then(|&cw| page.get(off + 1..off + 1 + 2 * usize::from(cw))).unwrap_or_default();
                    let istd = body.get(0..2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
                    (istd, body.get(2..).unwrap_or_default())
                }
            };
            runs.push(Run { lo, hi, istd, grpprl });
        }
    }
    runs
}

/// The sprms of a Word 6.0/95 grpprl as (opcode, operand). Reading stops at an opcode whose
/// operand length is not known, since the rest can't be framed, and at a lone trailing 0 (the
/// padding of a PAPX to whole words).
fn sprms(grpprl: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut rest = grpprl;
    std::iter::from_fn(move || {
        let (&op, after) = rest.split_first()?;
        let len = match op {
            // 0x55-0x5C are the character toggles of [MS-DOC]'s Prm0 isprm table (bold, italic,
            // strike, outline, shadow, small caps, caps, hidden), all one byte; 0x55, 0x56, 0x5A
            // and 0x5B are also seen in real files.
            0x05 | 0x18 | 0x19 | 0x33 | 0x55..=0x5C | 0x5E | 0x62 | 0x68 | 0x75 | 0xB7 | 0xB9 => 1,
            0x10 | 0x11 | 0x13 | 0x15 | 0x16 | 0x26 | 0x27 | 0x28 | 0x29 | 0x2F | 0x50 | 0x5D | 0x60 | 0x61 | 0x63 | 0xB6 | 0xB8 | 0xBD | 0xC0 => 2,
            0x14 => 4,
            // A count byte, then that many bytes.
            0x0F | 0x44 | 0xBF => 1 + usize::from(*after.first()?),
            // A 2-byte count, then one byte fewer than it says (as Word 97's sprmTDefTable).
            0xBE => 2 + usize::from(u16::from_le_bytes([*after.first()?, *after.get(1)?])).saturating_sub(1),
            _ => return None,
        };
        let operand = after.get(..len)?;
        rest = after.get(len..).unwrap_or_default();
        Some((op, operand))
    })
}

/// Append a Word 97 Prl: opcode, then operand.
fn prl(out: &mut Vec<u8>, op: u16, operand: &[u8]) {
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(operand);
}

/// A Word 6.0/95 CHPX grpprl as the Word 97 sprms it stands for. Sprms not read are left out.
fn translate_chp(grpprl: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (op, v) in sprms(grpprl) {
        match op {
            chp::BOLD => prl(&mut out, w97::C_BOLD, v),
            chp::ITALIC => prl(&mut out, w97::C_ITALIC, v),
            chp::SMALL_CAPS => prl(&mut out, w97::C_SMALL_CAPS, v),
            chp::CAPS => prl(&mut out, w97::C_CAPS, v),
            // Only none and single were seen; any other value is left unread.
            chp::UNDERLINE if matches!(v, [0] | [1]) => prl(&mut out, w97::C_KUL, v),
            chp::COLOR => prl(&mut out, w97::C_ICO, v),
            chp::SIZE => prl(&mut out, w97::C_HPS, v),
            chp::FONT => prl(&mut out, w97::C_FTC0, v),
            _ => {}
        }
    }
    out
}

/// A Word 6.0/95 PAPX grpprl as the Word 97 sprms it stands for, table rows included. Sprms not
/// read are left out.
fn translate_pap(grpprl: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (op, v) in sprms(grpprl) {
        match op {
            // Only 0-3 were seen: another value is left unread.
            pap::JC if matches!(v, [0..=3]) => prl(&mut out, w97::P_JC80, v),
            pap::RIGHT => prl(&mut out, w97::P_DXA_RIGHT80, v),
            pap::LEFT => prl(&mut out, w97::P_DXA_LEFT80, v),
            pap::FIRST => prl(&mut out, w97::P_DXA_LEFT1_80, v),
            // Only "at least" spacing was seen (a positive height, not a multiple): the other
            // forms are left unread.
            pap::LINE => {
                if let [l0, l1, 0, 0] = v
                    && i16::from_le_bytes([*l0, *l1]) > 0
                {
                    prl(&mut out, w97::P_DYA_LINE, v);
                }
            }
            pap::IN_TABLE => prl(&mut out, w97::P_IN_TABLE, v),
            pap::TTP => prl(&mut out, w97::P_TTP, v),
            pap::ROW_JC => prl(&mut out, w97::T_JC, v),
            pap::ROW_HEIGHT => prl(&mut out, w97::T_ROW_HEIGHT, v),
            pap::DEF_TABLE => def_table(&mut out, v),
            _ => {}
        }
    }
    out
}

/// A Word 6.0/95 table definition as a Word 97 `sprmTDefTable` holding the cell edges (the only
/// part the table reader uses) and one `sprmTMerge` per horizontal merge.
fn def_table(out: &mut Vec<u8>, v: &[u8]) {
    let Some(&itc) = v.get(2) else { return };
    let itc = usize::from(itc).min(MAX_CELLS);
    let Some(edges) = v.get(3..3 + 2 * (itc + 1)) else { return };
    // Word 97 operand: cb (u16: the bytes after it, plus one), the cell count, the edges.
    let cb = (1 + edges.len() + 1) as u16;
    out.extend_from_slice(&w97::T_DEF_TABLE.to_le_bytes());
    out.extend_from_slice(&cb.to_le_bytes());
    out.push(itc as u8);
    out.extend_from_slice(edges);
    let cells = v.get(3 + edges.len()..).unwrap_or_default();
    let mut merges: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for i in 0..=itc {
        let rgf = cells.get(10 * i..10 * i + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
        let continues = i < itc && rgf & 0x0002 != 0 && rgf & 0x0001 == 0;
        if continues {
            continue;
        }
        // A merge runs from its first cell through every cell that continues it.
        if let Some(a) = start.take()
            && i > a + 1
        {
            merges.push((a, i));
        }
        if i < itc && rgf & 0x0001 != 0 {
            start = Some(i);
        }
    }
    // Right to left: the table reader folds each merge's cells into its first one as it goes, so
    // a merge further right must be applied before one to its left moves its cells.
    for (a, b) in merges.into_iter().rev() {
        prl(out, w97::T_MERGE, &[a as u8, b as u8]);
    }
}

/// The byte ranges whose code page is not Windows-1252: each run's code page is its (last) font's
/// charset's. A run that names no font uses font 0. Sorted by start and not overlapping.
fn code_page_runs(runs: &[Run], fonts: &[Font]) -> Vec<CodePageRun> {
    let mut out: Vec<CodePageRun> = runs
        .iter()
        .filter_map(|r| {
            let code_page = code_page_of(fonts, last_font(r.grpprl).unwrap_or(0));
            (r.lo < r.hi && code_page != 1252).then_some(CodePageRun { lo: r.lo, hi: r.hi, code_page })
        })
        .collect();
    out.sort_by_key(|r| r.lo);
    // A corrupt file may list overlapping runs: the first one wins.
    let mut kept: Vec<CodePageRun> = Vec::with_capacity(out.len());
    for r in out {
        if kept.last().is_none_or(|last| r.lo >= last.hi) {
            kept.push(r);
        }
    }
    kept
}

/// The font a CHPX or Prc grpprl names (sprm 0x5D), if any: the last one, as a later sprm
/// overrides an earlier one when the formatting is applied (a font before an opcode of unknown
/// length stands).
fn last_font(grpprl: &[u8]) -> Option<u16> {
    sprms(grpprl).filter(|(op, _)| *op == chp::FONT).filter_map(|(_, v)| Some(u16::from_le_bytes([*v.first()?, *v.get(1)?]))).last()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_page_named_by_many_bin_entries_is_read_once() {
        // A hostile bin table: a million entries all naming page 7, and one naming page 3.
        let n = 1_000_001usize;
        let mut plc = vec![0u8; 4 * (n + 1)];
        for i in 0..n {
            plc.extend_from_slice(&(if i == 5 { 3u16 } else { 7u16 }).to_le_bytes());
        }
        assert_eq!(super::distinct_pages(&plc, n), [3, 7]);
    }

    #[test]
    fn bin_entries_past_the_cap_are_not_read() {
        let n = super::MAX_BINS + 1;
        let mut plc = vec![0u8; 4 * (n + 1)];
        for i in 0..n {
            plc.extend_from_slice(&(if i == n - 1 { 9u16 } else { 7u16 }).to_le_bytes());
        }
        assert_eq!(super::distinct_pages(&plc, n), [7]);
    }
}

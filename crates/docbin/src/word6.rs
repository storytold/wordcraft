//! Word 6.0 and Word 95 `.doc` files (`wIdent` 0xA5DC, `nFib` 0x0065 and 0x0068).
//!
//! No public specification describes this version: [MS-DOC] covers Word 97 and later. Everything
//! here is observed, from the layout of real files checked against what Word shows when it opens
//! them, plus the Word 97 structures in [MS-DOC] that these files visibly share. Nothing else
//! is assumed.
//!
//! What is read so far: the main text, its paragraphs and fields, and the code page of each run.
//! Formatting, styles, sections, headers and footers, notes, tables and pictures are not read yet.
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
//!   character bin table, 15 the font table and 33 the piece table.
//! - Text is one byte per character, in the code page of the font of its run. The FIB's language
//!   does not choose it: files marked English hold Russian and Czech text that Word shows as such.
//! - The character bin table holds 2-byte page numbers (Word 97: 4 bytes). The FKP pages have the
//!   Word 97 layout, but their sprms have 1-byte opcodes. Opcode 0x5D, with a 2-byte operand, is
//!   the run's font.
//! - A font table entry is a size byte (the entry's length − 1), a flags byte, a 2-byte weight, the
//!   charset byte, one more byte, then the name in 8-bit characters.

use wordcraft_doc::encoding::charset_code_page;

use crate::DocbinError;
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
/// Code page runs kept: far more than any real file's, and few enough to stay small.
const MAX_RUNS: usize = 1_000_000;
const OFF_FC_MIN: usize = 0x18;
const OFF_CCP_TEXT: usize = 0x34;
const OFF_PAIRS: usize = 0x58;
/// Pairs read: up to and including the piece table's.
const PAIRS: usize = 34;
const PAIR_BTE_CHPX: usize = 12;
const PAIR_FONTS: usize = 15;
const PAIR_CLX: usize = 33;
/// More characters than Word's own documents get near: the FIB is corrupt (as for Word 97).
const MAX_CCP: u32 = 100_000_000;
/// The font table's entries: more than any real file has.
const MAX_FONTS: usize = 0x1_0000;
/// FKP page size in bytes.
const PAGE: usize = 512;

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

/// The text of the file: its piece table (or, for a file that was not fast-saved, the one
/// stretch of text after the FIB), with how its characters are stored and decoded.
///
/// 8-bit text decodes in the code page of, in order: the document's character set when it isn't
/// Windows ([`OFF_CHSE`]); else the font a piece's Prm names (as Word 95 writes when a fast save
/// changes a font); else the font of the character run; else Windows-1252.
pub(crate) fn pieces(word: &[u8], fib: &Fib6) -> Result<PieceTable, DocbinError> {
    let charsets = fib.pair(PAIR_FONTS).map(|(fc, lcb)| font_charsets(word, fc, lcb)).unwrap_or_default();
    let code_page_of = |font: u16| -> u16 { charsets.get(usize::from(font)).and_then(|&c| charset_code_page(i32::from(c))).unwrap_or(1252) };
    let windows = fib.chse == 0;
    let default_code_page = match fib.chse {
        0 => 1252,
        0x0100 => MAC_ROMAN,
        0x0002 => SYMBOL_CODE_PAGE,
        // Not a code page: bytes above ASCII read as U+FFFD.
        _ => 0,
    };
    let code_pages = match fib.pair(PAIR_BTE_CHPX) {
        Some((fc, lcb)) if windows => code_page_runs(word, fc, lcb, &charsets),
        _ => Vec::new(),
    };
    let text = Word6Text { wide: fib.wide, default_code_page, code_pages };
    if fib.complex {
        let (fc, lcb) = fib.pair(PAIR_CLX).ok_or_else(|| DocbinError::Malformed("the fast-saved file has no piece table".into()))?;
        // A Prm1's Prc may name the piece's font (sprm 0x5D); a Prm0 can't, its operand being one
        // byte.
        let piece_code_page = |prm: u16, prcs: &[Vec<u8>]| -> Option<u16> {
            if !windows || prm & 1 == 0 {
                return None;
            }
            font(prcs.get(usize::from(prm >> 1))?).map(code_page_of)
        };
        PieceTable::parse_word6(word, fc, lcb, text, piece_code_page)
    } else {
        // Word reads no more characters than the text's byte range (fcMin..fcMac) holds, one byte
        // each or two for UTF-16, whatever the character count says.
        let unit = if fib.wide { 2 } else { 1 };
        let ccp = fib.ccp_text.min(fib.fc_mac.saturating_sub(fib.fc_min) / unit);
        Ok(PieceTable::contiguous_word6(fib.fc_min, ccp, text))
    }
}

/// Mac OS Roman, the 8-bit text of a Macintosh Word document.
const MAC_ROMAN: u16 = wordcraft_doc::encoding::MAC_ROMAN_CODE_PAGE;

/// The charset of every font in the font table at `word[fc..fc+lcb]`, by font index. A truncated
/// entry ends the table.
fn font_charsets(word: &[u8], fc: u32, lcb: u32) -> Vec<u8> {
    let Some(table) = (fc as usize).checked_add(lcb as usize).and_then(|end| word.get(fc as usize..end)) else { return Vec::new() };
    // The table starts with its own size, which bounds the entries along with `lcb`.
    let size = table.get(0..2).map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])));
    let table = table.get(..size.min(table.len())).unwrap_or_default();
    let mut charsets = Vec::new();
    let mut at = 2usize;
    while let Some(&cb_m1) = table.get(at) {
        let next = at + usize::from(cb_m1) + 1;
        // The charset must lie inside this entry (an entry too short to hold it is corrupt) and
        // the entry inside the table.
        let Some(&charset) = table.get(at + 4).filter(|_| at + 4 < next && next <= table.len()) else { break };
        if charsets.len() >= MAX_FONTS {
            break;
        }
        charsets.push(charset);
        at = next;
    }
    charsets
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

/// The byte ranges whose code page is not Windows-1252, from the character bin table at
/// `word[fc..fc+lcb]`: each run's code page is its font's charset's. A run that names no font
/// uses font 0. Sorted by start and not overlapping; unreadable pages are skipped.
fn code_page_runs(word: &[u8], fc: u32, lcb: u32, charsets: &[u8]) -> Vec<CodePageRun> {
    let Some(plc) = (fc as usize).checked_add(lcb as usize).and_then(|end| word.get(fc as usize..end)) else { return Vec::new() };
    // (n+1) u32 FCs, then n u16 page numbers.
    let Some(n) = plc.len().checked_sub(4).filter(|r| r % 6 == 0).map(|r| r / 6) else { return Vec::new() };
    let code_page_of = |font: u16| -> u16 { charsets.get(usize::from(font)).and_then(|&c| charset_code_page(i32::from(c))).unwrap_or(1252) };
    let mut runs: Vec<CodePageRun> = Vec::new();
    for pn in distinct_pages(plc, n) {
        if runs.len() >= MAX_RUNS {
            break;
        }
        let Some(page) = pn.checked_mul(PAGE).and_then(|at| word.get(at..at.checked_add(PAGE)?)) else { continue };
        let crun = usize::from(page.get(PAGE - 1).copied().unwrap_or(0));
        // The FCs and the offsets must fit before the count byte.
        if 4 * (crun + 1) + crun >= PAGE {
            continue;
        }
        let fc_at = |k: usize| page.get(4 * k..4 * k + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        for k in 0..crun {
            let (Some(lo), Some(hi)) = (fc_at(k), fc_at(k + 1)) else { break };
            let off = usize::from(page.get(4 * (crun + 1) + k).copied().unwrap_or(0)) * 2;
            let grpprl = match off {
                0 => &[][..],
                _ => page.get(off).and_then(|&cb| page.get(off + 1..off + 1 + usize::from(cb))).unwrap_or_default(),
            };
            let code_page = code_page_of(font(grpprl).unwrap_or(0));
            if lo < hi && code_page != 1252 {
                runs.push(CodePageRun { lo, hi, code_page });
            }
        }
    }
    runs.sort_by_key(|r| r.lo);
    // A corrupt file may list overlapping runs: the first one wins.
    let mut out: Vec<CodePageRun> = Vec::with_capacity(runs.len());
    for r in runs {
        if out.last().is_none_or(|last| r.lo >= last.hi) {
            out.push(r);
        }
    }
    out
}

/// The font a CHPX grpprl names (sprm 0x5D), if any: the last one, as a later sprm overrides an
/// earlier one when the formatting is applied. Parsing stops at an opcode whose operand length
/// is not known, since the rest of the grpprl can't be framed; a font found before it stands.
fn font(grpprl: &[u8]) -> Option<u16> {
    let mut found = None;
    let mut i = 0usize;
    while let Some(&op) = grpprl.get(i) {
        let len = match op {
            // 0x55-0x5C are the character toggles of [MS-DOC]'s Prm0 isprm table (bold, italic,
            // strike, outline, shadow, small caps, caps, hidden), all one byte; 0x55, 0x56, 0x5A
            // and 0x5B are also seen in real files.
            0x55..=0x5C | 0x5E | 0x62 | 0x68 | 0x75 => 1,
            0x50 | 0x5D | 0x60 | 0x61 | 0x63 => 2,
            // Its operand starts with its own length.
            0x44 => match grpprl.get(i + 1) {
                Some(&n) => 1 + usize::from(n),
                None => break,
            },
            _ => break,
        };
        let Some(operand) = grpprl.get(i + 1..i + 1 + len) else { break };
        if op == 0x5D
            && let [a, b] = operand
        {
            found = Some(u16::from_le_bytes([*a, *b]));
        }
        i += 1 + len;
    }
    found
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

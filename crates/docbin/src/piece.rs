//! The piece table ([MS-DOC] `Clx`/`Pcdt`/`PlcPcd`): where every character position lives in
//! the WordDocument stream, and how it is encoded.
//!
//! Word 6.0/95 files ([`crate::word6`]) keep the same `Clx` shape, but every piece's text is at a
//! plain byte offset: 8-bit text in the code page of its piece's or run's font (or the document's),
//! or UTF-16 in a file with the FIB's fExtChar flag.

use wordcraft_doc::encoding::{code_page_byte, cp1252, is_byte_code_page};

use crate::DocbinError;

/// Most pieces any real file has; more means a corrupt `PlcPcd`.
const MAX_PIECES: usize = 1_000_000;

/// How a piece's characters are stored.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Text {
    /// UTF-16LE at `fc`.
    Utf16,
    /// Windows-1252, one byte per character at `fc / 2` (Word 97 FcCompressed).
    Compressed,
    /// One byte per character at `fc` itself, in the code page [`PieceTable::code_page_at`] gives
    /// that byte (Word 6.0/95).
    Bytes,
}

/// One piece: its text at `fc` (for Word 97, the 30-bit offset of FcCompressed).
#[derive(Clone, Copy, Debug)]
struct Pcd {
    fc: u32,
    text: Text,
    /// `Prm`: either a single sprm (Prm0) or an index into the Clx's Prc grpprls (Prm1). Always 0
    /// in Word 6.0/95 pieces, whose Prms use that version's sprm numbers.
    prm: u16,
    /// For a Word 6.0/95 [`Text::Bytes`] piece whose Prm names a font: that font's code page,
    /// which outranks the runs' ([`PieceTable::code_page_at`]).
    code_page: Option<u16>,
}

impl Pcd {
    /// The byte offset of the character `step / 2` characters into the piece. `step` is twice
    /// the CP offset, so one formula serves all three encodings.
    fn fc_at(&self, step: u32) -> Option<u32> {
        match self.text {
            Text::Utf16 => self.fc.checked_add(step),
            Text::Compressed => self.fc.checked_add(step)?.checked_div(2),
            Text::Bytes => self.fc.checked_add(step / 2),
        }
    }
}

/// The code page of the 8-bit text in the byte range `[lo, hi)` of the WordDocument stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CodePageRun {
    pub(crate) lo: u32,
    pub(crate) hi: u32,
    pub(crate) code_page: u16,
}

/// How a Word 6.0/95 file's text is stored and decoded ([`crate::word6`]).
pub(crate) struct Word6Text {
    /// UTF-16 rather than 8-bit (the FIB's fExtChar flag).
    pub(crate) wide: bool,
    /// The code page of 8-bit text that no piece or run gives one for.
    pub(crate) default_code_page: u16,
    /// The code page of each byte range, from its run's font (sorted by `lo`, not overlapping).
    pub(crate) code_pages: Vec<CodePageRun>,
}

/// The parsed piece table plus the grpprls of the Prcs that Prm1 references.
pub(crate) struct PieceTable {
    /// n+1 character-position starts.
    cps: Vec<u32>,
    pieces: Vec<Pcd>,
    prc_grpprls: Vec<Vec<u8>>,
    /// For [`Text::Bytes`] pieces: the code page of each byte range, sorted by `lo`, not
    /// overlapping. Bytes outside every range are in `default_code_page`.
    code_pages: Vec<CodePageRun>,
    default_code_page: u16,
}

impl PieceTable {
    /// Parse the `Clx` at `table[fc .. fc+lcb]`.
    pub(crate) fn parse(table: &[u8], fc: u32, lcb: u32) -> Result<PieceTable, DocbinError> {
        Self::parse_clx(table, fc, lcb, None)
    }

    /// Parse a Word 6.0/95 `Clx` at `word[fc .. fc+lcb]`: the Word 97 shape, but every piece's text
    /// is at its plain byte offset, and its Prm is not applied as Word 97 sprms.
    /// `piece_code_page` gives the code page a piece's raw Prm imposes (with the Clx's Prc
    /// grpprls to look Prm1 indices up in), if any.
    pub(crate) fn parse_word6(
        word: &[u8],
        fc: u32,
        lcb: u32,
        text: Word6Text,
        piece_code_page: impl Fn(u16, &[Vec<u8>]) -> Option<u16>,
    ) -> Result<PieceTable, DocbinError> {
        let mut t = Self::parse_clx(word, fc, lcb, Some(text.wide))?;
        let prcs = std::mem::take(&mut t.prc_grpprls);
        for p in &mut t.pieces {
            // The raw Prm was kept for this; it is not applied as a Word 97 one.
            p.code_page = if text.wide { None } else { piece_code_page(p.prm, &prcs) };
            p.prm = 0;
        }
        t.code_pages = text.code_pages;
        t.default_code_page = text.default_code_page;
        Ok(t)
    }

    /// One Word 6.0/95 piece: `ccp` characters from byte `fc` on. This covers a file that was not
    /// fast-saved, which has no `Clx`.
    pub(crate) fn contiguous_word6(fc: u32, ccp: u32, text: Word6Text) -> PieceTable {
        let kind = if text.wide { Text::Utf16 } else { Text::Bytes };
        PieceTable {
            cps: vec![0, ccp],
            pieces: vec![Pcd { fc, text: kind, prm: 0, code_page: None }],
            prc_grpprls: Vec::new(),
            code_pages: text.code_pages,
            default_code_page: text.default_code_page,
        }
    }

    /// `word6`: `None` for Word 97 and later, `Some(wide)` for a Word 6.0/95 file (whose raw Prms
    /// and Prcs are kept for [`PieceTable::parse_word6`]).
    fn parse_clx(table: &[u8], fc: u32, lcb: u32, word6: Option<bool>) -> Result<PieceTable, DocbinError> {
        let fc = fc as usize;
        let lcb = lcb as usize;
        if lcb == 0 {
            return Err(DocbinError::Malformed("empty Clx".into()));
        }
        let end = fc.checked_add(lcb).ok_or_else(|| DocbinError::Malformed("Clx offset overflow".into()))?;
        let clx = table.get(fc..end).ok_or_else(|| DocbinError::Malformed(format!("Clx at {fc}+{lcb} is outside the Table stream")))?;
        // A Clx is an array of Prc (clxt 0x01, each holding a grpprl that Prm1 values can
        // reference) followed by the Pcdt (clxt 0x02) that holds the piece table.
        let mut prc_grpprls = Vec::new();
        let mut i = 0usize;
        let plc = loop {
            let Some(&clxt) = clx.get(i) else {
                return Err(DocbinError::Malformed("Clx ends before the Pcdt".into()));
            };
            match clxt {
                0x01 => {
                    let cb = clx
                        .get(i + 1..i + 3)
                        .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
                        .ok_or_else(|| DocbinError::Malformed("truncated Prc".into()))?;
                    let stop = i.checked_add(3).and_then(|v| v.checked_add(cb)).ok_or_else(|| DocbinError::Malformed("Prc size overflow".into()))?;
                    if let Some(g) = clx.get(i + 3..stop)
                        && prc_grpprls.len() < MAX_PIECES
                    {
                        prc_grpprls.push(g.to_vec());
                    }
                    i = stop;
                }
                0x02 => {
                    let lcb_plc = clx
                        .get(i + 1..i + 5)
                        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                        .ok_or_else(|| DocbinError::Malformed("truncated Pcdt".into()))?;
                    let start = i.checked_add(5).ok_or_else(|| DocbinError::Malformed("Pcdt offset overflow".into()))?;
                    let stop = start.checked_add(lcb_plc).ok_or_else(|| DocbinError::Malformed("Pcdt size overflow".into()))?;
                    break clx.get(start..stop).ok_or_else(|| DocbinError::Malformed("PlcPcd is outside the Clx".into()))?;
                }
                other => return Err(DocbinError::Malformed(format!("unexpected Clx item 0x{other:02X}"))),
            }
        };
        // PlcPcd: aCp[n+1] u32s followed by aPcd[n] Pcds (8 bytes each).
        if plc.len() < 4 || (plc.len() - 4) % 12 != 0 {
            return Err(DocbinError::Malformed(format!("PlcPcd is {} bytes", plc.len())));
        }
        let n = (plc.len() - 4) / 12;
        if n > MAX_PIECES {
            return Err(DocbinError::Limit(format!("piece table has {n} pieces")));
        }
        let mut cps = Vec::with_capacity(n + 1);
        for j in 0..=n {
            let at = j * 4;
            let Some(b) = plc.get(at..at + 4) else {
                return Err(DocbinError::Malformed("truncated aCp".into()));
            };
            // Keep the CPs non-decreasing so binary search over them is sound: an inverted
            // piece in a corrupt file becomes an empty one.
            let c = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            cps.push(c.max(cps.last().copied().unwrap_or(0)));
        }
        let mut pieces = Vec::with_capacity(n);
        for j in 0..n {
            let at = (n + 1) * 4 + j * 8 + 2;
            let Some(fc_raw) = plc.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) else {
                return Err(DocbinError::Malformed("truncated aPcd".into()));
            };
            let prm = plc.get(at + 4..at + 6).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
            pieces.push(match word6 {
                // A plain byte offset, with no compression bit: 8-bit text, or UTF-16 in a file with
                // fExtChar. The raw Prm is kept for `parse_word6`.
                Some(wide) => Pcd { fc: fc_raw, text: if wide { Text::Utf16 } else { Text::Bytes }, prm, code_page: None },
                None => {
                    let text = if fc_raw & 0x4000_0000 != 0 { Text::Compressed } else { Text::Utf16 };
                    Pcd { fc: fc_raw & 0x3FFF_FFFF, text, prm, code_page: None }
                }
            });
        }
        Ok(PieceTable { cps, pieces, prc_grpprls, code_pages: Vec::new(), default_code_page: 1252 })
    }

    /// The code page of the 8-bit text byte at `fc` (in [`Text::Bytes`] pieces).
    fn code_page_at(&self, fc: u32) -> u16 {
        let i = self.code_pages.partition_point(|r| r.lo <= fc);
        match i.checked_sub(1).and_then(|i| self.code_pages.get(i)) {
            Some(r) if fc < r.hi => r.code_page,
            _ => self.default_code_page,
        }
    }

    /// The Prls this piece's `Prm` adds. Bit 0 (`fComplex`) discriminates: a Prm0
    /// ([MS-DOC] §2.9.215) holds a 7-bit `isprm` in bits 1-7 and its one-byte operand in
    /// bits 8-15; a Prm1 (§2.9.216) holds a zero-based index into the Clx's Prcs in bits
    /// 1-15.
    pub(crate) fn piece_prm(&self, cp: u32) -> PrmRef<'_> {
        let Some(p) = self.piece_of(cp) else { return PrmRef::None };
        if p.prm & 1 == 0 {
            let isprm = ((p.prm >> 1) & 0x7F) as u8;
            if isprm == 0 {
                return PrmRef::None;
            }
            PrmRef::Sprm0 { isprm, val: (p.prm >> 8) as u8 }
        } else {
            PrmRef::Grpprl(self.prc_grpprls.get((p.prm >> 1) as usize).map(Vec::as_slice))
        }
    }

    fn piece_of(&self, cp: u32) -> Option<&Pcd> {
        let i = self.cps.partition_point(|&c| c <= cp).checked_sub(1)?;
        if i >= self.pieces.len() {
            return None;
        }
        self.pieces.get(i)
    }

    /// The FC of a character position in the space the FKP bin tables use: true byte
    /// offsets into the WordDocument stream. A compressed piece stores its base `fc`
    /// doubled (`byte = fc/2`), so the doubling unwinds here; an uncompressed piece's `fc`
    /// already is its byte offset and each character covers two bytes.
    pub(crate) fn fc_of_cp(&self, cp: u32) -> Option<u32> {
        let p = self.piece_of(cp)?;
        let i = self.cps.partition_point(|&c| c <= cp).checked_sub(1)?;
        let cp0 = *self.cps.get(i)?;
        p.fc_at(cp.checked_sub(cp0)?.checked_mul(2)?)
    }

    /// The piece holding `cp`, with the CP range it covers: for every CP in that range the
    /// answers of [`PieceTable::fc_of_cp`] and the piece's Prm come from this span alone,
    /// so a sequential walk searches the piece table once per piece.
    pub(crate) fn span(&self, cp: u32) -> Option<PieceSpan<'_>> {
        let i = self.cps.partition_point(|&c| c <= cp).checked_sub(1)?;
        let pcd = self.pieces.get(i)?;
        let cp_lo = *self.cps.get(i)?;
        let cp_hi = *self.cps.get(i + 1)?;
        Some(PieceSpan { cp_lo, cp_hi, pcd })
    }

    /// The CP just past the last piece: no text exists at or beyond it.
    pub(crate) fn cp_end(&self) -> u32 {
        self.cps.last().copied().unwrap_or(0)
    }

    /// The character stored at `cp` (control characters included) and how many CPs it
    /// covers (2 for a surrogate pair), found by binary search over the pieces.
    /// `Err(next)` when there is no readable character at `cp` (a gap between pieces, an
    /// inverted piece, or piece bytes past the end of the stream): `next` is the CP to
    /// resume at, always greater than `cp` (`u32::MAX` past the last piece), so callers
    /// skip an unreadable range in one step instead of one CP at a time.
    pub(crate) fn char_at(&self, word: &[u8], cp: u32) -> Result<(char, u32), u32> {
        let i = self.cps.partition_point(|&c| c <= cp);
        // The next piece boundary above `cp`, or the end of the CP space.
        let next = self.cps.get(i).copied().filter(|&n| n > cp).unwrap_or(u32::MAX);
        let Some(i) = i.checked_sub(1) else { return Err(next) };
        let (Some(p), Some(&cp0)) = (self.pieces.get(i), self.cps.get(i)) else { return Err(u32::MAX) };
        let Some(off) = cp.checked_sub(cp0) else { return Err(next) };
        let off = off as usize;
        match p.text {
            Text::Compressed => {
                // 8-bit, Windows-1252, one byte per character, stored at fc/2.
                let at = ((p.fc / 2) as usize).checked_add(off);
                return match at.and_then(|a| word.get(a)) {
                    Some(&b) => Ok((cp1252(b), 1)),
                    None => Err(next),
                };
            }
            Text::Bytes => {
                // 8-bit, one byte per character at fc, in its run's code page.
                let at = (p.fc as usize).checked_add(off);
                return match at.and_then(|a| Some((u32::try_from(a).ok()?, *word.get(a)?))) {
                    Some((a, b)) => Ok((byte_char(p.code_page.unwrap_or_else(|| self.code_page_at(a)), b), 1)),
                    None => Err(next),
                };
            }
            Text::Utf16 => {}
        }
        // UTF-16LE, one 16-bit unit per character, stored at fc.
        let unit = |k: usize| -> Option<u16> {
            let at = (p.fc as usize).checked_add(k.checked_mul(2)?)?;
            word.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]))
        };
        let Some(u) = unit(off) else { return Err(next) };
        if (0xD800..0xDC00).contains(&u)
            && cp.saturating_add(1) < next
            && let Some(lo) = unit(off.saturating_add(1)).filter(|lo| (0xDC00..0xE000).contains(lo))
            && let Some(Ok(c)) = char::decode_utf16([u, lo]).next()
        {
            return Ok((c, 2));
        }
        Ok((char::from_u32(u as u32).unwrap_or('\u{FFFD}'), 1))
    }
}

/// One piece and the CPs `[cp_lo, cp_hi)` it covers ([`PieceTable::span`]).
#[derive(Clone, Copy)]
pub(crate) struct PieceSpan<'a> {
    cp_lo: u32,
    cp_hi: u32,
    pcd: &'a Pcd,
}

impl PieceSpan<'_> {
    pub(crate) fn contains(&self, cp: u32) -> bool {
        self.cp_lo <= cp && cp < self.cp_hi
    }

    /// [`PieceTable::fc_of_cp`] for a CP inside the span.
    pub(crate) fn fc_at(&self, cp: u32) -> Option<u32> {
        self.pcd.fc_at(cp.checked_sub(self.cp_lo)?.checked_mul(2)?)
    }

    /// The raw Prm of the piece: two CPs with the same value get the same [`PrmRef`].
    pub(crate) fn prm(&self) -> u16 {
        self.pcd.prm
    }
}

/// What a piece's `Prm` refers to.
pub(crate) enum PrmRef<'a> {
    None,
    /// Prm0: a short sprm index into the isprm table, with a 1-byte operand.
    Sprm0 {
        isprm: u8,
        val: u8,
    },
    /// Prm1: a grpprl from a Prc (absent if out of range).
    Grpprl(Option<&'a [u8]>),
}

/// Windows' code page number for symbol fonts (CP_SYMBOL).
pub(crate) const SYMBOL_CODE_PAGE: u16 = 42;

/// One byte of 8-bit text in `code_page`. The East Asian code pages store a character as a lead
/// and a trail byte, which one CP per byte can't represent, so their bytes above ASCII read as
/// U+FFFD rather than as unrelated Windows-1252 letters.
fn byte_char(code_page: u16, b: u8) -> char {
    if code_page == SYMBOL_CODE_PAGE {
        // Each character from the space up to its private-use code point, as Word reads a document
        // marked as Symbol; control characters (paragraph marks, fields) stay themselves.
        if b < 0x20 { char::from(b) } else { char::from_u32(0xF000 + u32::from(b)).unwrap_or('\u{FFFD}') }
    } else if is_byte_code_page(code_page) {
        code_page_byte(code_page, b)
    } else if b.is_ascii() {
        char::from(b)
    } else {
        '\u{FFFD}'
    }
}

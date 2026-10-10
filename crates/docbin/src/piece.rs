//! The piece table ([MS-DOC] `Clx`/`Pcdt`/`PlcPcd`): where every character position lives in
//! the WordDocument stream, and how it is encoded.

use wordcraft_doc::encoding::cp1252;

use crate::DocbinError;

/// Most pieces any real file has; more means a corrupt `PlcPcd`.
const MAX_PIECES: usize = 1_000_000;

/// One piece: text at FcCompressed `fc` (30-bit offset + compression bit).
#[derive(Clone, Copy, Debug)]
struct Pcd {
    fc: u32,
    compressed: bool,
    /// `Prm`: either a single sprm (Prm0) or an index into the Clx's Prc grpprls (Prm1).
    prm: u16,
}

/// The parsed piece table plus the grpprls of the Prcs that Prm1 references.
pub(crate) struct PieceTable {
    /// n+1 character-position starts.
    cps: Vec<u32>,
    pieces: Vec<Pcd>,
    prc_grpprls: Vec<Vec<u8>>,
}

impl PieceTable {
    /// Parse the `Clx` at `table[fc .. fc+lcb]`.
    pub(crate) fn parse(table: &[u8], fc: u32, lcb: u32) -> Result<PieceTable, DocbinError> {
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
            cps.push(u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        }
        let mut pieces = Vec::with_capacity(n);
        for j in 0..n {
            let at = (n + 1) * 4 + j * 8 + 2;
            let Some(fc_raw) = plc.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) else {
                return Err(DocbinError::Malformed("truncated aPcd".into()));
            };
            let prm = plc.get(at + 4..at + 6).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
            pieces.push(Pcd { fc: fc_raw & 0x3FFF_FFFF, compressed: fc_raw & 0x4000_0000 != 0, prm });
        }
        Ok(PieceTable { cps, pieces, prc_grpprls })
    }

    /// The Prls this piece's `Prm` adds: Prm0 holds one short sprm (isprm, val), Prm1
    /// references a grpprl from one of the Clx's Prcs ([MS-DOC] Prm).
    pub(crate) fn piece_prm(&self, cp: u32) -> PrmRef<'_> {
        match self.piece_of(cp) {
            Some(p) if p.prm & 0x8000 == 0 && (p.prm & 0x7F) != 0 => PrmRef::Sprm0 { isprm: (p.prm & 0x7F) as u8, val: (p.prm >> 8) as u8 },
            Some(p) if p.prm & 0x8000 != 0 => PrmRef::Grpprl(self.prc_grpprls.get((p.prm & 0x7FFF) as usize).map(|v| v.as_slice())),
            _ => PrmRef::None,
        }
    }

    fn piece_of(&self, cp: u32) -> Option<&Pcd> {
        let i = self.cps.partition_point(|&c| c <= cp).checked_sub(1)?;
        if i >= self.pieces.len() {
            return None;
        }
        self.pieces.get(i)
    }

    /// The FC of a character position: within a piece, consecutive characters are 2 FC units
    /// apart in both encodings (compressed text stores `fc = 2 × byte offset`).
    pub(crate) fn fc_of_cp(&self, cp: u32) -> Option<u32> {
        let p = self.piece_of(cp)?;
        let i = self.cps.partition_point(|&c| c <= cp).checked_sub(1)?;
        let cp0 = *self.cps.get(i)?;
        p.fc.checked_add((cp - cp0).checked_mul(2)? & 0x3FFF_FFFF)
    }

    /// Text of the CP range `[start, end)` as it is stored (control characters included);
    /// pieces with inverted or out-of-range CPs are skipped, truncated pieces read short.
    pub(crate) fn text(&self, word: &[u8], start: u32, end: u32) -> String {
        if end <= start {
            return String::new();
        }
        let mut out = String::new();
        for (i, p) in self.pieces.iter().enumerate() {
            let (Some(&cp0), Some(&cp1)) = (self.cps.get(i), self.cps.get(i + 1)) else { break };
            if cp1 <= cp0 || cp1 <= start || cp0 >= end {
                continue;
            }
            let from = start.max(cp0) - cp0;
            let to = end.min(cp1) - cp0;
            if p.compressed {
                // 8-bit, Windows-1252, one byte per character, stored at fc/2.
                let base = (p.fc / 2) as usize;
                let bytes = base.checked_add(from as usize).and_then(|s| word.get(s..base + to as usize));
                if let Some(bytes) = bytes {
                    out.extend(bytes.iter().map(|&b| cp1252(b)));
                }
            } else {
                // UTF-16LE, one 16-bit unit per character, stored at fc.
                let base = p.fc as usize;
                let unit = base.checked_add(from as usize * 2).and_then(|s| word.get(s..base + to as usize * 2));
                if let Some(unit) = unit {
                    let units: Vec<u16> = unit.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
            }
            if cp1 >= end {
                break;
            }
        }
        out
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

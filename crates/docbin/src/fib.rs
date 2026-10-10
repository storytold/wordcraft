//! The File Information Block ([MS-DOC] §2.5): `FibBase` plus the variable sections
//! (`FibRgW97`, `FibRgLw97`, the `RgFcLcb` blob, `FibRgCswNew`) at the start of the
//! WordDocument stream.

use crate::DocbinError;

/// `wIdent` of every Word 97-2003 file (Word 6/95 wrote 0xA5DC).
const W_IDENT: u16 = 0xA5EC;
/// Offsets within the FIB, in bytes (see the spec's Fib layout).
const OFF_FLAGS: usize = 0x0A;
const OFF_CSW: usize = 0x20;
const OFF_CSLW: usize = 0x3E;
const OFF_RGLW: usize = 0x40;
const OFF_CB_RG_FCLCB: usize = 0x98;
/// `csw` (count of 16-bit values in `FibRgW97`) — fixed by the spec.
const CSW: u16 = 0x000E;
/// `cslw` (count of 32-bit values in `FibRgLw97`) — fixed by the spec.
const CSLW: u16 = 0x0016;
/// `cbRgFcLcb` for each known `nFib` ([MS-DOC] Fib.cbRgFcLcb table).
const CB_RG_FCLCB: &[(u16, u16)] = &[(0x00C1, 0x005D), (0x00D9, 0x006C), (0x0101, 0x0088), (0x010C, 0x00A4), (0x0112, 0x00B7)];

/// Indices of `fc`/`lcb` pairs inside `FibRgFcLcb97` (each pair is 8 bytes: fc then lcb).
pub(crate) mod pair {
    pub(crate) const STSHF: usize = 1;
    pub(crate) const PLCF_SED: usize = 6;
    pub(crate) const PLCF_HDD: usize = 11;
    pub(crate) const PLCF_BTE_CHPX: usize = 12;
    pub(crate) const PLCF_BTE_PAPX: usize = 13;
    pub(crate) const STTBF_FFN: usize = 15;
    pub(crate) const CLX: usize = 33;
    /// `fcPlfLst`: the list definitions (LSTF array, with the LVLs appended after the blob).
    pub(crate) const PLF_LST: usize = 73;
    /// `fcPlfLfo`: the list format overrides (LFO array with LFOData appended).
    pub(crate) const PLF_LFO: usize = 74;
}

/// Character-position ranges of the subdocuments, from `FibRgLw97`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SubDocs {
    pub(crate) text: u32,
    pub(crate) ftn: u32,
    pub(crate) hdd: u32,
    pub(crate) atn: u32,
    pub(crate) edn: u32,
    pub(crate) txbx: u32,
    pub(crate) hdr_txbx: u32,
}

impl SubDocs {
    /// Total CPs across all subdocuments, checked against a cap.
    pub(crate) fn total(&self) -> Result<u64, DocbinError> {
        let t = u64::from(self.text)
            + u64::from(self.ftn)
            + u64::from(self.hdd)
            + u64::from(self.atn)
            + u64::from(self.edn)
            + u64::from(self.txbx)
            + u64::from(self.hdr_txbx);
        // Word's own documents stay far below this; a larger total means a corrupt FIB
        // (the ccp fields are signed and "MUST be 0, 1, or greater").
        if t > 100_000_000 {
            return Err(DocbinError::Limit(format!("document claims {t} characters")));
        }
        Ok(t)
    }
}

/// The parsed FIB.
#[derive(Clone, Debug)]
pub(crate) struct Fib {
    pub(crate) encrypted: bool,
    /// Selects 1Table (true) or 0Table (false).
    pub(crate) which_tbl_stm: bool,
    pub(crate) ccp: SubDocs,
    /// The `RgFcLcb` pairs flattened as fc₀, lcb₀, fc₁, lcb₁, …
    pub(crate) pairs: Vec<u32>,
}

impl Fib {
    /// The (fc, lcb) pair at `FibRgFcLcb97` index `i` (later blobs follow contiguously).
    pub(crate) fn pair(&self, i: usize) -> Option<(u32, u32)> {
        let fc = *self.pairs.get(i * 2)?;
        let lcb = *self.pairs.get(i * 2 + 1)?;
        Some((fc, lcb))
    }

    /// Parse the FIB from the head of the WordDocument stream.
    pub(crate) fn parse(word: &[u8]) -> Result<Fib, DocbinError> {
        let u16le = |at: usize| -> u16 { u16::from_le_bytes([word.get(at).copied().unwrap_or(0), word.get(at + 1).copied().unwrap_or(0)]) };
        let u32le = |at: usize| -> u32 {
            u32::from_le_bytes([
                word.get(at).copied().unwrap_or(0),
                word.get(at + 1).copied().unwrap_or(0),
                word.get(at + 2).copied().unwrap_or(0),
                word.get(at + 3).copied().unwrap_or(0),
            ])
        };
        if word.len() < OFF_RGLW + 88 {
            return Err(DocbinError::NotWord(format!("WordDocument stream is {} bytes", word.len())));
        }
        let w_ident = u16le(0);
        if w_ident == 0xA5DC {
            return Err(DocbinError::NotWord("Word 6.0/95 format, only Word 97-2003 is supported".into()));
        }
        if w_ident != W_IDENT {
            return Err(DocbinError::NotWord(format!("magic 0x{w_ident:04X}, expected 0x{W_IDENT:04X}")));
        }
        let flags = u16le(OFF_FLAGS);
        let nfib = u16le(2);
        if u16le(OFF_CSW) != CSW {
            return Err(DocbinError::NotWord(format!("csw is 0x{:04X}", u16le(OFF_CSW))));
        }
        if u16le(OFF_CSLW) != CSLW {
            return Err(DocbinError::NotWord(format!("cslw is 0x{:04X}", u16le(OFF_CSLW))));
        }
        // rglw u32 indices per FibRgLw97: cbMac, r1, r2, ccpText, ccpFtn, ccpHdd, r3, ccpAtn,
        // ccpEdn, ccpTxbx, ccpHdrTxbx, …
        let lw = |i: usize| -> u32 { u32le(OFF_RGLW + i * 4) };
        // The ccp fields are signed in the spec; negative values are corrupt input, clamped.
        let ccp = |i: usize| -> u32 { (lw(i) as i32).max(0) as u32 };
        let sub = SubDocs { text: ccp(3), ftn: ccp(4), hdd: ccp(5), atn: ccp(7), edn: ccp(8), txbx: ccp(9), hdr_txbx: ccp(10) };
        sub.total()?;

        let cb_rg_fclcb = u16le(OFF_CB_RG_FCLCB) as usize;
        if cb_rg_fclcb == 0 {
            return Err(DocbinError::NotWord("empty RgFcLcb".into()));
        }
        if let Some(&(_, expected)) = CB_RG_FCLCB.iter().find(|(n, _)| *n == nfib)
            && cb_rg_fclcb < expected as usize
        {
            // Word itself always writes the full blob for its version; anything smaller is
            // corrupt, but a *larger* blob (newer writer) is fine — we just read what's there.
            return Err(DocbinError::NotWord(format!("cbRgFcLcb is 0x{cb_rg_fclcb:04X} for nFib 0x{nfib:04X}")));
        }
        let blob_off = OFF_CB_RG_FCLCB + 2;
        let blob_len = cb_rg_fclcb.saturating_mul(8);
        if word.len() < blob_off + blob_len {
            return Err(DocbinError::NotWord("truncated RgFcLcb".into()));
        }
        let mut pairs = Vec::with_capacity(cb_rg_fclcb * 2);
        for i in 0..cb_rg_fclcb * 2 {
            pairs.push(u32le(blob_off + i * 4));
        }

        Ok(Fib { encrypted: flags & 0x0100 != 0, which_tbl_stm: flags & 0x0200 != 0, ccp: sub, pairs })
    }
}

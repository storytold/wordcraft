//! The stylesheet (`STSH`) and the font table (`SttbfFfn`) from the Table stream
//! ([MS-DOC] §2.9.271 and §2.9.247).

use crate::DocbinError;

/// Largest stylesheet we accept (styles are 0x0FFE max by the spec; stay stricter).
const MAX_STYLES: usize = 4096;

/// One raw style record (`STD`).
#[derive(Clone, Debug, Default)]
pub(crate) struct RawStyle {
    /// 1 paragraph, 2 character, 3 table, 4 numbering.
    pub(crate) stk: u8,
    /// Zero-based index of the parent style; 0x0FFF = none.
    pub(crate) istd_base: u16,
    pub(crate) name: String,
    /// Paragraph-formatting UPX grpprl (paragraph styles).
    pub(crate) papx: Vec<u8>,
    /// Character-formatting UPX grpprl (paragraph and character styles).
    pub(crate) chpx: Vec<u8>,
}

/// Parse the `STSH` at `table[fc..fc+lcb]`.
pub(crate) fn parse(table: &[u8], fc: u32, lcb: u32) -> Result<Vec<RawStyle>, DocbinError> {
    let fc = fc as usize;
    let lcb = lcb as usize;
    let end = fc.checked_add(lcb).ok_or_else(|| DocbinError::Malformed("STSH overflow".into()))?;
    let stsh = table.get(fc..end).ok_or_else(|| DocbinError::Malformed("STSH outside the Table stream".into()))?;
    if stsh.len() < 4 {
        return Ok(Vec::new());
    }
    // LPStshi: cbStshi u16 + Stshi; the style count and the size of each Stdf are what we need.
    let cb_stshi = u16::from_le_bytes([stsh[0], stsh[1]]) as usize;
    let stshi = stsh.get(2..2 + cb_stshi).unwrap_or(&[]);
    if stshi.len() < 4 {
        return Ok(Vec::new());
    }
    let cstd = u16::from_le_bytes([stshi[0], stshi[1]]) as usize;
    let cb_std_base = u16::from_le_bytes([stshi[2], stshi[3]]) as usize;
    // 10 (Word 97) or 18 (with StdfPost2000); anything else is corrupt but tolerated at 10.
    let cb_std_base = if cb_std_base == 0x12 { 18 } else { 10 };
    let cstd = cstd.min(MAX_STYLES);
    let mut rest = stsh.get(2 + cb_stshi..).unwrap_or(&[]);
    let mut out = Vec::with_capacity(cstd);
    for _ in 0..cstd {
        if rest.len() < 2 || out.len() >= MAX_STYLES {
            break;
        }
        let cb_std = u16::from_le_bytes([rest[0], rest[1]]) as usize;
        let (std, next) = rest.split_at(rest.len().min(2 + cb_std));
        rest = next;
        if cb_std == 0 {
            out.push(RawStyle::default()); // empty style: keeps istd numbering aligned
            continue;
        }
        let Some(stdf) = std.get(2..2 + cb_std_base.min(cb_std.saturating_sub(2))) else {
            out.push(RawStyle::default());
            continue;
        };
        // StdfBase ([MS-DOC] §2.9.260, 10 bytes): sti(12)+flags(4); stk(4)+istdBase(12);
        // cupx(4)+istdNext(12); bchUpe(16) (= cbStd); grfstd(16). istdBase is a zero-based
        // index into the stylesheet, 0x0FFF meaning "no parent".
        let stk = (stdf.get(2).copied().unwrap_or(1) & 0x0F).max(1);
        let w1 = stdf.get(2..4).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0xFFF1);
        let istd_base = w1 >> 4;
        let w2 = stdf.get(4..6).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
        let cupx = (w2 & 0x0F) as usize;
        let _istd_next = w2 >> 4; // next-paragraph style; unused for now
        let mut at = 2 + cb_std_base;
        // xstzName: Xst (cch u16 + UTF-16LE) + terminator u16.
        let mut name = String::new();
        if let Some(cch) = std.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).map(|c| (c as usize).min(256)) {
            if let Some(units) = std.get(at + 2..at + 2 + cch * 2) {
                let chars: Vec<u16> = units.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                name = String::from_utf16_lossy(&chars);
            }
            at += 2 + cch * 2 + 2;
        }
        // grLPUpxSw: cupx entries, each cbUpx u16 + data (+ pad to even).
        let mut upxs: Vec<Vec<u8>> = Vec::new();
        for _ in 0..cupx.min(3) {
            let Some(cb) = std.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).map(|v| v as usize) else { break };
            let data = std.get(at + 2..at + 2 + cb).unwrap_or(&[]).to_vec();
            upxs.push(data);
            at += 2 + cb + (cb & 1); // padded to even; cb in the header excludes the pad
        }
        // Paragraph styles: UPX0 is a PAPX (istd u16 + grpprl), UPX1 a raw grpprl of
        // character sprms; character styles: UPX0 is the character grpprl.
        let mut papx = Vec::new();
        let mut chpx = Vec::new();
        if stk == 1 {
            if let Some(u) = upxs.first() {
                papx = u.get(2..).unwrap_or(&[]).to_vec(); // skip the istd of GrpPrlAndIstd
            }
            if let Some(u) = upxs.get(1) {
                chpx = u.clone();
            }
        } else if stk == 2
            && let Some(u) = upxs.first()
        {
            chpx = u.clone();
        }
        out.push(RawStyle { stk, istd_base, name, papx, chpx });
    }
    Ok(out)
}

/// Font names from the `SttbfFfn` at `table[fc..fc+lcb]` ([MS-DOC] §2.9.286): an STTB
/// (§2.2.4) whose entries are FFN records (§2.9.82). Each entry is a length (`cchData`: a
/// one-byte byte count in the non-extended table the spec requires, a two-byte count of
/// 16-bit units in an extended one) followed by the FFN, and that length alone decides where
/// the next entry starts: an FFN ends with its primary name and an optional alternate name
/// (`xszAlt`), so scanning for the primary name's terminator would land inside the latter.
pub(crate) fn fonts(table: &[u8], fc: u32, lcb: u32) -> Vec<String> {
    let end = match (fc as usize).checked_add(lcb as usize) {
        Some(e) if e <= table.len() => e,
        _ => return Vec::new(),
    };
    let Some(sttb) = table.get(fc as usize..end) else { return Vec::new() };
    let u16at = |o: usize| -> Option<u16> { sttb.get(o..o.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]])) };
    let Some(first) = u16at(0) else { return Vec::new() };
    // FFN: ffid(1) wWeight(2) chs(1) ixchSzAlt(1) panose(10) fs(24) = 39 bytes, then the
    // primary name, NUL-terminated, as UTF-16LE in the Word 97 files we accept (8-bit names
    // from older writers fall back to Windows-1252).
    let name_of = |ffn: &[u8]| -> String {
        let tail = ffn.get(FFN_HEADER.min(ffn.len())..).unwrap_or(&[]);
        let units: Vec<u16> = tail.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).take(64).collect();
        let s = String::from_utf16_lossy(&units);
        if s.chars().any(|c| c as u32 > 0x7F || c.is_ascii_graphic() || c == ' ') || s.is_empty() {
            s
        } else {
            // An 8-bit name reads as garbage through UTF-16; fall back to Windows-1252.
            tail.iter().take_while(|&&b| b != 0).take(64).map(|&b| wordcraft_doc::encoding::cp1252(b)).collect()
        }
    };
    // STTB header: [fExtend 0xFFFF] cData(2) cbExtra(2); each entry is followed by cbExtra
    // bytes (0 for font tables, but skip them if present).
    let (extended, cdata, cb_extra, mut at) =
        if first == 0xFFFF { (true, u16at(2).unwrap_or(0), u16at(4).unwrap_or(0), 6usize) } else { (false, first, u16at(2).unwrap_or(0), 4usize) };
    let mut out = Vec::new();
    for _ in 0..cdata.min(1024) {
        let (len_bytes, cb) = if extended {
            let Some(cb) = u16at(at) else { break };
            (2usize, usize::from(cb).saturating_mul(2))
        } else {
            let Some(&cb) = sttb.get(at) else { break };
            (1usize, usize::from(cb))
        };
        let start = at.saturating_add(len_bytes);
        let stop = start.saturating_add(cb);
        // A record cut short by the end of the table still yields what it holds.
        out.push(name_of(sttb.get(start..stop.min(sttb.len())).unwrap_or(&[])));
        at = stop.saturating_add(usize::from(cb_extra));
    }
    out
}

/// Bytes of an FFN before its primary name ([MS-DOC] §2.9.82).
const FFN_HEADER: usize = 39;

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
    /// 0x0FFF = none.
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
        // StdfBase (10 bytes): sti(12)+flags(4); stk(4)+istdNext(12); cupx(4)+istdBase(12);
        // bchUpe(16) (= cbStd); grfstd(16). Validated against Word's own files: bchUpe equals
        // cbStd and style 0 is sti 0 ("Normal").
        let stk = (stdf.get(2).copied().unwrap_or(1) & 0x0F).max(1);
        let w1 = stdf.get(2..4).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0xFFF1);
        let _istd_next = w1 >> 4; // next-paragraph style; unused for now
        let w2 = stdf.get(4..6).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
        let cupx = (w2 & 0x0F) as usize;
        let istd_base = w2 >> 4;
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
        } else if stk == 2 && let Some(u) = upxs.first() {
            chpx = u.clone();
        }
        out.push(RawStyle { stk, istd_base, name, papx, chpx });
    }
    Ok(out)
}

/// Font names from the `SttbfFfn` at `table[fc..fc+lcb]`: an STTB of FFN records. Extended
/// tables carry a 2-byte length per record; non-extended ones rely on each FFN's NUL-
/// terminated name ([MS-DOC] §2.9.243/§2.9.247).
pub(crate) fn fonts(table: &[u8], fc: u32, lcb: u32) -> Vec<String> {
    let end = match (fc as usize).checked_add(lcb as usize) {
        Some(e) if e <= table.len() => e,
        _ => return Vec::new(),
    };
    let sttb = &table[fc as usize..end];
    let u16at = |o: usize| -> Option<u16> { sttb.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])) };
    let Some(first) = u16at(0) else { return Vec::new() };
    // FFN header: ffid(1) wWeight(2) ixchSzAlt(2) chs(1) panose(10) fs(24) = 40 bytes, then
    // the name, NUL-terminated, as UTF-16LE in every extended table and in the Word 97 files
    // we accept from non-extended ones.
    let name_of = |ffn: &[u8]| -> String {
        let tail = ffn.get(40.min(ffn.len())..).unwrap_or(&[]);
        let units: Vec<u16> = tail.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).take(64).collect();
        let s = String::from_utf16_lossy(&units);
        if s.chars().any(|c| c as u32 > 0x7F || c.is_ascii_graphic() || c == ' ') || s.is_empty() {
            s
        } else {
            // An 8-bit name reads as garbage through UTF-16; fall back to Windows-1252.
            tail.iter().take_while(|&&b| b != 0).take(64).map(|&b| wordcraft_doc::encoding::cp1252(b)).collect()
        }
    };
    let mut out = Vec::new();
    if first == 0xFFFF {
        // Extended: fExtend, cbExtra, cData, then cbData + FFN + extra per record.
        let cdata = u16at(4).unwrap_or(0).min(1024) as usize;
        let mut at = 6usize;
        for _ in 0..cdata {
            let Some(cb) = u16at(at).map(|v| v as usize) else { break };
            if let Some(ffn) = sttb.get(at + 2..at + 2 + cb) {
                out.push(name_of(ffn));
            }
            at += 2 + cb;
        }
    } else {
        // Non-extended: cData, cbExtra (0 for Ffn tables), then self-delimiting FFNs.
        let cdata = first.min(1024) as usize;
        let cb_extra = u16at(2).unwrap_or(0) as usize;
        let mut at = 4usize;
        for _ in 0..cdata {
            let rest = sttb.get(at..).unwrap_or(&[]);
            if rest.is_empty() {
                break;
            }
            // The name ends at the UTF-16 NUL; keep a cap for hostile inputs.
            let name_len = rest[40.min(rest.len())..].as_chunks::<2>().0.iter().position(|c| c == &[0, 0]).unwrap_or(64).saturating_mul(2);
            let rec_len = 40 + name_len + 2;
            out.push(name_of(rest));
            at += rec_len + cb_extra;
        }
    }
    out
}

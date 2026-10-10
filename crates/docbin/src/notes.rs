//! Footnotes, endnotes and bookmarks: `PlcffndRef`/`PlcffndTxt`, `PlcfendRef`/`PlcfendTxt`,
//! and `SttbfBkmk` + `Plcfbkf`/`Plcfbkl` ([MS-DOC] §2.9).

/// Cap on notes and bookmarks per document.
const MAX: usize = 10_000;

/// Parse the note reference/story PLCs. `ref_pair`/`txt_pair` are the FIB pair indices
/// (`(2, 3)` for footnotes, `(46, 47)` for endnotes); story ranges are relative to the
/// note subdocument.
pub(crate) fn parse_notes(table: &[u8], fib: &crate::fib::Fib, ref_pair: usize, txt_pair: usize) -> Vec<(u32, (u32, u32))> {
    let refs = fib.pair(ref_pair).and_then(|(fc, lcb)| plc_of(table, fc, lcb, 2)).unwrap_or_default();
    let txts = fib.pair(txt_pair).and_then(|(fc, lcb)| plc_of(table, fc, lcb, 0)).unwrap_or_default();
    if txts.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    // The reference PLC's aFtnIdx order matches the story order; pair them by index.
    for (i, &cp) in refs.iter().enumerate() {
        if let (Some(a), Some(b)) = (txts.get(i), txts.get(i + 1))
            && b > a
        {
            out.push((cp, (*a, *b)));
        }
    }
    out
}

/// The CPs of a PLC (the trailing element is a limit and is kept so callers can pair
/// range ends) plus the data-element size check. Returns the CPs only.
fn plc_of(table: &[u8], fc: u32, lcb: u32, data_size: usize) -> Option<Vec<u32>> {
    if lcb == 0 {
        return None;
    }
    let plc = table.get(fc as usize..(fc as usize).saturating_add(lcb as usize))?;
    if plc.len() < 4 {
        return None;
    }
    // aCP has one more element than aData: n = (len - data_size) / (4 + data_size).
    let n = (plc.len() - data_size) / (4 + data_size);
    if n == 0 || n > MAX {
        return None;
    }
    let mut cps = Vec::with_capacity(n.min(MAX));
    for i in 0..=n.min(MAX) {
        match plc.get(i * 4..i * 4 + 4) {
            Some(b) => cps.push(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            None => break,
        }
    }
    Some(cps)
}

/// A bookmark boundary to insert while walking.
pub(crate) enum Mark {
    Start(String),
    End(String),
}

/// Parse `SttbfBkmk` + `Plcfbkf` + `Plcfbkl` into (cp, event) pairs sorted for a sequential
/// walk. Hidden bookmarks (leading `_`) are kept: the engine can decide.
pub(crate) fn parse_bookmarks(table: &[u8], fib: &crate::fib::Fib) -> Vec<(u32, Mark)> {
    let names = fib.pair(crate::fib::pair::STTBF_BKMK).and_then(|(fc, lcb)| sttb_strings(table, fc, lcb)).unwrap_or_default();
    let starts = fib.pair(crate::fib::pair::PLCF_BKF).and_then(|(fc, lcb)| plc_of(table, fc, lcb, 4)).unwrap_or_default();
    let ends = fib.pair(crate::fib::pair::PLCF_BKL).and_then(|(fc, lcb)| plc_of(table, fc, lcb, 2)).unwrap_or_default();
    let n = starts.len().saturating_sub(1).min(ends.len().saturating_sub(1)).min(names.len()).min(MAX);
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let (Some(&a), Some(&b)) = (starts.get(i), ends.get(i)) else { break };
        if b < a {
            continue;
        }
        if let Some(name) = names.get(i) {
            if name.is_empty() {
                continue;
            }
            out.push((a, Mark::Start(name.clone())));
            out.push((b, Mark::End(name.clone())));
        }
    }
    out.sort_by_key(|(cp, m)| (*cp, matches!(m, Mark::End(_))));
    out
}

/// Strings of an extended STTB (fExtend = 0xFFFF, 2-byte chars, no extra data).
fn sttb_strings(table: &[u8], fc: u32, lcb: u32) -> Option<Vec<String>> {
    let b = table.get(fc as usize..(fc as usize).saturating_add(lcb as usize))?;
    if b.len() < 6 {
        return None;
    }
    let c_data = u16::from_le_bytes([b[2], b[3]]) as usize;
    if c_data > MAX {
        return None;
    }
    let mut at = 6usize;
    let mut out = Vec::with_capacity(c_data);
    for _ in 0..c_data {
        let cch = match b.get(at..at + 2) {
            Some(x) => u16::from_le_bytes([x[0], x[1]]) as usize,
            None => break,
        };
        at += 2;
        let cch = cch.min(40);
        let chars = b.get(at..at + cch * 2)?;
        at += cch * 2;
        let s: String = chars.as_chunks::<2>().0.iter().map(|c| char::from_u32(u16::from_le_bytes(*c) as u32).unwrap_or('\u{FFFD}')).collect();
        out.push(s);
    }
    Some(out)
}

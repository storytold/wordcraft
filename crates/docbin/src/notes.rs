//! Footnotes, endnotes and bookmarks: `PlcffndRef`/`PlcffndTxt`, `PlcfendRef`/`PlcfendTxt`,
//! and `SttbfBkmk` + `Plcfbkf`/`Plcfbkl` ([MS-DOC] §2.9).

/// Cap on notes and bookmarks per document.
const MAX: usize = 10_000;

/// Parse the note reference/story PLCs. `ref_pair`/`txt_pair` are the FIB pair indices
/// (`(2, 3)` for footnotes, `(46, 47)` for endnotes); story ranges are relative to the
/// note subdocument.
///
/// The story CPs of a well-formed PLC ascend, so the stories are disjoint. A hostile table
/// that steps back would make many notes read the same text again (each one materialised
/// as its own part), so a story starting before the end of the last one kept is dropped:
/// every subdocument CP becomes at most one note.
pub(crate) fn parse_notes(table: &[u8], fib: &crate::fib::Fib, ref_pair: usize, txt_pair: usize) -> Vec<(u32, (u32, u32))> {
    let refs = fib.pair(ref_pair).and_then(|(fc, lcb)| plc_of(table, fc, lcb, 2)).unwrap_or_default();
    let txts = fib.pair(txt_pair).and_then(|(fc, lcb)| plc_of(table, fc, lcb, 0)).unwrap_or_default();
    if txts.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut kept_to = 0u32;
    // The reference PLC's aFtnIdx order matches the story order; pair them by index.
    for (i, &cp) in refs.iter().enumerate() {
        if let (Some(&a), Some(&b)) = (txts.get(i), txts.get(i + 1))
            && b > a
            && a >= kept_to
        {
            out.push((cp, (a, b)));
            kept_to = b;
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

/// Strings of an STTB: extended (fExtend = 0xFFFF, 2-byte length and UTF-16 chars) or not
/// (1-byte length and Windows-1252 chars). `cbExtra` bytes after each string are skipped.
/// Names are kept to 40 characters, but the reader always advances by the stored length.
fn sttb_strings(table: &[u8], fc: u32, lcb: u32) -> Option<Vec<String>> {
    let b = table.get(fc as usize..(fc as usize).saturating_add(lcb as usize))?;
    let extended = b.get(0..2)? == [0xFF, 0xFF];
    let head = if extended { 2 } else { 0 };
    let h = b.get(head..head + 4)?;
    let c_data = u16::from_le_bytes([h[0], h[1]]) as usize;
    let cb_extra = u16::from_le_bytes([h[2], h[3]]) as usize;
    if c_data > MAX {
        return None;
    }
    let mut at = head + 4;
    let mut out = Vec::with_capacity(c_data);
    for _ in 0..c_data {
        let (cch, unit) = if extended {
            match b.get(at..at + 2) {
                Some(x) => (u16::from_le_bytes([x[0], x[1]]) as usize, 2),
                None => break,
            }
        } else {
            match b.get(at) {
                Some(&x) => (x as usize, 1),
                None => break,
            }
        };
        at += unit;
        let Some(chars) = b.get(at..at + cch * unit) else { break };
        at += cch * unit;
        // The string's extra data must be there too, or the table is cut short.
        if b.get(at..at + cb_extra).is_none() {
            break;
        }
        at += cb_extra;
        let s: String = if extended {
            chars.as_chunks::<2>().0.iter().take(40).map(|c| char::from_u32(u16::from_le_bytes(*c) as u32).unwrap_or('\u{FFFD}')).collect()
        } else {
            chars.iter().take(40).map(|&c| wordcraft_doc::encoding::cp1252(c)).collect()
        };
        out.push(s);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::sttb_strings;

    fn ext_sttb(names: &[&str], extra: &[u8]) -> Vec<u8> {
        let mut b = vec![0xFF, 0xFF];
        b.extend_from_slice(&(names.len() as u16).to_le_bytes());
        b.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        for n in names {
            let units: Vec<u16> = n.encode_utf16().collect();
            b.extend_from_slice(&(units.len() as u16).to_le_bytes());
            for u in units {
                b.extend_from_slice(&u.to_le_bytes());
            }
            b.extend_from_slice(extra);
        }
        b
    }

    #[test]
    fn long_bookmark_name_does_not_desync_table() {
        let long = "A".repeat(50);
        let b = ext_sttb(&[&long, "bm2"], &[]);
        let got = sttb_strings(&b, 0, b.len() as u32).unwrap();
        assert_eq!(got, vec!["A".repeat(40), "bm2".to_string()]);
    }

    #[test]
    fn sttb_extra_data_is_skipped() {
        let b = ext_sttb(&["first", "bm2"], &[1, 2, 3, 4]);
        let got = sttb_strings(&b, 0, b.len() as u32).unwrap();
        assert_eq!(got, vec!["first".to_string(), "bm2".to_string()]);
    }

    #[test]
    fn a_name_without_its_extra_data_is_dropped() {
        let mut b = ext_sttb(&["first", "bm2"], &[1, 2, 3, 4]);
        b.truncate(b.len() - 2);
        let got = sttb_strings(&b, 0, b.len() as u32).unwrap();
        assert_eq!(got, vec!["first".to_string()]);
    }

    #[test]
    fn non_extended_sttb_reads_byte_strings() {
        let mut b = Vec::new();
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        for n in ["one", "caf\u{e9}"] {
            let bytes: Vec<u8> = n.chars().map(|c| c as u8).collect();
            b.push(bytes.len() as u8);
            b.extend_from_slice(&bytes);
            b.extend_from_slice(&[9, 9]);
        }
        let got = sttb_strings(&b, 0, b.len() as u32).unwrap();
        assert_eq!(got, vec!["one".to_string(), "caf\u{e9}".to_string()]);
    }
}

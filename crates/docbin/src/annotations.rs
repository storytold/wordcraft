//! Comments (annotations, [MS-DOC] §2.3.4): `PlcfandRef` (reference CPs with an `ATRDPre10`
//! each) and `PlcfandTxt` (each comment's text in the comment subdocument), the author names of
//! `GrpXstAtnOwners`, and the commented ranges: the annotation bookmarks of `SttbfAtnBkmk`
//! (an `ATNBE` tag each) with `PlcfAtnBkf`/`PlcfAtnBkl`.

use std::collections::HashMap;

use crate::fib::{Fib, pair};

/// Most comments per document (as for notes and bookmarks).
const MAX: usize = 10_000;
/// Most author names ([MS-DOC] caps `GrpXstAtnOwners` at 0x7FFF entries).
const MAX_AUTHORS: usize = 0x7FFF;
/// Characters kept of an author name: more than the specification's 55 (`cch` < 56), so a
/// slightly non-conforming name is kept whole, while a hostile one is still bounded.
const MAX_NAME: usize = 255;
/// Size of an `ATRDPre10` (§2.9.7).
const ATRD: usize = 30;
/// Size of an `FBKF` (§2.9.70).
const FBKF: usize = 4;
/// Size of an `ATNBE` (§2.9.4).
const ATNBE: usize = 10;

/// One comment as read from the tables.
pub(crate) struct Annotation {
    /// Its reference mark (0x05) in the main document.
    pub(crate) ref_cp: u32,
    /// Its text, relative to the comment subdocument (starting with its own 0x05 mark).
    pub(crate) story: (u32, u32),
    pub(crate) author: String,
    pub(crate) initials: String,
    /// The commented range `[start, end)` in the main document; `None` on an empty range, or when
    /// the file's bookmark for it is missing or broken: the comment is then at its reference.
    pub(crate) range: Option<(u32, u32)>,
}

/// The document's comments, in reference order.
///
/// Like note stories (`notes::parse_notes`), the text ranges of a well-formed `PlcfandTxt`
/// ascend; one starting before the end of the last one kept is dropped, so a hostile table
/// can't make many comments materialise the same text again.
pub(crate) fn parse(table: &[u8], fib: &Fib) -> Vec<Annotation> {
    let Some((refs, atrds)) = fib.pair(pair::PLCF_AND_REF).and_then(|p| plc(table, p, ATRD)) else { return Vec::new() };
    let Some((txts, _)) = fib.pair(pair::PLCF_AND_TXT).and_then(|p| plc(table, p, 0)) else { return Vec::new() };
    let authors = authors(table, fib.pair(pair::GRP_XST_ATN_OWNERS));
    let ranges = ranges(table, fib);
    let mut out = Vec::new();
    let mut kept_to = 0u32;
    // The last CP of the reference PLC is a limit, not a reference.
    for (i, &ref_cp) in refs.iter().take(refs.len().saturating_sub(1)).take(MAX).enumerate() {
        let (Some(&a), Some(&b)) = (txts.get(i), txts.get(i + 1)) else { break };
        if b <= a || a < kept_to {
            continue;
        }
        kept_to = b;
        let atrd = atrds.get(i * ATRD..(i + 1) * ATRD).unwrap_or_default();
        let initials = atrd
            .get(0..2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]).min(9) as usize)
            .and_then(|n| atrd.get(2..2 + n * 2))
            .map(utf16)
            .unwrap_or_default();
        let author = atrd.get(20..22).and_then(|b| authors.get(u16::from_le_bytes([b[0], b[1]]) as usize)).cloned().unwrap_or_default();
        // lTagBkmk is -1 exactly when the comment is on an empty range. Any other value is a tag:
        // `ATNBE.lTag` is unsigned, so one past i32::MAX reads as negative here and is still matched.
        let tag = atrd.get(26..30).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).filter(|t| *t != -1);
        let range = tag.and_then(|t| ranges.get(&(t as u32)).copied());
        out.push(Annotation { ref_cp, story: (a, b), author, initials, range });
    }
    out
}

/// A PLC's CPs (`n + 1` of them, the last a limit) and its data elements (`size` bytes each).
fn plc(table: &[u8], (fc, lcb): (u32, u32), size: usize) -> Option<(Vec<u32>, &[u8])> {
    if lcb == 0 {
        return None;
    }
    let b = table.get(fc as usize..(fc as usize).checked_add(lcb as usize)?)?;
    // 4·(n + 1) bytes of CPs, then n data elements. A `PlcfandTxt` has one CP more than there
    // are comments (the guard's), so it may hold one element more than `MAX`.
    let n = b.len().checked_sub(4)? / (4 + size);
    if n == 0 || n > MAX + 1 {
        return None;
    }
    let (cps, data) = b.split_at_checked((n + 1) * 4)?;
    Some((cps.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect(), data))
}

/// UTF-16LE text, unpaired surrogates replaced.
fn utf16(b: &[u8]) -> String {
    char::decode_utf16(b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c))).map(|c| c.unwrap_or('\u{FFFD}')).collect()
}

/// The author names: `GrpXstAtnOwners` is a run of `Xst`s (a u16 count, then UTF-16 characters)
/// filling its `lcb`, with no count of its own.
fn authors(table: &[u8], at: Option<(u32, u32)>) -> Vec<String> {
    let Some(b) = at.and_then(|(fc, lcb)| table.get(fc as usize..(fc as usize).saturating_add(lcb as usize))) else { return Vec::new() };
    let mut out = Vec::new();
    let mut at = 0usize;
    while out.len() < MAX_AUTHORS {
        let Some(h) = b.get(at..at + 2) else { break };
        let cch = u16::from_le_bytes([h[0], h[1]]) as usize;
        let Some(chars) = b.get(at + 2..at + 2 + cch * 2) else { break };
        at += 2 + cch * 2;
        out.push(utf16(chars).chars().take(MAX_NAME).collect());
    }
    out
}

/// The commented ranges by bookmark tag. `SttbfAtnBkmk` and `PlcfAtnBkf` are parallel: the
/// i-th bookmark's tag is in the i-th `ATNBE`, its start is the i-th CP of the `Plcfbkf`, and its
/// end is the CP of the `Plcfbkl` (CPs only) that its `FBKF.ibkl` indexes. An empty range (a
/// comment on one has no tag, `lTagBkmk` -1) or one ending before it starts, or an index or tag
/// that isn't there, is left out; the first range of a repeated tag is kept.
fn ranges(table: &[u8], fib: &Fib) -> HashMap<u32, (u32, u32)> {
    let mut out = HashMap::new();
    let tags = tags(table, fib.pair(pair::STTBF_ATN_BKMK));
    let Some((starts, fbkfs)) = fib.pair(pair::PLCF_ATN_BKF).and_then(|p| plc(table, p, FBKF)) else { return out };
    let Some((ends, _)) = fib.pair(pair::PLCF_ATN_BKL).and_then(|p| plc(table, p, 0)) else { return out };
    // Neither PLC's last CP is a bookmark's.
    let (starts, ends) = (starts.split_last().map_or(&[][..], |(_, s)| s), ends.split_last().map_or(&[][..], |(_, e)| e));
    for (i, (&tag, &start)) in tags.iter().zip(starts).enumerate() {
        let Some(f) = fbkfs.get(i * FBKF..i * FBKF + 2) else { break };
        let Some(&end) = ends.get(u16::from_le_bytes([f[0], f[1]]) as usize) else { continue };
        if end > start {
            out.entry(tag).or_insert((start, end));
        }
    }
    out
}

/// The `lTag` of each `ATNBE` in a `SttbfAtnBkmk`: an extended STTB (`fExtend` 0xFFFF, 2-byte
/// `cData`) whose strings are empty, each followed by `cbExtra` bytes holding an `ATNBE`.
fn tags(table: &[u8], at: Option<(u32, u32)>) -> Vec<u32> {
    let Some(b) = at.and_then(|(fc, lcb)| table.get(fc as usize..(fc as usize).saturating_add(lcb as usize))) else { return Vec::new() };
    let u16_at = |at: usize| b.get(at..at + 2).map(|x| u16::from_le_bytes([x[0], x[1]]) as usize);
    if u16_at(0) != Some(0xFFFF) {
        return Vec::new();
    }
    let (Some(c_data), Some(cb_extra)) = (u16_at(2), u16_at(4)) else { return Vec::new() };
    if cb_extra < ATNBE {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut at = 6usize;
    for _ in 0..c_data.min(MAX) {
        // The strings should be empty, but a length is skipped like any other STTB's.
        let Some(cch) = u16_at(at) else { break };
        at += 2 + cch * 2;
        let Some(extra) = b.get(at..at + cb_extra) else { break };
        at += cb_extra;
        out.push(extra.get(2..6).map(|t| u32::from_le_bytes([t[0], t[1], t[2], t[3]])).unwrap_or(u32::MAX));
    }
    out
}

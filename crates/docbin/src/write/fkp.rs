//! Formatted disk pages ([MS-DOC] §2.9.43 ChpxFkp, §2.9.177 PapxFkp) and the bin tables that
//! index them (`PlcBteChpx`, `PlcBtePapx`), plus the PLC and STTB containers the Table stream
//! is made of.

/// Bytes in a formatted disk page.
const PAGE: usize = 512;
/// Most runs a `ChpxFkp` holds ([MS-DOC]: crun MUST be at most 0x65).
const MAX_CHPX_RUNS: usize = 0x65;
/// Most paragraphs a `PapxFkp` holds ([MS-DOC]: crun MUST be at most 0x1D).
const MAX_PAPX_RUNS: usize = 0x1D;
/// Size of a `BxPap`: a word offset and a 12-byte `PHE` (left zero: Word recomputes heights).
const BX: usize = 13;

/// One run of text sharing its character formatting: `[fc_start, fc_end)` in the WordDocument
/// stream, with the CHPX grpprl (empty = no direct formatting).
pub(crate) struct ChpxRun<'a> {
    pub(crate) fc_start: u32,
    pub(crate) fc_end: u32,
    pub(crate) grpprl: &'a [u8],
}

/// One paragraph: `[fc_start, fc_end)` (its mark included), its style and PAPX grpprl.
pub(crate) struct PapxRun<'a> {
    pub(crate) fc_start: u32,
    pub(crate) fc_end: u32,
    pub(crate) istd: u16,
    pub(crate) grpprl: &'a [u8],
}

/// Pages ready for the WordDocument stream and the bin table (`aFc` + `aPn`) that indexes them.
pub(crate) struct Paged {
    pub(crate) pages: Vec<[u8; PAGE]>,
    /// `aFc`: the first FC of each page, then the last page's end.
    fcs: Vec<u32>,
}

impl Paged {
    /// The `PlcBte*` for these pages once they sit from page number `first_pn` on.
    pub(crate) fn plc_bte(&self, first_pn: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.fcs.len() * 4 + self.pages.len() * 4);
        for fc in &self.fcs {
            v.extend_from_slice(&fc.to_le_bytes());
        }
        for i in 0..self.pages.len() {
            v.extend_from_slice(&first_pn.saturating_add(i as u32).to_le_bytes());
        }
        v
    }
}

/// A page being filled: the `rgfc` entries so far and the property bytes, which grow down from
/// the end of the page (the last byte is `crun`).
struct Page {
    buf: [u8; PAGE],
    fcs: Vec<u32>,
    /// Word offset (÷2) of each entry's properties (0 = none).
    offs: Vec<u8>,
    /// Lowest byte used by property data.
    low: usize,
    /// Property bytes already on the page and where they are, so equal ones are stored once.
    stored: Vec<(Vec<u8>, u8)>,
}

impl Page {
    fn new() -> Page {
        Page { buf: [0; PAGE], fcs: Vec::new(), offs: Vec::new(), low: PAGE - 1, stored: Vec::new() }
    }

    /// Bytes the header takes with `crun` entries of `entry` bytes each.
    fn header(crun: usize, entry: usize) -> usize {
        (crun + 1) * 4 + crun * entry
    }

    /// Place `data` (already in its on-page form), returning its word offset, if it fits beside
    /// a header for `crun` entries.
    fn place(&mut self, data: &[u8], crun: usize, entry: usize) -> Option<u8> {
        if data.is_empty() {
            return (Page::header(crun, entry) <= self.low).then_some(0);
        }
        if let Some((_, off)) = self.stored.iter().find(|(d, _)| d == data) {
            return (Page::header(crun, entry) <= self.low).then_some(*off);
        }
        let at = self.low.checked_sub(data.len())? & !1;
        if at < Page::header(crun, entry) || at / 2 > u8::MAX as usize {
            return None;
        }
        self.buf.get_mut(at..at + data.len())?.copy_from_slice(data);
        self.low = at;
        let off = (at / 2) as u8;
        self.stored.push((data.to_vec(), off));
        Some(off)
    }

    /// Write the header and `crun`; `entry` is the size of each `rgb`/`rgbx` element.
    fn finish(mut self, end_fc: u32, entry: usize) -> ([u8; PAGE], u32) {
        let crun = self.offs.len();
        let first = self.fcs.first().copied().unwrap_or(end_fc);
        self.fcs.push(end_fc);
        for (i, fc) in self.fcs.iter().enumerate() {
            if let Some(b) = self.buf.get_mut(i * 4..i * 4 + 4) {
                b.copy_from_slice(&fc.to_le_bytes());
            }
        }
        let rgb_at = (crun + 1) * 4;
        for (i, off) in self.offs.iter().enumerate() {
            if let Some(b) = self.buf.get_mut(rgb_at + i * entry) {
                *b = *off;
            }
        }
        self.buf[PAGE - 1] = crun as u8;
        (self.buf, first)
    }
}

/// Page the character runs into `ChpxFkp`s: each run's `Chpx` is `cb` then the grpprl (at most
/// 255 bytes; the caller truncates).
pub(crate) fn chpx_pages(runs: &[ChpxRun<'_>]) -> Paged {
    let mut out = Paged { pages: Vec::new(), fcs: Vec::new() };
    let mut page = Page::new();
    for r in runs {
        let mut data = Vec::new();
        if !r.grpprl.is_empty() {
            data.push(r.grpprl.len().min(255) as u8);
            data.extend_from_slice(r.grpprl.get(..255).unwrap_or(r.grpprl));
        }
        let off = if page.offs.len() < MAX_CHPX_RUNS { page.place(&data, page.offs.len() + 1, 1) } else { None };
        let off = match off {
            Some(o) => o,
            None => {
                let (buf, first) = std::mem::replace(&mut page, Page::new()).finish(r.fc_start, 1);
                out.pages.push(buf);
                out.fcs.push(first);
                // A fresh page always has room for one Chpx of at most 256 bytes.
                page.place(&data, 1, 1).unwrap_or(0)
            }
        };
        page.fcs.push(r.fc_start);
        page.offs.push(off);
    }
    let end = runs.last().map_or(0, |r| r.fc_end);
    if !page.offs.is_empty() {
        let (buf, first) = page.finish(end, 1);
        out.pages.push(buf);
        out.fcs.push(first);
    }
    out.fcs.push(end);
    out
}

/// A `PapxInFkp` ([MS-DOC] §2.9.175): the istd and grpprl with the size byte(s) in front: `cb`
/// (2×cb−1 bytes follow) for an odd size, else 0 and `cb'` (2×cb' bytes follow).
fn papx_in_fkp(istd: u16, grpprl: &[u8]) -> Vec<u8> {
    let mut body = istd.to_le_bytes().to_vec();
    body.extend_from_slice(grpprl);
    let mut v = Vec::with_capacity(body.len() + 2);
    if body.len() % 2 == 1 {
        v.push(body.len().div_ceil(2) as u8);
    } else {
        v.push(0);
        v.push((body.len() / 2) as u8);
    }
    v.extend_from_slice(&body);
    v
}

/// Largest PAPX grpprl a page takes: one paragraph alone on a page must still fit.
pub(crate) const MAX_PAPX_GRPPRL: usize = 480;

/// Page the paragraphs into `PapxFkp`s.
pub(crate) fn papx_pages(paras: &[PapxRun<'_>]) -> Paged {
    let mut out = Paged { pages: Vec::new(), fcs: Vec::new() };
    let mut page = Page::new();
    for p in paras {
        let data = papx_in_fkp(p.istd, p.grpprl.get(..MAX_PAPX_GRPPRL).unwrap_or(p.grpprl));
        // Even an unformatted paragraph names its style, so every BX points at a PAPX.
        let off = if page.offs.len() < MAX_PAPX_RUNS { page.place(&data, page.offs.len() + 1, BX) } else { None };
        let off = match off {
            Some(o) => o,
            None => {
                let (buf, first) = std::mem::replace(&mut page, Page::new()).finish(p.fc_start, BX);
                out.pages.push(buf);
                out.fcs.push(first);
                page.place(&data, 1, BX).unwrap_or(0)
            }
        };
        page.fcs.push(p.fc_start);
        page.offs.push(off);
    }
    let end = paras.last().map_or(0, |p| p.fc_end);
    if !page.offs.is_empty() {
        let (buf, first) = page.finish(end, BX);
        out.pages.push(buf);
        out.fcs.push(first);
    }
    out.fcs.push(end);
    out
}

/// A PLC ([MS-DOC] §2.2.2): `cps` (one more than the data elements), then the elements.
pub(crate) fn plc(cps: &[u32], data: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(cps.len() * 4 + data.iter().map(|d| d.len()).sum::<usize>());
    for cp in cps {
        v.extend_from_slice(&cp.to_le_bytes());
    }
    for d in data {
        v.extend_from_slice(d);
    }
    v
}

/// An extended STTB ([MS-DOC] §2.2.4): `fExtend`, `cData`, `cbExtra`, then each string as a
/// count of UTF-16 units and the units, followed by its `extra` bytes.
pub(crate) fn sttb_ext(strings: &[(String, Vec<u8>)], cb_extra: u16) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFF];
    v.extend_from_slice(&(strings.len().min(u16::MAX as usize) as u16).to_le_bytes());
    v.extend_from_slice(&cb_extra.to_le_bytes());
    for (s, extra) in strings.iter().take(u16::MAX as usize) {
        let units: Vec<u16> = s.encode_utf16().take(u16::MAX as usize).collect();
        v.extend_from_slice(&(units.len() as u16).to_le_bytes());
        for u in units {
            v.extend_from_slice(&u.to_le_bytes());
        }
        let mut e = extra.clone();
        e.resize(cb_extra as usize, 0);
        v.extend_from_slice(&e);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fkp::Bins;

    #[test]
    fn chpx_pages_read_back() {
        let bold = [0x35u8, 0x08, 1];
        let runs: Vec<ChpxRun> =
            (0..300u32).map(|i| ChpxRun { fc_start: 0x400 + i * 2, fc_end: 0x402 + i * 2, grpprl: if i % 2 == 0 { &bold } else { &[] } }).collect();
        let paged = chpx_pages(&runs);
        assert!(paged.pages.len() >= 3, "300 runs need several pages");
        let mut word = vec![0u8; 0x1000];
        for p in &paged.pages {
            word.extend_from_slice(p);
        }
        let bte = paged.plc_bte(8);
        let bins = Bins::parse(&bte, 0, bte.len() as u32).unwrap();
        for i in 0..300u32 {
            let g = bins.chpx(&word, 0x400 + i * 2);
            assert_eq!(g, if i % 2 == 0 { &bold[..] } else { &[][..] }, "run {i}");
        }
    }

    #[test]
    fn papx_pages_read_back_odd_and_even_sizes() {
        let odd = [0x03u8, 0x24, 1]; // 2 + 3 = 5 bytes with the istd
        let even = [0x03u8, 0x24, 1, 0]; // padded to 6 by the istd
        let paras: Vec<PapxRun> = (0..70u32)
            .map(|i| PapxRun {
                fc_start: 0x400 + i * 10,
                fc_end: 0x40A + i * 10,
                istd: i as u16 % 3,
                grpprl: if i % 2 == 0 { &odd } else { &even[..3] },
            })
            .collect();
        let paged = papx_pages(&paras);
        assert!(paged.pages.len() >= 3);
        let mut word = vec![0u8; 0x1000];
        for p in &paged.pages {
            word.extend_from_slice(p);
        }
        let bte = paged.plc_bte(8);
        let bins = Bins::parse(&bte, 0, bte.len() as u32).unwrap();
        for i in 0..70u32 {
            let (istd, g) = bins.papx(&word, 0x400 + i * 10 + 9);
            assert_eq!(istd, i as u16 % 3);
            assert_eq!(g, &odd[..]);
        }
    }
}

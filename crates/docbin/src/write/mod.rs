//! The `.doc` writer: a [`Document`] as a Word 97-2003 binary file ([MS-DOC]).
//!
//! The file is what Word 97 itself writes, minus what WordCraft has no use for: a compound file
//! with the WordDocument stream (the FIB, the text as one Unicode piece, the formatting pages and
//! the section properties), the 1Table stream (stylesheet, fonts, piece table, document
//! properties, the PLCs locating sections, headers, notes, fields and bookmarks, the lists), a
//! Data stream with the pictures, and the summary information.
//!
//! Kept: text, character and paragraph formatting, styles, lists, tables (one level; nested
//! tables become paragraphs of their cell), sections with headers and footers, footnotes and
//! endnotes, fields and hyperlinks, bookmarks, inline pictures, tracked insertions and deletions,
//! and the document properties. Floating pictures are written in line; shapes, charts, text
//! boxes and comments are left out, each kind logged once per save.

mod fkp;
mod grpprl;
mod lists;
mod picture;
mod story;
mod styles;
mod summary;
mod tables;

use std::collections::HashMap;

use wordcraft_doc::Document;
use wordcraft_doc::para::{NoteKind, Paragraph};
use wordcraft_doc::section::SectionProps;

use crate::DocbinError;
use crate::fib::pair;
use grpprl::{CharCtx, Grpprl};
use story::{Lookup, PARA, Text};

/// Where the text starts in the WordDocument stream: right after the FIB (900 bytes).
const TEXT_FC: u32 = 0x400;
/// Bytes in the FIB this writer produces: FibBase, csw + FibRgW97, cslw + FibRgLw97,
/// cbRgFcLcb + FibRgFcLcb97 (0x5D pairs), cswNew.
const FIB_LEN: usize = 0x9A + 0x5D * 8 + 2;
/// Most characters written: the WordDocument stream's FCs are 32-bit and each character takes
/// two bytes; stay far inside.
const MAX_CP: usize = 256 << 20;
/// Size of a Dop97.
const DOP97_LEN: usize = 500;

/// Write `doc` as a Word 97-2003 document.
pub fn write(doc: &Document) -> Result<Vec<u8>, DocbinError> {
    let plan = styles::plan(&doc.styles);
    let normal = plan.istd.get("Normal").copied().unwrap_or(0);
    let lfos = lists::lfo_indexes(&doc.numbering);
    let mut look = Lookup::new(doc, plan.istd.clone(), normal, lfos);
    // Font 0 is the document's default font, as in every Word file; the stylesheet header names
    // it as the standard font.
    let default_font = doc.styles.default_chr.font.clone().unwrap_or_else(|| "Times New Roman".to_string());
    let f_ascii = look.font(&default_font);
    let f_ea = doc.styles.default_chr.font_ea.as_deref().map_or(f_ascii, |f| look.font(f));
    let f_cs = doc.styles.default_chr.font_cs.as_deref().map_or(f_ascii, |f| look.font(f));
    let stsh = styles::stsh(&doc.styles, &plan, &mut look, [f_ascii, f_ea, f_cs]);
    let lists = lists::write(&doc.numbering, &mut look);

    let mut t = Text::new(look);
    // The main document; it ends with a plain paragraph (a table or a section break can't end it).
    t.blocks(&doc.body, true);
    t.close_fields();
    let ends_plain = doc.body.last().and_then(|b| b.as_para()).is_some_and(|p| p.section.is_none());
    if !ends_plain || t.paras.last().is_none_or(|p| p.end_cp != t.cp()) {
        t.mark(&Paragraph::new(), PARA, &Grpprl::default());
    }
    let ccp_text = t.cp();
    let mut sections: Vec<(u32, SectionProps)> = std::mem::take(&mut t.sections);
    sections.push((ccp_text, doc.last_section.clone()));

    // Footnotes, then the header document, then endnotes, each a subdocument.
    let (ccp_ftn, ftn_txt) = notes(&mut t, doc, NoteKind::Footnote);
    let hdd_start = t.cp();
    let hdd_stories = headers(&mut t, doc, &sections);
    let ccp_hdd = t.cp() - hdd_start;
    let (ccp_edn, edn_txt) = notes(&mut t, doc, NoteKind::Endnote);
    if ccp_ftn + ccp_hdd + ccp_edn > 0 {
        // One more paragraph mark after the last subdocument ([MS-DOC] §2.8.35).
        t.mark(&Paragraph::new(), PARA, &Grpprl::default());
    }
    let ccp_all = t.cp();
    if t.units.len() > MAX_CP {
        return Err(DocbinError::Limit(format!("the document has {} characters, more than a .doc holds", t.units.len())));
    }
    log_dropped(&t.dropped);

    // ---- WordDocument stream: FIB, text, formatting pages, section properties.
    let mut word = vec![0u8; TEXT_FC as usize];
    for u in &t.units {
        word.extend_from_slice(&u.to_le_bytes());
    }
    let fc = |cp: u32| TEXT_FC + cp * 2;
    pad_to(&mut word, 512);
    let runs = t.char_runs();
    let chpx_runs: Vec<fkp::ChpxRun> = runs.iter().map(|&(a, b, g)| fkp::ChpxRun { fc_start: fc(a), fc_end: fc(b), grpprl: g }).collect();
    let chpx = fkp::chpx_pages(&chpx_runs);
    let chpx_pn = (word.len() / 512) as u32;
    for p in &chpx.pages {
        word.extend_from_slice(p);
    }
    let mut start = 0u32;
    let papx_runs: Vec<fkp::PapxRun> = t
        .paras
        .iter()
        .map(|p| {
            let r = fkp::PapxRun { fc_start: fc(start), fc_end: fc(p.end_cp), istd: p.istd, grpprl: p.grpprl.as_slice() };
            start = p.end_cp;
            r
        })
        .collect();
    let papx = fkp::papx_pages(&papx_runs);
    let papx_pn = (word.len() / 512) as u32;
    for p in &papx.pages {
        word.extend_from_slice(p);
    }
    let mut sepx_at = Vec::with_capacity(sections.len());
    for (_, s) in &sections {
        pad_to(&mut word, 2);
        sepx_at.push(word.len() as u32);
        let g = grpprl::sepx(s);
        word.extend_from_slice(&(g.len() as u16).to_le_bytes());
        word.extend_from_slice(&g.0);
    }
    let cb_mac = word.len() as u32;

    // ---- Table stream.
    let mut tbl = TableStream::default();
    tbl.put(pair::STSHF, &stsh);
    if let Some(&v) = tbl.pairs.get(&pair::STSHF) {
        tbl.pairs.insert(0, v); // fcStshfOrig: the same stylesheet
    }
    // Notes: references in the main text and the stories in their subdocuments.
    for (kind, ref_pair, txt_pair, ccp, txt) in [
        (NoteKind::Footnote, pair::PLCF_FND_REF, pair::PLCF_FND_TXT, ccp_ftn, &ftn_txt),
        (NoteKind::Endnote, pair::PLCF_END_REF, pair::PLCF_END_TXT, ccp_edn, &edn_txt),
    ] {
        if ccp == 0 {
            continue;
        }
        let refs: Vec<&story::NoteRefAt> = t.notes.iter().filter(|n| n.kind == kind).collect();
        let mut cps: Vec<u32> = refs.iter().map(|n| n.cp).collect();
        cps.push(ccp_text);
        let idx: Vec<[u8; 2]> = refs.iter().enumerate().map(|(i, n)| if n.auto { (i as u16 + 1).to_le_bytes() } else { [0, 0] }).collect();
        tbl.put(ref_pair, &fkp::plc(&cps, &idx.iter().map(|x| &x[..]).collect::<Vec<_>>()));
        let mut tcps = txt.clone();
        tcps.push(ccp - 1);
        tcps.push(ccp);
        tbl.put(txt_pair, &fkp::plc(&tcps, &[]));
    }
    // Sections.
    let mut sed_cps = vec![0u32];
    sed_cps.extend(sections.iter().map(|(end, _)| *end));
    let seds: Vec<[u8; 12]> = sepx_at
        .iter()
        .map(|at| {
            let mut s = [0u8; 12];
            s[2..6].copy_from_slice(&at.to_le_bytes());
            s[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
            s
        })
        .collect();
    tbl.put(pair::PLCF_SED, &fkp::plc(&sed_cps, &seds.iter().map(|x| &x[..]).collect::<Vec<_>>()));
    if ccp_hdd > 0 {
        let mut cps = hdd_stories;
        cps.push(ccp_hdd - 1);
        cps.push(ccp_hdd);
        tbl.put(pair::PLCF_HDD, &fkp::plc(&cps, &[]));
    }
    tbl.put(pair::PLCF_BTE_CHPX, &chpx.plc_bte(chpx_pn));
    tbl.put(pair::PLCF_BTE_PAPX, &papx.plc_bte(papx_pn));
    // Fields, one PLC per subdocument with CPs relative to it.
    let ftn_start = ccp_text;
    let edn_start = hdd_start + ccp_hdd;
    for (i, a, len) in [(16usize, 0, ccp_text), (18, ftn_start, ccp_ftn), (17, hdd_start, ccp_hdd), (48, edn_start, ccp_edn)] {
        let f: Vec<&story::FieldChar> = t.fields.iter().filter(|f| f.cp >= a && f.cp < a + len).collect();
        if f.is_empty() {
            continue;
        }
        let mut cps: Vec<u32> = f.iter().map(|f| f.cp - a).collect();
        cps.push(len);
        tbl.put(i, &fkp::plc(&cps, &f.iter().map(|f| &f.fld[..]).collect::<Vec<_>>()));
    }
    // Bookmarks in the main text (with an end): names, starts with the index of their end, ends.
    let mut marks: Vec<(String, u32, u32)> =
        t.bookmarks.iter().filter_map(|(n, a, b)| b.filter(|&b| b <= ccp_text && *a <= b).map(|b| (n.clone(), *a, b))).take(0x3FFF).collect();
    marks.sort_by_key(|m| m.1);
    if !marks.is_empty() {
        let mut ends: Vec<(u32, usize)> = marks.iter().enumerate().map(|(i, m)| (m.2, i)).collect();
        ends.sort();
        let mut ibkl = vec![0u16; marks.len()];
        for (pos, (_, i)) in ends.iter().enumerate() {
            if let Some(slot) = ibkl.get_mut(*i) {
                *slot = pos as u16;
            }
        }
        let names: Vec<(String, Vec<u8>)> = marks.iter().map(|m| (m.0.chars().take(40).collect(), Vec::new())).collect();
        tbl.put(pair::STTBF_BKMK, &fkp::sttb_ext(&names, 0));
        let mut cps: Vec<u32> = marks.iter().map(|m| m.1).collect();
        cps.push(ccp_text + 1);
        let fbkf: Vec<[u8; 4]> = ibkl
            .iter()
            .map(|i| {
                let mut b = [0u8; 4];
                b[0..2].copy_from_slice(&i.to_le_bytes());
                b
            })
            .collect();
        tbl.put(pair::PLCF_BKF, &fkp::plc(&cps, &fbkf.iter().map(|x| &x[..]).collect::<Vec<_>>()));
        let mut cps: Vec<u32> = ends.iter().map(|e| e.0).collect();
        cps.push(ccp_text + 1);
        tbl.put(pair::PLCF_BKL, &fkp::plc(&cps, &[]));
    }
    tbl.put(31, &dop(doc));
    // SttbfAssoc MUST be there: 18 strings (template, title, author…), all empty here.
    tbl.put(32, &fkp::sttb_ext(&vec![(String::new(), Vec::new()); 18], 0));
    // The piece table: one Unicode piece holding every character.
    let mut plc = fkp::plc(&[0, ccp_all], &[]);
    plc.extend_from_slice(&0u16.to_le_bytes());
    plc.extend_from_slice(&TEXT_FC.to_le_bytes());
    plc.extend_from_slice(&0u16.to_le_bytes());
    let mut clx = vec![0x02];
    clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    clx.extend_from_slice(&plc);
    tbl.put(pair::CLX, &clx);
    if t.look.authors.len() > 1 {
        let authors: Vec<(String, Vec<u8>)> = t.look.authors.iter().map(|a| (a.clone(), Vec::new())).collect();
        tbl.put(51, &fkp::sttb_ext(&authors, 0));
    }
    if let Some(l) = &lists {
        pad_to(&mut tbl.bytes, 2);
        tbl.pairs.insert(pair::PLF_LST, (tbl.bytes.len() as u32, l.lcb_plf_lst));
        tbl.bytes.extend_from_slice(&l.plf_lst);
        tbl.put(pair::PLF_LFO, &l.plf_lfo);
    }
    // The font table last: every font the text and styles name is in it now.
    tbl.put(pair::STTBF_FFN, &styles::font_table(&t.look.fonts));

    // ---- FIB.
    let has_pic = !t.pictures.data.is_empty();
    let fib = fib(cb_mac, [ccp_text, ccp_ftn, ccp_hdd, ccp_edn], &tbl.pairs, has_pic);
    if let Some(head) = word.get_mut(..FIB_LEN) {
        head.copy_from_slice(&fib);
    }

    // ---- The compound file.
    let mut streams: Vec<(&str, Vec<u8>)> = vec![("WordDocument", word), ("1Table", tbl.bytes)];
    if has_pic {
        streams.push(("Data", std::mem::take(&mut t.pictures.data)));
    }
    streams.push(("\u{5}SummaryInformation", summary::stream(&doc.core)));
    compound(&streams)
}

/// The Table stream being built, and the FIB pair (fc, lcb) of each structure in it.
#[derive(Default)]
struct TableStream {
    bytes: Vec<u8>,
    pairs: HashMap<usize, (u32, u32)>,
}

impl TableStream {
    /// Append a structure (word-aligned) and record it as FIB pair `i`; nothing for an empty one.
    fn put(&mut self, i: usize, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        pad_to(&mut self.bytes, 2);
        self.pairs.insert(i, (self.bytes.len() as u32, bytes.len() as u32));
        self.bytes.extend_from_slice(bytes);
    }
}

/// Write the stories of the notes of `kind` the main text referred to, in reference order:
/// the subdocument's length and each story's start CP (relative to it).
fn notes(t: &mut Text<'_>, doc: &Document, kind: NoteKind) -> (u32, Vec<u32>) {
    let start = t.cp();
    let parts: Vec<u32> = t.notes.iter().filter(|n| n.kind == kind).map(|n| n.part).collect();
    if parts.is_empty() {
        return (0, Vec::new());
    }
    let mut starts = Vec::with_capacity(parts.len());
    for id in parts {
        starts.push(t.cp() - start);
        let blocks = doc.parts.get(&id).map(|p| p.blocks.as_slice()).unwrap_or_default();
        t.note_story(kind, id, blocks);
        t.close_fields();
    }
    // The guard paragraph mark after the last story.
    t.mark(&Paragraph::new(), PARA, &Grpprl::default());
    (t.cp() - start, starts)
}

/// Write the header document: the six (empty) note separator stories, then six stories per
/// section (even header, header, even footer, footer, first-page header, first-page footer),
/// each ended by a guard paragraph mark. The story start CPs (relative); nothing is written
/// when no section has a header or footer.
fn headers(t: &mut Text<'_>, doc: &Document, sections: &[(u32, SectionProps)]) -> Vec<u32> {
    let any = sections.iter().any(|(_, s)| {
        [s.headers.default, s.headers.first, s.headers.even, s.footers.default, s.footers.first, s.footers.even].iter().any(Option::is_some)
    });
    if !any {
        return Vec::new();
    }
    let start = t.cp();
    let mut cps = vec![0u32; 6];
    for (_, s) in sections {
        for id in [s.headers.even, s.headers.default, s.footers.even, s.footers.default, s.headers.first, s.footers.first] {
            cps.push(t.cp() - start);
            let Some(part) = id.and_then(|id| doc.parts.get(&id)) else { continue };
            if part.blocks.is_empty() {
                t.mark(&Paragraph::new(), PARA, &Grpprl::default());
            } else {
                t.blocks(&part.blocks, false);
            }
            t.close_fields();
            t.mark(&Paragraph::new(), PARA, &Grpprl::default());
        }
    }
    // The final mark the last CP of Plcfhdd ends at ([MS-DOC] §2.8.22).
    t.mark(&Paragraph::new(), PARA, &Grpprl::default());
    cps
}

/// Log, once per save, each kind of content the file leaves out.
fn log_dropped(d: &story::Dropped) {
    for (n, what) in [
        (d.shapes, "shapes, charts and text boxes are left out"),
        (d.comments, "comments are left out"),
        (d.nested_tables, "tables inside table cells become paragraphs of their cell"),
        (d.floating_pictures, "floating pictures are placed in line with the text"),
        (d.pictures, "pictures in formats a .doc can't hold are left out"),
        (d.equations, "equations are written as their linear text"),
    ] {
        if n > 0 {
            log::warn!(".doc save: {n} × {what}");
        }
    }
}

fn pad_to(v: &mut Vec<u8>, align: usize) {
    while !v.len().is_multiple_of(align) {
        v.push(0);
    }
}

/// The FIB for nFib 0x00C1 ([MS-DOC] §2.5): `ccp` is text, footnotes, headers, endnotes.
fn fib(cb_mac: u32, ccp: [u32; 4], pairs: &HashMap<usize, (u32, u32)>, has_pic: bool) -> Vec<u8> {
    let mut v = vec![0u8; FIB_LEN];
    let put16 = |v: &mut Vec<u8>, at: usize, x: u16| {
        if let Some(b) = v.get_mut(at..at + 2) {
            b.copy_from_slice(&x.to_le_bytes());
        }
    };
    put16(&mut v, 0x00, 0xA5EC); // wIdent
    put16(&mut v, 0x02, 0x00C1); // nFib
    put16(&mut v, 0x06, 0x0409); // lid
    // fHasPic, fWhichTblStm (1Table), fExtChar.
    put16(&mut v, 0x0A, 0x0200 | 0x1000 | if has_pic { 0x0008 } else { 0 });
    put16(&mut v, 0x0C, 0x00BF); // nFibBack
    put16(&mut v, 0x20, 0x000E); // csw
    put16(&mut v, 0x22 + 13 * 2, 0x0409); // FibRgW97.lidFE
    put16(&mut v, 0x3E, 0x0016); // cslw
    let put32 = |v: &mut Vec<u8>, at: usize, x: u32| {
        if let Some(b) = v.get_mut(at..at + 4) {
            b.copy_from_slice(&x.to_le_bytes());
        }
    };
    put32(&mut v, 0x40, cb_mac);
    put32(&mut v, 0x40 + 3 * 4, ccp[0]);
    put32(&mut v, 0x40 + 4 * 4, ccp[1]);
    put32(&mut v, 0x40 + 5 * 4, ccp[2]);
    put32(&mut v, 0x40 + 8 * 4, ccp[3]);
    put16(&mut v, 0x98, 0x005D); // cbRgFcLcb
    for (&i, &(fc, lcb)) in pairs {
        if i < 0x5D {
            put32(&mut v, 0x9A + i * 8, fc);
            put32(&mut v, 0x9A + i * 8 + 4, lcb);
        }
    }
    // cswNew = 0: the last two bytes stay zero.
    v
}

/// A Dop97 ([MS-DOC] §2.7.4): footnotes at the bottom of the page, endnotes at the end of the
/// document, both numbered from 1; the default tab stop, hyphenation zone and the settings the
/// document carries; 100 % zoom.
fn dop(doc: &Document) -> Vec<u8> {
    let s = &doc.settings;
    let mut v = vec![0u8; DOP97_LEN];
    // fFacingPages (different odd and even headers), fpc = 1 (bottom of page).
    v[0] = u8::from(s.even_odd_headers) | (1 << 5);
    v[2..4].copy_from_slice(&(1u16 << 2).to_le_bytes()); // rncFtn 0, nFtn 1
    let mut flags: u32 = 0;
    if s.auto_hyphenation {
        flags |= 1 << 12; // fAutoHyphen
    }
    if s.hyphenate_caps {
        flags |= 1 << 11; // fHyphCapitals
    }
    if s.track_changes {
        flags |= 1 << 15; // fRevMarking
    }
    if s.mirror_margins {
        flags |= 1 << 21; // fMirrorMargins
    }
    v[4..8].copy_from_slice(&flags.to_le_bytes());
    let tab = if s.default_tab > 0.0 { grpprl::twips(s.default_tab) } else { 720 };
    v[10..12].copy_from_slice(&tab.to_le_bytes()); // dxaTab
    let hot = if s.hyphenation_zone > 0.0 { grpprl::twips(s.hyphenation_zone) } else { 360 };
    v[14..16].copy_from_slice(&hot.to_le_bytes()); // dxaHotZ
    v[20..24].copy_from_slice(&dttm(&doc.core.created).to_le_bytes());
    v[24..28].copy_from_slice(&dttm(&doc.core.modified).to_le_bytes());
    v[32..34].copy_from_slice(&(doc.core.revision.clamp(1, u16::MAX as u32) as u16).to_le_bytes());
    v[52..54].copy_from_slice(&(1u16 << 2).to_le_bytes()); // rncEdn 0, nEdn 1
    v[54..56].copy_from_slice(&3u16.to_le_bytes()); // epc: end of document
    v[82..84].copy_from_slice(&((100u16 << 3) | (1 << 12)).to_le_bytes()); // pctWwdSaved, zkSaved
    v
}

/// A `DTTM` ([MS-DOC] §2.9.75) from an ISO 8601 date; 0 when there is none.
pub(crate) fn dttm(iso: &str) -> u32 {
    use chrono::{Datelike, Timelike};
    let Some(t) = parse_time(iso) else { return 0 };
    let year = t.year();
    if !(1900..=2411).contains(&year) {
        return 0;
    }
    (t.minute() & 0x3F)
        | ((t.hour() & 0x1F) << 6)
        | ((t.day() & 0x1F) << 11)
        | ((t.month() & 0xF) << 16)
        | ((((year - 1900) as u32) & 0x1FF) << 20)
        | ((t.weekday().num_days_from_sunday() & 7) << 29)
}

/// An ISO 8601 date or date-time (UTC when it doesn't say).
pub(crate) fn parse_time(iso: &str) -> Option<chrono::NaiveDateTime> {
    let s = iso.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.naive_utc());
    }
    for f in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(t) = chrono::NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), f) {
            return Some(t);
        }
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// The compound file holding `streams` at its root.
fn compound(streams: &[(&str, Vec<u8>)]) -> Result<Vec<u8>, DocbinError> {
    use std::io::Write;
    let err = |e: std::io::Error| DocbinError::Container(e.to_string());
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).map_err(err)?;
    for (name, data) in streams {
        let mut s = comp.create_stream(format!("/{name}")).map_err(err)?;
        s.write_all(data).map_err(err)?;
    }
    comp.flush().map_err(err)?;
    Ok(comp.into_inner().into_inner())
}

#[cfg(test)]
mod tests;

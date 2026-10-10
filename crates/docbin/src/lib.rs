//! WordCraft reader for the Word 97-2003 binary file format (`.doc`, [MS-DOC]).
//!
//! [`read`] turns the bytes of a `.doc` file into a [`wordcraft_doc::Document`]. There is no
//! writer: saving always goes through the other formats. The reader is written from the public
//! specification ([MS-DOC], Word (.doc) Binary File Format, Microsoft Open Specifications) and,
//! like every WordCraft parser, is lenient by design (unknown structures are skipped, bad numbers
//! fall back to defaults) and bounded (stream sizes, piece counts and property counts are
//! capped), so hostile input yields an error or a best-effort document, never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod fib;
mod fkp;
mod fmt;
mod list;
mod piece;
mod sections;
mod sprm;
mod stsh;
mod table;

use std::io::{Read, Seek};

use wordcraft_doc::para::Paragraph;
use wordcraft_doc::para::Run;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::props::NumRef;
use wordcraft_doc::section::HeaderSet;
use wordcraft_doc::styles::StyleSheet;
use wordcraft_doc::{Document, PartKind};

use fkp::Bins;
use piece::{PieceTable, PrmRef};
use sprm::Prl;
use table::ParaOut;

/// Largest stream we materialise (bytes); the engine already caps whole files at 2 GB.
const MAX_STREAM: u64 = 1 << 30;
/// Most header/footer parts we create (matches the docx reader's cap).
const MAX_PARTS: usize = 50_000;
/// `sprmPIlvl` / `sprmPIlfo`: the paragraph's list level and list.
const P_ILVL: u16 = 0x260A;
const P_ILFO: u16 = 0x460B;

/// Errors from reading a Word 97-2003 binary file.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum DocbinError {
    /// The bytes are not a readable compound file (OLE2/CFB container).
    #[error("not a Word document (compound file): {0}")]
    Container(String),
    /// The compound file is readable but is not a Word 97-2003 document.
    #[error("not a Word 97-2003 document: {0}")]
    NotWord(String),
    /// The document is password-protected (XOR obfuscation or RC4 encryption).
    #[error("encrypted or password-protected .doc files are not supported")]
    Encrypted,
    /// The input exceeds a safety limit (stream size, piece count…).
    #[error("limit exceeded: {0}")]
    Limit(String),
    /// A structure is malformed beyond recovery.
    #[error("malformed document: {0}")]
    Malformed(String),
}

/// Read a `.doc` (Word 97-2003 binary) file into a [`Document`].
pub fn read(bytes: &[u8]) -> Result<Document, DocbinError> {
    let mut comp = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).map_err(|e| DocbinError::Container(e.to_string()))?;
    let word = stream(&mut comp, "WordDocument").ok_or_else(|| DocbinError::NotWord("no WordDocument stream".into()))?;
    if word.len() as u64 > MAX_STREAM {
        return Err(DocbinError::Limit(format!("WordDocument stream is {} bytes", word.len())));
    }
    let fib = fib::Fib::parse(&word)?;
    if fib.encrypted {
        return Err(DocbinError::Encrypted);
    }
    // The FIB names the Table stream (0Table or 1Table); fall back to whichever exists,
    // since files written by other writers sometimes carry the wrong bit.
    let named = if fib.which_tbl_stm { "1Table" } else { "0Table" };
    let other = if fib.which_tbl_stm { "0Table" } else { "1Table" };
    let table = stream(&mut comp, named).or_else(|| stream(&mut comp, other)).unwrap_or_default();
    if table.len() as u64 > MAX_STREAM {
        return Err(DocbinError::Limit(format!("Table stream is {} bytes", table.len())));
    }

    // The piece table locates every character in the WordDocument stream.
    let (fc_clx, lcb_clx) =
        fib.pair(fib::pair::CLX).filter(|(_, lcb)| *lcb > 0).ok_or_else(|| DocbinError::Malformed("the FIB has no piece table (Clx)".into()))?;
    let pieces = piece::PieceTable::parse(&table, fc_clx, lcb_clx)?;

    // Styles, fonts and the formatting bin tables; each is optional in broken files.
    let fonts = fib.pair(fib::pair::STTBF_FFN).map(|(f, l)| stsh::fonts(&table, f, l)).unwrap_or_default();
    let raw_styles = match fib.pair(fib::pair::STSHF) {
        Some((f, l)) => stsh::parse(&table, f, l)?,
        None => Vec::new(),
    };
    let chpx_bins = match fib.pair(fib::pair::PLCF_BTE_CHPX) {
        Some((f, l)) => Bins::parse(&table, f, l)?,
        None => Bins::parse(&[], 0, 0)?,
    };
    let papx_bins = match fib.pair(fib::pair::PLCF_BTE_PAPX) {
        Some((f, l)) => Bins::parse(&table, f, l)?,
        None => Bins::parse(&[], 0, 0)?,
    };

    let mut doc = Document::new();
    doc.body.clear();
    doc.styles = fmt::stylesheet(&raw_styles, &fonts);
    doc.numbering = list::parse(&table, &fib, &fonts);
    let sheet: StyleSheet = doc.styles.clone();
    let sects = sections::parse(&word, &table, &fib);
    let stories = sections::header_stories(&table, &fib);
    // The header subdocument begins after the main text and the footnotes; its stories' CPs
    // are relative to it.
    let hdd_base = fib.ccp.text + fib.ccp.ftn;

    // Header and footer parts: skip the six separator stories, then six stories per section
    // in the order even header, odd (default) header, even footer, odd (default) footer,
    // first header, first footer. Empty stories inherit from the previous section.
    let mut hf_ids: Vec<(usize, usize, u32)> = Vec::new();
    for si in 0..sects.len() {
        for k in 0..6usize {
            let Some((a, b)) = stories.get(6 + si * 6 + k) else { break };
            if b <= a || doc.parts.len() >= MAX_PARTS {
                continue;
            }
            let kind = match k {
                0 | 1 | 4 => PartKind::Header,
                _ => PartKind::Footer,
            };
            let out = walk(&word, &pieces, &chpx_bins, &papx_bins, hdd_base + a, hdd_base + b, &fonts, &raw_styles, &sheet);
            let blocks = table::assemble(out);
            if !blocks.is_empty() {
                let id = doc.add_part(kind, blocks);
                hf_ids.push((si, k, id));
            }
        }
    }

    // Sections: each section's properties attach to the paragraph that ends it (its story of
    // header/footer parts included); the section reaching the end of the text is the final one.
    let mut paras = walk(&word, &pieces, &chpx_bins, &papx_bins, 0, fib.ccp.text, &fonts, &raw_styles, &sheet);
    let last = sects.last().map(|s| s.props.clone());
    for (si, sec) in sects.iter().enumerate() {
        let mut props = sec.props.clone();
        let id_of = |k: usize| hf_ids.iter().find(|(s, kk, _)| *s == si && *kk == k).map(|(_, _, id)| *id);
        props.headers = HeaderSet { even: id_of(0), default: id_of(1), first: id_of(4) };
        props.footers = HeaderSet { even: id_of(2), default: id_of(3), first: id_of(5) };
        let is_final = si + 1 == sects.len() || sec.end_cp >= fib.ccp.text;
        if is_final {
            continue; // handled below through `last`
        }
        for p in paras.iter_mut() {
            if p.end_cp == sec.end_cp {
                p.para.section = Some(Box::new(props));
                break;
            }
        }
    }
    if let Some(mut l) = last {
        let si = sects.len().saturating_sub(1);
        let id_of = |k: usize| hf_ids.iter().find(|(s, kk, _)| *s == si && *kk == k).map(|(_, _, id)| *id);
        l.headers = HeaderSet { even: id_of(0), default: id_of(1), first: id_of(4) };
        l.footers = HeaderSet { even: id_of(2), default: id_of(3), first: id_of(5) };
        doc.last_section = l;
    }

    doc.body = table::assemble(paras);
    doc.ensure_nonempty();
    Ok(doc)
}

/// State while assembling one paragraph from per-CP formatting.
#[derive(Default)]
struct ParaBuild {
    text: String,
    runs: Vec<Run>,
    props: wordcraft_doc::props::ParaProps,
    mark: CharProps,
    /// The current run's properties, so runs only break when formatting changes.
    cur: Option<CharProps>,
}

impl ParaBuild {
    fn push(&mut self, c: char, props: CharProps) {
        let len = c.len_utf8();
        match &self.cur {
            Some(p) if *p == props => {
                if let Some(r) = self.runs.last_mut() {
                    r.len += len;
                }
            }
            _ => {
                self.runs.push(Run { len, props: props.clone() });
                self.cur = Some(props);
            }
        }
        self.text.push(c);
    }
}

/// Walk a CP range, splitting paragraphs at 0x0D/0x07 marks, applying the direct character
/// formatting of each CP and the paragraph formatting of each mark. Works for the main
/// document and for any subdocument range (headers, footers, notes…).
fn walk(
    word: &[u8],
    pieces: &PieceTable,
    chpx_bins: &Bins,
    papx_bins: &Bins,
    cp_start: u32,
    cp_end: u32,
    fonts: &[String],
    raw_styles: &[stsh::RawStyle],
    sheet: &StyleSheet,
) -> Vec<ParaOut> {
    let mut out = Vec::new();
    let mut pb = ParaBuild::default();
    let mut cp = cp_start;
    while cp < cp_end {
        let c = pieces.text(word, cp, cp + 1).chars().next().unwrap_or('\u{FFFD}');
        match c {
            '\r' | '\u{7}' => {
                // Paragraph mark: its PAPX (queried at the FC of the *next* character)
                // carries the paragraph's style and direct formatting; its CHPX the mark's.
                let fc_next = pieces.fc_of_cp(cp).and_then(|f| f.checked_add(2)).unwrap_or(0);
                let (istd, papx) = papx_bins.papx(word, fc_next);
                if let Some(st) = raw_styles.get(istd as usize).filter(|s| !s.name.is_empty()) {
                    pb.props.style = Some(fmt::style_id(&st.name));
                }
                let mut row = table::decode(papx);
                row.cell_mark = c == '\u{7}';
                let mut ilvl = 0u8;
                let mut ilfo = 0i32;
                for prl in sprm::iter(papx) {
                    if prl.op == P_ILVL {
                        ilvl = prl.operand.first().copied().unwrap_or(0).min(8);
                        continue;
                    }
                    if prl.op == P_ILFO {
                        ilfo = match prl.operand {
                            [b0, b1, ..] => i16::from_le_bytes([*b0, *b1]) as i32,
                            _ => 0,
                        };
                        continue;
                    }
                    if prl.sgc() == 1 {
                        fmt::apply_para(&mut pb.props, &prl);
                    }
                }
                if let Some(num) = num_of(ilfo) {
                    pb.props.numbering = Some(NumRef { num, level: ilvl });
                }
                let fc = pieces.fc_of_cp(cp).unwrap_or(0);
                pb.mark = char_props(chpx_bins.chpx(word, fc), fonts, &CharProps::default());
                let done = std::mem::take(&mut pb);
                out.push(ParaOut {
                    para: Paragraph { text: done.text, runs: done.runs, props: done.props, mark: done.mark, ..Default::default() },
                    end_cp: cp + 1,
                    row,
                });
            }
            '\t' => push_formatted(&mut pb, cp, '\t', word, pieces, chpx_bins, fonts, sheet),
            '\u{B}' => push_formatted(&mut pb, cp, '\n', word, pieces, chpx_bins, fonts, sheet),
            '\u{C}' => push_formatted(&mut pb, cp, '\u{C}', word, pieces, chpx_bins, fonts, sheet),
            '\u{E}' => push_formatted(&mut pb, cp, '\u{E}', word, pieces, chpx_bins, fonts, sheet),
            '\u{1E}' => push_formatted(&mut pb, cp, '\u{2011}', word, pieces, chpx_bins, fonts, sheet),
            '\u{1F}' => push_formatted(&mut pb, cp, '\u{AD}', word, pieces, chpx_bins, fonts, sheet),
            // Field characters (0x13/0x14/0x15), object anchors and note references
            // (0x01–0x08) are handled by later passes; for now they are dropped.
            c if (c as u32) < 0x20 => {}
            c => push_formatted(&mut pb, cp, c, word, pieces, chpx_bins, fonts, sheet),
        }
        cp += c.len_utf16().max(1) as u32;
    }
    if !pb.text.is_empty() {
        out.push(ParaOut {
            para: Paragraph { text: pb.text, runs: pb.runs, props: pb.props, mark: pb.mark, ..Default::default() },
            end_cp: cp_end,
            row: table::RowInfo::default(),
        });
    }
    out
}

/// `sprmPIlfo` operand → the num id, if the paragraph is in a list. Values 0xF802-0xFFFF are
/// the negation of a 1-based index and keep the paragraph's own indents, which we do by
/// leaving the level's indents out of the paragraph (they never enter `ParaProps` anyway).
fn num_of(ilfo: i32) -> Option<u32> {
    match ilfo {
        0x0001..=0x07FE => Some(ilfo as u32),
        0xF802..=0xFFFF => Some((-(ilfo as i16)) as i32 as u32),
        _ => None,
    }
}

/// Resolve the character properties for the character at `cp`: the CHPX grpprl of its FC,
/// plus the piece's Prm, over the paragraph style's base character props.
fn push_formatted(pb: &mut ParaBuild, cp: u32, c: char, word: &[u8], pieces: &PieceTable, chpx_bins: &Bins, fonts: &[String], sheet: &StyleSheet) {
    let fc = pieces.fc_of_cp(cp).unwrap_or(0);
    let grpprl = chpx_bins.chpx(word, fc);
    let base = pb.props.style.as_deref().and_then(|id| sheet.get(id)).map(|s| s.chr.clone()).unwrap_or_default();
    let mut props = char_props(grpprl, fonts, &base);
    apply_prm(&mut props, pieces.piece_prm(cp), fonts, &base);
    pb.push(c, props);
}

/// Apply a piece's Prm (Prm0 short sprm or Prm1 grpprl) to character properties.
fn apply_prm(props: &mut CharProps, prm: PrmRef<'_>, fonts: &[String], base: &CharProps) {
    match prm {
        PrmRef::None => {}
        PrmRef::Sprm0 { isprm, val } => {
            if let Some(op) = sprm::from_prm0(isprm) {
                let operand = [val];
                let prl = Prl { op, operand: &operand };
                if prl.sgc() == 2 {
                    fmt::apply_char(props, &prl, base, fonts);
                }
            }
        }
        PrmRef::Grpprl(Some(g)) => {
            for prl in sprm::iter(g) {
                if prl.sgc() == 2 {
                    fmt::apply_char(props, &prl, base, fonts);
                }
            }
        }
        PrmRef::Grpprl(None) => {}
    }
}

/// Character props from a CHPX grpprl applied over `base`.
fn char_props(grpprl: &[u8], fonts: &[String], base: &CharProps) -> CharProps {
    let mut p = base.clone();
    for prl in sprm::iter(grpprl) {
        if prl.sgc() == 2 {
            fmt::apply_char(&mut p, &prl, base, fonts);
        }
    }
    p
}

/// Read a root-level stream fully, or `None` if it is absent.
fn stream<F: Read + Seek>(comp: &mut cfb::CompoundFile<F>, name: &str) -> Option<Vec<u8>> {
    let mut s = comp.open_stream(format!("/{name}")).ok()?;
    let mut v = Vec::new();
    // Read with a cap one byte above the limit so oversized streams are still caught by the
    // caller instead of exhausting memory here.
    s.by_ref().take(MAX_STREAM + 1).read_to_end(&mut v).ok()?;
    Some(v)
}

#[cfg(test)]
mod tests;

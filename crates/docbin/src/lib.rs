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
mod piece;
mod sprm;
mod stsh;

use std::io::{Read, Seek};

use wordcraft_doc::para::Paragraph;
use wordcraft_doc::para::Run;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::styles::StyleSheet;
use wordcraft_doc::{Document, para_block};

use fkp::Bins;
use piece::{PieceTable, PrmRef};
use sprm::Prl;

/// Largest stream we materialise (bytes); the engine already caps whole files at 2 GB.
const MAX_STREAM: u64 = 1 << 30;

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
    let sheet: &StyleSheet = &doc.styles;
    doc.body = walk(&word, &pieces, &chpx_bins, &papx_bins, fib.ccp.text, &fonts, &raw_styles, sheet);
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

/// Walk the main document's CPs, splitting paragraphs at 0x0D/0x07 marks, applying the
/// direct character formatting of each CP and the paragraph formatting of each mark.
fn walk(
    word: &[u8],
    pieces: &PieceTable,
    chpx_bins: &Bins,
    papx_bins: &Bins,
    ccp_text: u32,
    fonts: &[String],
    raw_styles: &[stsh::RawStyle],
    sheet: &StyleSheet,
) -> wordcraft_doc::Blocks {
    let mut blocks = Vec::new();
    let mut pb = ParaBuild::default();
    let mut cp = 0u32;
    while cp < ccp_text {
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
                for prl in sprm::iter(papx) {
                    if prl.sgc() == 1 {
                        fmt::apply_para(&mut pb.props, &prl);
                    }
                }
                let fc = pieces.fc_of_cp(cp).unwrap_or(0);
                pb.mark = char_props(chpx_bins.chpx(word, fc), fonts, &CharProps::default());
                let done = std::mem::take(&mut pb);
                blocks.push(para_block(Paragraph { text: done.text, runs: done.runs, props: done.props, mark: done.mark, ..Default::default() }));
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
        blocks.push(para_block(Paragraph { text: pb.text, runs: pb.runs, props: pb.props, mark: pb.mark, ..Default::default() }));
    }
    blocks
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

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
mod piece;

use std::io::{Read, Seek};

use wordcraft_doc::Document;
use wordcraft_doc::para::Paragraph;
use wordcraft_doc::para_block;

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
        fib.pair(fib::PAIR_CLX).filter(|(_, lcb)| *lcb > 0).ok_or_else(|| DocbinError::Malformed("the FIB has no piece table (Clx)".into()))?;
    let pieces = piece::PieceTable::parse(&table, fc_clx, lcb_clx)?;
    let main = pieces.text(&word, 0, fib.ccp.text);

    let mut doc = Document::new();
    doc.body.clear();
    doc.body = paragraphs(&main);
    doc.ensure_nonempty();
    Ok(doc)
}

/// Split stored text into paragraphs at paragraph and cell/row marks (0x0D, 0x07), mapping
/// Word's special characters to the model's ([MS-DOC] §2.3.7 "special characters").
fn paragraphs(text: &str) -> wordcraft_doc::Blocks {
    let mut blocks = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        match c {
            '\r' | '\u{7}' => {
                blocks.push(wordcraft_doc::para_block(Paragraph::with_text(&cur, Default::default())));
                cur.clear();
            }
            '\t' => cur.push('\t'),
            '\u{B}' => cur.push('\n'),
            '\u{C}' => cur.push('\u{C}'),
            '\u{E}' => cur.push('\u{E}'),
            '\u{1E}' => cur.push('\u{2011}'),
            '\u{1F}' => cur.push('\u{AD}'),
            // Field characters (0x13/0x14/0x15), object anchors and note references
            // (0x01–0x08) are handled by later passes; for now they are dropped.
            c if (c as u32) < 0x20 => {}
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        blocks.push(para_block(Paragraph::with_text(&cur, Default::default())));
    }
    blocks
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

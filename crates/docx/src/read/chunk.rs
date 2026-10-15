//! `w:altChunk` (ECMA-376 §17.17.2.1): content kept in another format (HTML, MHT, RTF, plain
//! text, another WordprocessingML document) that Word brings into the document where the chunk
//! stands when it opens the file. A nested WordprocessingML package is read here; other formats
//! go to the caller's [`ChunkImporter`] (the format crates sit beside this one), and without one
//! the chunk is left out.

use std::sync::Arc;

use wordcraft_doc::{Block, Blocks, Document};

use super::story::StoryCtx;
use super::{Reader, part_of};
use crate::package::{ContentTypes, MAX_PART, MAX_TOTAL, Package, Rels, rt};
use crate::xml::El;

/// Most chunks brought in from one file (nested documents' chunks included).
pub(crate) const MAX_CHUNKS: usize = 256;
/// Deepest a WordprocessingML chunk may nest in another.
pub(crate) const MAX_CHUNK_NESTING: u8 = 4;

/// An alternative format chunk's content, as a [`ChunkImporter`] gets it.
pub struct AltChunk<'a> {
    /// The part's content type (`application/xhtml+xml`, `message/rfc822`, `application/rtf`,
    /// `text/plain`…), empty when the package doesn't give one.
    pub content_type: &'a str,
    /// The part's name (`word/afchunk.mht`): its extension tells the format when the content type
    /// doesn't.
    pub part: &'a str,
    pub bytes: &'a [u8],
}

/// Turns a chunk the docx reader can't read itself into a document whose body takes the chunk's
/// place; `None` leaves the chunk out.
pub type ChunkImporter<'a> = &'a dyn Fn(&AltChunk) -> Option<Document>;

/// How [`crate::read_with`] reads a package.
#[derive(Clone, Copy, Default)]
pub struct ReadOptions<'a> {
    /// The password of an encrypted package.
    pub password: Option<&'a str>,
    /// Imports `w:altChunk` content in formats other than WordprocessingML.
    pub chunks: Option<ChunkImporter<'a>>,
}

impl Reader<'_> {
    /// Bring the chunk `e` names into `out` where it stands (`depth`: of the XML around it).
    pub(crate) fn read_alt_chunk(&mut self, sc: &mut StoryCtx, e: &El, rels: &Rels, out: &mut Blocks, depth: usize) {
        let Some(part) = e.attr("r:id").and_then(|id| part_of(rels, id, rt::AF_CHUNK)) else { return };
        let pkg = self.pkg;
        let Some(bytes) = pkg.get(&part) else {
            log::warn!("docx: alternative format chunk {part} is missing");
            return;
        };
        // Every chunk's bytes count against one budget for the whole file, however often the same
        // part is named and however deep chunks nest.
        if self.chunks.left == 0 || bytes.len() as u64 > self.chunks.bytes {
            log::warn!("docx: more than {MAX_CHUNKS} alternative format chunks or {MAX_TOTAL} bytes of them; the rest are left out");
            return;
        }
        self.chunks.left -= 1;
        self.chunks.bytes -= bytes.len() as u64;
        let ct = self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(&part).unwrap_or("").to_string();
        let chunk = AltChunk { content_type: &ct, part: &part, bytes };
        // A WordprocessingML chunk is a package; one that isn't Word's fails to read (logged).
        let is_docx = bytes.starts_with(b"PK\x03\x04");
        let doc = if is_docx { self.read_nested(bytes, depth) } else { self.chunks.import.and_then(|f| f(&chunk)) };
        let Some(mut doc) = doc else {
            log::warn!("docx: alternative format chunk {part} ({ct}) is in a format WordCraft can't bring in");
            return;
        };
        // The chunk flows in the section it stands in; its own section breaks don't come along.
        for b in &mut doc.body {
            if let Block::Para(p) = &**b
                && p.section.is_some()
                && let Block::Para(p) = Arc::make_mut(b)
            {
                p.section = None;
            }
        }
        // Bookmarks and comment marks read before the chunk stay before it.
        self.flush_pending(sc, out);
        // `w:matchSrc`: a Word document's content keeps its own styles (other formats' formatting
        // is their own already).
        let keep = is_docx && e.child("w:altChunkPr").and_then(|p| p.child("w:matchSrc")).is_some_and(crate::units::on_off);
        out.extend(self.doc.adopt_blocks(doc, keep).into_iter().map(Arc::new));
    }

    /// A WordprocessingML chunk: its document, read with the same importer and what is left of
    /// the budgets, its XML depth counting on from `depth`.
    fn read_nested(&mut self, bytes: &[u8], depth: usize) -> Option<Document> {
        if self.chunks.nesting >= MAX_CHUNK_NESTING {
            log::warn!("docx: alternative format chunks nest deeper than {MAX_CHUNK_NESTING}; the inner ones are left out");
            return None;
        }
        let pkg =
            Package::open_limited(bytes, MAX_PART, self.chunks.bytes).map_err(|e| log::warn!("docx: unreadable nested document chunk: {e}")).ok()?;
        self.chunks.bytes = self.chunks.bytes.saturating_sub(pkg.size());
        let state = ChunkState { nesting: self.chunks.nesting + 1, depth: depth + 1, ..self.chunks };
        let (doc, left, bytes) = super::read_opened_in(&pkg, state).map_err(|e| log::warn!("docx: unreadable nested document chunk: {e}")).ok()?;
        self.chunks.left = left;
        self.chunks.bytes = bytes;
        Some(doc)
    }
}

/// What a reader may still bring in from chunks.
#[derive(Clone, Copy)]
pub(crate) struct ChunkState<'a> {
    /// Chunks left in the file's budget ([`MAX_CHUNKS`]).
    pub left: usize,
    /// Chunk bytes (nested packages unpacked) left in the file's budget.
    pub bytes: u64,
    /// How deep in nested WordprocessingML chunks this reader is.
    pub nesting: u8,
    /// The XML depth its stories start at (a nested document's: that of the chunk naming it).
    pub depth: usize,
    pub import: Option<ChunkImporter<'a>>,
}

impl<'a> ChunkState<'a> {
    pub fn new(import: Option<ChunkImporter<'a>>) -> Self {
        ChunkState { left: MAX_CHUNKS, bytes: MAX_TOTAL, nesting: 0, depth: 0, import }
    }
}

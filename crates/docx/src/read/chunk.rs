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
        let doc = if bytes.starts_with(b"PK\x03\x04") { self.read_nested(bytes, depth) } else { self.chunks.import.and_then(|f| f(&chunk)) };
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
        let mut blocks = self.doc.adopt_blocks(doc, &mut self.next_perm);
        // Bookmarks and comment marks read before the chunk stay before it.
        self.flush_pending_before(sc, out, &mut blocks);
        out.extend(blocks.into_iter().map(Arc::new));
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
        let (doc, left, bytes, graphic) =
            super::read_opened_in(&pkg, state).map_err(|e| log::warn!("docx: unreadable nested document chunk: {e}")).ok()?;
        self.chunks.left = left;
        self.chunks.bytes = bytes;
        self.chunks.graphic = graphic;
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
    /// Graphic work left in the file's budget ([`super::MAX_GRAPHIC_WORK`]): nested documents'
    /// charts and diagrams count against it too.
    pub graphic: usize,
    /// How deep in nested WordprocessingML chunks this reader is.
    pub nesting: u8,
    /// The XML depth its stories start at (a nested document's: that of the chunk naming it).
    pub depth: usize,
    pub import: Option<ChunkImporter<'a>>,
}

impl<'a> ChunkState<'a> {
    pub fn new(import: Option<ChunkImporter<'a>>) -> Self {
        ChunkState { left: MAX_CHUNKS, bytes: MAX_TOTAL, graphic: super::MAX_GRAPHIC_WORK, nesting: 0, depth: 0, import }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use wordcraft_doc::{Block, Document, InlineObject};

    use super::ChunkState;
    use crate::package::Package;

    /// A package whose body is `body`, with the document relationship `rel` (id, type, target) to
    /// the part `(name, bytes)`.
    fn package(body: &str, (id, ty, target): (&str, &str, &str), (name, bytes): (&str, &[u8])) -> Vec<u8> {
        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
        let rels =
            |inner: &str| format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{inner}</Relationships>"#);
        let rel = |id: &str, ty: &str, target: &str| {
            format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{ty}" Target="{target}"/>"#)
        };
        let files: [(&str, Vec<u8>); 4] = [
            ("_rels/.rels", rels(&rel("rId1", "officeDocument", "word/document.xml")).into_bytes()),
            ("word/document.xml", format!(r#"<w:document {ns}><w:body>{body}</w:body></w:document>"#).into_bytes()),
            ("word/_rels/document.xml.rels", rels(&rel(id, ty, target)).into_bytes()),
            (name, bytes.to_vec()),
        ];
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zw.write_all(&bytes).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }

    /// Whether the body's first paragraph's chart is drawn.
    fn drawn(d: &Document) -> bool {
        let Some(Block::Para(p)) = d.body.first().map(|b| &**b) else { panic!("no paragraph") };
        p.objects.iter().any(|o| matches!(o, InlineObject::Graphic { graphic, .. } if !graphic.items.is_empty()))
    }

    #[test]
    fn a_nested_documents_charts_count_against_the_files_graphic_budget() {
        let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let drawing = r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="2743200" cy="1828800"/><wp:docPr id="5" name="G"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rIdChart"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
        let inner = package(drawing, ("rIdChart", "chart", "charts/chart1.xml"), ("word/charts/chart1.xml", chart.as_bytes()));
        let outer = package(r#"<w:altChunk r:id="rIdC"/>"#, ("rIdC", "aFChunk", "afchunk.docx"), ("word/afchunk.docx", &inner));
        let pkg = Package::open(&outer).unwrap();
        let read = |graphic| {
            let (d, .., left) = super::super::read_opened_in(&pkg, ChunkState { graphic, ..ChunkState::new(None) }).unwrap();
            (drawn(&d), left)
        };
        let (shown, left) = read(super::super::MAX_GRAPHIC_WORK);
        assert!(shown);
        assert!(left < super::super::MAX_GRAPHIC_WORK, "the nested chart's work is the file's");
        // With the file's budget spent, the nested document doesn't get one of its own.
        assert_eq!(read(0), (false, 0));
    }
}

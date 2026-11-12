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

/// ZIP permits arbitrary prefixes and trailing data. Check complete end-record candidates,
/// rather than a bare signature in text, before treating an unknown chunk as a package.
fn package_candidate(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"PK\x03\x04") {
        return true;
    }
    let mut central_header = false;
    let mut end_record = false;
    for (at, signature) in bytes.windows(4).enumerate() {
        if signature == b"PK\x01\x02" {
            central_header = true;
        } else if signature == b"PK\x05\x06"
            && let Some(record) = bytes.get(at..at.saturating_add(22))
        {
            let comment = usize::from(u16::from_le_bytes([record[20], record[21]]));
            end_record |= at.checked_add(22).and_then(|n| n.checked_add(comment)).is_some_and(|end| end <= bytes.len());
        }
        if central_header && end_record {
            return true;
        }
    }
    false
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
        if self.mc_error.borrow().is_some() {
            return;
        }
        let ct = self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(&part).unwrap_or("").to_string();
        let word_named = part.rsplit_once('.').is_some_and(|(_, ext)| ["docx", "docm", "dotx", "dotm"].iter().any(|x| ext.eq_ignore_ascii_case(x)))
            || ct.to_ascii_lowercase().contains("wordprocessingml");
        // A package part can be referenced repeatedly under different path spellings; key by its
        // stable storage instead of rescanning a potentially large part for every reference.
        let is_docx = *self.chunk_package_candidates.entry((bytes.as_ptr() as usize, bytes.len())).or_insert_with(|| package_candidate(bytes));
        // Every chunk's bytes count against one budget for the whole file, however often the same
        // part is named and however deep chunks nest. An uninspected package may have requirements.
        if self.chunks.left == 0 || bytes.len() as u64 > self.chunks.bytes {
            if is_docx || word_named {
                self.mc_error.borrow_mut().get_or_insert_with(|| {
                    crate::DocxError::Limit(format!("cannot inspect alternative format chunk {part}: chunk count or byte budget exhausted"))
                });
            } else {
                log::warn!("docx: more than {MAX_CHUNKS} alternative format chunks or {MAX_TOTAL} bytes of them; the rest are left out");
            }
            return;
        }
        self.chunks.left -= 1;
        self.chunks.bytes -= bytes.len() as u64;
        let chunk = AltChunk { content_type: &ct, part: &part, bytes };
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
        // `w:matchSrc`: a Word document's content keeps its own styles (other formats' formatting
        // is their own already).
        let keep = is_docx && e.child("w:altChunkPr").and_then(|p| p.child("w:matchSrc")).is_some_and(crate::units::on_off);
        // The pictures the chunk's own reader dropped are the file's too (strict export, #6005),
        // each named by the chunk and its part in it (`word/afchunk1.docx > word/header1.xml`).
        // A chunk in another format brings none: those importers record nothing yet (#6007).
        let nested = std::mem::take(&mut doc.missing_pictures);
        let list_len = nested.list.len() as u64;
        for mut item in nested.list {
            let inner = if item.story.is_empty() { nested.main.as_str() } else { item.story.as_str() };
            item.story = format!("{part} > {inner}");
            self.doc.missing_pictures.push(item);
        }
        // The nested records past its list are the file's too; their stories carry the chunk's
        // part, so none is one the outer list already counted.
        let unlisted = nested.total.saturating_sub(list_len);
        self.doc.missing_pictures.add_unlisted(&part, unlisted);
        self.doc.missing_pictures.truncated |= nested.truncated;
        let mut blocks = self.doc.adopt_blocks(doc, keep, &mut self.next_perm);
        // Bookmarks and comment marks read before the chunk stay before it.
        self.flush_pending_before(sc, out, &mut blocks);
        out.extend(blocks.into_iter().map(Arc::new));
    }

    /// A WordprocessingML chunk: its document, read with the same importer and what is left of
    /// the budgets, its XML depth counting on from `depth`.
    fn read_nested(&mut self, bytes: &[u8], depth: usize) -> Option<Document> {
        if self.chunks.nesting >= MAX_CHUNK_NESTING {
            // The unread package may contain required markup or MustUnderstand directives.
            self.mc_error
                .borrow_mut()
                .get_or_insert_with(|| crate::DocxError::Limit(format!("alternative format chunks nest deeper than {MAX_CHUNK_NESTING}")));
            return None;
        }
        let pkg = Package::open_limited(bytes, MAX_PART, self.chunks.bytes)
            .map_err(|e| {
                // A budget refusal is not evidence that the unread chunk contains no required
                // markup, so it must not be silently discarded as an optional broken chunk.
                if matches!(e, crate::DocxError::Limit(_)) {
                    self.mc_error.borrow_mut().get_or_insert_with(|| e.clone());
                }
                log::warn!("docx: unreadable nested document chunk: {e}");
            })
            .ok()?;
        self.chunks.bytes = self.chunks.bytes.saturating_sub(pkg.size());
        let state = ChunkState { nesting: self.chunks.nesting + 1, depth: depth + 1, ..self.chunks };
        let (doc, left, bytes, graphic, ink) = super::read_opened_in(&pkg, state)
            .map_err(|e| {
                if e.fatal {
                    self.mc_error.borrow_mut().get_or_insert_with(|| e.error.clone());
                }
                log::warn!("docx: unreadable nested document chunk: {}", e.error);
            })
            .ok()?;
        if doc.passthrough.contains_key(crate::write::REQUIRED_SOURCE_GUARD) {
            self.required_source.set(true);
        }
        self.chunks.left = left;
        self.chunks.bytes = bytes;
        self.chunks.graphic = graphic;
        self.chunks.ink = ink;
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
    /// Ink parsing and geometry work left in the file, also shared by nested documents.
    /// Failed attempts remain charged without starving unrelated chart/diagram previews.
    pub ink: usize,
    /// How deep in nested WordprocessingML chunks this reader is.
    pub nesting: u8,
    /// The XML depth its stories start at (a nested document's: that of the chunk naming it).
    pub depth: usize,
    pub import: Option<ChunkImporter<'a>>,
}

impl<'a> ChunkState<'a> {
    pub fn new(import: Option<ChunkImporter<'a>>) -> Self {
        ChunkState { left: MAX_CHUNKS, bytes: MAX_TOTAL, graphic: super::MAX_GRAPHIC_WORK, ink: super::ink::MAX_WORK, nesting: 0, depth: 0, import }
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
    fn nested_ink_spends_one_file_wide_ink_allowance_without_spending_chart_work() {
        use wordcraft_doc::graphic::GraphicItem;
        let ink = br#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0,1 1</trace></ink>"#;
        let inner = package(r#"<w:p><w:r><w:contentPart r:id="rInk"/></w:r></w:p>"#, ("rInk", "customXml", "ink.xml"), ("word/ink.xml", ink));
        let mut zip = zip::ZipWriter::new_append(std::io::Cursor::new(inner)).unwrap();
        zip.start_file("[Content_Types].xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/ink.xml" ContentType="application/inkml+xml"/></Types>"#).unwrap();
        let inner = zip.finish().unwrap().into_inner();
        let read = |copies: usize, ink| {
            let outer = package(&r#"<w:altChunk r:id="rIdC"/>"#.repeat(copies), ("rIdC", "aFChunk", "chunk.docx"), ("word/chunk.docx", &inner));
            let pkg = Package::open(&outer).unwrap();
            let (d, _, _, graphic, left) = super::super::read_opened_in(&pkg, ChunkState { ink, ..ChunkState::new(None) }).unwrap();
            assert_eq!(graphic, super::super::MAX_GRAPHIC_WORK);
            let vectors = d
                .body
                .iter()
                .filter_map(|b| match &**b {
                    Block::Para(p) => Some(p),
                    _ => None,
                })
                .flat_map(|p| &p.objects)
                .filter(|o| matches!(o, InlineObject::Graphic { graphic, .. } if matches!(graphic.items.first(), Some(GraphicItem::Path { .. }))))
                .count();
            (vectors, left)
        };
        let limit = super::super::ink::MAX_WORK;
        let (vectors, left) = read(1, limit);
        assert_eq!(vectors, 1);
        assert!(left < limit);
        assert_eq!(read(2, limit), (2, limit - 2 * (limit - left)));
        assert_eq!(read(2, limit - left), (1, 0), "the second nested document must not reset the ink budget");
        assert_eq!(read(1, 0), (0, 0));
    }

    #[test]
    fn unsupported_must_understand_in_a_nested_document_is_not_dropped() {
        let wrap = |body: &str| {
            let inner = package(body, ("rIdUnused", "unused", "unused.bin"), ("word/unused.bin", b""));
            package(r#"<w:altChunk r:id="rIdC"/>"#, ("rIdC", "aFChunk", "afchunk.docx"), ("word/afchunk.docx", &inner))
        };
        // A bound but unsupported requirement is detected after parsing; an unbound one is
        // detected while parsing. Both must propagate through the outer optional chunk.
        for required in [r#"xmlns:f="urn:future" mc:MustUnderstand="f""#, r#"mc:MustUnderstand="missing""#] {
            let body = format!(
                r#"<w:p xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" {required}><w:r><w:t>required</w:t></w:r></w:p>"#
            );
            assert!(matches!(crate::read(&wrap(&body)), Err(crate::DocxError::Xml(_))));
        }
        // The classification must also survive more than one optional-chunk boundary.
        let body = r#"<w:p xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:f="urn:future" mc:MustUnderstand="f"/>"#;
        let middle = wrap(body);
        let outer = package(r#"<w:altChunk r:id="rIdC"/>"#, ("rIdC", "aFChunk", "afchunk.docx"), ("word/afchunk.docx", &middle));
        assert!(matches!(crate::read(&outer), Err(crate::DocxError::Xml(_))));
        // A generally malformed nested document still follows the established optional-chunk
        // policy, rather than being misclassified just because its public error is also Xml.
        assert!(crate::read(&wrap("<w:p><w:r></w:p>")).is_ok());
    }

    #[test]
    fn chunk_budgets_do_not_hide_nested_required_content() {
        for body in [
            r#"<w:p xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:f="urn:future" mc:MustUnderstand="f"/>"#,
            r#"<w:p xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:f="urn:future" mc:Ignorable="f" mc:PreserveElements="f:*"><f:opaque/></w:p>"#,
        ] {
            let inner = package(body, ("unused", "unused", "unused.bin"), ("word/unused.bin", b""));
            let mut prefixed = b"self-extracting stub".to_vec();
            prefixed.extend_from_slice(&inner);
            prefixed.extend_from_slice(b"trailing data");
            for bytes in [&inner, &prefixed] {
                let outer = package(r#"<w:altChunk r:id="rIdC"/>"#, ("rIdC", "aFChunk", "afchunk.bin"), ("word/afchunk.bin", bytes));
                let pkg = Package::open(&outer).unwrap();
                for state in [ChunkState { left: 0, ..ChunkState::new(None) }, ChunkState { bytes: 0, ..ChunkState::new(None) }] {
                    assert!(matches!(super::super::read_opened_in(&pkg, state), Err(e) if matches!(e.error, crate::DocxError::Limit(_))));
                }
            }
        }
    }

    #[test]
    fn prefixed_unknown_type_chunk_is_inspected() {
        for body in [
            r#"<w:p xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:f="urn:future" mc:MustUnderstand="f"/>"#,
            r#"<w:p xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:f="urn:future" mc:Ignorable="f" mc:PreserveElements="f:*"><f:opaque/></w:p>"#,
        ] {
            let inner = package(body, ("unused", "unused", "unused.bin"), ("word/unused.bin", b""));
            let mut prefixed = b"self-extracting stub".to_vec();
            prefixed.extend_from_slice(&inner);
            let outer = package(r#"<w:altChunk r:id="rIdC"/>"#, ("rIdC", "aFChunk", "afchunk.bin"), ("word/afchunk.bin", &prefixed));
            if body.contains("MustUnderstand") {
                assert!(matches!(crate::read(&outer), Err(crate::DocxError::Xml(_))));
            } else {
                let doc = crate::read(&outer).unwrap();
                assert!(doc.passthrough.contains_key(crate::write::REQUIRED_SOURCE_GUARD));
            }
        }
    }

    #[test]
    fn non_package_chunks_are_not_misclassified_when_budget_is_exhausted() {
        for bytes in [&b"plain text PK\x05\x06"[..], &b"<html><body>PK\x03\x04 and PK\x05\x06</body></html>"[..]] {
            let outer = package(r#"<w:altChunk r:id="rIdC"/>"#, ("rIdC", "aFChunk", "afchunk.html"), ("word/afchunk.html", bytes));
            let pkg = Package::open(&outer).unwrap();
            for state in [ChunkState { left: 0, ..ChunkState::new(None) }, ChunkState { bytes: 0, ..ChunkState::new(None) }] {
                assert!(super::super::read_opened_in(&pkg, state).is_ok());
            }
        }
    }

    #[test]
    fn nested_xml_limit_does_not_hide_required_markup() {
        let body = format!(
            "{}<f:opaque xmlns:f=\"urn:future\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:MustUnderstand=\"f\"/>{}",
            "<w:p>".repeat(crate::xml::MAX_DEPTH + 1),
            "</w:p>".repeat(crate::xml::MAX_DEPTH + 1)
        );
        let inner = package(&body, ("unused", "unused", "unused.bin"), ("word/unused.bin", b""));
        let outer = package(r#"<w:altChunk r:id="rIdC"/>"#, ("rIdC", "aFChunk", "afchunk.docx"), ("word/afchunk.docx", &inner));
        assert!(matches!(crate::read(&outer), Err(crate::DocxError::Limit(_))));
    }

    #[test]
    fn a_nested_documents_charts_count_against_the_files_graphic_budget() {
        let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let drawing = r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="2743200" cy="1828800"/><wp:docPr id="5" name="G"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rIdChart"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
        let inner = package(drawing, ("rIdChart", "chart", "charts/chart1.xml"), ("word/charts/chart1.xml", chart.as_bytes()));
        let outer = package(r#"<w:altChunk r:id="rIdC"/>"#, ("rIdC", "aFChunk", "afchunk.docx"), ("word/afchunk.docx", &inner));
        let pkg = Package::open(&outer).unwrap();
        let read = |graphic| {
            let (d, _, _, left, _) = super::super::read_opened_in(&pkg, ChunkState { graphic, ..ChunkState::new(None) }).unwrap();
            (drawn(&d), left)
        };
        let (shown, left) = read(super::super::MAX_GRAPHIC_WORK);
        assert!(shown);
        assert!(left < super::super::MAX_GRAPHIC_WORK, "the nested chart's work is the file's");
        // With the file's budget spent, the nested document doesn't get one of its own.
        assert_eq!(read(0), (false, 0));
    }
}

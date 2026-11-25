//! EPUB import (read-only; export is tracked separately, see #2815).
//!
//! The package is read by craft-libs' `craft-epub` ([`craft_epub::Epub::open`]): the OCF
//! container, the package document's metadata, manifest and spine (EPUB 2 and 3), every entry
//! inside the shared ZIP entry-count and byte budgets that don't trust the sizes the archive
//! declares. Each spine item is an XHTML content document; this reads it with the existing HTML
//! importer ([`crate::html::parse_with`]), resolving the images it references from the package
//! relative to the content document's own path, and concatenates every chapter's blocks in
//! spine order. The navigation document and NCX are not read: the body's own headings already
//! carry the structure a Navigation pane or Table of Contents field would use.
//!
//! This is a first pass: fixed-layout EPUBs, scripted content, DRM and spines that point at
//! non-XHTML content are not specifically handled (the HTML importer reads what text it can from
//! malformed markup; it does not reject it).

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use craft_epub::Epub;
use wordcraft_doc::Document;

use crate::model::{self, Flow};

/// Parse an EPUB package into the flow model.
pub fn parse(bytes: &[u8]) -> Result<Flow, String> {
    parse_lang(bytes).map(|(flow, _)| flow)
}

/// Import an EPUB package into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    model::begin_import();
    let (flow, lang) = parse_lang(bytes)?;
    let mut shared = model::to_doc(&flow);
    if let Some(lang) = lang {
        Arc::make_mut(&mut shared.styles).default_chr.lang = Some(lang);
    }
    wordcraft_docx::from_shared_model(shared).map_err(|e| e.to_string())
}

/// [`parse`], with the package's `dc:language`, if any (the flow model has no language field;
/// a caller applies it to the document's default run properties, as the HTML exporter reads it
/// back out, #2815).
fn parse_lang(bytes: &[u8]) -> Result<(Flow, Option<String>), String> {
    let epub = Epub::open(bytes).map_err(|e| format!("EPUB: {e}"))?;
    let spine: Vec<String> = {
        // A spine that lists one chapter many times would otherwise parse it again for every
        // `itemref`: a few-KB file could keep the importer busy up to the whole read budget.
        let mut seen = HashSet::new();
        epub.package.spine_paths().filter(|p| seen.insert(*p)).map(str::to_string).collect()
    };
    let mut flow = Flow::default();
    flow.meta.title = epub.package.title.clone();
    flow.meta.author = epub.package.creators.first().cloned().unwrap_or_default();
    let lang = epub.package.language.clone().filter(|s| !s.trim().is_empty());

    let epub = RefCell::new(epub);
    for path in &spine {
        // A chapter the archive lacks, or one past what's left of the read budget, is skipped:
        // the rest of the book still opens.
        let Ok(Some(chapter_bytes)) = epub.borrow_mut().read(path) else { continue };
        let xhtml = crate::html::decode(&chapter_bytes);
        let loader: &dyn Fn(&str) -> Option<Arc<Vec<u8>>> = &|src| epub.borrow_mut().read_href(path, src).ok().flatten().map(Arc::new);
        let mut chapter = crate::html::parse_with(&xhtml, loader);
        flow.blocks.append(&mut chapter.blocks);
    }
    Ok((flow, lang))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = craft_zip::Writer::new();
        for (name, data) in entries {
            let compression = if *name == "mimetype" { craft_zip::Compression::Stored } else { craft_zip::Compression::Deflated };
            z.add(name, data, compression).unwrap();
        }
        z.finish().unwrap()
    }

    const CONTAINER: &[u8] = br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    fn opf(spine_items: &str) -> Vec<u8> {
        format!(
            r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Test Book</dc:title>
<dc:creator xmlns:dc="http://purl.org/dc/elements/1.1/">Jane Author</dc:creator>
<dc:language xmlns:dc="http://purl.org/dc/elements/1.1/">en</dc:language></metadata>
<manifest>
<item id="c1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
<item id="c2" href="text/chapter2.xhtml" media-type="application/xhtml+xml"/>
<item id="img1" href="images/cover.png" media-type="image/png"/>
</manifest>
<spine>{spine_items}</spine>
</package>"#
        )
        .into_bytes()
    }

    const CHAPTER1: &[u8] = br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Chapter One</h1><p>First chapter text.</p></body></html>"#;

    fn sample_package() -> Vec<u8> {
        let chapter2 =
            br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Chapter Two</h1><p>Second chapter, with a <img src="../images/cover.png"/> picture.</p></body></html>"#;
        let png = [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        zip_of(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", &opf(r#"<itemref idref="c1"/><itemref idref="c2"/>"#)),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/text/chapter2.xhtml", chapter2),
            ("OEBPS/images/cover.png", &png),
        ])
    }

    fn text_of(doc: &Document) -> String {
        String::from_utf8_lossy(&crate::txt::export(doc)).into_owned()
    }

    #[test]
    fn imports_chapters_in_spine_order_with_metadata_and_image() {
        let doc = import(&sample_package()).unwrap();
        assert_eq!(doc.core.title, "Test Book");
        assert_eq!(doc.core.creator, "Jane Author");
        assert_eq!(doc.styles.default_chr.lang.as_deref(), Some("en"));
        let text = text_of(&doc);
        let c1 = text.find("Chapter One").expect("chapter one heading");
        let f1 = text.find("First chapter text").expect("chapter one body");
        let c2 = text.find("Chapter Two").expect("chapter two heading");
        let f2 = text.find("Second chapter").expect("chapter two body");
        assert!(c1 < f1 && f1 < c2 && c2 < f2, "chapters interleaved out of spine order: {text}");
        assert!(!doc.media.is_empty(), "expected the referenced image to be imported into the document's media");
    }

    /// A small book laid out as real EPUB 2 tools write one: a stored `mimetype` first, the
    /// package document at the root of a folder whose name has a space, an NCX, an
    /// escaped `&amp;` in the title, a percent-escaped chapter href and a stylesheet.
    #[test]
    fn imports_a_real_shaped_epub2_book() {
        let container = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="My Book/book.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
        let opf = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
<dc:title>Fish &amp; Chips</dc:title>
<dc:creator opf:role="aut">Ann Smith</dc:creator>
<dc:creator opf:role="aut">Bob Jones</dc:creator>
<dc:language>en-GB</dc:language>
<dc:identifier id="uid">urn:uuid:0b6f1a52-3c4f-4b1e-9b2a-6d1f2f0c9e11</dc:identifier>
</metadata>
<manifest>
<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
<item id="css" href="style.css" media-type="text/css"/>
<item id="ch1" href="Text/Chapter%201.xhtml" media-type="application/xhtml+xml"/>
<item id="ch2" href="Text/chapter2.xhtml" media-type="application/xhtml+xml"/>
</manifest>
<spine toc="ncx"><itemref idref="ch1"/><itemref idref="ch2"/></spine>
</package>"#;
        let ncx = br#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap>
<navPoint id="n1" playOrder="1"><navLabel><text>One</text></navLabel><content src="Text/Chapter%201.xhtml"/></navPoint>
</navMap></ncx>"#;
        let ch1 = br#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd">
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title><link rel="stylesheet" href="../style.css" type="text/css"/></head>
<body><h2 class="chapter">Chapter 1</h2><p>It was a bright cold day.</p></body></html>"#;
        let ch2 = br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h2>Chapter 2</h2><p>The end.</p></body></html>"#;
        let bytes = zip_of(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container),
            ("My Book/book.opf", opf),
            ("My Book/toc.ncx", ncx),
            ("My Book/style.css", b"p { text-indent: 1em; }"),
            ("My Book/Text/Chapter 1.xhtml", ch1),
            ("My Book/Text/chapter2.xhtml", ch2),
        ]);
        assert!(craft_epub::is_epub(&bytes));
        let doc = import(&bytes).unwrap();
        assert_eq!(doc.core.title, "Fish & Chips");
        assert_eq!(doc.core.creator, "Ann Smith");
        assert_eq!(doc.styles.default_chr.lang.as_deref(), Some("en-GB"));
        let text = text_of(&doc);
        let a = text.find("bright cold day").expect("chapter 1 body");
        let b = text.find("The end.").expect("chapter 2 body");
        assert!(a < b, "{text}");
    }

    /// A ZIP64 end record that declares millions of entries is refused before the archive's
    /// directory is read, as every other ZIP reader here refuses it.
    #[test]
    fn zip64_entry_count_bomb_is_refused() {
        let bomb = wordcraft_doc::zip_guard::zip64_entry_count_sample(5_000_000);
        let err = import(&bomb).unwrap_err();
        assert!(err.contains("entries"), "{err}");
    }

    /// 10,000 `itemref`s naming one chapter read and parse it once, not 10,000 times.
    #[test]
    fn a_chapter_repeated_in_the_spine_is_imported_once() {
        let spine = r#"<itemref idref="c1"/>"#.repeat(10_000);
        let bytes = zip_of(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", &opf(&spine)),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ]);
        let text = text_of(&import(&bytes).unwrap());
        assert_eq!(text.matches("First chapter text").count(), 1, "{text}");
    }

    #[test]
    fn missing_container_is_an_error() {
        let bytes = zip_of(&[("mimetype", b"application/epub+zip")]);
        assert!(import(&bytes).is_err());
    }

    #[test]
    fn not_a_zip_is_an_error() {
        assert!(import(b"not a zip file").is_err());
    }

    /// Truncated mid-element, after the rootfile it names: the package is refused, not opened
    /// as whatever of it the reader still found.
    #[test]
    fn container_truncated_after_its_useful_content_is_still_an_error() {
        let truncated = br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>"#;
        let bytes = zip_of(&[("META-INF/container.xml", &truncated[..]), ("OEBPS/content.opf", &opf(""))]);
        assert!(import(&bytes).is_err());
    }

    /// Same, for the package document: truncated after its manifest but before its spine, which
    /// would otherwise import a document with no chapters and no error.
    #[test]
    fn package_document_truncated_mid_spine_is_still_an_error() {
        let truncated = br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Test Book</dc:title></metadata>
<manifest><item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="c1"/>"#;
        let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", &truncated[..])]);
        assert!(import(&bytes).is_err());
    }

    #[test]
    fn spine_item_missing_from_zip_is_skipped_not_fatal() {
        let bytes = zip_of(&[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", &opf(r#"<itemref idref="c1"/>"#)),
            // chapter1.xhtml deliberately absent
        ]);
        let doc = import(&bytes).unwrap();
        assert_eq!(doc.core.title, "Test Book");
    }

    /// An image `src` that climbs out of the package only ever names an archive entry, never a
    /// file on disk: the chapter still imports, without the picture.
    #[test]
    fn path_traversal_image_src_cannot_escape_the_archive() {
        let chapter = br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Text <img src="../../../../../../etc/passwd"/></p></body></html>"#;
        let bytes = zip_of(&[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", &opf(r#"<itemref idref="c1"/>"#)),
            ("OEBPS/text/chapter1.xhtml", chapter),
        ]);
        let doc = import(&bytes).unwrap();
        assert!(doc.media.is_empty());
        assert!(text_of(&doc).contains("Text"));
    }
}

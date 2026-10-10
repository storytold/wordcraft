//! WordCraft DOCX reader and writer (ECMA-376 / ISO/IEC 29500 WordprocessingML).
//!
//! [`read`] turns a `.docx` package into a [`wordcraft_doc::Document`]; [`write`] does the
//! reverse. Both are written from the public specification. The reader is lenient (unknown
//! markup is skipped, bad numbers fall back to defaults) and bounded (zip entry sizes, total
//! decompressed size, XML depth and element counts are capped), so hostile input yields an
//! error or a best-effort document, never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod package;
mod read;
mod units;
mod write;
mod xml;

pub use read::read;
pub use write::write;

/// Errors from reading or writing a DOCX package.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum DocxError {
    /// The bytes are not a readable zip archive.
    #[error("not a valid zip package: {0}")]
    Zip(String),
    /// A required part is absent.
    #[error("missing part: {0}")]
    MissingPart(String),
    /// A part is not well-formed XML.
    #[error("malformed XML: {0}")]
    Xml(String),
    /// The input exceeds a safety limit (size, nesting, element count).
    #[error("limit exceeded: {0}")]
    Limit(String),
    /// The package is readable but isn't a WordprocessingML document.
    #[error("not a Word document: {0}")]
    NotWord(String),
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use wordcraft_doc::{Document, Source};

    #[test]
    fn bibliography_round_trip() {
        let mut doc = Document::new();
        doc.sources.push(Source {
            tag: "Rivera2021".into(),
            kind: "book".into(),
            author: "Rivera, Alex; Chen, Mei".into(),
            title: "Shared Spaces".into(),
            year: "2021".into(),
            publisher: "Harbor Books".into(),
            city: "Portland".into(),
            ..Default::default()
        });

        let bytes = write(&doc).expect("docx write");
        let doc2 = read(&bytes).expect("docx read");
        assert_eq!(doc2.sources.len(), 1, "round-tripped sources: {:?}", doc2.sources);
        assert_eq!(doc2.sources[0].author, "Rivera, Alex; Chen, Mei");
        assert_eq!(doc2.sources[0].title, "Shared Spaces");
    }
}

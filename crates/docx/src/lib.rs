//! WordCraft DOCX reader and writer (ECMA-376 / ISO/IEC 29500 WordprocessingML).
//!
//! [`read`] turns a `.docx` package into a [`wordcraft_doc::Document`]; [`write`] does the
//! reverse. Both are written from the public specification. The reader is lenient (unknown
//! markup is skipped, bad numbers fall back to defaults) and bounded (zip entry sizes, total
//! decompressed size, XML depth and element counts are capped), so hostile input yields an
//! error or a best-effort document, never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod custom;
mod package;
mod read;
mod units;
mod write;
mod xml;

pub use read::read;
pub use write::{write, write_as};

/// Which kind of WordprocessingML package to write. The main part's content type differs per
/// kind, and Word refuses a file whose content type doesn't match its extension.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flavor {
    /// `.docx`
    #[default]
    Document,
    /// `.docm`: may carry a VBA project.
    MacroDocument,
    /// `.dotx`
    Template,
    /// `.dotm`: may carry a VBA project.
    MacroTemplate,
}

impl Flavor {
    /// The flavour a file extension (without the dot, any case) names.
    pub fn from_ext(ext: &str) -> Option<Flavor> {
        match ext.to_ascii_lowercase().as_str() {
            "docx" => Some(Flavor::Document),
            "docm" => Some(Flavor::MacroDocument),
            "dotx" => Some(Flavor::Template),
            "dotm" => Some(Flavor::MacroTemplate),
            _ => None,
        }
    }

    /// Content type of the main document part.
    pub fn main_content_type(self) -> &'static str {
        match self {
            Flavor::Document => "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            Flavor::MacroDocument => "application/vnd.ms-word.document.macroEnabled.main+xml",
            Flavor::Template => "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
            Flavor::MacroTemplate => "application/vnd.ms-word.template.macroEnabledTemplate.main+xml",
        }
    }

    /// Can this package hold macros?
    pub fn macros(self) -> bool {
        matches!(self, Flavor::MacroDocument | Flavor::MacroTemplate)
    }
}

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

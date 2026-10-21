//! WordCraft PDF export through the shared `craft-pdf` writer and Krilla drawing primitives.
//!
//! [`export`] lays the document out and draws each page's display list
//! ([`wordcraft_layout::display::page_display`]) into a PDF page:
//!
//! - **Text as real text:** glyph runs are written with embedded, subsetted fonts and a Unicode
//!   mapping taken from the paragraph text, so text is selectable, searchable and extractable.
//!   Synthetic bold (fill + stroke) and italic (skew) follow the renderer.
//! - **Graphics:** fills (highlight, shading, cell fills), rules and underlines (solid, dotted,
//!   dashed, double, wave), shapes, pictures (decoded with `image`; JPEG passed through, the rest
//!   embedded losslessly; WMF and EMF drawn as vector paths and embedded bitmaps) with cropping and
//!   opacity, the page colour and the watermark.
//! - **Interactive:** link annotations for hyperlinked runs (URLs and internal `#bookmark`
//!   links) and a document outline built from the headings (outline levels).
//! - **Metadata:** title, author, subject, keywords, language, creation date.
//! - **Tagged PDF (best effort):** one structure element per paragraph (`H1`–`H6` for headings,
//!   `P` otherwise), `Figure` for pictures with their alt text; headers, footers, decorations and
//!   the watermark are artifacts.
//!
//! Nothing here panics on hostile documents: sizes are clamped, unknown fonts or undecodable
//! pictures are skipped (a grey box stands in for a picture).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod extract;

/// The Unicode text of every page, or an error when this build cannot read PDFs.
#[cfg(not(target_arch = "wasm32"))]
pub use extract::{Verdict, VerdictDetail, extract_page_text, extract_verdict_detail};

/// Always an error: the web build can't read PDFs.
#[cfg(target_arch = "wasm32")]
pub fn extract_page_text(_bytes: &[u8]) -> Result<Vec<String>, String> {
    Err("PDF text extraction is not available on wasm".into())
}

/// What a build could learn about a PDF's text layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSearchability {
    /// The bytes carry extractable document text.
    Searchable,
    /// The bytes are a PDF with no extractable document text and no page-dominating picture (for
    /// example, a font-only PDF). An image-only page is [`TextSearchability::Uncertain`], not this.
    NotSearchable,
    /// At least one page draws a picture that dominates it, so the page's own text (if any) cannot
    /// verify the picture's content: coverage cannot be verified. This is **not** the same as
    /// success.
    Uncertain,
    /// This build cannot inspect PDF text at all (wasm has no PDF interpreter); verification is
    /// unavailable, which is **not** the same as success.
    Unavailable,
}

impl TextSearchability {
    /// `Some(true)`/`Some(false)` when the PDF was actually inspected and the answer is definite,
    /// `None` when it could not be verified (an uncertain page, or a build with no extractor).
    pub fn is_searchable(self) -> Option<bool> {
        match self {
            TextSearchability::Searchable => Some(true),
            TextSearchability::NotSearchable => Some(false),
            TextSearchability::Uncertain | TextSearchability::Unavailable => None,
        }
    }
}

/// Judge a PDF from the result of an extraction attempt: `None` is a build with no extractor.
///
/// Kept separate from the `cfg`-gated extraction so the "no extractor" arm is covered off wasm
/// too, and so a bare `%PDF` signature can never be mistaken for extracted text.
pub fn searchability_verdict(extracted_text: Option<bool>) -> TextSearchability {
    match extracted_text {
        Some(true) => TextSearchability::Searchable,
        Some(false) => TextSearchability::NotSearchable,
        None => TextSearchability::Unavailable,
    }
}

/// Whether `bytes` is a PDF with searchable text, judged page by page.
#[cfg(not(target_arch = "wasm32"))]
pub fn text_searchability(bytes: &[u8]) -> TextSearchability {
    if bytes.len() < 8 || !bytes.starts_with(b"%PDF") {
        return TextSearchability::NotSearchable;
    }
    match extract::extract_verdict(bytes) {
        extract::Verdict::Searchable => TextSearchability::Searchable,
        extract::Verdict::NotSearchable => TextSearchability::NotSearchable,
        extract::Verdict::Uncertain => TextSearchability::Uncertain,
    }
}

/// Always [`TextSearchability::Unavailable`]: the web build can't read PDFs.
#[cfg(target_arch = "wasm32")]
pub fn text_searchability(_bytes: &[u8]) -> TextSearchability {
    searchability_verdict(None)
}

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU16;
use std::sync::Arc;

use craft_pdf::write::{self, Metadata, WriteError, Writer, backend as krilla};
use krilla::annotation::{Annotation, LinkAnnotation, Target};
use krilla::color::rgb;
use krilla::destination::XyzDestination;
use krilla::error::KrillaError;
use krilla::geom::{Path, PathBuilder, Point, Size, Transform};
use krilla::image::Image;
use krilla::metadata::DateTime;
use krilla::num::NormalizedF32;
use krilla::outline::{Outline, OutlineNode};
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, LineCap, LineJoin, Stroke, StrokeDash};
use krilla::surface::Surface;
use krilla::tagging::{Artifact, ArtifactType, ContentTag, Identifier, Node, SpanTag, Tag, TagGroup, TagKind, TagTree};
use krilla::text::{Font, GlyphId, KrillaGlyph};
use wordcraft_doc::graphic::GraphicKind;
use wordcraft_doc::para::{InlineObject, ShapeKind};
use wordcraft_doc::{Document, Path as DocPath, Rgb, StoryRef};
use wordcraft_fonts::FaceRef;
use wordcraft_geom::Rect;
use wordcraft_layout::display::Paint;
use wordcraft_layout::display::{DisplayOptions, Draw, ItemDisplay, Stroke as LineStyle};
use wordcraft_layout::para::{ClKind, LineEnd};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, NumeralMode, Page, Placed, layout};
use wordcraft_metafile::{Picture, PlacedItem};

mod export;
mod exporter;
mod glyphs;
mod metadata;
mod options;
mod paint;
mod picture;
mod tags;

pub use export::{export, export_layout};
#[cfg(test)]
use exporter::Exporter;
#[cfg(test)]
use glyphs::{baseline, cluster_ends, left_to_right, split_clusters, visual_order};
#[cfg(test)]
use metadata::{civil, parse_iso};
pub use options::{PdfError, PdfOptions};
#[cfg(test)]
use paint::shape_path;
#[cfg(test)]
use picture::{load_image, load_image_within};

#[cfg(test)]
mod tests;

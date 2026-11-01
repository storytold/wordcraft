//! WordCraft document model.
//!
//! A [`Document`] holds the main story (`body`) and secondary stories (`parts`: headers,
//! footers, notes, comments, text boxes). A story is a list of [`Block`]s: paragraphs and tables;
//! table cells hold block lists of their own. Blocks are `Arc`-shared, so cloning a document is
//! cheap (undo snapshots, handing a frozen copy to the renderer) and edits copy only what they
//! touch (`Arc::make_mut`).
//!
//! Positions ([`Pos`]) name a story, a [`Path`] to a paragraph and a byte offset in it. Paths
//! compare in document order.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bidi;
mod byte_store;
pub mod cfb_guard;
pub mod chart;
mod cjk_numbers;
pub mod cluster;
pub mod collate;
pub mod connector;
pub mod control;
pub mod edit;
pub mod effects;
pub mod encoding;
pub mod eq;
pub mod fields;
pub mod fill;
pub mod freeform;
pub mod graphic;
pub mod ime_text;
pub mod legacy_form;
pub mod line_dash;
pub mod link;
pub mod locale_number;
pub mod math;
pub mod math_edit;
pub mod math_latex;
pub mod math_linear;
pub mod math_symbols;
pub mod numbering;
pub mod para;
pub mod props;
pub mod resolve;
pub mod section;
pub mod smart_art;
pub mod styles;
pub mod symbol_fonts;
pub mod table;
pub mod wordart;
pub mod wrap;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use control::{ContentControl, ControlKind, ControlLock, ControlRange, ControlWrap};
pub use fields::FieldRange;
pub use numbering::{ListKind, Numbering};
pub use para::{InlineObject, Paragraph, Run};
pub use props::{Align, CharProps, ParaProps, Rgb, TextColor};
pub use section::SectionProps;
pub use styles::{Style, StyleKind, StyleSheet};
pub use table::{Cell, Row, Table};

mod review;

pub use review::{AUTO_TEXT_GALLERY, BuildingBlock, Comment, QUICK_PARTS_GALLERY, Revision, RevisionKind};

mod block;
mod compare;
mod document;
mod error;
mod parts;
mod position;
mod settings;
mod text_walk;

pub use block::{Block, Blocks, BoxBudget, MAX_TEXT_BOX_DEPTH};
pub use compare::para_block;
use document::MediaIndex;
pub use error::{DocError, Result};
pub use parts::{MAX_LINKED_BOXES, MAX_TEXT_BOX_INSET, NoteSeparators, Part, PartKind, PermRange, TEXT_BOX_INSET, TextBody, free_id};
pub use position::{Path, Pos, StoryRef};
pub use settings::{
    COMPAT_MODE_CURRENT, CoreProps, CustomProp, DEFAULT_GRID, DEFAULT_HYPHENATION_ZONE, LEGACY_COMPAT_MODE, PagePicture, Settings, Source,
    THEME_COLORS, Watermark, Zoom, ZoomFit,
};
pub use text_walk::count_words;
pub(crate) use text_walk::{PARA_WALK_DEPTH, walk_between};

/// A document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Document {
    pub body: Blocks,
    /// Properties of the last section (OOXML's body-level `w:sectPr`).
    pub last_section: SectionProps,
    pub parts: BTreeMap<u32, Part>,
    pub styles: StyleSheet,
    pub numbering: Numbering,
    pub comments: BTreeMap<u32, Comment>,
    pub revisions: Vec<Revision>,
    pub settings: Settings,
    pub core: CoreProps,
    /// Bibliography sources (References › Manage Sources).
    pub sources: Vec<Source>,
    /// Custom document properties, in file order.
    pub custom_props: Vec<CustomProp>,
    /// The document's own footnote separators and continuation notice.
    #[serde(skip_serializing_if = "NoteSeparators::is_empty")]
    pub footnote_separators: NoteSeparators,
    /// The document's own endnote separators and continuation notice.
    #[serde(skip_serializing_if = "NoteSeparators::is_empty")]
    pub endnote_separators: NoteSeparators,
    /// Embedded media (images) by key. Native JSON writes each as base64 (`byte_store.rs`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", with = "byte_store")]
    pub media: BTreeMap<String, Arc<Vec<u8>>>,
    /// Source-format parts we keep verbatim for round-trip (e.g. custom XML), by part name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", with = "byte_store")]
    pub passthrough: BTreeMap<String, Arc<Vec<u8>>>,
    /// The document's own building blocks (its glossary document), in file order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<BuildingBlock>,
    /// Lookup cache for [`Document::add_media`]; not part of the document (never saved, ignored
    /// by `==`).
    #[serde(skip)]
    media_index: MediaIndex,
}

#[cfg(test)]
mod test_log;
#[cfg(test)]
mod tests;

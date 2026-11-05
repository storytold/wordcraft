//! WordCraft document model.
//!
//! A [`Document`] holds the main story (`body`) and secondary stories (`parts`: headers,
//! footers, notes, comments, text boxes). A story is a list of [`Block`]s: paragraphs and tables;
//! table cells hold block lists of their own. Blocks, the style sheet and the list definitions
//! are `Arc`-shared, so cloning a document is cheap (undo snapshots, handing a frozen copy to the
//! renderer) and edits copy only what they touch (`Arc::make_mut`).
//!
//! Positions ([`Pos`]) name a story, a [`Path`] to a paragraph and a byte offset in it. Paths
//! compare in document order.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bidi;
mod byte_store;
pub mod cfb_guard;
pub mod chart;
pub mod chat;
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
pub mod kept;
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
pub mod picfx;
pub mod props;
pub mod resolve;
pub mod ribbon;
pub mod section;
pub mod smart_art;
pub mod styles;
pub mod symbol_fonts;
pub mod table;
pub mod wordart;
pub mod work;
pub mod wrap;
pub mod zip_guard;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use control::{ContentControl, ControlKind, ControlLock, ControlRange, ControlWrap};
pub use fields::FieldRange;
pub use numbering::{ListKind, Numbering};
pub use para::{InlineObject, Paragraph, Run};
pub use props::{Align, CharProps, ParaProps, Rgb, TextColor};
pub use ribbon::{Ribbon, RibbonButton, RibbonGroup, RibbonTab};
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
    COMPAT_MODE_CURRENT, CoreProps, CustomProp, DEFAULT_GRID, DEFAULT_HYPHENATION_ZONE, LEGACY_COMPAT_MODE, OdfLayout, PagePicture, Settings, Source,
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
    /// Shared, like the blocks: an undo step copies it only when a command changes it (a full
    /// copy is kilobytes a style, so typing into a document with hundreds of styles cost megabytes
    /// a step).
    pub styles: Arc<StyleSheet>,
    /// Shared like [`Document::styles`].
    pub numbering: Arc<Numbering>,
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
    /// The document's own ribbon tabs, groups and macro buttons (`customUI14.xml`). `None` = none,
    /// or a richer customisation kept unchanged as a package part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ribbon: Option<Ribbon>,
    /// Where new story and comment ids come from: in a live session, a block of ids no other
    /// person's copy uses (see [`IdSpace`]). Not part of the document.
    #[serde(skip)]
    pub id_space: IdSpace,
    /// The document chat (craft-collab `docs/chat.md`): saved in the file, shared in a live
    /// session, never part of undo. Shared, so undo snapshots don't copy it.
    #[serde(skip_serializing_if = "chat::arc_is_empty")]
    pub chat: Arc<chat::Chat>,
    /// Lookup cache for [`Document::add_media`]; not part of the document (never saved, ignored
    /// by `==`).
    #[serde(skip)]
    media_index: MediaIndex,
    /// Markup kept from the file it was opened from, for writing back what isn't edited (see
    /// [`kept`]); never saved, ignored by `==`.
    #[serde(skip)]
    pub kept: kept::Kept,
}

#[cfg(test)]
mod test_log;
#[cfg(test)]
mod tests;

/// Where a document makes new story (part), comment and list ids. The default (`start` 0): one
/// past the highest ([`free_id`]). In a live session each person's copy makes them in its own
/// block of 65,536 ids from `start`, never below `floor` (so an id given up by an undo isn't
/// given again while others may still have the old story), and two people adding a footnote or a
/// comment at once don't pick the same id. Not part of the document: every `IdSpace` equals
/// every other.
#[derive(Clone, Copy, Debug, Default)]
pub struct IdSpace {
    pub start: u32,
    /// The lowest id this copy may make next.
    pub floor: u32,
}

impl PartialEq for IdSpace {
    fn eq(&self, _: &IdSpace) -> bool {
        true
    }
}

impl IdSpace {
    /// Ids in a block.
    pub const BLOCK: u32 = 1 << 16;

    /// The block of ids from `start`.
    pub fn new(start: u32) -> IdSpace {
        IdSpace { start, floor: start }
    }

    /// Whether `id` is in this block.
    pub fn contains(&self, id: u32) -> bool {
        self.start > 0 && (self.start..self.start.saturating_add(Self::BLOCK)).contains(&id)
    }

    /// A new id not in `taken`, at least `floor`; [`IdSpace::take`] it. `None` when every id
    /// is taken.
    pub fn next(&self, taken: impl Iterator<Item = u32> + Clone, floor: u32) -> Option<u32> {
        if self.start > 0 {
            let end = self.start.saturating_add(Self::BLOCK);
            let lo = self.start.max(self.floor);
            let mine = taken.clone().filter(|id| (self.start..end).contains(id)).max().map(|m| m.saturating_add(1)).unwrap_or(lo).max(lo).max(floor);
            if mine < end {
                return Some(mine);
            }
            // The block is full: one past the highest.
        }
        free_id(taken.clone().max(), taken, floor)
    }

    /// Records that `id` was made, so it isn't made again.
    pub fn take(&mut self, id: u32) {
        if self.contains(id) {
            self.floor = self.floor.max(id.saturating_add(1));
        }
    }
}

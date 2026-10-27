//! OpenDocument Text (`.odt`, ODF 1.3).
//!
//! Export writes `mimetype` (stored, first), `META-INF/manifest.xml`, `content.xml` (paragraphs,
//! `text:h` headings with outline levels, spans with automatic text styles, links, bookmarks,
//! line breaks / tabs / `text:s`, nested `text:list`s with generated list styles, tables with
//! header rows, column/row spans and cell shading, pictures as `draw:frame`/`draw:image` with the
//! bytes under `Pictures/`, page breaks), footnotes and endnotes (`text:note` with
//! `text:note-citation` and `text:note-body`), comments (`office:annotation` with `dc:creator`,
//! `dc:date`, `meta:creator-initials` and its paragraphs, plus `office:annotation-end` for the
//! commented range), `styles.xml` (Standard, Heading 1–6, Title, Quotations, Preformatted Text,
//! page layout) and `meta.xml`.
//!
//! Import reads the same subset back from `content.xml` / `styles.xml` (style inheritance
//! resolved, both automatic and common styles), including notes and comments, and skipping text
//! boxes and drawing shapes. Comment replies, resolved state and note numbering formats
//! have no counterpart in ODF 1.3 and are not kept.
//!
//! The format-neutral flow model has no notes or comments, so they travel as reserved bookmark
//! names (starting with [`MARK`]) through it and are swapped for the document model's note
//! references and comment ranges on import, and the other way round on export.
//!
//! Tracked changes (ODF 1.3 Part 3 §5.5) go both ways: `text:tracked-changes` holds one
//! `text:changed-region` per change; an insertion is the body text between
//! `text:change-start`/`text:change-end`, a deletion keeps the deleted paragraphs in its region
//! and leaves a `text:change` point in the body. They map to `Document::revisions` and the
//! `ins`/`del` marks of runs and paragraph marks; a deleted paragraph mark is a region of
//! several paragraphs. Format changes are read as no change.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use wordcraft_doc::para::{InlineObject, NoteKind, OBJ};
use wordcraft_doc::props::VAlign;
use wordcraft_doc::section::{Columns, SectionStart};
use wordcraft_doc::{Align, Block, Blocks, CharProps, Comment, Document, Paragraph, PartKind, RevisionKind, Rgb, SectionProps};

use crate::model::{self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, RevMark, make_img, mime_of};

mod body;
mod import;
mod limits;
mod package;
mod styles;
mod writer;
mod xml;

use body::{Body, Extras, MARK, MAX_NOTES, MarkKind, attach_extras, for_paras, mark_name, parse_mark};
use import::Pages;
#[cfg(test)]
use import::{Pictures, apply_page_layout, read_entry, read_meta};
pub use import::{import, parse};
#[cfg(test)]
use limits::MAX_XML_DEPTH;
pub use package::export;
use styles::Styles;
use writer::prepare_export;
#[cfg(test)]
use writer::x;
#[cfg(test)]
use xml::length;
use xml::{Ev, events, get};

mod page_parts;

#[cfg(test)]
mod tests;

//! OpenDocument Text (`.odt`, ODF 1.3).
//!
//! Export ([`export`]) writes from the full document model through [`write`] the documents that
//! writer can write whole ([`write::can_write`], see `docs/odt-writer.md`). The others go through
//! the exchange `Flow` ([`export_flow`]), which writes `mimetype` (stored, first),
//! `META-INF/manifest.xml`, `content.xml` (paragraphs, `text:h` headings with outline levels,
//! spans with automatic text styles, links, bookmarks, line breaks / tabs / `text:s`, nested
//! `text:list`s with generated list styles, tables with header rows, column/row spans and cell
//! shading, pictures as `draw:frame`/`draw:image` with the bytes under `Pictures/`, page breaks),
//! `styles.xml` (Standard, Heading 1–6, Title, Quotations, Preformatted Text, page layout) and
//! `meta.xml`.
//!
//! Import reads the same subset back from `content.xml` / `styles.xml` (style inheritance
//! resolved, both automatic and common styles), skipping notes, annotations, text boxes and drawing shapes.
//!
//! Tracked changes (ODF 1.3 Part 3 §5.5) go both ways: `text:tracked-changes` holds one
//! `text:changed-region` per change; an insertion is the body text between
//! `text:change-start`/`text:change-end`, a deletion keeps the deleted paragraphs in its region
//! and leaves a `text:change` point in the body. They map to `Document::revisions` and the
//! `ins`/`del` marks of runs and paragraph marks; a deleted paragraph mark is a region of
//! several paragraphs. Format changes are read as no change.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use wordcraft_doc::props::VAlign;
use wordcraft_doc::section::{Columns, SectionStart};
use wordcraft_doc::{Align, Document, RevisionKind, Rgb, SectionProps};

use crate::model::{self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, RevMark, make_img, mime_of};

mod body;
mod import;
mod limits;
mod package;
mod styles;
mod writer;
mod xml;

use body::Body;
use import::Pages;
#[cfg(test)]
use import::{Pictures, apply_page_layout, read_entry, read_meta};
pub use import::{import, parse};
#[cfg(test)]
use limits::MAX_XML_DEPTH;
use package::{columns_xml, export_flow};
use styles::Styles;
use writer::{NS, pt, x};
#[cfg(test)]
use xml::length;
#[cfg(test)]
use xml::local;
use xml::{Ev, events, get};

mod page_parts;
mod write;

/// Write an `.odt` package: from the full document model ([`write`]) when it writes everything
/// `doc` holds, else from the exchange flow ([`export_flow`]).
pub fn export(doc: &Document) -> Result<Vec<u8>, String> {
    if write::can_write(doc) { write::write(doc) } else { export_flow(doc) }
}

#[cfg(test)]
mod tests;

//! OpenDocument Text (`.odt`, ODF 1.3).
//!
//! Both ways, through the flow model ([`crate::model`]) with what ODT keeps besides
//! ([`model::FlowOpts`]): paragraphs and headings with their spacing, indents, line spacing,
//! contextual spacing, alignment, borders, background and tab stops; character formatting (the
//! extras in [`model::Fmt::extra`] included); custom paragraph and character styles by name, and
//! the look of the heading, title, quotation and preformatted styles; lists with their identity,
//! number formats, labels and positions, and heading numbering (`text:outline-style`); tables with
//! column widths, spans, header rows, cell borders, padding, shading and vertical alignment;
//! pictures (in line or floating), equations as formula objects, text boxes; links, bookmarks,
//! text fields, footnotes and endnotes, comments with ranges and replies; sections with their own
//! master pages, page setup, columns and headers and footers; document defaults, metadata and
//! LibreOffice's layout settings (`settings.xml`, kept from the file or Word-compatible).
//!
//! Export writes `mimetype` (stored, first), `META-INF/manifest.xml`, `content.xml`,
//! `styles.xml`, `meta.xml`, `settings.xml`, pictures under `Pictures/` and formula
//! sub-documents. Import reads `content.xml`, `styles.xml` (style inheritance resolved, automatic
//! and common styles), `meta.xml`, `settings.xml` and the parts frames point at; drawing shapes
//! are still skipped.
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
use wordcraft_doc::props::{Borders, VAlign};
use wordcraft_doc::section::{Columns, SectionStart};
use wordcraft_doc::{Align, Document, RevisionKind, Rgb, SectionProps};

use crate::model::{self, Cell, FBlock, FComment, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, RevMark, make_img, mime_of};

mod body;
mod cell_formula;
mod chart;
mod crypt;
mod datastyle;
mod fields;
mod flat;
mod forms;
mod formula_size;
mod frames;
mod geometry;
mod import;
mod kept;
mod limits;
mod objects;
mod package;
mod parts;
mod path;
mod shapes;
mod strokes;
mod styles;
mod writer;
mod xml;

use body::Body;
pub use crypt::{CryptError, decrypt, encrypt, is_encrypted};
pub use flat::{export_flat, import_flat, import_flat_on};
use import::Pages;
#[cfg(test)]
use import::{Pictures, apply_page_layout, read_entry, read_meta};
pub use import::{import, import_on, parse};
#[cfg(test)]
use limits::{MAX_EXTRA_SPACES, MAX_XML_DEPTH};
pub use package::{TEMPLATE_MIME, TEXT_MIME, export, export_template};
use styles::Styles;
#[cfg(test)]
use writer::x;
#[cfg(test)]
use xml::length;
use xml::{Ev, events, get};

mod page_parts;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_more;

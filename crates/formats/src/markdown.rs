//! Markdown: a CommonMark subset plus GitHub tables and strikethrough.
//!
//! Import: ATX and setext headings, paragraphs with soft/hard breaks, block quotes (→ Quote
//! style), fenced and indented code (→ one Code paragraph per line), thematic breaks (→ a
//! bottom-border paragraph), bullet and ordered lists (nested by indentation), GFM pipe tables,
//! and inlines: emphasis/strong (the CommonMark delimiter algorithm), `~~strike~~`, code spans,
//! inline links, autolinks, images (`data:` URIs become pictures, others their alt text),
//! backslash escapes, entities, HTML comments (`<!-- ... -->`), `<br>` and `<a id>` anchors.
//!
//! Export writes the same constructs; pictures become `![alt](media-name)` placeholders.

use std::collections::HashMap;
use std::sync::Arc;

use wordcraft_doc::table::{MAX_COLS, MAX_ROWS};
use wordcraft_doc::{Align, Document};

use crate::html::decode_entity;
use crate::model::{self, Cell, FBlock, FTable, Fmt, Inline, Kind, ListInfo, Para, make_img};

mod blocks;
mod export;
mod inlines;

/// Table cells a Markdown document makes in all, header rows included (hostile input). A short
/// row is padded to its header's width, so a 197 KB file of one-cell rows under a 63-column
/// header made 2 million cells and took 3 to 5 GB to convert; past this, rows are left out.
pub(crate) const MAX_TABLE_CELLS: usize = 1 << 18;

pub use blocks::{Refs, import, import_on, parse};
pub(crate) use export::escape;
pub use export::export;
#[cfg(test)]
use export::{escape_line_start, inlines_md, table_md, write_blocks};
pub use inlines::parse_inlines;

#[cfg(test)]
use blocks::split_row;
#[cfg(test)]
mod tests;

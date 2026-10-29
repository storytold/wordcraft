//! Rich Text Format.
//!
//! Export: font and colour tables, a small stylesheet (headings with outline levels, Title,
//! Quote, Code), list tables (`\listtable` / `\listoverridetable` with `\listtext` labels for
//! older readers), document info, page setup, paragraph formatting (`\pard`, alignment, page
//! break before), character formatting (`\b \i \ul \strike \super \sub \f \fs \cf \chcbpat`),
//! hyperlinks as `HYPERLINK` fields, bookmarks, pictures (`\pngblip` / `\jpegblip`) and tables
//! (`\trowd … \cellx … \cell … \row`, header rows, vertical merges, shading).
//!
//! Import: a tolerant reader for groups and destinations (unknown `\*` destinations, headers,
//! footers, notes and objects are skipped), `\par \line \tab \page`, `\uN` with `\ucN`
//! fallback skipping, `\'hh` and 8-bit text in the document's code page (`\ansicpg`, `\mac`, `\pc`, `\pca`) or
//! the current font's (`\fcharset`, `\cpg`: Cyrillic, Central European, Greek, Mac OS Roman, the
//! East Asian multibyte pages 932/936/949/950…; Windows-1252 when none is named). A multibyte
//! code page's bytes stream through one decoder, so a lead byte and its trail byte decode
//! together and a run that looks like a UTF-8 BOM is not mistaken for one; the formatting words
//! above, stylesheet-based
//! headings and `\outlinelevel`, lists through `\ls`/`\ilvl`, fields, bookmarks, pictures and
//! tables. Group nesting and picture sizes are capped.

use std::collections::HashMap;
use std::sync::Arc;

use wordcraft_doc::props::VAlign;
use wordcraft_doc::{Align, Document, Rgb};

use crate::model::{self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, RevMark, make_img};

mod date;
mod export;
mod parse;
mod reader;
mod state;
mod styles;
mod text;
mod word;

pub use date::{dttm_to_iso, iso_to_dttm};
#[cfg(test)]
use export::{Tables, Writer};
pub use export::{can_write_picture, esc, export};
pub use parse::{import, parse};
use reader::Reader;
#[cfg(test)]
use state::MAX_SECTIONS;
#[cfg(test)]
use word::hyperlink;

#[cfg(test)]
mod tests;

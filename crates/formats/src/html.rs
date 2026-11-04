//! HTML.
//!
//! Import is a tolerant tag-soup reader (no DOM): a tokenizer plus a stack of open elements with
//! HTML's implicit closes for `p`, `li`, `tr`, `td`/`th` and headings. It understands `p`, `div`,
//! `h1`–`h6`, `b`/`strong`, `i`/`em`, `u`, `s`/`strike`/`del`, `sup`/`sub`, `code`/`kbd`, `a`
//! (links and `id`/`name` anchors), `br`, `hr`, `ul`/`ol`/`li` (nested), `table`/`tr`/`td`/`th`
//! (colspan/rowspan), `img` (`data:` URIs, other sources through a caller's [`ImageLoader`]), `blockquote`, `pre`, `font`, `ruby`
//! (`rb`/`rt`/`rp`: furigana and other phonetic guides, #1681), and the CSS
//! properties `color`, `background-color`, `font-weight`, `font-style`, `font-size`,
//! `font-family`, `text-decoration`, `vertical-align`, `text-align`, `white-space` and page
//! breaks, and on paragraphs `margin`, `text-indent` and `line-height`. CSS comes from `style`
//! attributes and from `style` elements' rules with simple selectors (`p`, `.note`, `p.note`),
//! applied in cascade order (specificity, then source order, then the `style` attribute).
//! Scripts are skipped; nesting is capped.
//!
//! Export writes clean semantic HTML with inline CSS, pictures as `data:` URIs and ruby as
//! `<ruby>`.

use std::sync::Arc;

use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::props::VAlign;
use wordcraft_doc::{Align, Document};

use crate::model::{
    self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, Spacing, base64_encode, data_uri, make_img, mime_of, parse_color,
};

mod builder;
mod css;
mod entity;
mod export;
mod import;
mod lexer;
mod math_ruby;

pub use builder::ImageLoader;
pub use css::css_len_pt;
pub use entity::{decode, decode_as, decode_entity, unescape};
pub use export::{esc, export, export_flow, export_fragment, inlines_html};
#[cfg(test)]
use import::parse_in;
pub use import::{PAGE_FONT, import, import_on, import_page, import_with, parse, parse_with};

#[cfg(test)]
mod tests;

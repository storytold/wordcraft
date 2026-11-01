//! LaTeX (`.tex`) import and export.
//!
//! **Export** writes a standalone LaTeX 2ε `article` that compiles with pdfLaTeX, XeLaTeX or
//! LuaLaTeX using only packages every TeX distribution ships (`fontenc`, `ulem`, `xcolor`,
//! `multirow`, `amsmath`, `hyperref`). Headings become unnumbered sectioning commands (Word headings
//! carry no numbers), lists `itemize`/`enumerate`, tables `tabular` with the document's column
//! widths, equations `$…$` / `\[…\]`, and character formatting the matching text commands. Pictures
//! can't live inside a single `.tex` file, so each is replaced by its alternative text.
//!
//! **Import** reads the everyday subset of LaTeX that documents are written in: the preamble's
//! `\title`/`\author`, sectioning commands, paragraphs, lists, `quote`, `center`/`flush…`,
//! `verbatim`-like environments, `tabular` (with `\multicolumn`/`\multirow`), inline and display math
//! (kept as equations), text formatting commands and declarations, colours, links, accents and
//! TeX's special characters and ligatures. It is not a TeX engine: macros aren't expanded. Unknown
//! commands are dropped while their braced arguments are kept as text, so no words are lost; layout
//! commands (`\vspace`, `\setlength`, `\usepackage`…) are dropped with their arguments.
//!
//! Like the other text formats, the parser is lenient and never panics on hostile input: group and
//! environment nesting is depth-limited and the input size is capped.

use crate::model::{self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Para};
use wordcraft_doc::{Align, Document, Rgb};

mod export;
mod lex;
mod limits;
mod math;
mod parser;
mod table;
mod units;

#[cfg(test)]
use export::Writer;
pub use export::{escape_text, export};
#[cfg(test)]
use lex::{Tok, tokenize};
#[cfg(test)]
use limits::MAX_NEST;
pub use limits::MAX_SOURCE;
pub use math::{latex_to_linear, linear_to_latex};
pub use parser::{import, parse};
#[cfg(test)]
use units::{length_pt, spec_widths};

#[cfg(test)]
mod tests;

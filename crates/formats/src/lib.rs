//! WordCraft import/export for the "other" text formats: plain text, Markdown (CommonMark subset
//! plus GFM tables and strikethrough), HTML (and MHT web archives, read-only), RTF, OpenDocument
//! Text and LaTeX — and, read-only, spreadsheet cells for mail-merge recipient lists ([`sheet`]:
//! .xlsx/.xlsm/.xls/.ods) and citation libraries for Manage Sources ([`bibtex`], [`ris`],
//! [`csljson`]: .bib/.ris/CSL-JSON, via [`import_sources`]).
//!
//! Every format maps to a small flow model ([`model::Flow`]: paragraphs with a kind, list
//! membership, alignment and formatted spans; tables of cells), which [`model::to_doc`] and
//! [`model::from_doc`] convert to and from the full [`Document`]. All parsers are written from
//! the public specifications, are lenient by design and never panic on hostile input: nesting is
//! depth-limited and sizes are capped.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bibtex;
pub mod comments;
pub mod csljson;
pub mod html;
pub mod latex;
pub mod markdown;
pub mod mathml;
pub mod mht;
pub mod model;
pub mod odt;
pub mod ris;
pub mod rtf;
pub mod sheet;
pub mod txt;

use wordcraft_doc::{Document, Source};

/// Most entries an ODT or spreadsheet package may declare, as many as a Word package
/// (`wordcraft_docx::package::MAX_ENTRIES`); checked before `zip` parses them (#3660).
pub const MAX_ZIP_ENTRIES: u64 = 20_000;

/// Extensions this crate handles.
pub const EXTENSIONS: &[&str] =
    &["txt", "text", "md", "markdown", "html", "htm", "xhtml", "mht", "mhtml", "rtf", "odt", "ott", "fodt", "tex", "latex", "ltx"];

/// Import `bytes` in the format named by `ext` (no dot, any case). `None` = not our format.
pub fn import(ext: &str, bytes: &[u8]) -> Option<Result<Document, String>> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let r = match ext.as_str() {
        "txt" | "text" => Ok(txt::import(bytes)),
        "md" | "markdown" => Ok(markdown::import(&txt::decode(bytes))),
        "html" | "htm" | "xhtml" => Ok(html::import_page(&html::decode(bytes), &|_| None)),
        "mht" | "mhtml" => Ok(mht::import(bytes)),
        "rtf" => rtf::import(bytes),
        "odt" | "ott" => odt::import(bytes),
        "fodt" => odt::import_flat(bytes),
        "tex" | "latex" | "ltx" => Ok(latex::import(bytes)),
        _ => return None,
    };
    Some(r.map(|mut d| {
        d.ensure_nonempty();
        d
    }))
}

/// [`import`] onto `page` (its size and margins) for the formats whose files give no page size:
/// plain text, Markdown, HTML and MHT start on it, and their tables size to it (#1381). The other
/// formats keep the page their file gives.
pub fn import_on(ext: &str, bytes: &[u8], page: &wordcraft_doc::SectionProps) -> Option<Result<Document, String>> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let mut doc = match ext.as_str() {
        "odt" | "ott" => return Some(odt::import_on(bytes, page)),
        "fodt" => return Some(odt::import_flat_on(bytes, page)),
        "txt" | "text" => txt::import_on(bytes, page),
        "md" | "markdown" => markdown::import_on(&txt::decode(bytes), page),
        "html" | "htm" | "xhtml" => html::import_on(&html::decode(bytes), &|_| None, page),
        "mht" | "mhtml" => mht::import_on(bytes, page),
        _ => return import(&ext, bytes),
    };
    doc.ensure_nonempty();
    Some(Ok(doc))
}

// Citation libraries (Manage Sources) are read-only and are not documents: they share the
// crate's caps rather than the flow model.

/// Most bytes of a citation library read (`bib`/`ris`/`json`); longer input is refused. Public
/// so `references.importSources` can refuse an oversized file before reading it in.
pub const MAX_LIBRARY_BYTES: usize = 16 << 20;
/// Most sources kept from one library; further entries are skipped.
const MAX_SOURCES: usize = 10_000;
/// Longest source field value kept, in characters; longer values are cut off.
const MAX_VALUE: usize = 4_000;
/// Deepest brace/bracket nesting read; deeper input is refused.
const MAX_NEST: usize = 32;

/// A field value cut off at [`MAX_VALUE`] characters, at a character boundary.
fn clamp_value(mut s: String) -> String {
    if s.chars().count() > MAX_VALUE {
        let cut = s.char_indices().nth(MAX_VALUE).map_or(s.len(), |(i, _)| i);
        s.truncate(cut);
    }
    s
}

/// Citation-library bytes as text: UTF-8 first (BOM stripped), then Windows-1252 (a superset of
/// latin-1 that also covers curly quotes and dashes), so no byte is lost or replaced.
fn utf8_or_latin1(b: &[u8]) -> String {
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(b);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        // Windows-1252 keeps latin-1's 0xA0–0xFF and turns 0x80–0x9F into the printable
        // characters reference managers on Windows write (curly quotes, dashes, €), where
        // strict latin-1 would keep invisible C1 controls.
        Err(_) => encoding_rs::WINDOWS_1252.decode_without_bom_handling(b).0.into_owned(),
    }
}

/// A citation key made safe for a `CITATION <tag>` field instruction (one whitespace-free
/// word): anything but letters, digits and `_ . / : -` collapses into single dashes.
pub(crate) fn sanitize_tag(key: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in key.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '-') {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    out
}

/// Import a citation library (References ▸ Manage Sources) in the format named by `ext`.
///
/// Extensions this handles: "bib", "ris", "json" (CSL-JSON). `None` = not our format;
/// `Some(Err(msg))` = ours but unusable. Sources are not a [`Document`], so this stands apart
/// from [`import`]; the caps shared by the three parsers are 16 MiB of input, 10,000 sources,
/// 4,000 characters per field value and 32 levels of nesting.
pub fn import_sources(ext: &str, bytes: &[u8]) -> Option<Result<Vec<Source>, String>> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    Some(match ext.as_str() {
        "bib" => bibtex::import(bytes),
        "ris" => ris::import(bytes),
        "json" => csljson::import(bytes),
        _ => return None,
    })
}

/// Export `doc` in the format named by `ext`. `None` = not our format.
pub fn export(ext: &str, doc: &Document) -> Option<Result<Vec<u8>, String>> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    Some(match ext.as_str() {
        "txt" | "text" => Ok(txt::export(doc)),
        "md" | "markdown" => Ok(markdown::export(doc).into_bytes()),
        "html" | "htm" | "xhtml" => Ok(html::export(doc).into_bytes()),
        "rtf" => Ok(rtf::export(doc).into_bytes()),
        "odt" => odt::export(doc),
        "ott" => odt::export_template(doc),
        "fodt" => odt::export_flat(doc),
        "tex" | "latex" | "ltx" => Ok(latex::export(doc).into_bytes()),
        _ => return None,
    })
}

/// How [`export_with`] writes a copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportOptions {
    /// Print Markup: HTML, Markdown and plain text end with the document's comments (who wrote
    /// each, the text it comments on, what it says, its replies). Other formats keep comments
    /// their own way or not at all.
    pub comments: bool,
}

/// [`export`] with options.
pub fn export_with(ext: &str, doc: &Document, opts: &ExportOptions) -> Option<Result<Vec<u8>, String>> {
    let out = export(ext, doc)?;
    if !opts.comments {
        return Some(out);
    }
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    Some(out.map(|bytes| comments::append(&ext, doc, bytes)))
}

#[cfg(test)]
mod test_log;
#[cfg(test)]
mod tests;

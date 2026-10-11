//! WordCraft import/export for the "other" text formats: plain text, Markdown (CommonMark subset
//! plus GFM tables and strikethrough), HTML, RTF, OpenDocument Text and LaTeX — and, read-only,
//! spreadsheet cells for mail-merge recipient lists ([`sheet`]: .xlsx/.xlsm/.ods).
//!
//! Every format maps to a small flow model ([`model::Flow`]: paragraphs with a kind, list
//! membership, alignment and formatted spans; tables of cells), which [`model::to_doc`] and
//! [`model::from_doc`] convert to and from the full [`Document`]. All parsers are written from
//! the public specifications, are lenient by design and never panic on hostile input: nesting is
//! depth-limited and sizes are capped.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod html;
pub mod latex;
pub mod markdown;
pub mod model;
pub mod odt;
pub mod rtf;
pub mod sheet;
pub mod txt;

use wordcraft_doc::Document;

/// Extensions this crate handles.
pub const EXTENSIONS: &[&str] = &["txt", "text", "md", "markdown", "html", "htm", "xhtml", "rtf", "odt", "tex", "latex", "ltx"];

/// Import `bytes` in the format named by `ext` (no dot, any case). `None` = not our format.
pub fn import(ext: &str, bytes: &[u8]) -> Option<Result<Document, String>> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let r = match ext.as_str() {
        "txt" | "text" => Ok(txt::import(bytes)),
        "md" | "markdown" => Ok(markdown::import(&txt::decode(bytes))),
        "html" | "htm" | "xhtml" => Ok(html::import(&html::decode(bytes))),
        "rtf" => rtf::import(bytes),
        "odt" => odt::import(bytes),
        "tex" | "latex" | "ltx" => Ok(latex::import(bytes)),
        _ => return None,
    };
    Some(r.map(|mut d| {
        d.ensure_nonempty();
        d
    }))
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
        "tex" | "latex" | "ltx" => Ok(latex::export(doc).into_bytes()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;

//! Saving a Single File Web Page (`.mht`, `.mhtml`): the page [`crate::html`] writes and its
//! pictures in one MIME message, as RFC 2557 describes (a `multipart/related` message whose
//! first part is the page, RFC 2387).
//!
//! - The page is the root part, `text/html` in UTF-8, quoted-printable (RFC 2045 §6.7), with
//!   the `Content-Location` [`PAGE_LOCATION`].
//! - Each distinct picture is a part of its own, base64 (RFC 2045 §6.8), at
//!   `file:///document_files/imageNNN.ext`; the page names it relative to its own location
//!   (`document_files/imageNNN.ext`), which RFC 2557 §5 resolves against the page's
//!   `Content-Location`, as a browser or Word saves a page with its pictures in a folder beside
//!   it. A picture shown twice is stored once.
//! - The document's title is the message's `Subject` in RFC 2047 encoded words.
//!
//! The output is the same bytes for the same document: no date, and a fixed boundary that no
//! part can contain (see [`BOUNDARY`]). The work is linear in the size of the page.

use std::collections::HashMap;

use wordcraft_doc::Document;

/// The delimiter between parts. It holds `=_`, which neither encoding can produce: in
/// quoted-printable an `=` is always followed by two hex digits or a line break, and base64 has
/// no `_` (RFC 2045 §6.7 and RFC 2046 §5.1.1 suggest this), so no part's content can contain it.
pub(super) const BOUNDARY: &str = "----=_WordCraft_Part_0";
/// Where the page is (its `Content-Location`), which its pictures' relative names resolve
/// against.
pub(super) const PAGE_LOCATION: &str = "file:///document.htm";
/// The folder beside the page that its pictures are in, named relative to the page.
pub(super) const FILES: &str = "document_files";
/// Longest encoded line, line break not counted (RFC 2045 §6.7 rule 5, §6.8).
const MAX_LINE: usize = 76;
/// Most characters of the title written in the `Subject` header.
const MAX_SUBJECT_CHARS: usize = 256;
/// Most UTF-8 bytes in one encoded word: 42 bytes are 56 base64 characters, so a word
/// (`=?utf-8?B?…?=`, 68 characters) and the `Subject: ` before the first stay within the 78
/// characters a header line should keep to (RFC 2047 §2, RFC 5322 §2.1.1).
const WORD_BYTES: usize = 42;
/// How [`crate::html::export`] writes a picture: `<img src="data:<type>;base64,<data>"`.
const IMG_DATA: &str = "<img src=\"data:";

/// One picture part.
struct Picture<'a> {
    /// Its media type, as the page's `data:` URI gave it.
    mime: &'a str,
    /// Its bytes in base64, as the page held them.
    base64: &'a str,
    /// Its file name in [`FILES`].
    name: String,
}

/// `doc` as a Single File Web Page: the page [`crate::html::export`] writes, with its pictures
/// as parts of their own, and the document's title as the message's subject.
pub fn export(doc: &Document) -> Vec<u8> {
    export_html(&crate::html::export(doc), &doc.core.title)
}

/// `html`, a page written by [`crate::html::export`] (with Print Markup's comment list, say), as
/// a Single File Web Page whose subject is `title`. Every picture the page holds as a base64
/// `data:` URI becomes a part the page names by its location; anything else stays as it is.
pub fn export_html(html: &str, title: &str) -> Vec<u8> {
    let (page, pictures) = take_pictures(html);
    let pictures_len: usize = pictures.iter().map(|p| p.base64.len() + p.base64.len() / MAX_LINE * 2 + 160).sum();
    let mut out = String::with_capacity(page.len() + page.len() / 4 + pictures_len + 512);
    out.push_str("MIME-Version: 1.0\r\n");
    if let Some(subject) = subject(title) {
        out.push_str(&subject);
    }
    out.push_str(&format!("Content-Type: multipart/related; type=\"text/html\";\r\n\tboundary=\"{BOUNDARY}\"\r\n\r\n"));
    // No preamble: the first delimiter starts the body.
    out.push_str(&format!("--{BOUNDARY}\r\n"));
    part_headers(&mut out, "text/html; charset=\"utf-8\"", "quoted-printable", PAGE_LOCATION);
    quoted_printable(page.as_bytes(), &mut out);
    for p in &pictures {
        out.push_str(&format!("\r\n--{BOUNDARY}\r\n"));
        part_headers(&mut out, p.mime, "base64", &format!("file:///{FILES}/{}", p.name));
        wrap(p.base64, &mut out);
    }
    out.push_str(&format!("\r\n--{BOUNDARY}--\r\n"));
    out.into_bytes()
}

/// A part's headers and the empty line that ends them.
fn part_headers(out: &mut String, content_type: &str, encoding: &str, location: &str) {
    out.push_str(&format!("Content-Type: {content_type}\r\nContent-Transfer-Encoding: {encoding}\r\nContent-Location: {location}\r\n\r\n"));
}

/// The page with each base64 `data:` picture replaced by its name in [`FILES`], and the
/// pictures, each distinct one once, in the order the page first shows them.
///
/// Only [`crate::html::export`]'s own markup can hold `<img src="data:`: it escapes `<` and `"`
/// in text and attribute values, so text that reads like a picture stays text.
fn take_pictures(html: &str) -> (String, Vec<Picture<'_>>) {
    let mut page = String::with_capacity(html.len());
    let mut pictures: Vec<Picture<'_>> = Vec::new();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut rest = html;
    while let Some(at) = rest.find(IMG_DATA) {
        page.push_str(rest.get(..at).unwrap_or(""));
        // From `data:` to the closing quote.
        let uri_and_after = rest.get(at + IMG_DATA.len() - "data:".len()..).unwrap_or("");
        let uri = uri_and_after.find('"').and_then(|end| uri_and_after.get(..end).map(|uri| (uri, end)));
        match uri.and_then(|(uri, end)| data_uri(uri).map(|d| (d, end))) {
            Some(((mime, base64), end)) => {
                let i = *seen.entry(base64).or_insert_with(|| {
                    let name = format!("image{:03}.{}", pictures.len() + 1, extension(mime));
                    pictures.push(Picture { mime, base64, name });
                    pictures.len() - 1
                });
                page.push_str("<img src=\"");
                page.push_str(FILES);
                page.push('/');
                page.push_str(pictures.get(i).map(|p| p.name.as_str()).unwrap_or(""));
                // The closing quote and the rest of the tag follow.
                rest = uri_and_after.get(end..).unwrap_or("");
            }
            None => {
                page.push_str(IMG_DATA);
                rest = rest.get(at + IMG_DATA.len()..).unwrap_or("");
            }
        }
    }
    page.push_str(rest);
    (page, pictures)
}

/// The media type and base64 data of a `data:<type>;base64,<data>` URI, when the type is a
/// plain `type/subtype` token and the data is base64 (RFC 2397); `None` for anything else.
fn data_uri(uri: &str) -> Option<(&str, &str)> {
    let (mime, data) = uri.strip_prefix("data:")?.split_once(";base64,")?;
    let (kind, sub) = mime.split_once('/')?;
    let token = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b));
    let base64 = !data.is_empty() && data.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b));
    (token(kind) && token(sub) && base64).then_some((mime, data))
}

/// The file extension for a picture of media type `mime`.
fn extension(mime: &str) -> &'static str {
    match mime.to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        "image/x-icon" => "ico",
        _ => "bin",
    }
}

/// Encode `data` quoted-printable (RFC 2045 §6.7) onto `out`: each line break (LF or CRLF) is
/// a CRLF, printable ASCII other than `=` stands as it is, a space or tab too unless it ends a
/// line, and every other byte is `=XX`; lines longer than [`MAX_LINE`] are cut with `=` soft
/// line breaks, never inside an `=XX`.
pub(super) fn quoted_printable(data: &[u8], out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for (n, line) in data.split(|&b| b == b'\n').enumerate() {
        if n > 0 {
            out.push_str("\r\n");
        }
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let mut column = 0;
        for (i, &b) in line.iter().enumerate() {
            let ends_line = i + 1 == line.len();
            let literal = matches!(b, 33..=60 | 62..=126) || (matches!(b, b' ' | b'\t') && !ends_line);
            let width = if literal { 1 } else { 3 };
            // Room for the `=` of a soft line break.
            if column + width > MAX_LINE - 1 {
                out.push_str("=\r\n");
                column = 0;
            }
            if literal {
                out.push(char::from(b));
            } else {
                out.push('=');
                out.push(char::from(HEX.get(usize::from(b >> 4)).copied().unwrap_or(b'0')));
                out.push(char::from(HEX.get(usize::from(b & 15)).copied().unwrap_or(b'0')));
            }
            column += width;
        }
    }
}

/// `base64` in lines of [`MAX_LINE`] characters, CRLF between them.
fn wrap(base64: &str, out: &mut String) {
    for (n, line) in base64.as_bytes().chunks(MAX_LINE).enumerate() {
        if n > 0 {
            out.push_str("\r\n");
        }
        // `take_pictures` only keeps ASCII base64.
        out.push_str(std::str::from_utf8(line).unwrap_or(""));
    }
}

/// The `Subject` header line(s) for `title`: its first [`MAX_SUBJECT_CHARS`] characters, control
/// characters (line breaks among them) as spaces, in RFC 2047 `B` encoded words of whole
/// characters, one per line; `None` for an empty title.
pub(super) fn subject(title: &str) -> Option<String> {
    let clean: String = title.chars().take(MAX_SUBJECT_CHARS).map(|c| if c.is_control() { ' ' } else { c }).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return None;
    }
    let mut words: Vec<String> = Vec::new();
    let mut chunk = String::new();
    for c in clean.chars() {
        if chunk.len() + c.len_utf8() > WORD_BYTES {
            words.push(encoded_word(&chunk));
            chunk.clear();
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(encoded_word(&chunk));
    }
    // Folding white space between encoded words is not part of the text (RFC 2047 §6.2).
    Some(format!("Subject: {}\r\n", words.join("\r\n ")))
}

fn encoded_word(text: &str) -> String {
    format!("=?utf-8?B?{}?=", crate::model::base64_encode(text.as_bytes()))
}

#[cfg(test)]
mod tests;

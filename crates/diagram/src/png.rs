//! Reading a PNG's text chunks: where draw.io keeps the diagram it exported.
//!
//! `File › Export as › PNG` in draw.io (with *Include a copy of my diagram* on, which is the
//! default for PNG) writes the diagram's `mxfile` XML into the PNG as a text chunk whose keyword
//! is `mxfile`, as base64 of a deflated XML string. The picture is then a WordCraft picture like
//! any other — and still editable, because its source is inside it.
//!
//! WordCraft never re-encodes a picture on the way in or out of a `.docx` (the bytes are copied as
//! they are), so the chunk survives a save. What does lose it is anything that rewrites the pixels
//! (Compress Pictures, Corrections…), which writes a fresh PNG.

/// A PNG's 8-byte signature.
const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// The chunk keyword draw.io gives the diagram's XML (`tEXt`/`zTXt`).
const MXFILE: &str = "mxfile";
/// The desktop app writes the bare model under this keyword (`zTXt`, deflated).
const GRAPH_MODEL: &str = "mxGraphModel";
/// A text chunk's inflated text is capped, so a malformed file can't allocate without bound.
const MAX_TEXT: usize = 16 * 1024 * 1024;

/// One text chunk of a PNG: `tEXt`, `zTXt` or `iTXt`, decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChunk {
    /// The chunk's keyword (for draw.io, `mxfile`).
    pub keyword: String,
    /// The text; compressed chunks are inflated. `iTXt` gives its UTF-8 text as bytes.
    pub text: Vec<u8>,
}

/// A PNG's chunks: `(type, data)` for each one, starting after the signature.
///
/// A file that isn't a PNG, or that ends mid-chunk, yields what parses cleanly and stops.
pub struct Chunks<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for Chunks<'a> {
    type Item = (&'a [u8; 4], &'a [u8]);
    fn next(&mut self) -> Option<Self::Item> {
        // length (4) + type (4) + crc (4).
        if self.rest.len() < 12 {
            return None;
        }
        let len = u32::from_be_bytes([self.rest[0], self.rest[1], self.rest[2], self.rest[3]]) as usize;
        // A chunk longer than the rest of the file: truncated or hostile.
        let end = len.checked_add(12)?;
        if end > self.rest.len() {
            return None;
        }
        let (head, tail) = self.rest.split_at(end);
        self.rest = tail;
        let kind: &'a [u8; 4] = head[4..8].try_into().ok()?;
        Some((kind, &head[8..8 + len]))
    }
}

/// Walk a PNG's chunks. `None` when `bytes` doesn't start with the PNG signature.
pub fn chunks(bytes: &[u8]) -> Option<Chunks<'_>> {
    (bytes.len() >= 8 && bytes[..8] == SIG).then(|| Chunks { rest: &bytes[8..] })
}

/// The picture's size in pixels, from the header chunk.
pub fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    let (kind, data) = chunks(bytes)?.next()?;
    if kind != b"IHDR" || data.len() < 8 {
        return None;
    }
    let w = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let h = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    Some((w, h))
}

/// The PNG's text chunks (`tEXt`, `zTXt`, `iTXt`), decoded.
pub fn text_chunks(bytes: &[u8]) -> Vec<TextChunk> {
    let mut out = Vec::new();
    let Some(it) = chunks(bytes) else { return out };
    for (kind, data) in it {
        match kind {
            b"tEXt" => {
                if let Some((keyword, text)) = split_keyword(data) {
                    out.push(TextChunk { keyword, text: text.to_vec() });
                }
            }
            // keyword\0 compression method compressed text
            b"zTXt" => {
                let Some((keyword, rest)) = split_keyword(data) else { continue };
                let Some((_, compressed)) = rest.split_first() else { continue };
                match inflate(compressed) {
                    Some(text) => out.push(TextChunk { keyword, text }),
                    None => log::debug!("PNG zTXt `{keyword}`: its text didn't inflate"),
                }
            }
            // keyword\0 compression flag compression method language\0 translated keyword\0 text
            b"iTXt" => {
                let Some((keyword, rest)) = split_keyword(data) else { continue };
                let Some((&flag, rest)) = rest.split_first() else { continue };
                let Some((_, rest)) = rest.split_first() else { continue };
                let Some(rest) = skip_nul(rest).and_then(skip_nul) else { continue };
                let text = if flag == 1 {
                    match inflate(rest) {
                        Some(text) => text,
                        None => continue,
                    }
                } else {
                    rest.to_vec()
                };
                out.push(TextChunk { keyword, text });
            }
            _ => {}
        }
    }
    out
}

/// Is this PNG a draw.io diagram: a PNG with the diagram's XML inside it?
///
/// Both spellings draw.io writes count: `mxfile` (the web app's and the CLI's export, base64 of
/// a deflated XML) and `mxGraphModel` (the desktop app's *Include a copy of my diagram*, a
/// deflated bare model).
pub fn is_drawio(bytes: &[u8]) -> bool {
    let Some(it) = chunks(bytes) else { return false };
    it.filter(|(kind, _)| matches!(*kind, b"tEXt" | b"zTXt" | b"iTXt"))
        .filter_map(|(_, data)| split_keyword(data))
        .any(|(keyword, _)| is_diagram_keyword(&keyword))
}

/// The diagram's XML, if this PNG carries it.
///
/// The `mxfile` chunk holds base64 of the deflated XML (or, in older files and some tools, the
/// XML as it stands); the desktop app's `mxGraphModel` chunk holds the deflated XML directly.
/// `None` when there is no diagram chunk, or when its text won't decode.
pub fn drawio_xml(bytes: &[u8]) -> Option<String> {
    let chunk = text_chunks(bytes).into_iter().find(|c| is_diagram_keyword(&c.keyword))?;
    decode_xml(&chunk.text)
}

/// Either spelling of the chunk that carries a diagram.
fn is_diagram_keyword(keyword: &str) -> bool {
    keyword.eq_ignore_ascii_case(MXFILE) || keyword.eq_ignore_ascii_case(GRAPH_MODEL)
}

/// `keyword\0text`: the keyword is Latin-1, and not empty nor longer than 79 bytes.
fn split_keyword(data: &[u8]) -> Option<(String, &[u8])> {
    let at = data.iter().position(|b| *b == 0)?;
    let (keyword, text) = data.split_at(at);
    if keyword.is_empty() || keyword.len() > 79 {
        return None;
    }
    Some((String::from_utf8_lossy(keyword).into_owned(), &text[1..]))
}

/// Past one NUL-terminated string.
fn skip_nul(data: &[u8]) -> Option<&[u8]> {
    let at = data.iter().position(|b| *b == 0)?;
    Some(&data[at + 1..])
}

/// Inflate `data`: draw.io deflates with a zlib wrapper, some tools write raw deflate.
fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use miniz_oxide::inflate::{decompress_to_vec, decompress_to_vec_zlib_with_limit};
    decompress_to_vec_zlib_with_limit(data, MAX_TEXT).ok().or_else(|| decompress_to_vec(data).ok())
}

/// base64 of deflated XML, the XML as it stands, or the desktop app's percent-encoded XML.
fn decode_xml(text: &[u8]) -> Option<String> {
    use base64::Engine as _;
    let first = text.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(text.len());
    let last = text.len() - text.iter().rev().take_while(|b| b.is_ascii_whitespace()).count();
    let trimmed = &text[first..last.max(first)];
    if trimmed.first() == Some(&b'<') {
        return String::from_utf8(trimmed.to_vec()).ok();
    }
    if trimmed.first() == Some(&b'%') {
        return percent_decode(trimmed);
    }
    let raw = base64::engine::general_purpose::STANDARD.decode(trimmed).ok()?;
    let xml = inflate(&raw).unwrap_or(raw);
    String::from_utf8(xml).ok()
}

/// `%3Cmxfile%20…` to text (`encodeURIComponent`'s form: `+` is literal).
fn percent_decode(text: &[u8]) -> Option<String> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'%' && i + 3 <= text.len() {
            out.push(u8::from_str_radix(std::str::from_utf8(&text[i + 1..i + 3]).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(text[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG with an `IHDR` and a single `tEXt` chunk, written by hand.
    fn png_with(keyword: &str, text: &[u8]) -> Vec<u8> {
        let mut out = SIG.to_vec();
        let mut ihdr = 40u32.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&30u32.to_be_bytes());
        ihdr.push(8); // bit depth
        ihdr.push(6); // colour type
        out.extend_from_slice(&(ihdr.len() as u32).to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&ihdr);
        out.extend_from_slice(&0u32.to_be_bytes());

        let mut data = keyword.as_bytes().to_vec();
        data.push(0);
        data.extend_from_slice(text);
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(b"tEXt");
        out.extend_from_slice(&data);
        out.extend_from_slice(&0u32.to_be_bytes());
        out
    }

    /// Deflate `xml` as one stored zlib block, with the Adler-32 a zlib stream ends with:
    /// enough to check the reader (no compressor here).
    fn zlib_stored(xml: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let parts: Vec<&[u8]> = xml.chunks(0xffff).collect();
        for (i, part) in parts.iter().enumerate() {
            // BFINAL, then BTYPE 00 (stored), then the length and its one's complement.
            out.push(u8::from(i == parts.len() - 1));
            let n = part.len() as u16;
            out.extend_from_slice(&n.to_le_bytes());
            out.extend_from_slice(&(!n).to_le_bytes());
            out.extend_from_slice(part);
        }
        let mut a = 1u32;
        let mut b = 0u32;
        for byte in xml {
            a = (a + u32::from(*byte)) % 65521;
            b = (b + a) % 65521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    #[test]
    fn reads_the_header_size() {
        assert_eq!(size(&png_with("mxfile", b"<mxfile/>")), Some((40, 30)));
    }

    #[test]
    fn spots_a_drawio_png() {
        let png = png_with("mxfile", b"<mxfile><diagram/></mxfile>");
        assert!(is_drawio(&png));
        assert_eq!(drawio_xml(&png).as_deref(), Some("<mxfile><diagram/></mxfile>"));
    }

    #[test]
    fn a_plain_png_is_not_a_diagram() {
        let png = png_with("Title", b"holiday");
        assert!(!is_drawio(&png));
        assert_eq!(drawio_xml(&png), None);
    }

    #[test]
    fn reads_base64_of_deflated_xml() {
        use base64::Engine as _;
        let xml = b"<mxfile><diagram name=\"Page-1\"/></mxfile>";
        let text = base64::engine::general_purpose::STANDARD.encode(zlib_stored(xml));
        let png = png_with("mxfile", text.as_bytes());
        assert!(is_drawio(&png));
        assert_eq!(drawio_xml(&png).as_deref(), Some(std::str::from_utf8(xml).unwrap()));
    }

    #[test]
    fn a_chunk_that_runs_past_the_end_is_dropped() {
        let mut png = png_with("mxfile", b"<mxfile/>");
        png.truncate(png.len() - 3);
        assert_eq!(size(&png), Some((40, 30)), "what came before it still reads");
        assert!(!is_drawio(&png));
    }

    #[test]
    fn something_that_is_not_a_png_is_ignored() {
        assert_eq!(size(b"not a png at all, not at all"), None);
        assert!(!is_drawio(b"not a png at all, not at all"));
    }

    /// Appends a `zTXt` chunk: `keyword\0` method, then the zlib stream — what the desktop app
    /// writes for *Include a copy of my diagram*.
    fn with_ztxt(mut png: Vec<u8>, keyword: &str, zlib: &[u8]) -> Vec<u8> {
        let mut data = keyword.as_bytes().to_vec();
        data.push(0);
        data.push(0); // the compression method: zlib
        data.extend_from_slice(zlib);
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(b"zTXt");
        png.extend_from_slice(&data);
        png.extend_from_slice(&0u32.to_be_bytes());
        png
    }

    #[test]
    fn reads_the_desktop_app_ztxt_model() {
        let xml = b"<mxGraphModel><root/></mxGraphModel>";
        let png = with_ztxt(png_with("Title", b"holiday"), GRAPH_MODEL, &zlib_stored(xml));
        assert!(is_drawio(&png));
        assert_eq!(drawio_xml(&png).as_deref(), Some(std::str::from_utf8(xml).unwrap()));
    }

    #[test]
    fn reads_the_desktop_app_url_encoded_model() {
        // The real desktop spelling: a `zTXt` whose payload is **raw** deflate (no zlib wrapper)
        // of percent-encoded XML.
        let enc: &[u8] = b"%3Cmxfile%20host%3D%22app.diagrams.net%22%3E%3C/mxfile%3E";
        let zlib = zlib_stored(enc);
        let raw = &zlib[2..zlib.len() - 4]; // strip the zlib header and the Adler-32
        let png = with_ztxt(png_with("Title", b"holiday"), GRAPH_MODEL, raw);
        assert!(is_drawio(&png));
        assert_eq!(drawio_xml(&png).as_deref(), Some("<mxfile host=\"app.diagrams.net\"></mxfile>"));
    }

    #[test]
    fn reads_the_editor_exported_url_encoded_text() {
        // What the editor's own export comes back as: a plain `tEXt` whose text is the
        // percent-encoded XML, no base64 and no deflate.
        let png = png_with(MXFILE, b"%3Cmxfile%20host%3D%22local%22%3E%3C/mxfile%3E");
        assert!(is_drawio(&png));
        assert_eq!(drawio_xml(&png).as_deref(), Some("<mxfile host=\"local\"></mxfile>"));
    }
}

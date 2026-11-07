//! A tiny in-code `.docx` builder for the integration tests. Documents are assembled as zip
//! entries; knobs let a test mutate one part at a time to violate exactly one rule.

use std::io::{Cursor, Write};

use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// The namespaces a normal WordprocessingML document declares.
pub const NS: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
    r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#,
);

/// One namespace declaration for a scan fixture, without the built-in ones.
pub fn ns(w: bool, wp: bool) -> String {
    let mut s = String::new();
    if w {
        s.push_str(r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#);
    }
    if wp {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing""#);
    }
    s
}

pub const CT_HEADER: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
pub const CT_FOOTER: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
pub const CT_FOOTNOTES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";
pub const CT_ENDNOTES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml";
pub const CT_COMMENTS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
pub const CT_COMMENTS_EXT: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
pub const CT_COMMENTS_IDS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsIds+xml";
pub const CT_NUMBERING: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
pub const CT_SETTINGS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";
pub const CT_CUSTOMXML_PROPS: &str = "application/vnd.openxmlformats-officedocument.customXmlProperties+xml";
pub const CT_GLOSSARY: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml";
pub const CT_STYLES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
pub const CT_MAIN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
pub const CT_TEMPLATE_MAIN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";
pub const CT_CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
pub const CT_PEOPLE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml";
pub const CT_WEB_SETTINGS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml";

/// `[Content_Types].xml` with the two standard defaults, the main-document override, then the
/// extra [`defaults`] and [`overrides`] the caller names.
pub fn content_types_xml(defaults: &[(&str, &str)], overrides: &[(&str, &str)]) -> Vec<u8> {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    );
    s.push_str(r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#);
    s.push_str(r#"<Default Extension="xml" ContentType="application/xml"/>"#);
    for (ext, ct) in defaults {
        s.push_str(&format!(r#"<Default Extension="{ext}" ContentType="{ct}"/>"#));
    }
    s.push_str(&format!(r#"<Override PartName="/word/document.xml" ContentType="{CT_MAIN}"/>"#));
    for (part, ct) in overrides {
        s.push_str(&format!(r#"<Override PartName="{part}" ContentType="{ct}"/>"#));
    }
    s.push_str("</Types>");
    s.into_bytes()
}

/// The content-type overrides a document with a header, footnotes, comments, styles and a PNG.
pub fn rich_content_types() -> Vec<u8> {
    content_types_xml(
        &[("png", "image/png")],
        &[
            ("/word/header1.xml", CT_HEADER),
            ("/word/footnotes.xml", CT_FOOTNOTES),
            ("/word/comments.xml", CT_COMMENTS),
            ("/word/styles.xml", CT_STYLES),
        ],
    )
}

/// A document part with `body` as the body content and the standard namespaces.
pub fn document_xml(body: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}</w:body></w:document>"#).into_bytes()
}

/// A relationships part with the given `<Relationship>` children.
pub fn rels_xml(children: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{children}</Relationships>"#
    )
    .into_bytes()
}

/// The package relationships that point at the main document part.
pub fn package_rels_xml() -> Vec<u8> {
    rels_xml(
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#,
    )
}

/// Replace every occurrence of the same-length `from` with `to` (used to force a duplicate zip
/// entry name, which the zip writer itself refuses to create).
pub fn replace_all(data: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len(), "replacement must preserve length");
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

/// Corrupt the compressed data of `name` in a zip so it fails to inflate (CRC mismatch), leaving
/// the central directory intact (so the package is still a readable zip but the part is unusable).
pub fn corrupt_entry_data(zip: &[u8], name: &str) -> Vec<u8> {
    const LOCAL: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
    let mut out = zip.to_vec();
    for i in 0..out.len().saturating_sub(30) {
        if out.get(i..i + 4) != Some(&LOCAL) {
            continue;
        }
        let nlen = usize::from(u16::from_le_bytes([out[i + 26], out[i + 27]]));
        let elen = usize::from(u16::from_le_bytes([out[i + 28], out[i + 29]]));
        if out.get(i + 30..i + 30 + nlen) != Some(name.as_bytes()) {
            continue;
        }
        let data = i + 30 + nlen + elen;
        if data < out.len() {
            out[data] ^= 0xFF;
        }
        return out;
    }
    out
}

/// Claim `method` (a ZIP compression-method id) for `name` in both the local header and the
/// central directory, so the reader cannot open the entry though the archive itself stays
/// readable. The stored bytes are untouched.
pub fn claim_entry_method(zip: &[u8], name: &str, method: u16) -> Vec<u8> {
    const LOCAL: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
    const CENTRAL: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
    let mut out = zip.to_vec();
    // (signature, name-length field, name field, method field). The two headers place these at
    // different offsets, so all four are given explicitly.
    for (sig, nlen_at, name_at, method_at) in [(LOCAL, 26usize, 30usize, 8usize), (CENTRAL, 28, 46, 10)] {
        for i in 0..out.len().saturating_sub(30) {
            if out.get(i..i + 4) != Some(&sig) {
                continue;
            }
            let nlen = usize::from(u16::from_le_bytes([out[i + nlen_at], out[i + nlen_at + 1]]));
            if out.get(i + name_at..i + name_at + nlen) == Some(name.as_bytes()) {
                out[i + method_at..i + method_at + 2].copy_from_slice(&method.to_le_bytes());
            }
        }
    }
    out
}

/// A document under construction.
pub struct Doc {
    pub entries: Vec<(String, Vec<u8>)>,
}

impl Doc {
    /// The minimal valid `.docx`: content types, package rels, a one-paragraph main part and an
    /// empty main relationships part.
    pub fn minimal() -> Doc {
        Doc {
            entries: vec![
                ("[Content_Types].xml".to_string(), content_types_xml(&[], &[])),
                ("_rels/.rels".to_string(), package_rels_xml()),
                ("word/document.xml".to_string(), document_xml(r#"<w:p><w:r><w:t>Hello</w:t></w:r></w:p>"#)),
                ("word/_rels/document.xml.rels".to_string(), rels_xml("")),
            ],
        }
    }

    /// Set a part, adding it when it does not exist yet.
    pub fn set(&mut self, name: &str, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        match self.entries.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = bytes,
            None => self.entries.push((name.to_string(), bytes)),
        }
    }

    /// Remove a part.
    pub fn remove(&mut self, name: &str) {
        self.entries.retain(|(n, _)| n != name);
    }

    /// Replace the main document part's body.
    pub fn set_document(&mut self, body: &str) {
        self.set("word/document.xml", document_xml(body));
    }

    /// Replace the raw main document part bytes.
    pub fn set_document_raw(&mut self, xml: &str) {
        self.set("word/document.xml", xml.as_bytes().to_vec());
    }

    /// Replace the main relationships part.
    pub fn set_doc_rels(&mut self, children: &str) {
        self.set("word/_rels/document.xml.rels", rels_xml(children));
    }

    /// Add a part and the override that gives it a content type.
    pub fn add_with_override(&mut self, name: &str, content_type: &str, bytes: impl Into<Vec<u8>>) {
        self.set(name, bytes);
        let mut types = String::from_utf8(self.get("[Content_Types].xml").to_vec()).expect("utf-8");
        let insert = format!(r#"<Override PartName="/{name}" ContentType="{content_type}"/>"#);
        if !types.contains(&insert) {
            types = types.replace("</Types>", &format!("{insert}</Types>"));
            self.set("[Content_Types].xml", types);
        }
    }

    /// The raw bytes of a part (panics when absent).
    pub fn get(&self, name: &str) -> &[u8] {
        &self.entries.iter().find(|(n, _)| n == name).expect("part exists").1
    }

    /// Build the zip bytes.
    pub fn build(&self) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, bytes) in &self.entries {
            w.start_file(name.as_str(), opts).expect("start file");
            w.write_all(bytes).expect("write file");
        }
        w.finish().expect("finish zip").into_inner()
    }
}

/// A `.docx` with a header relationship and part (used by the rich and REL/REF-HEADER fixtures).
pub fn add_header(doc: &mut Doc) {
    doc.set("word/header1.xml", format!(r#"<?xml version="1.0"?><w:hdr {NS}><w:p><w:r><w:t>Head</w:t></w:r></w:p></w:hdr>"#));
}

/// Encode `text` as UTF-16LE with a byte-order mark, for the encoding canary.
pub fn utf16_le_with_bom(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

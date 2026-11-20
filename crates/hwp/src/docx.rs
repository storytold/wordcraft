//! A minimal WordprocessingML (.docx) package writer for the converter's output.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::BTreeMap;

use crate::error::{Error, Result};

pub const W_NS: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" "#,
    r#"xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" "#,
    r#"xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup""#
);
pub const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub fn rel(kind: &str) -> String {
    format!("{R}/{kind}")
}

const CT: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.";
pub fn ct(kind: &str) -> String {
    format!("{CT}{kind}+xml")
}
pub const CT_CORE: &str = "application/vnd.openxmlformats-package.core-properties+xml";
pub const CT_CUSTOM: &str = "application/vnd.openxmlformats-officedocument.custom-properties+xml";

const IMAGE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("bmp", "image/bmp"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("emf", "image/x-emf"),
    ("wmf", "image/x-wmf"),
    ("svg", "image/svg+xml"),
    ("webp", "image/webp"),
];

/// The picture type its first bytes tell, for a picture whose name gives none we know.
fn sniff_image(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG") {
        Some("png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if data.starts_with(b"GIF8") {
        Some("gif")
    } else if data.starts_with(b"BM") {
        Some("bmp")
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        Some("tif")
    } else {
        None
    }
}

fn image_type(ext: &str) -> Option<&'static str> {
    IMAGE_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, t)| *t)
}

/// XML text escaping (& < >), as Python's `xml.sax.saxutils.escape`.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            _ => o.push(c),
        }
    }
    o
}

/// A quoted attribute value, as Python's `quoteattr` (double quotes unless the value has some).
pub fn attr(s: &str) -> String {
    let mut v = esc(s).replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;");
    if v.contains('"') {
        if v.contains('\'') {
            v = v.replace('"', "&quot;");
            format!("\"{v}\"")
        } else {
            format!("'{v}'")
        }
    } else {
        format!("\"{v}\"")
    }
}

/// Picture bytes in bounds: the size and the format sniffed from the first bytes.
pub fn pixel_size(data: &[u8]) -> Option<(u32, u32)> {
    let be32 = |o: usize| data.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let le16 = |o: usize| data.get(o..o + 2).map(|b| u32::from(u16::from_le_bytes([b[0], b[1]])));
    let be16 = |o: usize| data.get(o..o + 2).map(|b| u32::from(u16::from_be_bytes([b[0], b[1]])));
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if data.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if data.starts_with(b"BM") {
        let w = data.get(18..22).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))?;
        let h = data.get(22..26).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))?;
        return Some((w.unsigned_abs(), h.unsigned_abs()));
    }
    if data.starts_with(b"\xff\xd8") {
        let mut i = 2usize;
        let mut guard = 0;
        while i + 9 < data.len() && data.get(i) == Some(&0xFF) && guard < 10_000 {
            guard += 1;
            let marker = *data.get(i + 1)?;
            let length = be16(i + 2)? as usize;
            if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                return Some((be16(i + 7)?, be16(i + 5)?));
            }
            i = i.checked_add(2 + length)?;
        }
    }
    None
}

#[derive(Clone, Debug, Default)]
pub struct Part {
    /// (id, type, target, external)
    pub rels: Vec<(String, String, String, bool)>,
}

impl Part {
    pub fn add_rel(&mut self, kind: &str, target: &str, external: bool) -> String {
        if let Some((id, ..)) = self.rels.iter().find(|(_, t, g, e)| t == kind && g == target && *e == external) {
            return id.clone();
        }
        let id = format!("rId{}", self.rels.len() + 1);
        self.rels.push((id.clone(), kind.to_string(), target.to_string(), external));
        id
    }

    fn rels_xml(&self) -> String {
        let mut o = format!("{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
        for (id, t, tgt, ext) in &self.rels {
            let mode = if *ext { " TargetMode=\"External\"" } else { "" };
            o.push_str(&format!("<Relationship Id={} Type={} Target={}{mode}/>", attr(id), attr(t), attr(tgt)));
        }
        o.push_str("</Relationships>");
        o
    }
}

/// Parts are written in the order they were first added (Python dict order).
#[derive(Clone, Debug, Default)]
pub struct DocxPackage {
    pub files: Vec<(String, Vec<u8>)>,
    pub overrides: BTreeMap<String, String>,
    pub parts: Vec<(String, Part)>,
    /// source key -> media path relative to word/
    pub media: Vec<(String, String)>,
    media_n: usize,
    pub custom: Vec<(String, String)>,
}

impl DocxPackage {
    pub fn part(&mut self, name: &str) -> &mut Part {
        let i = match self.parts.iter().position(|(n, _)| n == name) {
            Some(i) => i,
            None => {
                self.parts.push((name.to_string(), Part::default()));
                self.parts.len() - 1
            }
        };
        // `i` indexes an element that exists (found or just pushed).
        &mut self.parts[i].1
    }

    pub fn put(&mut self, name: &str, xml: &str, content_type: Option<&str>) {
        self.set_file(name, xml.as_bytes().to_vec());
        if let Some(ct) = content_type {
            self.overrides.insert(format!("/{name}"), ct.to_string());
        }
    }

    fn set_file(&mut self, name: &str, data: Vec<u8>) {
        match self.files.iter_mut().find(|(n, _)| n == name) {
            Some(f) => f.1 = data,
            None => self.files.push((name.to_string(), data)),
        }
    }

    /// Store a picture once; its path relative to `word/` (for relationships).
    pub fn add_media(&mut self, key: &str, data: &[u8], ext: &str) -> String {
        if let Some((_, p)) = self.media.iter().find(|(k, _)| k == key) {
            return p.clone();
        }
        self.media_n += 1;
        let ext = ext.to_ascii_lowercase();
        // a type we don't know by name is named by its bytes, so the part's name and content
        // type say what it holds (PNG only when nothing tells)
        let ext = if image_type(&ext).is_some() { ext } else { sniff_image(data).unwrap_or("png").to_string() };
        let path = format!("media/image{}.{ext}", self.media_n);
        self.set_file(&format!("word/{path}"), data.to_vec());
        self.media.push((key.to_string(), path.clone()));
        path
    }

    pub fn finish(mut self) -> Result<Vec<u8>> {
        if !self.custom.is_empty() {
            let props: String = self
                .custom
                .iter()
                .enumerate()
                .map(|(i, (k, v))| {
                    format!(
                        "<property fmtid=\"{{D5CDD505-2E9C-101B-9397-08002B2CF9AE}}\" pid=\"{}\" name={}><vt:lpwstr>{}</vt:lpwstr></property>",
                        i + 2,
                        attr(k),
                        esc(v)
                    )
                })
                .collect();
            let xml = format!(
                "{DECL}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/custom-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\">{props}</Properties>"
            );
            self.put("docProps/custom.xml", &xml, Some(CT_CUSTOM));
        }
        let mut exts: Vec<String> = self
            .files
            .iter()
            .filter(|(n, _)| n.starts_with("word/media/"))
            .filter_map(|(n, _)| n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
            .collect();
        exts.sort();
        exts.dedup();
        let mut ct = format!(
            "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>"
        );
        for e in &exts {
            ct.push_str(&format!("<Default Extension={} ContentType={}/>", attr(e), attr(image_type(e).unwrap_or("application/octet-stream"))));
        }
        for (n, t) in &self.overrides {
            ct.push_str(&format!("<Override PartName={} ContentType={}/>", attr(n), attr(t)));
        }
        ct.push_str("</Types>");
        let custom = self.files.iter().any(|(n, _)| n == "docProps/custom.xml");
        let root_rels = format!(
            "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{R}/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>{}</Relationships>",
            if custom { format!("<Relationship Id=\"rId3\" Type=\"{R}/custom-properties\" Target=\"docProps/custom.xml\"/>") } else { String::new() }
        );
        {
            let mut z = craft_zip::Writer::new();
            let mut add = |name: &str, data: &[u8]| -> Result<()> { z.add(name, data, craft_zip::Compression::Deflated).map_err(Error::format) };
            add("[Content_Types].xml", ct.as_bytes())?;
            add("_rels/.rels", root_rels.as_bytes())?;
            for (name, part) in &self.parts {
                if !part.rels.is_empty() {
                    let (d, f) = name.rsplit_once('/').unwrap_or(("", name));
                    add(&format!("{d}/_rels/{f}.rels"), part.rels_xml().as_bytes())?;
                }
            }
            for (name, data) in &self.files {
                add(name, data)?;
            }
            z.finish().map_err(Error::format)
        }
    }
}

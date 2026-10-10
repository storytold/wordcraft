//! OPC package access: bounded zip reading, relationships, part names.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use crate::DocxError;
use crate::xml::{self, El};

/// Largest single part we decompress.
pub const MAX_PART: u64 = 256 * 1024 * 1024;
/// Largest total of all decompressed parts.
pub const MAX_TOTAL: u64 = 512 * 1024 * 1024;
/// Most zip entries we look at.
pub const MAX_ENTRIES: usize = 20_000;

/// Relationship type suffixes (after `.../relationships/`).
pub mod rt {
    pub const OFFICE_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
    pub const CORE: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
    pub const APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
    pub const CUSTOM: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";
    pub const STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
    pub const NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
    pub const SETTINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
    pub const THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
    pub const FOOTNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
    pub const ENDNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes";
    pub const COMMENTS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
    pub const COMMENTS_EX: &str = "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";
    pub const HEADER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
    pub const FOOTER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";
    pub const IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
    pub const HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    pub const BIBLIOGRAPHY: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/bibliography";
}

/// Does relationship type `t` end with `suffix` (ignoring the transitional/strict prefix)?
pub fn rel_is(t: &str, full: &str) -> bool {
    let tail = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    t == full || tail(t) == tail(full)
}

/// The decompressed parts of a package (bounded).
pub struct Package {
    files: BTreeMap<String, Vec<u8>>,
}

impl Package {
    pub fn open(bytes: &[u8]) -> Result<Package, DocxError> {
        Package::open_limited(bytes, MAX_PART, MAX_TOTAL)
    }

    pub fn open_limited(bytes: &[u8], max_part: u64, max_total: u64) -> Result<Package, DocxError> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| DocxError::Zip(e.to_string()))?;
        if zip.len() > MAX_ENTRIES {
            return Err(DocxError::Limit(format!("{} zip entries", zip.len())));
        }
        let mut files = BTreeMap::new();
        let mut total: u64 = 0;
        for i in 0..zip.len() {
            let mut f = match zip.by_index(i) {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("docx: skipping unreadable zip entry {i}: {e}");
                    continue;
                }
            };
            if f.is_dir() {
                continue;
            }
            let name = f.name().trim_start_matches('/').to_string();
            if f.size() > max_part {
                return Err(DocxError::Limit(format!("part {name} declares {} bytes", f.size())));
            }
            let room = max_part.min(max_total.saturating_sub(total));
            let mut buf = Vec::with_capacity(f.size().min(16 * 1024 * 1024) as usize);
            let read = (&mut f).take(room + 1).read_to_end(&mut buf);
            if let Err(e) = read {
                // A corrupt entry: keep going unless it's a part we can't do without.
                log::warn!("docx: zip entry {name} unreadable: {e}");
                continue;
            }
            if buf.len() as u64 > room {
                return Err(DocxError::Limit(format!("part {name} decompresses past the size limit")));
            }
            total = total.saturating_add(buf.len() as u64);
            files.insert(name, buf);
        }
        Ok(Package { files })
    }

    /// Part bytes by name (leading `/` ignored; case-insensitive fallback).
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let n = name.trim_start_matches('/');
        if let Some(v) = self.files.get(n) {
            return Some(v);
        }
        self.files.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_slice())
    }

    pub fn xml(&self, name: &str) -> Result<Option<El>, DocxError> {
        match self.get(name) {
            Some(b) => xml::parse(b).map(Some),
            None => Ok(None),
        }
    }

    /// Relationships of `part` (from `dir/_rels/name.rels`).
    pub fn rels(&self, part: &str) -> Rels {
        let (dir, file) = split_dir(part);
        let path = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let mut rels = Rels::default();
        let Ok(Some(root)) = self.xml(&path) else { return rels };
        for r in root.els().filter(|e| e.local() == "Relationship") {
            let (Some(id), Some(target)) = (r.attr("Id"), r.attr("Target")) else { continue };
            let external = r.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
            let resolved = if external { target.to_string() } else { resolve(dir, target) };
            rels.list.push(Rel { id: id.to_string(), kind: r.attr("Type").unwrap_or("").to_string(), target: resolved, external });
        }
        rels
    }
}

#[derive(Clone, Debug)]
pub struct Rel {
    pub id: String,
    pub kind: String,
    /// Package path (internal) or URL (external).
    pub target: String,
    pub external: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Rels {
    pub list: Vec<Rel>,
}

impl Rels {
    pub fn by_id(&self, id: &str) -> Option<&Rel> {
        self.list.iter().find(|r| r.id == id)
    }
    pub fn by_type(&self, full: &str) -> Option<&Rel> {
        self.list.iter().find(|r| rel_is(&r.kind, full))
    }
}

fn split_dir(part: &str) -> (&str, &str) {
    let p = part.trim_start_matches('/');
    match p.rfind('/') {
        Some(i) => (p.get(..i).unwrap_or(""), p.get(i + 1..).unwrap_or(p)),
        None => ("", p),
    }
}

/// Resolve a relative target against a source directory into a package path.
pub fn resolve(dir: &str, target: &str) -> String {
    let target = target.split('#').next().unwrap_or(target);
    let mut parts: Vec<&str> = if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Build a zip from (name, bytes) entries.
pub fn zip_entries(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, DocxError> {
    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(6));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        // Already-compressed media gain nothing from deflate.
        let lower = name.to_ascii_lowercase();
        let o = if [".png", ".jpg", ".jpeg", ".gif", ".webp"].iter().any(|e| lower.ends_with(e)) { stored } else { opts };
        zw.start_file(name.as_str(), o).map_err(|e| DocxError::Zip(e.to_string()))?;
        zw.write_all(bytes).map_err(|e| DocxError::Zip(e.to_string()))?;
    }
    let c = zw.finish().map_err(|e| DocxError::Zip(e.to_string()))?;
    Ok(c.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let e: Vec<(String, Vec<u8>)> = entries.iter().map(|(n, b)| (n.to_string(), b.clone())).collect();
        zip_entries(&e).unwrap()
    }

    #[test]
    fn decompression_bomb_is_rejected() {
        let z = zip_of(&[("word/document.xml", vec![b' '; 2_000_000])]);
        assert!(z.len() < 100_000);
        assert!(matches!(Package::open_limited(&z, 1_000_000, 10_000_000), Err(DocxError::Limit(_))));
        assert!(matches!(Package::open_limited(&z, 10_000_000, 1_000_000), Err(DocxError::Limit(_))));
        assert!(Package::open_limited(&z, 10_000_000, 10_000_000).is_ok());
    }

    #[test]
    fn lying_declared_size_is_rejected_or_bounded() {
        let mut z = zip_of(&[("a.xml", b"<a/>".to_vec())]);
        // Patch the uncompressed size in the central directory and local header to ~4 GB.
        let mut patched = 0;
        for sig in [[0x50u8, 0x4B, 0x01, 0x02], [0x50, 0x4B, 0x03, 0x04]] {
            if let Some(pos) = z.windows(4).position(|w| w == sig) {
                let off = if sig[2] == 1 { pos + 24 } else { pos + 22 };
                z[off..off + 4].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
                patched += 1;
            }
        }
        assert_eq!(patched, 2);
        match Package::open(&z) {
            Err(_) => {}
            Ok(p) => assert!(p.get("a.xml").is_none_or(|b| b.len() < 1024)),
        }
    }

    #[test]
    fn resolves_targets() {
        assert_eq!(resolve("word", "media/image1.png"), "word/media/image1.png");
        assert_eq!(resolve("word", "../customXml/item1.xml"), "customXml/item1.xml");
        assert_eq!(resolve("word", "/word/styles.xml"), "word/styles.xml");
        assert_eq!(resolve("", "word/document.xml"), "word/document.xml");
        assert_eq!(resolve("word", "../../../x"), "x");
    }
}

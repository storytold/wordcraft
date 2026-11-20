//! The HWPX container: a ZIP archive (OCF-style). `META-INF/container.xml` points at the package
//! file (`Contents/content.hpf`, an OPF document) whose manifest lists the header part, the
//! section parts and the binary items (pictures), and whose spine gives the reading order.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::xml::{self, Element};

/// One part, or one picture.
const MAX_PART_BYTES: u64 = 256 * 1024 * 1024;
/// The whole archive, uncompressed.
const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: u64 = 65_536;
const LIMITS: craft_zip::Limits = craft_zip::Limits { max_entries: MAX_ENTRIES, max_total_bytes: MAX_TOTAL_BYTES };

/// One archive entry, in archive order (kept for writing the package back).
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
    /// Stored (not deflated) in the original: `mimetype` must stay so.
    pub stored: bool,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub href: String,
    pub media_type: String,
}

/// An opened HWPX document, or an HWP 5.0 document rebuilt as one (see [`crate::hwp5`]).
#[derive(Clone, Debug, Default)]
pub struct Package {
    pub entries: Vec<Entry>,
    index: HashMap<String, usize>,
    /// The first `BinData/` entry with each file stem (its name without the folder and the
    /// extension): pictures referenced by stem only are found without going through every entry.
    bin_stem_at: HashMap<String, usize>,
    /// Manifest items; set with [`Package::set_items`], which keeps `item_at` in step.
    pub(crate) items: Vec<Item>,
    /// Where the first item with each id is in `items`, so finding a section or a picture
    /// doesn't go through the whole manifest (a manifest of a million items).
    item_at: HashMap<String, usize>,
    pub spine: Vec<String>,
    pub metadata: Vec<(String, String)>,
    pub rootfile: Option<String>,
    pub header_name: Option<String>,
    pub section_names: Vec<String>,
    /// HWP 5.0: the header rebuilt from DocInfo; `entries` hold the decompressed section and
    /// picture streams.
    pub(crate) hwp5_header: Option<Element>,
}

/// A ZIP archive whose first entry is an HWPX `mimetype` (or which holds HWPX parts).
pub fn is_hwpx(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"PK\x03\x04") {
        return false;
    }
    let Ok(mut z) = craft_zip::Archive::open(bytes, LIMITS) else { return false };
    if let Ok(Some(head)) = z.read_head("mimetype", 64)
        && String::from_utf8_lossy(&head).trim().contains("hwp")
    {
        return true;
    }
    z.names().any(|n| n.starts_with("Contents/section") && n.ends_with(".xml"))
}

impl Package {
    pub fn open(bytes: &[u8]) -> Result<Package> {
        let mut z = craft_zip::Archive::open(bytes, LIMITS).map_err(|e| match e {
            craft_zip::Error::TooManyEntries { .. } => Error::TooLarge("archive entries"),
            e => Error::format(format!("not an HWPX (ZIP) file: {e}")),
        })?;
        let mut pkg = Package::default();
        for i in 0..z.len() {
            let (Some(name), Some(false)) = (z.entry_name(i).map_err(Error::format)?, z.entry_is_dir(i).map_err(Error::format)?) else { continue };
            let stored = z.is_stored(&name).unwrap_or(false);
            // Reads within the part and archive limits, counting what really arrives.
            let data = match z.read(&name, MAX_PART_BYTES) {
                Ok(Some(data)) => data,
                Ok(None) => continue,
                Err(craft_zip::Error::EntryTooLarge { .. }) => return Err(Error::TooLarge("part")),
                Err(craft_zip::Error::BudgetSpent { .. }) => return Err(Error::TooLarge("archive")),
                Err(e) => return Err(Error::format(e)),
            };
            pkg.add_entry(Entry { name, data, stored });
        }
        pkg.rootfile = pkg.find_rootfile();
        if let Some(rf) = pkg.rootfile.clone() {
            pkg.read_opf(&rf);
        }
        pkg.header_name = pkg.find_header();
        pkg.section_names = pkg.find_sections();
        if pkg.section_names.is_empty() {
            return Err(Error::format("no section parts found (Contents/section*.xml)"));
        }
        Ok(pkg)
    }

    /// Add an entry (a later one of the same name replaces it).
    pub(crate) fn push_entry(&mut self, name: String, data: Vec<u8>) {
        match self.index.get(&name).copied() {
            Some(i) => {
                if let Some(e) = self.entries.get_mut(i) {
                    e.data = data;
                }
            }
            None => self.add_entry(Entry { name, data, stored: false }),
        }
    }

    fn add_entry(&mut self, e: Entry) {
        let at = self.entries.len();
        if let Some(f) = e.name.strip_prefix("BinData/") {
            let stem = f.rsplit_once('.').map(|(s, _)| s).unwrap_or(f);
            self.bin_stem_at.entry(stem.to_string()).or_insert(at);
        }
        self.index.insert(e.name.clone(), at);
        self.entries.push(e);
    }

    pub fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn read(&self, name: &str) -> Option<&[u8]> {
        self.index.get(name).and_then(|&i| self.entries.get(i)).map(|e| e.data.as_slice())
    }

    /// A part as an element tree; section parts get their paragraphs numbered.
    pub fn xml(&self, name: &str) -> Result<Element> {
        if let Some(head) = &self.hwp5_header {
            if self.header_name.as_deref() == Some(name) {
                return Ok(head.clone());
            }
            let data = self.read(name).ok_or_else(|| Error::format(format!("missing stream {name}")))?;
            return Ok(crate::hwp5::section(data));
        }
        let data = self.read(name).ok_or_else(|| Error::format(format!("missing part {name}")))?;
        xml::parse(data, self.section_names.iter().any(|s| s == name))
    }

    /// Manifest hrefs are package-root relative in practice; accept hpf-relative too.
    fn resolve(&self, href: &str) -> Option<String> {
        let href = href.replace('\\', "/");
        let href = href.trim_start_matches('/');
        if self.has(href) {
            return Some(href.to_string());
        }
        let dir = self.rootfile.as_deref().and_then(|r| r.rsplit_once('/')).map(|(d, _)| d)?;
        let alt = normalize_path(&format!("{dir}/{href}"));
        self.has(&alt).then_some(alt)
    }

    fn find_rootfile(&self) -> Option<String> {
        if let Ok(root) = self.xml("META-INF/container.xml") {
            for rf in root.iter("ocf:rootfile") {
                let p = rf.get("full-path").unwrap_or("");
                if p.ends_with(".hpf") && self.has(p) {
                    return Some(p.to_string());
                }
            }
        }
        self.has("Contents/content.hpf").then(|| "Contents/content.hpf".to_string())
    }

    fn read_opf(&mut self, name: &str) {
        let Ok(root) = self.xml(name) else { return };
        let mut items = Vec::new();
        for it in root.iter("opf:item") {
            if let Some(href) = self.resolve(it.get("href").unwrap_or("")) {
                let id = it.get("id").map(str::to_string).unwrap_or_else(|| href.clone());
                items.push(Item { id, href, media_type: it.get("media-type").unwrap_or("").to_string() });
            }
        }
        self.set_items(items);
        self.spine = root.iter("opf:itemref").map(|r| r.get("idref").unwrap_or("").to_string()).collect();
        if let Some(md) = root.find("opf:metadata") {
            for el in &md.children {
                let local = el.tag.rsplit(':').next().unwrap_or(&el.tag).to_string();
                let text = el.text.trim();
                if local == "meta" {
                    if let Some(n) = el.get("name") {
                        let v = if text.is_empty() { el.get("content").unwrap_or("") } else { text };
                        self.metadata.push((n.to_string(), v.trim().to_string()));
                    }
                } else if !text.is_empty() {
                    self.metadata.push((local, text.to_string()));
                }
            }
        }
    }

    /// The root element of a part, from its first 4 KB without parsing the whole part: "head",
    /// "sec", "" for another root, or "?" when the first 4 KB don't tell (a long comment).
    fn root_tag(&self, name: &str) -> &'static str {
        let head = self.read(name).map(|d| String::from_utf8_lossy(d.get(..d.len().min(4096)).unwrap_or(d)).into_owned()).unwrap_or_default();
        let mut rest = head.as_str();
        while let Some(i) = rest.find('<') {
            rest = rest.get(i + 1..).unwrap_or("");
            // a comment (which may hold tags) ends at "-->", a declaration or instruction at '>'
            let skip_to = if rest.starts_with("!--") {
                Some("-->")
            } else if rest.starts_with('?') || rest.starts_with('!') {
                Some(">")
            } else {
                None
            };
            if let Some(end) = skip_to {
                match rest.find(end) {
                    Some(e) => rest = rest.get(e + end.len()..).unwrap_or(""),
                    None => return "?",
                }
                continue;
            }
            let name: String = rest.chars().take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').collect();
            let local = name.rsplit(':').next().unwrap_or("");
            return match local {
                "head" => "head",
                "sec" => "sec",
                _ => "",
            };
        }
        "?"
    }

    fn find_header(&self) -> Option<String> {
        if let Some(it) = self.items.iter().find(|it| it.id == "header" || it.href.ends_with("/header.xml")) {
            return Some(it.href.clone());
        }
        self.has("Contents/header.xml").then(|| "Contents/header.xml".to_string())
    }

    fn find_sections(&self) -> Vec<String> {
        let header = self.header_name.as_deref();
        let mut out = Vec::new();
        for idref in &self.spine {
            if let Some(it) = self.item(idref)
                && it.href.ends_with(".xml")
                && Some(it.href.as_str()) != header
                // a section, or a part whose start doesn't tell: the manifest lists it
                && matches!(self.root_tag(&it.href), "sec" | "?")
            {
                out.push(it.href.clone());
            }
        }
        if !out.is_empty() {
            return out;
        }
        let mut found: Vec<(u32, String)> = self
            .entries
            .iter()
            .filter_map(|e| {
                let n = e.name.strip_prefix("Contents/section")?.strip_suffix(".xml")?;
                n.parse::<u32>().ok().map(|k| (k, e.name.clone()))
            })
            .collect();
        found.sort();
        found.into_iter().map(|(_, n)| n).collect()
    }

    pub(crate) fn set_items(&mut self, items: Vec<Item>) {
        self.item_at.clear();
        for (k, it) in items.iter().enumerate() {
            self.item_at.entry(it.id.clone()).or_insert(k);
        }
        self.items = items;
    }

    /// The first manifest item with this id.
    pub(crate) fn item(&self, id: &str) -> Option<&Item> {
        self.items.get(*self.item_at.get(id)?)
    }

    /// (path, bytes) of a binary item referenced by `binaryItemIDRef`.
    pub fn binary(&self, item_id: &str) -> Option<(&str, &[u8])> {
        let name = match self.item(item_id) {
            Some(it) => Some(it.href.as_str()),
            None if self.hwp5_header.is_some() => None,
            // Some writers reference BinData files by stem only.
            None => self.bin_stem_at.get(item_id).and_then(|&i| self.entries.get(i)).map(|e| e.name.as_str()),
        }?;
        let i = *self.index.get(name)?;
        let e = self.entries.get(i)?;
        Some((e.name.as_str(), e.data.as_slice()))
    }

    pub fn mimetype(&self) -> String {
        self.read("mimetype").map(|d| String::from_utf8_lossy(d).trim().to_string()).unwrap_or_default()
    }
}

/// Resolve "." and ".." in a '/'-separated path.
fn normalize_path(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
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

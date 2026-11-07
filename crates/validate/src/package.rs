//! Package model: bounded zip loading, content-type resolution, relationship parsing and the
//! reachability graph shared by the checks.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::Acc;
use crate::scan::ParseGuard;

/// The main document part's content type (ECMA-376 Part 1 §11.3.10).
pub(crate) const MAIN_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
/// The template main-document content type (ECMA-376 Part 1 §11.3.10), the other spelling of the
/// package's one main document content type.
pub(crate) const TEMPLATE_MAIN_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";
/// A header part's content type.
pub(crate) const HEADER_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
/// A footer part's content type.
pub(crate) const FOOTER_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
/// The comments part's content type.
pub(crate) const COMMENTS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
/// The commentsExtended part's content type.
pub(crate) const COMMENTS_EXTENDED_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
/// The commentsIds part's content type.
pub(crate) const COMMENTS_IDS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsIds+xml";
/// The footnotes part's content type.
pub(crate) const FOOTNOTES_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";
/// The endnotes part's content type.
pub(crate) const ENDNOTES_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml";
/// The numbering part's content type.
pub(crate) const NUMBERING_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
/// The styles part's content type.
pub(crate) const STYLES_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
/// The settings part's content type.
pub(crate) const SETTINGS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";
/// The font table part's content type.
pub(crate) const FONT_TABLE_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml";
/// The theme part's content type.
pub(crate) const THEME_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.theme+xml";
/// The glossary document part's content type.
pub(crate) const GLOSSARY_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml";
/// The web-settings part's content type (ECMA-376 Part 1 §11.3.13).
pub(crate) const WEB_SETTINGS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml";
/// The people part's content type (MS-DOCX; absent from ECMA-376 Part 1).
pub(crate) const PEOPLE_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml";
/// The chart part's content type (ECMA-376 Part 1 §14.2.1).
pub(crate) const CHART_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
/// The plain-XML content type a custom XML item and a bibliography part require
/// (ECMA-376 Part 1 §§15.2.5, 15.2.3).
pub(crate) const XML_CONTENT_TYPE: &str = "application/xml";
/// A custom-XML item-properties part's content type.
pub(crate) const CUSTOM_XML_PROPS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.customXmlProperties+xml";

/// One zip entry, held once for every check.
pub(crate) struct Entry {
    pub name: String,
    /// `None` when the part is a directory, too large, or could not be decompressed.
    pub bytes: Option<Vec<u8>>,
    pub is_dir: bool,
    /// The strictly-decoded text of an XML or `.rels` part; every consumer reads this, never the
    /// raw bytes.
    pub decoded: Option<String>,
    /// Why decoding failed, when it did.
    pub decode_error: Option<crate::scan::DecodeError>,
}

/// `[Content_Types].xml`: Override by part name, else Default by extension (both case-insensitive).
/// Duplicate declarations are recorded too, for `CT-DUP-DEFAULT`/`CT-DUP-OVERRIDE`.
#[derive(Default)]
pub(crate) struct ContentTypes {
    overrides: HashMap<String, String>,
    defaults: HashMap<String, String>,
    /// Extensions with more than one Default entry (lowercased).
    pub dup_defaults: BTreeSet<String>,
    /// Part names with more than one Override entry (lowercased, leading `/` stripped).
    pub dup_overrides: BTreeSet<String>,
}

impl ContentTypes {
    /// The effective content type of `part` (a package path, leading `/` ignored).
    pub fn of(&self, part: &str) -> Option<&str> {
        let part = part.trim_start_matches('/').to_ascii_lowercase();
        if let Some(t) = self.overrides.get(&part) {
            return Some(t);
        }
        let ext = part.rsplit_once('.').map(|(_, e)| e)?;
        self.defaults.get(ext).map(String::as_str)
    }
}

/// One relationship.
pub(crate) struct Rel {
    pub id: String,
    pub kind: String,
    /// Resolved package path for internal targets; `None` for external ones.
    pub target_abs: Option<String>,
    pub external: bool,
}

/// One `.rels` part.
pub(crate) struct RelsPart {
    pub name: String,
    pub rels: Vec<Rel>,
}

/// Everything loaded from a package once.
pub(crate) struct Package {
    pub entries: Vec<Entry>,
    pub content_types: ContentTypes,
    pub content_types_present: bool,
    pub rels: Vec<RelsPart>,
    pub main: Option<String>,
    /// The glossary document part, reached from the main document's relationships.
    pub glossary: Option<String>,
    /// Lowercased names of parts reachable from the package root.
    pub reachable: HashSet<String>,
    /// Lowercased names of parts classified as XML by the single classification shared by loading,
    /// decoding and the XML pass (J4).
    pub xml_parts: HashSet<String>,
}

/// Load a zip into the package model. Returns `None` only when the input is not a readable zip
/// (after a `PKG-NOT-ZIP` finding): every other problem becomes a finding or a coverage note.
pub(crate) fn load(bytes: &[u8], acc: &mut Acc) -> Option<Package> {
    acc.ran("PKG-NOT-ZIP");
    let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(bytes)) else {
        acc.add("PKG-NOT-ZIP", None, "the input is not a readable zip archive".to_string());
        return None;
    };
    let limits = acc.limits;
    if zip.len() > limits.entries {
        acc.note(format!("the archive declares {} entries; only the first {} were examined", zip.len(), limits.entries));
        acc.set_incomplete();
    }
    let limit = zip.len().min(limits.entries);
    let mut entries = Vec::new();
    let mut failed = HashSet::new();
    let mut total: u64 = 0;
    for i in 0..limit {
        let Ok(mut f) = zip.by_index(i) else { continue };
        let name = f.name().trim_start_matches('/').to_string();
        if f.is_dir() {
            entries.push(blank_entry(name, true));
            continue;
        }
        let size = f.size();
        if size > limits.part_bytes {
            acc.note(format!("part {name} is larger than the {}-byte per-part limit and was not checked", limits.part_bytes));
            acc.set_incomplete();
            entries.push(blank_entry(name, false));
            continue;
        }
        if total.saturating_add(size) > limits.total_bytes {
            acc.note(format!("part {name} would take the package past the {}-byte total limit and was not checked", limits.total_bytes));
            acc.set_incomplete();
            entries.push(blank_entry(name, false));
            continue;
        }
        let mut buf = Vec::with_capacity(size.min(1 << 20) as usize);
        match (&mut f).take(limits.part_bytes + 1).read_to_end(&mut buf) {
            Ok(_) if buf.len() as u64 > limits.part_bytes => {
                acc.note(format!("part {name} decompresses past the {}-byte per-part limit and was not checked", limits.part_bytes));
                acc.set_incomplete();
                entries.push(blank_entry(name, false));
            }
            Ok(_) => {
                total = total.saturating_add(buf.len() as u64);
                entries.push(text_entry(name, buf));
            }
            Err(_) => {
                acc.note(format!("part {name} could not be decompressed and was not checked"));
                failed.insert(name.to_ascii_lowercase());
                entries.push(blank_entry(name, false));
            }
        }
    }

    check_duplicate_entries(bytes, acc);
    check_case_collisions(&entries, acc);

    let ct_at = entries.iter().position(|e| !e.is_dir && e.name.eq_ignore_ascii_case("[Content_Types].xml"));
    acc.ran("PKG-NO-CONTENT-TYPES");
    let content_types_present = ct_at.is_some();
    if !content_types_present {
        acc.add("PKG-NO-CONTENT-TYPES", None, "the package has no [Content_Types].xml part, so part content types cannot be resolved".to_string());
    }
    let mut content_types = ContentTypes::default();
    let mut ct_guard = ParseGuard::new(limits.events, limits.depth);
    if let Some(e) = ct_at.and_then(|i| entries.get(i))
        && let Some(text) = &e.decoded
        && parse_content_types(text, &mut content_types, limits.entries, &mut ct_guard)
    {
        acc.note(format!("[Content_Types].xml declares more than {} entries; later ones were not recorded", limits.entries));
        acc.set_incomplete();
    }
    if ct_guard.exhausted() {
        acc.note(format!("[Content_Types].xml exceeded the XML {} budget; later declarations were not recorded", ct_guard.budget()));
        acc.set_incomplete();
    }

    let rels = load_rels(&entries, acc);

    // Classify XML parts once (J4) and decode every classified part, whichever its file name, so a
    // supporting or story part with an unconventional name still has its checks run (H5). The
    // classification is content type (+xml / text/xml / the rels type) OR a recognised relationship
    // role.
    let xml_parts = classified_xml_parts(&entries, &content_types, &rels);
    for e in entries.iter_mut().filter(|e| !e.is_dir) {
        if e.decoded.is_some() || e.decode_error.is_some() || e.bytes.is_none() {
            continue;
        }
        if xml_parts.contains(&e.name.to_ascii_lowercase()) {
            decode_entry(e);
        }
    }

    acc.ran("PKG-NO-MAIN");
    let package_rels = rels.iter().find(|r| r.name.eq_ignore_ascii_case("_rels/.rels"));
    let main = package_rels
        .and_then(|rp| rp.rels.iter().find(|r| !r.external && tail(&r.kind) == "officeDocument"))
        .and_then(|r| r.target_abs.as_deref())
        .and_then(|t| canonical(&entries, t));
    if main.is_none() {
        acc.add("PKG-NO-MAIN", None, no_main_message(package_rels));
    }

    if let Some(m) = &main
        && failed.contains(&m.to_ascii_lowercase())
    {
        acc.set_incomplete();
    }

    let glossary = glossary_part(&entries, &content_types, &rels, main.as_deref());

    let reachable = reachability(&rels, main.as_deref());
    Some(Package { entries, content_types, content_types_present, rels, main, glossary, reachable, xml_parts })
}

/// The glossary document part: the main document's `glossaryDocument` relationship target, else the
/// first part whose effective content type is the glossary type.
fn glossary_part(entries: &[Entry], ct: &ContentTypes, rels: &[RelsPart], main: Option<&str>) -> Option<String> {
    if let Some(main) = main {
        let want = rels_part_name(main);
        if let Some(rp) = rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want))
            && let Some(r) = rp.rels.iter().find(|r| !r.external && tail(&r.kind) == "glossaryDocument")
            && let Some(t) = &r.target_abs
            && let Some(name) = canonical(entries, t)
        {
            return Some(name);
        }
    }
    entries.iter().find(|e| !e.is_dir && ct.of(&e.name) == Some(GLOSSARY_CONTENT_TYPE)).map(|e| e.name.clone())
}

/// A directory entry, or one whose bytes were skipped.
fn blank_entry(name: String, is_dir: bool) -> Entry {
    Entry { name, bytes: None, is_dir, decoded: None, decode_error: None }
}

/// A file entry whose bytes are held: text parts are decoded once, strictly.
fn text_entry(name: String, bytes: Vec<u8>) -> Entry {
    let (decoded, decode_error) = if is_text_part(&name) {
        match crate::scan::decode_strict(&bytes) {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(e)),
        }
    } else {
        (None, None)
    };
    Entry { name, bytes: Some(bytes), is_dir: false, decoded, decode_error }
}

/// Decode a part identified as XML by its effective content type, whichever its file name (H5).
fn decode_entry(e: &mut Entry) {
    let Some(bytes) = e.bytes.as_deref() else { return };
    let (decoded, decode_error) = match crate::scan::decode_strict(bytes) {
        Ok(t) => (Some(t), None),
        Err(err) => (None, Some(err)),
    };
    e.decoded = decoded;
    e.decode_error = decode_error;
}

/// Whether a content type denotes XML text: a `+xml` suffix, `text/xml` or `application/xml`. The
/// relationships type (`application/vnd.openxmlformats-package.relationships+xml`) matches by
/// suffix.
pub(crate) fn is_xml_content_type(ct: &str) -> bool {
    ct.ends_with("+xml") || ct == "text/xml" || ct == "application/xml"
}

/// Whether a part is XML text (a part the XML pass decodes and parses).
pub(crate) fn is_text_part(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".xml") || l.ends_with(".rels")
}

/// Relationship kinds whose target is an XML part, for the single XML-part classification (J4).
fn is_xml_relationship_kind(kind: &str) -> bool {
    matches!(
        tail(kind),
        "officeDocument"
            | "styles"
            | "numbering"
            | "settings"
            | "comments"
            | "commentsExtended"
            | "commentsIds"
            | "footnotes"
            | "endnotes"
            | "header"
            | "footer"
            | "glossaryDocument"
            | "theme"
            | "fontTable"
            | "customXml"
            | "customXmlProps"
    )
}

/// The single classification of XML parts (J4), used by loading, decoding and `check_xml`: a part
/// whose effective content type is XML (`+xml` / `text/xml` / `application/xml`), a `.rels`
/// relationship part, or a part reached through a recognised XML relationship role. Returns the
/// lowercased names.
fn classified_xml_parts(entries: &[Entry], ct: &ContentTypes, rels: &[RelsPart]) -> HashSet<String> {
    let mut role_targets: HashSet<String> = HashSet::new();
    for rp in rels {
        for r in &rp.rels {
            if is_xml_relationship_kind(&r.kind)
                && let Some(t) = &r.target_abs
            {
                role_targets.insert(t.trim_start_matches('/').to_ascii_lowercase());
            }
        }
    }
    // The manifest is always XML-classified, independently of its own declarations (K1): an
    // Override-only manifest has no effective content type and no relationship role, so without
    // this it would never reach the XML scanner and an illegal reference or malformed syntax
    // inside it would go unreported.
    let mut set: HashSet<String> = HashSet::from(["[content_types].xml".to_string()]);
    for e in entries.iter().filter(|e| !e.is_dir) {
        let l = e.name.to_ascii_lowercase();
        if ct.of(&e.name).is_some_and(is_xml_content_type) || l.ends_with(".rels") || role_targets.contains(&l) {
            set.insert(l);
        }
    }
    set
}

/// Whether `name` is an XML part, by the classification stored on the package (J4).
pub(crate) fn is_xml_part(pkg: &Package, name: &str) -> bool {
    pkg.xml_parts.contains(&name.to_ascii_lowercase())
}

fn no_main_message(package_rels: Option<&RelsPart>) -> String {
    let Some(rp) = package_rels else {
        return "the package has no _rels/.rels part, so it has no main document part".to_string();
    };
    match rp.rels.iter().find(|r| !r.external && tail(&r.kind) == "officeDocument") {
        None => "the package's _rels/.rels has no officeDocument relationship".to_string(),
        Some(r) => format!("the officeDocument relationship targets {}, which is not in the package", r.target_abs.as_deref().unwrap_or("")),
    }
}

fn check_duplicate_entries(bytes: &[u8], acc: &mut Acc) {
    acc.ran("PKG-DUP-ENTRY");
    // The zip reader collapses duplicate names into one entry, so the central directory itself is
    // read to see every stored name (a package may legally list the same name twice).
    let Some(names) = central_directory_names(bytes) else {
        acc.note("the zip central directory could not be parsed; duplicate entry names were not checked".to_string());
        return;
    };
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for n in &names {
        *counts.entry(n.trim_start_matches('/')).or_default() += 1;
    }
    let mut dups: Vec<&str> = counts.into_iter().filter(|(_, c)| *c > 1).map(|(n, _)| n).collect();
    dups.sort_unstable();
    for n in dups {
        acc.add("PKG-DUP-ENTRY", None, format!("the entry {n} appears more than once in the archive"));
    }
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// The names in the zip central directory, in stored order, including duplicates. `None` when the
/// directory cannot be parsed (not a zip, or a layout this reader does not understand), in which
/// case duplicate names are simply not checked.
fn central_directory_names(bytes: &[u8]) -> Option<Vec<String>> {
    let eocd = find_eocd(bytes)?;
    let mut count = u64::from(u16_at(bytes, eocd + 10)?);
    let mut cd_off = u64::from(u32_at(bytes, eocd + 16)?);
    let cd_size = u64::from(u32_at(bytes, eocd + 12)?);
    if count == 0xFFFF || cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF {
        let (c, o) = zip64_directory(bytes, eocd)?;
        count = c;
        cd_off = o;
    }
    if count > 1_000_000 {
        return None;
    }
    let mut names = Vec::with_capacity(count.min(1 << 16) as usize);
    let mut i = usize::try_from(cd_off).ok()?;
    for _ in 0..count {
        if u32_at(bytes, i)? != 0x0201_4B50 {
            return None;
        }
        let name_len = usize::from(u16_at(bytes, i + 28)?);
        let extra_len = usize::from(u16_at(bytes, i + 30)?);
        let comment_len = usize::from(u16_at(bytes, i + 32)?);
        let name = bytes.get(i + 46..i + 46 + name_len)?;
        names.push(String::from_utf8_lossy(name).into_owned());
        i = i.checked_add(46 + name_len + extra_len + comment_len)?;
    }
    Some(names)
}

/// The zip64 end-of-central-directory `(entry count, central directory offset)`.
fn zip64_directory(bytes: &[u8], eocd: usize) -> Option<(u64, u64)> {
    let locator = eocd.checked_sub(20)?;
    if u32_at(bytes, locator)? != 0x0706_4B50 {
        return None;
    }
    let off = usize::try_from(u64_at(bytes, locator + 8)?).ok()?;
    if u32_at(bytes, off)? != 0x0606_4B50 {
        return None;
    }
    Some((u64_at(bytes, off + 32)?, u64_at(bytes, off + 48)?))
}

/// The offset of the end-of-central-directory record, matched against the trailing comment length.
fn find_eocd(bytes: &[u8]) -> Option<usize> {
    const SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
    let start = bytes.len().saturating_sub(22 + 0xFFFF);
    let mut i = bytes.len().checked_sub(22)?;
    loop {
        if bytes.get(i..i + 4) == Some(&SIG) {
            let comment_len = usize::from(u16_at(bytes, i + 20)?);
            if i + 22 + comment_len == bytes.len() {
                return Some(i);
            }
        }
        if i == start {
            return None;
        }
        i -= 1;
    }
}

fn check_case_collisions(entries: &[Entry], acc: &mut Acc) {
    acc.ran("PKG-CASE-COLLISION");
    let mut groups: HashMap<String, Vec<&str>> = HashMap::new();
    for e in entries.iter().filter(|e| !e.is_dir) {
        groups.entry(e.name.to_ascii_lowercase()).or_default().push(e.name.as_str());
    }
    let mut collisions: Vec<Vec<&str>> = groups
        .into_values()
        .filter_map(|mut v| {
            v.sort_unstable();
            v.dedup();
            (v.len() > 1).then_some(v)
        })
        .collect();
    collisions.sort();
    for v in collisions {
        acc.add("PKG-CASE-COLLISION", None, format!("part names differ only by ASCII case: {}", v.join(", ")));
    }
}

fn load_rels(entries: &[Entry], acc: &mut Acc) -> Vec<RelsPart> {
    let cap = acc.limits.entries;
    let limits = acc.limits;
    let mut rels = Vec::new();
    for e in entries.iter().filter(|e| !e.is_dir) {
        if !e.name.to_ascii_lowercase().ends_with(".rels") {
            continue;
        }
        let Some(dir) = rels_dir(&e.name) else { continue };
        let Some(text) = &e.decoded else { continue };
        let mut guard = ParseGuard::new(limits.events, limits.depth);
        let (parsed, c) = parse_rels(&dir, text, cap, &mut guard);
        if c {
            acc.note(format!("{} declares more than {cap} relationships; later ones were not recorded", e.name));
            acc.set_incomplete();
        }
        if guard.exhausted() {
            acc.note(format!("{} exceeded the XML {} budget; later relationships were not recorded", e.name, guard.budget()));
            acc.set_incomplete();
        }
        rels.push(RelsPart { name: e.name.clone(), rels: parsed });
    }
    rels
}

/// The source directory of a `.rels` part, or `None` when it is not a relationship part.
pub(crate) fn rels_dir(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if lower == "_rels/.rels" {
        return Some(String::new());
    }
    if let Some(i) = lower.rfind("/_rels/") {
        return Some(name.get(..i).unwrap_or("").to_string());
    }
    lower.starts_with("_rels/").then(String::new)
}

/// The `.rels` part name that holds `part`'s relationships.
pub(crate) fn rels_part_name(part: &str) -> String {
    let p = part.trim_start_matches('/');
    match p.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{p}.rels"),
    }
}

fn canonical(entries: &[Entry], name: &str) -> Option<String> {
    let n = name.trim_start_matches('/');
    entries.iter().find(|e| !e.is_dir && e.name.eq_ignore_ascii_case(n)).map(|e| e.name.clone())
}

/// The stored spelling of the part at `name` (case-insensitive, leading `/` ignored), if present.
pub(crate) fn canonical_entry(pkg: &Package, name: &str) -> Option<String> {
    canonical(&pkg.entries, name)
}

/// The entry at `name` (case-insensitive, leading `/` ignored), if present.
pub(crate) fn entry_by_name<'a>(pkg: &'a Package, name: &str) -> Option<&'a Entry> {
    let n = name.trim_start_matches('/');
    pkg.entries.iter().find(|e| !e.is_dir && e.name.eq_ignore_ascii_case(n))
}

fn reachability(rels: &[RelsPart], main: Option<&str>) -> HashSet<String> {
    let by_name: HashMap<String, usize> = rels.iter().enumerate().map(|(i, r)| (r.name.to_ascii_lowercase(), i)).collect();
    let mut seen = HashSet::new();
    seen.insert("[content_types].xml".to_string());
    seen.insert("_rels/.rels".to_string());
    let mut queue: Vec<String> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    for rp in rels.iter().filter(|r| r.name.eq_ignore_ascii_case("_rels/.rels")) {
        for r in &rp.rels {
            if let Some(t) = &r.target_abs {
                seen.insert(t.to_ascii_lowercase());
                queue.push(t.clone());
            }
        }
    }
    if let Some(m) = main {
        queue.push(m.to_string());
    }
    while let Some(part) = queue.pop() {
        if !visited.insert(part.to_ascii_lowercase()) {
            continue;
        }
        if let Some(i) = by_name.get(&rels_part_name(&part).to_ascii_lowercase()) {
            for r in &rels[*i].rels {
                if let Some(t) = &r.target_abs {
                    seen.insert(t.to_ascii_lowercase());
                    queue.push(t.clone());
                }
            }
        }
    }
    seen
}

/// Resolve an internal relationship target against a source directory into a package path,
/// following the OPC resolution steps: strip any query/fragment (OPC §§6.3.2, 6.4),
/// percent-decode each segment, and root-clamp `..` segments (a leading `/` is
/// package-absolute). Part-name equivalence itself is ASCII case-insensitive (§6.2.2.3) and is
/// applied by the caller when it looks the result up.
pub(crate) fn resolve(dir: &str, target: &str) -> String {
    let target = target.split(['?', '#']).next().unwrap_or(target);
    let mut parts: Vec<String> =
        if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|s| !s.is_empty()).map(percent_decode).collect() };
    for seg in target.split('/') {
        let seg = percent_decode(seg);
        match seg.as_str() {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(seg),
        }
    }
    parts.join("/")
}

/// Percent-decode a URI path segment (`%20` → space); invalid escapes are left as written.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let (Some(hi), Some(lo)) = (b.get(i + 1).and_then(|c| hex(*c)), b.get(i + 2).and_then(|c| hex(*c)))
        {
            out.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// The last `/`-separated segment of a relationship type.
pub(crate) fn tail(t: &str) -> &str {
    t.rsplit('/').next().unwrap_or(t)
}

/// Parse `[Content_Types].xml`. Returns `true` when the declaration budget stopped it early.
fn parse_content_types(text: &str, ct: &mut ContentTypes, cap: usize, guard: &mut ParseGuard) -> bool {
    let mut reader = Reader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut count = 0usize;
    let mut capped = false;
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if !guard.enter() {
                    break;
                }
                if count >= cap {
                    capped = true;
                } else {
                    record_content_type(&e, ct);
                    count += 1;
                }
            }
            Ok(Event::Empty(e)) => {
                if count >= cap {
                    capped = true;
                } else {
                    record_content_type(&e, ct);
                    count += 1;
                }
            }
            Ok(Event::End(_)) => guard.leave(),
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    capped
}

fn record_content_type(e: &BytesStart, ct: &mut ContentTypes) {
    let local = e.local_name();
    let l = local.as_ref();
    if l != b"Override" && l != b"Default" {
        return;
    }
    let key = attr_local(e, b"PartName").or_else(|| attr_local(e, b"Extension"));
    let Some(key) = key else { return };
    let Some(value) = attr_local(e, b"ContentType") else { return };
    let key = key.trim_start_matches('/').to_ascii_lowercase();
    let (map, dups) = if l == b"Override" { (&mut ct.overrides, &mut ct.dup_overrides) } else { (&mut ct.defaults, &mut ct.dup_defaults) };
    if map.insert(key.clone(), value).is_some() {
        dups.insert(key);
    }
}

/// Parse a `.rels` part. Returns `(relationships, capped)` where `capped` is set when the budget
/// stopped further relationships from being recorded. The depth bound is tracked like every other
/// parser (J1), so a deeply nested relationships part stops before a later (possibly dangling)
/// relationship is read.
fn parse_rels(dir: &str, text: &str, cap: usize, guard: &mut ParseGuard) -> (Vec<Rel>, bool) {
    let mut rels = Vec::new();
    let mut reader = Reader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut capped = false;
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if !guard.enter() {
                    break;
                }
                push_rel(dir, &e, &mut rels, cap, &mut capped);
            }
            Ok(Event::Empty(e)) => push_rel(dir, &e, &mut rels, cap, &mut capped),
            Ok(Event::End(_)) => guard.leave(),
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (rels, capped)
}

/// Record one `<Relationship>` element, ignoring any other element.
fn push_rel(dir: &str, e: &BytesStart, rels: &mut Vec<Rel>, cap: usize, capped: &mut bool) {
    if e.local_name().as_ref() == b"Relationship"
        && let Some(rel) = parse_rel(dir, e)
    {
        if rels.len() >= cap {
            *capped = true;
        } else {
            rels.push(rel);
        }
    }
}

fn parse_rel(dir: &str, e: &BytesStart) -> Option<Rel> {
    let id = attr_local(e, b"Id")?;
    let kind = attr_local(e, b"Type").unwrap_or_default();
    let target = attr_local(e, b"Target").unwrap_or_default();
    let external = attr_local(e, b"TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
    let target_abs = (!external).then(|| resolve(dir, &target));
    Some(Rel { id, kind, target_abs, external })
}

/// An attribute by its local name regardless of prefix (content types and rels use unprefixed
/// or conventionally-prefixed attributes).
fn attr_local(e: &BytesStart, name: &[u8]) -> Option<String> {
    for a in e.attributes() {
        let a = a.ok()?;
        if a.key.local_name().as_ref() == name {
            return a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|c| c.into_owned());
        }
    }
    None
}

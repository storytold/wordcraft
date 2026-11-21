//! Audit a `.docx` file for hidden information before sharing.
//!
//! [`audit_bytes`] scans the raw OPC package (the ZIP bytes) and reports what a
//! recipient of the file could learn about its authors: names on tracked changes
//! and comments, creator/last-modified-by in document properties, hidden text,
//! macro/embedded-object parts, and package parts nothing references.
//!
//! The audit is visibility only: it never modifies the file. Sanitizing (removing
//! what it finds) is a separate, future step.
//!
//! Detection notes (documented, not hidden):
//!
//! - Matching is by namespace URI, not by prefix, so a file that spells the
//!   WordprocessingML namespace with an unusual prefix is still scanned.
//! - `w:vanish` hides text only when its `w:val` is absent or true-ish
//!   (ECMA-376 §17.18.67 `ST_OnOff`); `w:val="false"` is not flagged.
//! - Revisions and hidden text are scanned in every story part:
//!   `word/document.xml`, `word/header*.xml`, `word/footer*.xml`,
//!   `word/footnotes.xml` and `word/endnotes.xml`. Comment bodies in
//!   `word/comments.xml` are story content too and are scanned the same
//!   way, on top of the `w:comment` author attributes.
//! - `w:vanish` in `word/styles.xml` — in a style's run properties or in
//!   the document defaults — is reported as a hidden-text style: it hides
//!   every run the style (or the defaults) applies to. Parts the audit
//!   does not parse at all (e.g. `word/numbering.xml`, glossary parts)
//!   are listed under `not_scanned`.
//! - The report lists the parts it scanned and the parts it did not, so
//!   "no findings" never silently means "not looked at".
//! - A `dc:creator` / `cp:lastModifiedBy` value is reported as-is; producers
//!   that anonymize (WordCraft writes `Author`) are reported too, since the
//!   value is what a recipient sees.
//!
//! The audit fails closed: a part that cannot be read or parsed is an error,
//! never a silent pass.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;
use serde::Serialize;

#[cfg(test)]
mod tests;

// ECMA-376 Part 1 namespaces the audit understands.
const NS_W: &[u8] = b"http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_DC: &[u8] = b"http://purl.org/dc/elements/1.1/";
const NS_CP: &[u8] = b"http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
const NS_EP: &[u8] = b"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
const NS_CUST: &[u8] = b"http://schemas.openxmlformats.org/officeDocument/2006/custom-properties";
const NS_W15: &[u8] = b"http://schemas.microsoft.com/office/word/2012/wordml";
const NS_RELS: &[u8] = b"http://schemas.openxmlformats.org/package/2006/relationships";

/// Cap on ZIP entries read: a package is a handful of parts, not millions.
const MAX_PARTS: usize = 100_000;
/// Cap on decompressed bytes read from a single part (zip-bomb guard).
const MAX_PART_BYTES: u64 = 256 * 1024 * 1024;
/// Cap on total decompressed bytes across all scanned parts. Per-part caps
/// alone don't bound N parts that each compress well (zip-bomb guard), and
/// every scanned part is buffered, so the total also bounds audit memory.
/// 256 MiB matches the per-part cap and is far above real packages.
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
/// Cap on XML elements walked in a single part.
const MAX_XML_ELEMENTS: u64 = 2_000_000;

/// What the audit found, as a stable machine-readable vocabulary.
///
/// The [`FindingKind::id`] strings are the crate's public contract: they appear
/// in CLI output and serialized reports. Do not rename them; add a new variant
/// instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    /// `w:author`/`w:initials` on a tracked change (the `CT_TrackChange` and
    /// `CT_MoveBookmark` elements, ECMA-376 Part 1 §17.13.5).
    RevisionAuthor,
    /// `w:author`/`w:initials`/`w:date` on a `w:comment` (ECMA-376 §17.13.4.2).
    CommentAuthor,
    /// `w15:author` on a `w15:person` in `word/people.xml`: the roster of
    /// comment author names Word keeps for threaded comments.
    PeopleAuthor,
    /// `dc:creator` / `cp:lastModifiedBy` in `docProps/core.xml`
    /// (ECMA-376 Part 2 §12.3.1).
    CorePropertiesAuthor,
    /// `Company` / `Manager` in `docProps/app.xml` (ECMA-376 Part 2 §12.3.2).
    ExtendedProperties,
    /// A `w:vanish` run property (ECMA-376 §17.3.2.42): text that renders hidden.
    HiddenText,
    /// A `w:vanish` in `word/styles.xml` — a style's run properties or the
    /// document defaults: every run the style applies to renders hidden.
    StyleHiddenText,
    /// A VBA macro project part (by relationship type, or `word/vbaProject.bin`):
    /// the document carries macros.
    VbaProject,
    /// An embedded object part (by relationship type, or under
    /// `word/embeddings/`): an embedded (often OLE) object.
    OleEmbedding,
    /// A ZIP entry no relationship targets: leftover content nothing references.
    OrphanPart,
}

impl FindingKind {
    /// The stable machine-readable ID for this finding class.
    pub fn id(self) -> &'static str {
        match self {
            FindingKind::RevisionAuthor => "author-pii/revision",
            FindingKind::CommentAuthor => "author-pii/comment",
            FindingKind::PeopleAuthor => "author-pii/people",
            FindingKind::CorePropertiesAuthor => "author-pii/core-properties",
            FindingKind::ExtendedProperties => "author-pii/extended-properties",
            FindingKind::HiddenText => "hidden-text",
            FindingKind::StyleHiddenText => "hidden-text/style",
            FindingKind::VbaProject => "active-content/vba",
            FindingKind::OleEmbedding => "active-content/ole",
            FindingKind::OrphanPart => "package/orphan-part",
        }
    }

    /// The human-readable title for this finding class.
    pub fn title(self) -> &'static str {
        match self {
            FindingKind::RevisionAuthor => "author name on tracked change",
            FindingKind::CommentAuthor => "author name on comment",
            FindingKind::PeopleAuthor => "author name in people part",
            FindingKind::CorePropertiesAuthor => "author name in document properties",
            FindingKind::ExtendedProperties => "organization name in extended properties",
            FindingKind::HiddenText => "hidden text",
            FindingKind::StyleHiddenText => "hidden text style",
            FindingKind::VbaProject => "VBA macro project",
            FindingKind::OleEmbedding => "embedded object",
            FindingKind::OrphanPart => "orphan package part",
        }
    }

    /// How much attention this class deserves: `warn` for identifying or
    /// executable content, `info` for leftovers worth knowing about.
    pub fn severity(self) -> &'static str {
        match self {
            FindingKind::OrphanPart | FindingKind::ExtendedProperties => "info",
            _ => "warn",
        }
    }
}

/// One thing the audit found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuditFinding {
    /// Stable finding-class ID (see [`FindingKind::id`]).
    pub id: String,
    /// Human-readable title (see [`FindingKind::title`]).
    pub title: String,
    /// `"warn"` or `"info"`.
    pub severity: String,
    /// Where it was found: the part name plus the element or attribute,
    /// e.g. `word/document.xml <w:ins>`.
    pub location: String,
    /// The matched value: an author name, a property value, a part name.
    pub detail: String,
}

impl AuditFinding {
    fn new(kind: FindingKind, location: String, detail: String) -> AuditFinding {
        AuditFinding {
            id: kind.id().to_string(),
            title: kind.title().to_string(),
            severity: kind.severity().to_string(),
            location: sanitize_report_text(&location),
            detail: sanitize_report_text(&detail),
        }
    }
}

/// Report strings carry values copied out of an untrusted file (author
/// names, property values, part names). Control characters in them could
/// spoof terminal output or forge log lines for whoever prints the report,
/// so they are replaced with U+FFFD at this single construction point.
/// (serde_json escapes them in JSON output; plain-text printers get the
/// same safety.)
fn sanitize_report_text(text: &str) -> String {
    if text.chars().any(char::is_control) { text.chars().map(|c| if c.is_control() { '\u{fffd}' } else { c }).collect() } else { text.to_string() }
}

/// Why the audit could not run.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// The bytes are not a readable ZIP archive.
    #[error("not a readable ZIP archive: {0}")]
    Zip(String),
    /// A package part needed for the audit could not be parsed as XML.
    #[error("part {0}: {1}")]
    Xml(String, String),
    /// A hostile-input guard tripped (too many parts, part too large, …).
    #[error("{0}")]
    Limit(String),
}

/// The full audit result: the findings, plus which package parts were
/// scanned to produce them and which were present but not scanned. The
/// coverage lists are part of the result because "no findings" is only
/// meaningful next to what the scan actually covered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuditReport {
    /// Every finding, in deterministic order (by class, location, detail).
    pub findings: Vec<AuditFinding>,
    /// Package parts the audit read and parsed, sorted.
    pub scanned: Vec<String>,
    /// Package parts present in the file but not parsed, sorted. Listed so
    /// a clean result never silently hides unexamined content.
    pub not_scanned: Vec<String>,
}

/// Scan the raw `.docx` (OPC ZIP) bytes and return the findings.
///
/// Findings come back in a deterministic order (by class, location, detail).
/// Fails closed: unreadable or unparsable input is an [`AuditError`], never an
/// empty finding list that looks like a clean bill of health.
pub fn audit_bytes(bytes: &[u8]) -> Result<Vec<AuditFinding>, AuditError> {
    Ok(audit_report(bytes)?.findings)
}

/// Scan the raw `.docx` (OPC ZIP) bytes and return the full report,
/// including scan coverage (see [`AuditReport`]).
pub fn audit_report(bytes: &[u8]) -> Result<AuditReport, AuditError> {
    // One archive for the whole scan: re-opening it per part would re-parse
    // the central directory each time (quadratic on hostile input).
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| AuditError::Zip(e.to_string()))?;
    let names = part_names(&mut archive)?;
    let mut parts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut total: u64 = 0;
    for name in names.iter().filter(|n| is_scanned_part(n)) {
        let data = read_part(&mut archive, name, &mut total)?;
        parts.insert(name.clone(), data);
    }

    if !parts.contains_key("word/document.xml") {
        return Err(AuditError::Xml("word/document.xml".to_string(), "missing: not a Word document package".to_string()));
    }
    let mut findings = Vec::new();
    // BTreeMap iteration is sorted, so story parts scan in a stable order.
    for (name, xml) in &parts {
        if is_story_part(name) {
            scan_story_part(xml, name, &mut findings)?;
        }
    }
    if let Some(xml) = parts.get("word/comments.xml") {
        scan_comments(xml, &mut findings)?;
        // Comment bodies are story content: tracked changes and hidden
        // text inside a comment are findings like anywhere else, and
        // `scan_comments` alone (attributes only) would miss them.
        scan_story_part(xml, "word/comments.xml", &mut findings)?;
    }
    if let Some(xml) = parts.get("word/people.xml") {
        scan_people(xml, &mut findings)?;
    }
    if let Some(xml) = parts.get("word/styles.xml") {
        scan_styles(xml, &mut findings)?;
    }
    if let Some(xml) = parts.get("docProps/core.xml") {
        scan_core_properties(xml, &mut findings)?;
    }
    if let Some(xml) = parts.get("docProps/app.xml") {
        scan_extended_properties(xml, &mut findings)?;
    }
    if let Some(xml) = parts.get("docProps/custom.xml") {
        scan_custom_properties(xml, &mut findings)?;
    }
    scan_active_content(&names, &parts, &mut findings)?;
    scan_orphans(&names, &parts, &mut findings)?;

    findings.sort_by(|a, b| (&a.id, &a.location, &a.detail).cmp(&(&b.id, &b.location, &b.detail)));
    let scanned: Vec<String> = parts.keys().cloned().collect();
    let not_scanned: Vec<String> = names.iter().filter(|n| !parts.contains_key(*n)).cloned().collect();
    Ok(AuditReport { findings, scanned, not_scanned })
}

/// Parts the audit parses as XML. Everything else is only listed by name
/// (and reported under [`AuditReport::not_scanned`]).
fn is_scanned_part(name: &str) -> bool {
    is_story_part(name)
        || name == "word/comments.xml"
        || name == "word/people.xml"
        || name == "word/styles.xml"
        || name == "docProps/core.xml"
        || name == "docProps/app.xml"
        || name == "docProps/custom.xml"
        || name.ends_with(".rels")
}

/// Story parts carry the document text — and with it, tracked changes and
/// hidden text: the main document, headers, footers, footnotes, endnotes.
fn is_story_part(name: &str) -> bool {
    if name == "word/document.xml" || name == "word/footnotes.xml" || name == "word/endnotes.xml" {
        return true;
    }
    match name.strip_prefix("word/") {
        Some(file) => (file.starts_with("header") || file.starts_with("footer")) && file.ends_with(".xml"),
        None => false,
    }
}

/// Every non-directory entry name in the ZIP, in archive order.
fn part_names(archive: &mut zip::ZipArchive<Cursor<&[u8]>>) -> Result<Vec<String>, AuditError> {
    let len = archive.len();
    if len > MAX_PARTS {
        return Err(AuditError::Limit(format!("too many ZIP entries: {len}")));
    }
    let mut names = Vec::with_capacity(len.min(1024));
    for i in 0..len {
        let file = archive.by_index(i).map_err(|e| AuditError::Zip(e.to_string()))?;
        let name = file.name().to_string();
        if !name.ends_with('/') {
            names.push(name);
        }
    }
    Ok(names)
}

/// Read one part, capped against decompression bombs, and charge it against
/// the cumulative budget. Fails closed: with a single archive for the whole
/// scan, a listed name that can't be read is an error, never a silent skip.
fn read_part(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str, total: &mut u64) -> Result<Vec<u8>, AuditError> {
    let mut data = Vec::new();
    {
        let mut file = archive.by_name(name).map_err(|e| AuditError::Zip(format!("{name}: {e}")))?;
        let mut limited = (&mut file).take(MAX_PART_BYTES + 1);
        limited.read_to_end(&mut data).map_err(|e| AuditError::Zip(format!("{name}: {e}")))?;
    }
    if data.len() as u64 > MAX_PART_BYTES {
        return Err(AuditError::Limit(format!("part too large: {name}")));
    }
    *total = total.saturating_add(data.len() as u64);
    if *total > MAX_TOTAL_BYTES {
        return Err(AuditError::Limit(format!("package too large: {total} decompressed bytes over budget")));
    }
    Ok(data)
}

/// One element the walker saw: namespace URI, local name, decoded attributes.
struct Element {
    ns: Vec<u8>,
    local: String,
    /// (namespace URI, local name, decoded value) per attribute.
    attrs: Vec<(Vec<u8>, String, String)>,
}

/// Call `f` for every start/empty element in `xml`, resolving names by
/// namespace URI so producer-chosen prefixes do not matter.
fn for_each_element(xml: &[u8], part: &str, f: &mut impl FnMut(&Element) -> Result<(), AuditError>) -> Result<(), AuditError> {
    let mut reader = NsReader::from_reader(xml);
    {
        let config = reader.config_mut();
        config.trim_text_start = false;
        config.trim_text_end = false;
        config.expand_empty_elements = false;
        // Reject well-tokenized but mismatched closing tags: a malformed part
        // must fail the audit, never produce a partial (possibly clean-looking)
        // finding list.
        config.check_end_names = true;
    }
    let mut buf = Vec::new();
    let mut count: u64 = 0;
    let mut depth: u32 = 0;
    // XML documents have exactly one root element: a second top-level
    // element (or none at all) is malformed input, not a scannable part.
    let mut root_seen = false;
    let mut check_root = |depth: u32, part: &str| -> Result<(), AuditError> {
        if depth == 0 {
            if root_seen {
                return Err(AuditError::Xml(part.to_string(), "multiple root elements".to_string()));
            }
            root_seen = true;
        }
        Ok(())
    };
    loop {
        buf.clear();
        let (resolution, event) = reader.read_resolved_event_into(&mut buf).map_err(|e| AuditError::Xml(part.to_string(), e.to_string()))?;
        // Copy the namespace URI out so the reader borrow ends here; the
        // attribute resolver below needs `&mut reader` again.
        let ns: Vec<u8> = match &resolution {
            ResolveResult::Bound(n) => n.0.to_vec(),
            _ => Vec::new(),
        };
        match event {
            Event::Start(ref e) => {
                check_root(depth, part)?;
                depth = depth.saturating_add(1);
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                f(&element_of(&mut reader, &ns, e, part)?)?;
            }
            Event::Empty(ref e) => {
                check_root(depth, part)?;
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                f(&element_of(&mut reader, &ns, e, part)?)?;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
            }
            // XML allows only whitespace, comments and processing
            // instructions outside the root element: anything else is
            // malformed input, not a scannable part.
            Event::Text(ref t) => {
                if depth == 0 && !t.as_ref().iter().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')) {
                    return Err(AuditError::Xml(part.to_string(), "text outside the root element".to_string()));
                }
            }
            Event::CData(_) => {
                if depth == 0 {
                    return Err(AuditError::Xml(part.to_string(), "CDATA outside the root element".to_string()));
                }
            }
            Event::Eof => {
                if depth != 0 {
                    return Err(AuditError::Xml(part.to_string(), "unclosed elements at end of part".to_string()));
                }
                if !root_seen {
                    return Err(AuditError::Xml(part.to_string(), "no root element".to_string()));
                }
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolve one start tag into an [`Element`].
fn element_of(reader: &mut NsReader<&[u8]>, ns: &[u8], e: &BytesStart<'_>, part: &str) -> Result<Element, AuditError> {
    let ns = ns.to_vec();
    let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
    let mut attrs = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|err| AuditError::Xml(part.to_string(), err.to_string()))?;
        let key = attr.key;
        if key.as_ref() == b"xmlns" || key.as_ref().starts_with(b"xmlns:") {
            continue;
        }
        let (aresolution, alocal) = reader.resolver_mut().resolve_attribute(key);
        let ans = match aresolution {
            ResolveResult::Bound(ns) => ns.0.to_vec(),
            _ => Vec::new(),
        };
        let alocal = String::from_utf8_lossy(alocal.as_ref()).into_owned();
        let value = attr
            .normalized_value(XmlVersion::Explicit1_0)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).into_owned());
        attrs.push((ans, alocal, value));
    }
    Ok(Element { ns, local, attrs })
}

/// The value of the attribute with namespace `ns` and local name `local`, if present.
fn attr<'a>(el: &'a Element, ns: &[u8], local: &str) -> Option<&'a str> {
    el.attrs.iter().find(|(ans, aname, _)| ans.as_slice() == ns && aname == local).map(|(_, _, v)| v.as_str())
}

/// Collect the text content of every element matching (`ns`, `local`).
fn texts_of(xml: &[u8], part: &str, ns: &[u8], local: &str) -> Result<Vec<String>, AuditError> {
    let mut reader = NsReader::from_reader(xml);
    {
        let config = reader.config_mut();
        config.trim_text_start = false;
        config.trim_text_end = false;
        config.expand_empty_elements = false;
        // See `for_each_element`: malformed parts fail the audit.
        config.check_end_names = true;
    }
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut depth: u32 = 0;
    let mut open: u32 = 0;
    let mut current = String::new();
    let mut count: u64 = 0;
    loop {
        buf.clear();
        let (resolution, event) = reader.read_resolved_event_into(&mut buf).map_err(|e| AuditError::Xml(part.to_string(), e.to_string()))?;
        match event {
            Event::Start(ref e) => {
                open = open.saturating_add(1);
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                let el_ns: &[u8] = match &resolution {
                    ResolveResult::Bound(n) => n.0,
                    _ => b"",
                };
                let el_local = e.local_name();
                let el_local = String::from_utf8_lossy(el_local.as_ref());
                if depth == 0 && el_ns == ns && el_local.as_ref() == local {
                    depth = 1;
                    current.clear();
                } else if depth > 0 {
                    depth = depth.saturating_add(1);
                }
            }
            Event::Empty(ref e) => {
                open = open.saturating_add(1);
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                let el_ns: &[u8] = match &resolution {
                    ResolveResult::Bound(n) => n.0,
                    _ => b"",
                };
                let el_local = e.local_name();
                let el_local = String::from_utf8_lossy(el_local.as_ref());
                if depth == 0 && el_ns == ns && el_local.as_ref() == local {
                    // An empty target element contributes its (empty) text.
                    out.push(String::new());
                } else if depth > 0 {
                    // Empty elements open and close immediately: no depth change.
                }
                open = open.saturating_sub(1);
            }
            Event::End(_) => {
                open = open.saturating_sub(1);
                if depth > 0 {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        out.push(std::mem::take(&mut current));
                    }
                }
            }
            Event::Text(ref t) => {
                if depth > 0 {
                    let decoded = t.xml10_content().map(|v| v.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(t.as_ref()).into_owned());
                    current.push_str(&decoded);
                }
            }
            Event::CData(ref t) => {
                if depth > 0 {
                    current.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Event::Eof => {
                if open != 0 {
                    return Err(AuditError::Xml(part.to_string(), "unclosed elements at end of part".to_string()));
                }
                break;
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Local names of the WordprocessingML revision elements that carry `w:author`
/// (the `CT_TrackChange` and `CT_MoveBookmark` types plus the tracked
/// property-change types, ECMA-376 Part 1 §17.13.5).
/// Excluded: `w:tblGridChange` (`CT_TblGridChange`) and the `*RangeEnd`
/// markers (`CT_MarkupRange`), whose types define no `w:author`.
const REVISION_ELEMENTS: &[&str] = &[
    // Insertions, deletions, moves.
    "ins",
    "del",
    "moveFrom",
    "moveTo",
    // Move bookmarks.
    "moveFromRangeStart",
    "moveToRangeStart",
    // Table revisions.
    "cellIns",
    "cellDel",
    "cellMerge",
    // Custom XML markup revisions.
    "customXmlInsRangeStart",
    "customXmlDelRangeStart",
    "customXmlMoveFromRangeStart",
    "customXmlMoveToRangeStart",
    // Tracked property changes (run, paragraph, section, table, row, cell,
    // numbering): Word writes these for every tracked formatting change.
    "rPrChange",
    "pPrChange",
    "sectPrChange",
    "tblPrChange",
    "tblPrExChange",
    "trPrChange",
    "tcPrChange",
    "numberingChange",
];

/// `CT_OnOff` semantics (ECMA-376 §17.18.67): an absent `w:val` means the
/// property is on; only an explicit false-ish value turns it off.
fn on_off_enabled(el: &Element) -> bool {
    match attr(el, NS_W, "val") {
        None => true,
        Some(v) => !matches!(v.to_ascii_lowercase().as_str(), "false" | "off" | "0" | "f" | "no"),
    }
}

/// Author-bearing revision elements and `w:vanish` runs in one story part
/// (`word/document.xml`, a header/footer, footnotes, endnotes, or a comment
/// body in `word/comments.xml`).
fn scan_story_part(xml: &[u8], part: &str, findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    for_each_element(xml, part, &mut |el| {
        if el.ns.as_slice() != NS_W {
            return Ok(());
        }
        if REVISION_ELEMENTS.contains(&el.local.as_str()) {
            if let Some(author) = attr(el, NS_W, "author")
                && !author.is_empty()
            {
                let mut detail = format!("author=\"{author}\"");
                if let Some(initials) = attr(el, NS_W, "initials")
                    && !initials.is_empty()
                {
                    detail.push_str(&format!(" initials=\"{initials}\""));
                }
                findings.push(AuditFinding::new(FindingKind::RevisionAuthor, format!("{part} <w:{}>", el.local), detail));
            }
            return Ok(());
        }
        // ECMA-376 §17.3.2.42: `w:vanish` hides the run unless its `w:val`
        // explicitly turns it off.
        if el.local == "vanish" && on_off_enabled(el) {
            findings.push(AuditFinding::new(FindingKind::HiddenText, format!("{part} <w:vanish>"), "run property hides text".to_string()));
        }
        Ok(())
    })
}

/// `w:comment` authors in `word/comments.xml` (ECMA-376 §17.13.4.2).
fn scan_comments(xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    for_each_element(xml, "word/comments.xml", &mut |el| {
        if el.ns.as_slice() == NS_W
            && el.local == "comment"
            && let Some(author) = attr(el, NS_W, "author")
            && !author.is_empty()
        {
            let mut detail = format!("author=\"{author}\"");
            if let Some(initials) = attr(el, NS_W, "initials")
                && !initials.is_empty()
            {
                detail.push_str(&format!(" initials=\"{initials}\""));
            }
            if let Some(date) = attr(el, NS_W, "date")
                && !date.is_empty()
            {
                detail.push_str(&format!(" date=\"{date}\""));
            }
            findings.push(AuditFinding::new(FindingKind::CommentAuthor, "word/comments.xml <w:comment>".to_string(), detail));
        }
        Ok(())
    })
}

/// The `w:style` element [`scan_styles`] is currently inside, if any.
struct StyleContext {
    /// Open-element depth at which the style started; the style ends when
    /// the walker's depth drops below this.
    depth: u32,
    style_id: String,
    /// The style's display name, once its `w:name` child has been seen.
    name: Option<String>,
}

/// What a `w:vanish` in `word/styles.xml` hides, given the walk state.
fn style_vanish_detail(style: &Option<StyleContext>, in_defaults: bool) -> String {
    match style {
        Some(ctx) => {
            let label = ctx.name.as_deref().filter(|n| !n.is_empty()).unwrap_or(ctx.style_id.as_str());
            if label.is_empty() { "a style's run properties hide text".to_string() } else { format!("style \"{label}\" run properties hide text") }
        }
        None if in_defaults => "document default run properties hide text".to_string(),
        None => "run properties hide text".to_string(),
    }
}

/// The leaf elements [`scan_styles`] acts on inside a style or the document
/// defaults: a style's `w:name` (which labels later findings) and `w:vanish`
/// (the finding itself).
fn scan_styles_leaf(el: &Element, style: &mut Option<StyleContext>, in_defaults: bool, findings: &mut Vec<AuditFinding>) {
    match el.local.as_str() {
        "name" => {
            if let Some(ctx) = style
                && ctx.name.is_none()
                && let Some(v) = attr(el, NS_W, "val")
            {
                ctx.name = Some(v.to_string());
            }
        }
        "vanish" if on_off_enabled(el) => {
            findings.push(AuditFinding::new(
                FindingKind::StyleHiddenText,
                "word/styles.xml <w:vanish>".to_string(),
                style_vanish_detail(style, in_defaults),
            ));
        }
        _ => {}
    }
}

/// `w:vanish` in `word/styles.xml`: a style whose run properties hide text
/// hides every run that uses the style, and document defaults
/// (`w:docDefaults`) hide every run that does not override them. Neither is
/// visible in the rendered document, so both are findings. The walk tracks
/// the enclosing `w:style` so the finding can name it; unlike
/// [`for_each_element`] it needs end tags, so it drives its own reader (the
/// same bookkeeping as [`texts_of`]).
fn scan_styles(xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    let part = "word/styles.xml";
    let mut reader = NsReader::from_reader(xml);
    {
        let config = reader.config_mut();
        config.trim_text_start = false;
        config.trim_text_end = false;
        config.expand_empty_elements = false;
        // See `for_each_element`: malformed parts fail the audit.
        config.check_end_names = true;
    }
    let mut buf = Vec::new();
    let mut count: u64 = 0;
    let mut open: u32 = 0;
    let mut root_seen = false;
    let mut style: Option<StyleContext> = None;
    // Open-element depth of `w:docDefaults`, while inside it.
    let mut defaults: Option<u32> = None;
    loop {
        buf.clear();
        let (resolution, event) = reader.read_resolved_event_into(&mut buf).map_err(|e| AuditError::Xml(part.to_string(), e.to_string()))?;
        // Copy the namespace URI out so the reader borrow ends here; the
        // attribute resolver in `element_of` needs `&mut reader` again.
        let ns: Vec<u8> = match &resolution {
            ResolveResult::Bound(n) => n.0.to_vec(),
            _ => Vec::new(),
        };
        match event {
            Event::Start(ref e) => {
                if open == 0 {
                    if root_seen {
                        return Err(AuditError::Xml(part.to_string(), "multiple root elements".to_string()));
                    }
                    root_seen = true;
                }
                open = open.saturating_add(1);
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                let el = element_of(&mut reader, &ns, e, part)?;
                if el.ns.as_slice() == NS_W {
                    match el.local.as_str() {
                        "style" => {
                            style = Some(StyleContext { depth: open, style_id: attr(&el, NS_W, "styleId").unwrap_or("").to_string(), name: None });
                        }
                        "docDefaults" => defaults = Some(open),
                        _ => scan_styles_leaf(&el, &mut style, defaults.is_some(), findings),
                    }
                }
            }
            Event::Empty(ref e) => {
                if open == 0 {
                    if root_seen {
                        return Err(AuditError::Xml(part.to_string(), "multiple root elements".to_string()));
                    }
                    root_seen = true;
                }
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                let el = element_of(&mut reader, &ns, e, part)?;
                if el.ns.as_slice() == NS_W {
                    scan_styles_leaf(&el, &mut style, defaults.is_some(), findings);
                }
            }
            Event::End(_) => {
                open = open.saturating_sub(1);
                if style.as_ref().is_some_and(|ctx| open < ctx.depth) {
                    style = None;
                }
                if defaults.is_some_and(|depth| open < depth) {
                    defaults = None;
                }
            }
            Event::Eof => {
                if open != 0 {
                    return Err(AuditError::Xml(part.to_string(), "unclosed elements at end of part".to_string()));
                }
                if !root_seen {
                    return Err(AuditError::Xml(part.to_string(), "no root element".to_string()));
                }
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

/// `w15:person` author names in `word/people.xml`: the roster Word keeps of
/// everyone who has commented, names included even when the comments
/// themselves were later deleted.
fn scan_people(xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    for_each_element(xml, "word/people.xml", &mut |el| {
        if el.ns.as_slice() == NS_W15
            && el.local == "person"
            && let Some(author) = attr(el, NS_W15, "author")
            && !author.is_empty()
        {
            findings.push(AuditFinding::new(FindingKind::PeopleAuthor, "word/people.xml <w15:person>".to_string(), format!("author=\"{author}\"")));
        }
        Ok(())
    })
}

/// `dc:creator` / `cp:lastModifiedBy` in `docProps/core.xml`.
fn scan_core_properties(xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    for (ns, local, element) in [(NS_DC, "creator", "dc:creator"), (NS_CP, "lastModifiedBy", "cp:lastModifiedBy")] {
        for value in texts_of(xml, "docProps/core.xml", ns, local)? {
            let value = value.trim();
            if !value.is_empty() {
                findings.push(AuditFinding::new(FindingKind::CorePropertiesAuthor, format!("docProps/core.xml <{element}>"), value.to_string()));
            }
        }
    }
    Ok(())
}

/// `Company` / `Manager` in `docProps/app.xml`: organizational, not personal,
/// but still identifying — reported at `info` severity.
fn scan_extended_properties(xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    for local in ["Company", "Manager"] {
        for value in texts_of(xml, "docProps/app.xml", NS_EP, local)? {
            let value = value.trim();
            if !value.is_empty() {
                findings.push(AuditFinding::new(FindingKind::ExtendedProperties, format!("docProps/app.xml <{local}>"), value.to_string()));
            }
        }
    }
    Ok(())
}

/// Identity-named properties in `docProps/custom.xml` (ECMA-376 Part 2
/// §12.3.3). Custom properties are arbitrary name/value pairs, so only
/// names that identify a person or organization are reported; the value of
/// any other property is the author's own content, not metadata leakage.
const CUSTOM_IDENTITY_NAMES: &[&str] = &[
    "author",
    "creator",
    "created by",
    "createdby",
    "last modified by",
    "lastmodifiedby",
    "modified by",
    "modifiedby",
    "owner",
    "editor",
    "company",
    "manager",
];

fn scan_custom_properties(xml: &[u8], findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    let part = "docProps/custom.xml";
    let mut reader = NsReader::from_reader(xml);
    {
        let config = reader.config_mut();
        config.trim_text_start = false;
        config.trim_text_end = false;
        config.expand_empty_elements = false;
        // See `for_each_element`: malformed parts fail the audit.
        config.check_end_names = true;
    }
    let mut buf = Vec::new();
    let mut count: u64 = 0;
    let mut open: u32 = 0;
    // Depth inside the `property` element currently open (0 = outside one),
    // its `name` attribute, and its accumulated text (the value lives in a
    // child element, e.g. `vt:lpwstr`).
    let mut in_property: u32 = 0;
    let mut current_name = String::new();
    let mut current_value = String::new();
    loop {
        buf.clear();
        let (resolution, event) = reader.read_resolved_event_into(&mut buf).map_err(|e| AuditError::Xml(part.to_string(), e.to_string()))?;
        // Copy the namespace URI out so the reader borrow ends here; the
        // attribute resolver in `element_of` needs `&mut reader` again.
        let el_ns: Vec<u8> = match &resolution {
            ResolveResult::Bound(n) => n.0.to_vec(),
            _ => Vec::new(),
        };
        match event {
            Event::Start(ref e) => {
                open = open.saturating_add(1);
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                if in_property == 0 && el_ns.as_slice() == NS_CUST && e.local_name().as_ref() == b"property" {
                    let el = element_of(&mut reader, &el_ns, e, part)?;
                    current_name = attr(&el, b"", "name").unwrap_or("").to_string();
                    current_value.clear();
                    in_property = 1;
                } else if in_property > 0 {
                    in_property = in_property.saturating_add(1);
                }
            }
            Event::Empty(_) => {
                open = open.saturating_add(1);
                count += 1;
                if count > MAX_XML_ELEMENTS {
                    return Err(AuditError::Limit(format!("{part}: too many XML elements")));
                }
                // An empty element opens and closes at once; an empty
                // `property` has no value to report.
                open = open.saturating_sub(1);
            }
            Event::End(_) => {
                open = open.saturating_sub(1);
                if in_property > 0 {
                    in_property -= 1;
                    if in_property == 0 {
                        report_custom_property(&current_name, &current_value, findings);
                    }
                }
            }
            Event::Text(ref t) => {
                if in_property > 0 {
                    let decoded = t.xml10_content().map(|v| v.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(t.as_ref()).into_owned());
                    current_value.push_str(&decoded);
                }
            }
            Event::CData(ref t) => {
                if in_property > 0 {
                    current_value.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Event::Eof => {
                if open != 0 {
                    return Err(AuditError::Xml(part.to_string(), "unclosed elements at end of part".to_string()));
                }
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Report one custom property when its name identifies a person or
/// organization and its value is non-empty.
fn report_custom_property(name: &str, value: &str, findings: &mut Vec<AuditFinding>) {
    let value = value.trim();
    if value.is_empty() || !CUSTOM_IDENTITY_NAMES.contains(&name.trim().to_ascii_lowercase().as_str()) {
        return;
    }
    findings.push(AuditFinding::new(FindingKind::CorePropertiesAuthor, format!("docProps/custom.xml <property name=\"{name}\">"), value.to_string()));
}

/// Relationship types that mark active content: the VBA project part and
/// embedded objects/packages (ECMA-376 Part 2 §11, plus the Office VBA extension).
const RT_VBA_PROJECT: &str = "http://schemas.microsoft.com/office/2006/relationships/vbaProject";
const RT_OLE_OBJECT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject";
const RT_PACKAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/package";

/// Macro and embedded-object parts: reported as visibility, never blocked or
/// removed. Detection is by relationship type first — a reachable VBA or OLE
/// part may live at any valid package path — with the fixed names and
/// directories as fallback for parts whose relationships are missing.
fn scan_active_content(names: &[String], parts: &BTreeMap<String, Vec<u8>>, findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    // Parts already reported, so the two passes never double-report one part.
    let mut reported: BTreeSet<String> = BTreeSet::new();
    let mut report = |kind: FindingKind, location: String, detail: &str, findings: &mut Vec<AuditFinding>| {
        if reported.insert(location.clone()) {
            findings.push(AuditFinding::new(kind, location, detail.to_string()));
        }
    };
    for name in names {
        if name == "word/vbaProject.bin" {
            report(FindingKind::VbaProject, name.clone(), "VBA macro project present", findings);
        } else if name.starts_with("word/embeddings/") {
            report(FindingKind::OleEmbedding, name.clone(), "embedded object present", findings);
        }
    }
    for name in names.iter().filter(|n| n.ends_with(".rels")) {
        let Some(xml) = parts.get(name) else { continue };
        let source_dir = source_dir_of_rels(name);
        for_each_element(xml, name, &mut |el| {
            if el.ns.as_slice() == NS_RELS
                && el.local == "Relationship"
                && attr(el, b"", "TargetMode") != Some("External")
                && let Some(target) = attr(el, b"", "Target")
            {
                let resolved = resolve_target(&source_dir, target);
                match attr(el, b"", "Type").unwrap_or("") {
                    RT_VBA_PROJECT => report(FindingKind::VbaProject, resolved, "VBA macro project present (relationship type)", findings),
                    RT_OLE_OBJECT | RT_PACKAGE => {
                        report(FindingKind::OleEmbedding, resolved, "embedded object present (relationship type)", findings)
                    }
                    _ => {}
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

/// ZIP entries no relationship targets: content nothing references.
/// Reachability is computed by traversal from every reachable `*.rels`
/// part; external relationship targets (`TargetMode="External"`) are not
/// package parts. OPC part names are case-insensitive and relationship
/// targets are URIs, so reachability compares normalized names
/// (percent-decoded, ASCII-lowercased): a target spelled
/// `Media/Image%201.png` reaches the part `word/media/image 1.png`.
fn scan_orphans(names: &[String], parts: &BTreeMap<String, Vec<u8>>, findings: &mut Vec<AuditFinding>) -> Result<(), AuditError> {
    // Normalized name -> the real part name / bytes, for lookups that must
    // survive case and percent-encoding differences.
    let mut original_by_norm: BTreeMap<String, &str> = BTreeMap::new();
    for name in names {
        original_by_norm.entry(normalize_part_name(name)).or_insert(name.as_str());
    }
    let mut parts_by_norm: BTreeMap<String, &Vec<u8>> = BTreeMap::new();
    for (name, data) in parts {
        parts_by_norm.entry(normalize_part_name(name)).or_insert(data);
    }
    let mut reachable: BTreeSet<String> = BTreeSet::new();
    reachable.insert(normalize_part_name("[Content_Types].xml"));
    let mut stack = vec!["_rels/.rels".to_string()];
    while let Some(rels_name) = stack.pop() {
        let rels_norm = normalize_part_name(&rels_name);
        if !original_by_norm.contains_key(&rels_norm) || !reachable.insert(rels_norm) {
            continue;
        }
        let Some(xml) = parts_by_norm.get(&normalize_part_name(&rels_name)) else { continue };
        let source_dir = source_dir_of_rels(&rels_name);
        let mut targets = Vec::new();
        for_each_element(xml, &rels_name, &mut |el| {
            if el.ns.as_slice() == NS_RELS
                && el.local == "Relationship"
                && let Some(target) = attr(el, b"", "Target")
                && attr(el, b"", "TargetMode") != Some("External")
            {
                targets.push(resolve_target(&source_dir, target));
            }
            Ok(())
        })?;
        for target in targets {
            let target_norm = normalize_part_name(&target);
            if let Some(original) = original_by_norm.get(&target_norm)
                && reachable.insert(target_norm)
                && let Some(rels) = rels_of_part(original)
            {
                stack.push(rels);
            }
        }
    }
    for name in names {
        if !reachable.contains(&normalize_part_name(name)) {
            findings.push(AuditFinding::new(FindingKind::OrphanPart, name.clone(), "no relationship targets this part".to_string()));
        }
    }
    Ok(())
}

/// The comparison form of a part name: percent-decoded (relationship
/// targets are URIs, so a space may arrive as `%20`) and ASCII-lowercased
/// (OPC part names are case-insensitive, ECMA-376 Part 2 §9.1.1).
fn normalize_part_name(name: &str) -> String {
    percent_decode(name).to_ascii_lowercase()
}

/// Decode `%XX` escapes; a `%` not followed by two hex digits stays as-is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = ((bytes[i + 1] as char).to_digit(16), (bytes[i + 2] as char).to_digit(16))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The directory of the part a `*.rels` file describes:
/// `word/_rels/document.xml.rels` describes `word/document.xml`, whose
/// directory is `word`; `_rels/.rels` describes the package root.
fn source_dir_of_rels(rels: &str) -> String {
    let without_ext = rels.strip_suffix(".rels").unwrap_or(rels);
    match without_ext.rfind("/_rels/") {
        Some(i) => without_ext[..i].to_string(),
        None => String::new(),
    }
}

/// The `*.rels` path for a package part (`word/document.xml` →
/// `word/_rels/document.xml.rels`), per OPC §12.2.4. `None` for parts that
/// have no relationships part (`[Content_Types].xml`).
fn rels_of_part(part: &str) -> Option<String> {
    if part == "[Content_Types].xml" {
        return None;
    }
    let (dir, file) = match part.rfind('/') {
        Some(i) => (&part[..i], &part[i + 1..]),
        None => ("", part),
    };
    if file.is_empty() {
        return None;
    }
    Some(if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") })
}

/// Resolve a relationship target against the source part's directory, lexically.
/// Absolute targets (`/word/...`) resolve from the package root.
fn resolve_target(source_dir: &str, target: &str) -> String {
    let joined = if let Some(abs) = target.strip_prefix('/') {
        abs.to_string()
    } else if source_dir.is_empty() {
        target.to_string()
    } else {
        format!("{source_dir}/{target}")
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    segments.join("/")
}

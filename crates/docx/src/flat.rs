//! Word XML Document: a whole WordprocessingML package as one XML file (the Flat OPC format Word
//! saves as "Word XML Document (*.xml)"): a `pkg:package` root holding one `pkg:part` per package
//! part, each with its name and content type, and the part's XML inline (`pkg:xmlData`) or its
//! bytes in base64 (`pkg:binaryData`).
//!
//! Reading unpacks the parts into an in-memory [`Package`] and hands it to the DOCX reader;
//! writing runs the DOCX writer and lays its parts out in one file. Bounded like the zip reader:
//! every part counts against the part count, part size and total size limits whether it's kept
//! or not (a repeated name), data is checked before it's copied, and nesting and elements are
//! capped as the XML tree reader caps a part's.

use std::collections::{BTreeMap, HashSet};

use base64::Engine as _;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;
use wordcraft_doc::Document;

use crate::package::{ContentTypes, MAX_ENTRIES, MAX_PART, MAX_TOTAL, Package};
use crate::xml::{self, W, esc};
use crate::{DocxError, Flavor};

/// The Flat OPC package namespace.
pub const PKG_NS: &str = "http://schemas.microsoft.com/office/2006/xmlPackage";
/// The WordprocessingML 2003 namespace (Word 2003 XML Document).
pub const W2003_NS: &str = "http://schemas.microsoft.com/office/word/2003/wordml";

/// What kind of XML file a `.xml` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmlKind {
    /// A Flat OPC package (Word XML Document, Word 2007 and later).
    FlatOpc,
    /// A Word 2003 XML Document (WordprocessingML 2003).
    WordMl2003,
    /// Some other XML, or not XML at all.
    Other,
}

/// Elements and declarations looked at before the root element (comments, processing
/// instructions) before giving up.
const MAX_PROLOG_EVENTS: usize = 10_000;

/// The text of `bytes` as UTF-8: UTF-16 with a byte-order mark is transcoded, a UTF-8 one dropped.
fn utf8_text(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    match xml::utf16_to_utf8(bytes) {
        Some(s) => std::borrow::Cow::Owned(s.into_bytes()),
        None => std::borrow::Cow::Borrowed(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes)),
    }
}

/// Tell a Word XML file by its root element: `pkg:package` (Flat OPC) or `w:wordDocument`
/// (Word 2003), each in its namespace.
pub fn sniff_xml(bytes: &[u8]) -> XmlKind {
    let text = utf8_text(bytes);
    let mut r = NsReader::from_reader(text.as_ref());
    for _ in 0..MAX_PROLOG_EVENTS {
        match r.read_resolved_event() {
            Ok((res, Event::Start(e) | Event::Empty(e))) => {
                let ns = match res {
                    ResolveResult::Bound(ns) => ns.as_ref().to_vec(),
                    _ => Vec::new(),
                };
                return match (e.local_name().as_ref(), ns.as_slice()) {
                    (b"package", ns) if ns == PKG_NS.as_bytes() => XmlKind::FlatOpc,
                    (b"wordDocument", ns) if ns == W2003_NS.as_bytes() => XmlKind::WordMl2003,
                    _ => XmlKind::Other,
                };
            }
            Ok((_, Event::Eof)) | Err(_) => return XmlKind::Other,
            Ok(_) => {}
        }
    }
    XmlKind::Other
}

/// Read a Flat OPC file.
pub fn read(bytes: &[u8]) -> Result<Document, DocxError> {
    crate::read::read_opened(&unpack(bytes)?)
}

/// The data element of the part being read.
enum Data {
    /// `pkg:xmlData`: its content starts at this byte offset.
    Xml(usize),
    /// `pkg:binaryData`: the base64 text so far.
    Binary(String),
}

/// The part being read: name, content type, data.
struct Part {
    name: String,
    content_type: String,
    data: Option<Data>,
}

/// Deepest element nesting read: a part's inline XML starts below the package, part and data
/// elements and may nest as deep as the XML tree reader takes ([`xml::MAX_DEPTH`] open elements
/// and an empty one inside the deepest).
const MAX_NESTING: usize = 3 + xml::MAX_DEPTH + 1;

/// Most elements in one child of the package: a part, its data element and as many as the XML
/// tree reader takes in a part ([`xml::MAX_ELEMENTS`]).
const MAX_PART_ELEMENTS: usize = 2 + xml::MAX_ELEMENTS;

/// Longest base64 text of a part that decodes to at most `bytes` (whitespace isn't kept).
fn max_base64(bytes: u64) -> u64 {
    (bytes / 3).saturating_add(1).saturating_mul(4)
}

/// Bytes the next part may have: its own size limit, or what's left of the package's.
fn room(total: u64) -> u64 {
    MAX_PART.min(MAX_TOTAL.saturating_sub(total))
}

/// Unpack a Flat OPC file into its parts, with a `[Content_Types].xml` made from the parts'
/// content types (unless the file carries one).
pub(crate) fn unpack(bytes: &[u8]) -> Result<Package, DocxError> {
    let text = utf8_text(bytes);
    let src: &[u8] = text.as_ref();
    let mut r = NsReader::from_reader(src);
    r.config_mut().check_end_names = true;
    let mut depth = 0usize;
    // Children of the package (parts or anything else), and elements in the current one.
    let mut entries = 0usize;
    let mut elements = 0usize;
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut types: Vec<(String, String)> = Vec::new();
    let mut total: u64 = 0;
    let mut part: Option<Part> = None;
    // `xmlns` declarations on the package, part and data elements, in scope for the inline XML.
    let mut scope: Vec<Vec<(String, String)>> = Vec::new();
    let bad = |e: quick_xml::Error| DocxError::Xml(e.to_string());
    loop {
        let before = usize::try_from(r.buffer_position()).unwrap_or(usize::MAX);
        let (res, ev) = r.read_resolved_event().map_err(bad)?;
        let in_pkg = matches!(&res, ResolveResult::Bound(ns) if ns.as_ref() == PKG_NS.as_bytes());
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(ev, Event::Empty(_));
                depth += 1;
                if depth > MAX_NESTING {
                    return Err(DocxError::Limit("XML nesting too deep".into()));
                }
                if depth == 2 {
                    entries += 1;
                    elements = 0;
                    if entries > MAX_ENTRIES {
                        return Err(DocxError::Limit(format!("more than {MAX_ENTRIES} parts")));
                    }
                }
                elements += 1;
                if elements > MAX_PART_ELEMENTS {
                    return Err(DocxError::Limit("too many XML elements".into()));
                }
                let local = e.local_name().as_ref().to_vec();
                match depth {
                    1 if !(in_pkg && local == b"package") => {
                        return Err(DocxError::NotWord("the root element isn't a Flat OPC pkg:package".into()));
                    }
                    2 if in_pkg && local == b"part" => {
                        let name = attr(e, b"name").unwrap_or_default();
                        let content_type = attr(e, b"contentType").unwrap_or_default();
                        part = Some(Part { name, content_type, data: None });
                    }
                    3 if in_pkg => {
                        if let Some(p) = part.as_mut().filter(|p| p.data.is_none()) {
                            let pos = usize::try_from(r.buffer_position()).unwrap_or(usize::MAX);
                            p.data = match local.as_slice() {
                                // `<pkg:xmlData/>`: an empty part.
                                b"xmlData" if empty => Some(Data::Binary(String::new())),
                                b"xmlData" => Some(Data::Xml(pos)),
                                b"binaryData" => Some(Data::Binary(String::new())),
                                _ => None,
                            };
                        }
                    }
                    _ => {}
                }
                if depth <= 3 {
                    scope.push(ns_decls(e));
                }
                if empty {
                    if depth == 2
                        && let Some(p) = part.take()
                    {
                        finish_part(p, &mut files, &mut types, &mut total)?;
                    }
                    if depth <= 3 {
                        scope.pop();
                    }
                    depth -= 1;
                }
            }
            Event::End(_) => {
                // The inline XML ends where its data element's end tag starts.
                if depth == 3
                    && let Some(p) = part.as_mut()
                    && let Some(Data::Xml(start)) = p.data
                {
                    p.data = None;
                    let frag = src.get(start..before).unwrap_or_default();
                    // Refused before it's copied when it can't fit.
                    if frag.len() as u64 > room(total) {
                        return Err(DocxError::Limit(format!("part {} is larger than the size limit", p.name)));
                    }
                    let xml = with_scope(frag, &scope);
                    store(&p.name, &p.content_type, xml, &mut files, &mut types, &mut total)?;
                }
                if depth == 2
                    && let Some(p) = part.take()
                {
                    finish_part(p, &mut files, &mut types, &mut total)?;
                }
                if depth <= 3 {
                    scope.pop();
                }
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            Event::Text(ref t) if depth == 3 => add_base64(&mut part, t, total)?,
            Event::CData(ref t) if depth == 3 => add_base64(&mut part, t, total)?,
            Event::GeneralRef(ref g) if depth == 3 => match g.resolve_char_ref() {
                // A character reference stands for its character (`&#65;` is an `A`).
                Ok(Some(c)) => add_base64(&mut part, c.encode_utf8(&mut [0; 4]).as_bytes(), total)?,
                // An entity isn't base64: binary data holding one is broken, not shorter.
                _ => {
                    if let Some(Part { name, data: Some(Data::Binary(_)), .. }) = &part {
                        return Err(DocxError::Xml(format!("part {name} has invalid base64 data")));
                    }
                }
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if files.is_empty() {
        return Err(DocxError::NotWord("the Flat OPC package has no parts".into()));
    }
    if !files.contains_key("[Content_Types].xml") {
        let mut w = W::new();
        w.open("Types", &[("xmlns", "http://schemas.openxmlformats.org/package/2006/content-types")]);
        for (name, ct) in &types {
            w.empty("Override", &[("PartName", &format!("/{name}")), ("ContentType", ct)]);
        }
        w.close("Types");
        files.insert("[Content_Types].xml".into(), w.into_bytes());
    }
    Ok(Package::from_parts(files))
}

/// End of a `pkg:part`: store its binary data (inline XML was stored when it ended).
fn finish_part(p: Part, files: &mut BTreeMap<String, Vec<u8>>, types: &mut Vec<(String, String)>, total: &mut u64) -> Result<(), DocxError> {
    match p.data {
        Some(Data::Binary(b64)) => {
            let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(b64.trim_end_matches('='))
                .map_err(|_| DocxError::Xml(format!("part {} has invalid base64 data", p.name)))?;
            store(&p.name, &p.content_type, bytes, files, types, total)
        }
        // A part without data, or whose inline XML was already stored.
        _ => Ok(()),
    }
}

/// Text (or CDATA) inside a part's `pkg:binaryData`: its base64 without whitespace, checked
/// against the size limits before it's copied.
fn add_base64(part: &mut Option<Part>, text: &[u8], total: u64) -> Result<(), DocxError> {
    let Some(Part { data: Some(Data::Binary(b64)), .. }) = part else { return Ok(()) };
    let s = std::str::from_utf8(text).map_err(|e| DocxError::Xml(e.to_string()))?;
    let more = s.bytes().filter(|b| !b.is_ascii_whitespace()).count();
    if b64.len().saturating_add(more) as u64 > max_base64(room(total)) {
        return Err(DocxError::Limit("a binary part is larger than the size limit".into()));
    }
    b64.extend(s.chars().filter(|c| !c.is_ascii_whitespace()));
    Ok(())
}

/// Add a part, within the size limits; a repeated or empty name isn't stored but counts against
/// them all the same (as a repeated zip entry does).
fn store(
    name: &str,
    content_type: &str,
    bytes: Vec<u8>,
    files: &mut BTreeMap<String, Vec<u8>>,
    types: &mut Vec<(String, String)>,
    total: &mut u64,
) -> Result<(), DocxError> {
    let name = name.trim_start_matches('/');
    let len = bytes.len() as u64;
    if len > MAX_PART {
        return Err(DocxError::Limit(format!("part {name} is larger than the size limit")));
    }
    *total = total.saturating_add(len);
    if *total > MAX_TOTAL {
        return Err(DocxError::Limit("the package is larger than the size limit".into()));
    }
    if name.is_empty() || files.contains_key(name) {
        return Ok(());
    }
    if !content_type.is_empty() && !content_type.contains(['\r', '\n']) {
        types.push((name.to_string(), content_type.to_string()));
    }
    files.insert(name.to_string(), bytes);
    Ok(())
}

/// An attribute's unescaped value, by local name (any prefix).
fn attr(e: &BytesStart, local: &[u8]) -> Option<String> {
    e.attributes().with_checks(false).flatten().find(|a| a.key.local_name().as_ref() == local).map(|a| {
        match a.normalized_value(quick_xml::XmlVersion::Explicit1_0) {
            Ok(v) => v.into_owned(),
            Err(_) => String::from_utf8_lossy(&a.value).into_owned(),
        }
    })
}

/// The `xmlns` / `xmlns:p` declarations of a start tag, as written (value still escaped).
fn ns_decls(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .with_checks(false)
        .flatten()
        .filter(|a| a.key.as_ref() == b"xmlns" || a.key.as_ref().starts_with(b"xmlns:"))
        .filter(|a| !a.value.contains(&b'"'))
        .map(|a| (String::from_utf8_lossy(a.key.as_ref()).into_owned(), String::from_utf8_lossy(&a.value).into_owned()))
        .take(256)
        .collect()
}

/// The inline XML of a part as a document of its own: namespaces it inherits from the package
/// elements are declared on its root element (unless it declares them itself).
fn with_scope(frag: &[u8], scope: &[Vec<(String, String)>]) -> Vec<u8> {
    let decls: Vec<&(String, String)> = scope.iter().flatten().filter(|(k, _)| k != "xmlns:pkg").collect();
    let Some(open) = root_tag_start(frag) else { return frag.to_vec() };
    if decls.is_empty() {
        return frag.get(open..).unwrap_or_default().to_vec();
    }
    let rest = frag.get(open..).unwrap_or_default();
    let name_end = rest.iter().position(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/').unwrap_or(rest.len());
    let tag_end = rest.iter().position(|b| *b == b'>').unwrap_or(rest.len());
    let tag = String::from_utf8_lossy(rest.get(1..tag_end).unwrap_or_default());
    let start = BytesStart::from_content(tag.as_ref(), name_end.saturating_sub(1));
    let declared: HashSet<Vec<u8>> = start
        .attributes()
        .with_checks(false)
        .flatten()
        .filter(|a| a.key.as_ref() == b"xmlns" || a.key.as_ref().starts_with(b"xmlns:"))
        .map(|a| a.key.as_ref().to_vec())
        .collect();
    let mut out = Vec::with_capacity(rest.len() + 256);
    out.extend_from_slice(rest.get(..name_end).unwrap_or_default());
    // Innermost declarations win: walk from the data element out, skipping prefixes seen.
    let mut seen: Vec<&str> = Vec::new();
    for (k, v) in decls.iter().rev() {
        if declared.contains(k.as_bytes()) || seen.contains(&k.as_str()) {
            continue;
        }
        seen.push(k);
        out.extend_from_slice(format!(" {k}=\"{v}\"").as_bytes());
    }
    out.extend_from_slice(rest.get(name_end..).unwrap_or_default());
    out
}

/// Offset of the `<` of the first element in `frag` (after whitespace, comments and processing
/// instructions).
fn root_tag_start(frag: &[u8]) -> Option<usize> {
    let mut i = 0usize;
    loop {
        while frag.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let rest = frag.get(i..)?;
        if rest.starts_with(b"<!--") {
            i += rest.windows(3).position(|w| w == b"-->")? + 3;
        } else if rest.starts_with(b"<?") {
            i += rest.windows(2).position(|w| w == b"?>")? + 2;
        } else if rest.first() == Some(&b'<') {
            return Some(i);
        } else {
            return None;
        }
    }
}

/// [`write`], with what the package couldn't carry of the document's embedded objects (see
/// [`crate::write_with_report`]).
pub fn write_with_report(doc: &Document) -> Result<(Vec<u8>, Vec<crate::Loss>), DocxError> {
    let (parts, losses) = crate::write::write_parts_report(doc, Flavor::Document)?;
    Ok((flatten(&parts), losses))
}

/// Write `doc` as a Word XML Document (Flat OPC).
pub fn write(doc: &Document) -> Result<Vec<u8>, DocxError> {
    write_with_report(doc).map(|(bytes, _)| bytes)
}

/// The Flat OPC document holding package `parts` (as [`crate::write::write_parts`] gives them).
fn flatten(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let types = parts
        .iter()
        .find(|(n, _)| n == "[Content_Types].xml")
        .map(|(n, b)| ContentTypes::read(&Package::from_parts(BTreeMap::from([(n.clone(), b.clone())]))))
        .unwrap_or_default();
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<?mso-application progid=\"Word.Document\"?>\n");
    s.push_str(&format!("<pkg:package xmlns:pkg=\"{PKG_NS}\">"));
    for (name, bytes) in parts {
        if name == "[Content_Types].xml" {
            continue;
        }
        let ct = types.of(name).unwrap_or("application/octet-stream");
        s.push_str(&format!("<pkg:part pkg:name=\"/{}\" pkg:contentType=\"{}\"", esc(name), esc(ct)));
        match xml_fragment(ct, bytes) {
            Some(x) => {
                s.push_str("><pkg:xmlData>");
                s.push_str(x);
                s.push_str("</pkg:xmlData></pkg:part>");
            }
            None => {
                s.push_str(" pkg:compression=\"store\"><pkg:binaryData>");
                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                for (i, chunk) in b64.as_bytes().chunks(76).enumerate() {
                    if i > 0 {
                        s.push('\n');
                    }
                    s.push_str(std::str::from_utf8(chunk).unwrap_or_default());
                }
                s.push_str("</pkg:binaryData></pkg:part>");
            }
        }
    }
    s.push_str("</pkg:package>");
    s.into_bytes()
}

/// An XML part's markup without its XML declaration, to inline as `pkg:xmlData`; `None` for
/// a part that isn't UTF-8 XML (or has a DOCTYPE), which goes in base64.
fn xml_fragment<'a>(ct: &str, bytes: &'a [u8]) -> Option<&'a str> {
    let ct = ct.to_ascii_lowercase();
    if !(ct.ends_with("+xml") || ct.ends_with("/xml")) {
        return None;
    }
    let s = std::str::from_utf8(bytes).ok()?;
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s).trim_start();
    let s = match s.strip_prefix("<?xml") {
        Some(rest) => rest.split_once("?>").map(|(_, r)| r)?.trim_start(),
        None => s,
    };
    (s.starts_with('<') && !s.contains("<!DOCTYPE") && one_element(s)).then_some(s)
}

/// Whether `s` is exactly one well-formed element, with only whitespace, comments and processing
/// instructions around it. Kept bytes are inlined into `pkg:xmlData` only then; anything else could
/// close the wrapper and add parts of its own, so it goes out as `pkg:binaryData` (#3742).
fn one_element(s: &str) -> bool {
    let mut r = quick_xml::Reader::from_str(s);
    r.config_mut().check_comments = true;
    let (mut depth, mut roots) = (0usize, 0usize);
    loop {
        let attrs_ok = |e: &BytesStart| e.attributes().all(|a| a.is_ok());
        match r.read_event() {
            Ok(Event::Start(e)) => {
                if !attrs_ok(&e) || (depth == 0 && roots > 0) {
                    return false;
                }
                roots += usize::from(depth == 0);
                depth += 1;
            }
            Ok(Event::Empty(e)) => {
                if !attrs_ok(&e) || (depth == 0 && roots > 0) {
                    return false;
                }
                roots += usize::from(depth == 0);
            }
            Ok(Event::End(_)) => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return false,
            },
            Ok(Event::Text(t)) if depth == 0 => {
                if !t.iter().all(u8::is_ascii_whitespace) {
                    return false;
                }
            }
            Ok(Event::CData(_) | Event::GeneralRef(_)) if depth == 0 => return false,
            Ok(Event::Decl(_) | Event::DocType(_)) | Err(_) => return false,
            Ok(Event::Eof) => return depth == 0 && roots == 1,
            Ok(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_root_elements() {
        assert_eq!(
            sniff_xml(
                format!("<?xml version=\"1.0\"?><?mso-application progid=\"Word.Document\"?><!-- x --><p:package xmlns:p=\"{PKG_NS}\"/>").as_bytes()
            ),
            XmlKind::FlatOpc
        );
        assert_eq!(sniff_xml(format!("\u{FEFF}<w:wordDocument xmlns:w=\"{W2003_NS}\"/>").as_bytes()), XmlKind::WordMl2003);
        assert_eq!(sniff_xml(b"<package/>"), XmlKind::Other);
        assert_eq!(sniff_xml(b"<html><body/></html>"), XmlKind::Other);
        assert_eq!(sniff_xml(b"\x00\xff garbage"), XmlKind::Other);
        assert_eq!(sniff_xml(b""), XmlKind::Other);
    }

    #[test]
    fn inline_xml_inherits_namespaces_from_the_package() {
        let frag = b"\n <!-- c --> <w:document><w:body/></w:document>";
        let scope = vec![vec![("xmlns:pkg".to_string(), PKG_NS.to_string()), ("xmlns:w".to_string(), "urn:w".to_string())]];
        let out = String::from_utf8(with_scope(frag, &scope)).unwrap();
        assert_eq!(out, "<w:document xmlns:w=\"urn:w\"><w:body/></w:document>");
        let own = b"<w:document xmlns:w=\"urn:own\"/>";
        assert_eq!(with_scope(own, &scope), own.to_vec());
        assert_eq!(with_scope(b"no element", &scope), b"no element".to_vec());
    }

    #[test]
    fn inherited_namespaces_do_not_rescan_a_long_root_tag() {
        let scope = vec![(0..256).map(|i| (format!("xmlns:p{i}"), format!("urn:{i}"))).collect::<Vec<_>>()];
        let frag = format!("<w:document{}><w:body/></w:document>", " a=\"1\"".repeat(4_000_000));
        let out = with_scope(frag.as_bytes(), &scope);
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out.matches("xmlns:p").count(), 256);
    }

    #[test]
    fn nesting_is_capped_where_the_xml_tree_reader_stops() {
        let inline = |levels: usize| format!("{}<b/>{}", "<a>".repeat(levels), "</a>".repeat(levels));
        let package = |levels: usize| {
            format!(
                "<pkg:package xmlns:pkg=\"{PKG_NS}\"><pkg:part pkg:name=\"/a.xml\"><pkg:xmlData>{}</pkg:xmlData></pkg:part></pkg:package>",
                inline(levels)
            )
        };
        assert!(xml::parse(inline(xml::MAX_DEPTH).as_bytes()).is_ok());
        assert!(unpack(package(xml::MAX_DEPTH).as_bytes()).is_ok());
        assert!(matches!(xml::parse(inline(xml::MAX_DEPTH + 1).as_bytes()), Err(DocxError::Limit(_))));
        assert!(matches!(unpack(package(xml::MAX_DEPTH + 1).as_bytes()), Err(DocxError::Limit(_))));
    }

    #[test]
    fn kept_xml_that_would_break_out_of_its_part_is_written_in_base64() {
        // A part kept from a file is written back as it was read; inlined as is, markup that
        // closes the wrapper adds parts of its own (here a second core properties part, read
        // before the real one) (#3742).
        let evil = format!(
            "<a/></pkg:xmlData></pkg:part><pkg:part pkg:name=\"/docProps/core.xml\" pkg:contentType=\"application/vnd.openxmlformats-package.core-properties+xml\"><pkg:xmlData><cp:coreProperties xmlns:cp=\"{}\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Injected</dc:title></cp:coreProperties></pkg:xmlData></pkg:part><pkg:part pkg:name=\"/x.xml\"><pkg:xmlData><b>",
            "http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
        );
        let w_ns = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"";
        let rel = |id: &str, kind: &str, target: &str| format!("<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>");
        let rels = format!(
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}{}</Relationships>",
            rel("rId1", crate::package::rt::OFFICE_DOC, "word/document.xml"),
            rel("rId2", crate::package::rt::CUSTOM_XML, "customXml/item1.xml")
        );
        let types = "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/></Types>";
        let z = crate::package::zip_entries(&[
            ("[Content_Types].xml".into(), types.as_bytes().to_vec()),
            ("_rels/.rels".into(), rels.into_bytes()),
            ("word/document.xml".into(), format!("<w:document {w_ns}><w:body><w:p/></w:body></w:document>").into_bytes()),
            ("customXml/item1.xml".into(), evil.clone().into_bytes()),
        ])
        .unwrap();
        let mut doc = crate::read(&z).unwrap();
        assert!(doc.passthrough.values().any(|b| b.as_slice() == evil.as_bytes()), "the part is kept as read");
        doc.core.title = "Real".into();
        let flat = write(&doc).unwrap();
        let text = String::from_utf8_lossy(&flat);
        assert_eq!(text.matches("pkg:name=\"/docProps/core.xml\"").count(), 1, "{text}");
        let back = read(&flat).unwrap();
        assert_eq!(back.core.title, "Real");
        assert!(back.passthrough.values().any(|b| b.as_slice() == evil.as_bytes()), "the part survives, in base64");
    }

    #[test]
    fn only_one_well_formed_element_is_inlined() {
        let ct = "application/xml";
        for ok in ["<a/>", "<?xml version=\"1.0\"?>\n<a x=\"1\"><b>t &amp; <![CDATA[c]]></b></a>\n<!-- end -->", "<!-- c --><a/><?pi x?>"] {
            assert!(xml_fragment(ct, ok.as_bytes()).is_some(), "{ok}");
        }
        for bad in [
            "<a/><b/>",
            "<a></b>",
            "<a>",
            "<a/></x>",
            "<a/>text",
            "<a/>&amp;",
            "<a x=1/>",
            "<a/><?xml version=\"1.0\"?>",
            "<a></a></pkg:xmlData>",
            "<a><!-- x -- y --></a>",
            "<a/><!-- x--->",
        ] {
            assert!(xml_fragment(ct, bad.as_bytes()).is_none(), "{bad}");
        }
    }

    #[test]
    fn repeated_and_nameless_parts_count_against_the_size_limits() {
        let (mut files, mut types, mut total) = (BTreeMap::new(), Vec::new(), 0u64);
        for name in ["/a.bin", "/a.bin", ""] {
            store(name, "", vec![0; 10], &mut files, &mut types, &mut total).unwrap();
        }
        assert_eq!((files.len(), total), (1, 30));
        // Near the package limit, what's left of it is all the next part may have.
        total = MAX_TOTAL - 5;
        assert_eq!(room(total), 5);
        assert_eq!(max_base64(5), 8);
        assert!(matches!(store("/b.bin", "", vec![0; 6], &mut files, &mut types, &mut total), Err(DocxError::Limit(_))));
    }
}

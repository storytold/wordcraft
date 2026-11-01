//! EPUB 3 import (read-only; export is tracked separately, see #2815).
//!
//! An EPUB is an OCF ZIP package: `META-INF/container.xml` points at a package document (OPF),
//! which lists every resource in its manifest and the reading order in its spine. Each spine
//! item is an XHTML content document; this reads it with the existing HTML importer
//! ([`crate::html::parse_with`]), resolving the images it references from the ZIP archive
//! relative to the content document's own path, and concatenates every chapter's blocks in
//! spine order. The navigation document and NCX are not read: the body's own headings already
//! carry the structure a Navigation pane or Table of Contents field would use.
//!
//! This is a first pass: fixed-layout EPUBs, scripted content, DRM and EPUB 2 packages whose
//! spine points at non-XHTML content are not specifically handled (the HTML importer reads what
//! text it can from malformed markup; it does not reject it).
//!
//! Written from the public EPUB 3.3, OCF and XHTML specifications; no code from another EPUB
//! reader or converter is used.

use std::cell::RefCell;
use std::io::{Cursor, Read};
use std::sync::Arc;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use wordcraft_doc::Document;

use crate::model::{self, FBlock, Flow};

/// Largest single zip entry read (one content document or image).
const MAX_ENTRY: u64 = 64 << 20;
/// Total bytes read across every entry while importing one package (a zip-bomb guard: many
/// small declared sizes can't add up past this, the same budget style as the ODT importer's
/// `Pictures`).
const MAX_TOTAL: u64 = 256 << 20;
/// Most spine items read; further ones are ignored.
const MAX_SPINE: usize = 10_000;
/// Most manifest items read; further ones are ignored.
const MAX_MANIFEST: usize = 10_000;

/// Parse an EPUB package into the flow model.
pub fn parse(bytes: &[u8]) -> Result<Flow, String> {
    parse_lang(bytes).map(|(flow, _)| flow)
}

/// Import an EPUB package into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    let (flow, lang) = parse_lang(bytes)?;
    let mut doc = model::to_doc(&flow);
    if let Some(lang) = lang {
        doc.styles.default_chr.lang = Some(lang);
    }
    Ok(doc)
}

/// [`parse`], with the package's `dc:language`, if any (the flow model has no language field;
/// a caller applies it to the document's default run properties, as the HTML exporter reads it
/// back out, #2815).
fn parse_lang(bytes: &[u8]) -> Result<(Flow, Option<String>), String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("EPUB: not a valid package ({e})"))?;
    let container = read_entry(&mut z, "META-INF/container.xml", MAX_TOTAL).ok_or("EPUB: META-INF/container.xml is missing")?;
    let opf_path = container_rootfile(&container)
        .map_err(|()| "EPUB: META-INF/container.xml is truncated or malformed")?
        .ok_or("EPUB: container.xml names no package document")?;
    let opf_bytes = read_entry(&mut z, &opf_path, MAX_TOTAL).ok_or_else(|| format!("EPUB: package document {opf_path} is missing"))?;
    let opf_dir = dir_of(&opf_path);
    let pkg = read_package(&opf_bytes, &opf_dir).map_err(|()| format!("EPUB: package document {opf_path} is truncated or malformed"))?;

    let mut budget = MAX_TOTAL.saturating_sub(container.len() as u64).saturating_sub(opf_bytes.len() as u64);
    let mut blocks: Vec<FBlock> = Vec::new();
    for href in pkg.spine.iter().take(MAX_SPINE) {
        let Some(chapter_bytes) = read_entry(&mut z, href, budget) else { continue };
        budget = budget.saturating_sub(chapter_bytes.len() as u64);
        let xhtml = crate::html::decode(&chapter_bytes);
        let chapter_dir = dir_of(href);
        let images = ZipImages { z: RefCell::new(&mut z), dir: chapter_dir, budget: RefCell::new(budget) };
        let loader: &dyn Fn(&str) -> Option<Arc<Vec<u8>>> = &|src| images.load(src);
        let mut chapter = crate::html::parse_with(&xhtml, loader);
        budget = images.budget.into_inner();
        blocks.append(&mut chapter.blocks);
    }

    let mut flow = Flow { blocks, meta: Default::default() };
    flow.meta.title = pkg.title;
    flow.meta.author = pkg.creator;
    Ok((flow, pkg.language))
}

/// A package document's reading order and metadata.
#[derive(Default)]
struct Package {
    /// Spine items' hrefs, resolved to full in-zip paths, in reading order.
    spine: Vec<String>,
    title: String,
    creator: String,
    language: Option<String>,
}

/// `META-INF/container.xml`'s package document path (the first `rootfile`'s `full-path`).
/// `Err`: the XML is truncated or malformed. `Ok(None)`: well-formed, but names no package
/// document.
fn container_rootfile(xml: &[u8]) -> Result<Option<String>, ()> {
    let mut path = None;
    events(xml, |ev| {
        if path.is_none()
            && let Ev::Start(q, a) = ev
            && local(&q) == "rootfile"
        {
            path = get(&a, "full-path").map(str::to_string);
        }
    })?;
    Ok(path.filter(|s| !s.is_empty()))
}

/// The package document (OPF): manifest (id → href) and spine (idrefs, in order) resolved to
/// full in-zip paths under `opf_dir`, plus `dc:title`/`dc:creator`/`dc:language`.
fn read_package(xml: &[u8], opf_dir: &str) -> Result<Package, ()> {
    use std::collections::HashMap;
    let mut manifest: HashMap<String, String> = HashMap::new();
    let mut idrefs: Vec<String> = Vec::new();
    let mut pkg = Package::default();
    let mut in_metadata = false;
    let mut cur_meta: Option<&'static str> = None;
    // The *first* `dc:title`/`dc:creator`/`dc:language` wins when a package repeats one (OPF
    // allows several, e.g. alternate-script titles); these latch once that element ends, so a
    // later element of the same name no longer opens `cur_meta`. Within one element, every text
    // and entity-reference event appends (quick-xml splits text around `&amp;` etc. into several
    // events), instead of keeping only the first chunk.
    let mut title_done = false;
    let mut creator_done = false;
    let mut language_done = false;
    events(xml, |ev| match ev {
        Ev::Start(q, a) => match local(&q) {
            "metadata" => in_metadata = true,
            "title" if in_metadata && !title_done => cur_meta = Some("title"),
            "creator" if in_metadata && !creator_done => cur_meta = Some("creator"),
            "language" if in_metadata && !language_done => cur_meta = Some("language"),
            "item" if manifest.len() < MAX_MANIFEST => {
                if let (Some(id), Some(href)) = (get(&a, "id"), get(&a, "href")) {
                    manifest.insert(id.to_string(), resolve_path(opf_dir, href));
                }
            }
            "itemref" if idrefs.len() < MAX_SPINE => {
                if let Some(idref) = get(&a, "idref") {
                    idrefs.push(idref.to_string());
                }
            }
            _ => {}
        },
        Ev::End(q) => match local(&q) {
            "metadata" => in_metadata = false,
            "title" if cur_meta == Some("title") => {
                title_done = true;
                cur_meta = None;
            }
            "creator" if cur_meta == Some("creator") => {
                creator_done = true;
                cur_meta = None;
            }
            "language" if cur_meta == Some("language") => {
                language_done = true;
                cur_meta = None;
            }
            _ => {}
        },
        Ev::Text(t) => match cur_meta {
            Some("title") => pkg.title.push_str(&t),
            Some("creator") => pkg.creator.push_str(&t),
            Some("language") => pkg.language.get_or_insert_with(String::new).push_str(&t),
            _ => {}
        },
    })?;
    pkg.language = pkg.language.filter(|s| !s.trim().is_empty());
    pkg.spine = idrefs.into_iter().filter_map(|id| manifest.get(&id).cloned()).collect();
    Ok(pkg)
}

/// The directory portion of an in-package path (`a/b/c.xhtml` → `a/b`; `c.xhtml` → `""`).
fn dir_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => String::new(),
    }
}

/// Resolve a relative href against `base_dir`, applying `.`/`..` segments and stripping a
/// fragment or query. A hostile `../../../etc/passwd`-style href can't escape the archive: it
/// only ever names a zip entry, never a filesystem path, and `read_entry` fails closed when the
/// entry doesn't exist.
fn resolve_path(base_dir: &str, href: &str) -> String {
    let href = href.split(['#', '?']).next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = if href.starts_with('/') { Vec::new() } else { base_dir.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in href.split('/').take(4096) {
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

/// Minimal percent-decoding (`%20` → space); an invalid escape passes through unchanged.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Resolves `<img>` sources in one chapter's XHTML to the ZIP archive, within the remaining
/// read budget. `'a` is reborrowed fresh each chapter; `'b` is the archive's own (longer)
/// lifetime, over the original package bytes — keeping them separate lets each chapter borrow
/// `z` in turn instead of one borrow spanning the whole spine loop.
struct ZipImages<'a, 'b> {
    z: RefCell<&'a mut zip::ZipArchive<Cursor<&'b [u8]>>>,
    dir: String,
    budget: RefCell<u64>,
}

impl ZipImages<'_, '_> {
    fn load(&self, src: &str) -> Option<Arc<Vec<u8>>> {
        let path = resolve_path(&self.dir, src);
        if path.is_empty() {
            return None;
        }
        let room = *self.budget.borrow();
        let data = read_entry(&mut self.z.borrow_mut(), &path, room)?;
        *self.budget.borrow_mut() = room.saturating_sub(data.len() as u64);
        Some(Arc::new(data))
    }
}

fn read_entry(z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str, budget: u64) -> Option<Vec<u8>> {
    let f = z.by_name(name).ok()?;
    let cap = MAX_ENTRY.min(budget);
    if f.size() > cap {
        return None;
    }
    let mut out = Vec::new();
    f.take(cap).read_to_end(&mut out).ok()?;
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// A minimal XML event walker (container.xml and the OPF only; chapter content goes through the
// HTML importer). Mirrors the one in `odt.rs`; kept local so each format module stays
// self-contained.

type Attrs = Vec<(String, String)>;

fn attrs_of(e: &BytesStart<'_>) -> Attrs {
    let mut v = Vec::new();
    for a in e.attributes().with_checks(false).flatten().take(128) {
        let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let val =
            a.normalized_value(XmlVersion::Explicit1_0).map(|c| c.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned());
        v.push((k, val));
    }
    v
}

fn get<'a>(a: &'a Attrs, k: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn local(q: &str) -> &str {
    q.rsplit(':').next().unwrap_or(q)
}

enum Ev {
    Start(String, Attrs),
    End(String),
    Text(String),
}

/// Stream XML events (empty elements become start + end). `Err` for a parser error (including a
/// closing tag that doesn't match what it closes, or one with no open element to close — unlike
/// `odt.rs`'s lenient reader, `container.xml` and the package document are required by spec to
/// be well-formed, and the depth check below depends on End events being genuine), or for end of
/// input with an element still open — quick-xml returns an ordinary `Event::Eof` for both a
/// well-formed document and one truncated mid-element, so a caller that only looked at the
/// events it got could mistake a cut-off `container.xml` or package document for a complete, if
/// sparse, one and import a silently partial document.
fn events(xml: &[u8], mut f: impl FnMut(Ev)) -> Result<(), ()> {
    let mut r = quick_xml::Reader::from_reader(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth: usize = 0;
    loop {
        buf.clear();
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                f(Ev::Start(String::from_utf8_lossy(e.name().as_ref()).into_owned(), attrs_of(&e)));
            }
            Ok(Event::Empty(e)) => {
                let n = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                f(Ev::Start(n.clone(), attrs_of(&e)));
                f(Ev::End(n));
            }
            Ok(Event::End(e)) => {
                depth = match depth.checked_sub(1) {
                    Some(d) => d,
                    None => return Err(()),
                };
                f(Ev::End(String::from_utf8_lossy(e.name().as_ref()).into_owned()));
            }
            Ok(Event::Text(t)) => f(Ev::Text(t.decode().map(|c| c.into_owned()).unwrap_or_default())),
            Ok(Event::CData(t)) => f(Ev::Text(String::from_utf8_lossy(&t).into_owned())),
            Ok(Event::GeneralRef(r)) => {
                let name = String::from_utf8_lossy(&r).into_owned();
                let s = match name.as_str() {
                    "amp" => "&".to_string(),
                    "lt" => "<".to_string(),
                    "gt" => ">".to_string(),
                    "quot" => "\"".to_string(),
                    "apos" => "'".to_string(),
                    _ => r.resolve_char_ref().ok().flatten().map(|c| c.to_string()).unwrap_or_default(),
                };
                f(Ev::Text(s));
            }
            Ok(Event::Eof) => return if depth == 0 { Ok(()) } else { Err(()) },
            Err(_) => return Err(()),
            Ok(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            let _ = z.start_file(*name, o);
            let _ = z.write_all(data);
        }
        z.finish().map(|c| c.into_inner()).unwrap_or_default()
    }

    const CONTAINER: &[u8] = br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    fn opf(spine_items: &str) -> Vec<u8> {
        format!(
            r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Test Book</dc:title>
<dc:creator xmlns:dc="http://purl.org/dc/elements/1.1/">Jane Author</dc:creator>
<dc:language xmlns:dc="http://purl.org/dc/elements/1.1/">en</dc:language></metadata>
<manifest>
<item id="c1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
<item id="c2" href="text/chapter2.xhtml" media-type="application/xhtml+xml"/>
<item id="img1" href="images/cover.png" media-type="image/png"/>
</manifest>
<spine>{spine_items}</spine>
</package>"#
        )
        .into_bytes()
    }

    fn sample_package() -> Vec<u8> {
        let chapter1 = br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Chapter One</h1><p>First chapter text.</p></body></html>"#;
        let chapter2 =
            br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Chapter Two</h1><p>Second chapter, with a <img src="../images/cover.png"/> picture.</p></body></html>"#;
        let png = [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        zip_of(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", &opf(r#"<itemref idref="c1"/><itemref idref="c2"/>"#)),
            ("OEBPS/text/chapter1.xhtml", chapter1),
            ("OEBPS/text/chapter2.xhtml", chapter2),
            ("OEBPS/images/cover.png", &png),
        ])
    }

    #[test]
    fn resolves_relative_paths() {
        assert_eq!(resolve_path("OEBPS/text", "../images/cover.png"), "OEBPS/images/cover.png");
        assert_eq!(resolve_path("OEBPS", "text/chapter1.xhtml"), "OEBPS/text/chapter1.xhtml");
        assert_eq!(resolve_path("OEBPS/text", "chapter1.xhtml#frag"), "OEBPS/text/chapter1.xhtml");
        assert_eq!(resolve_path("", "cover%20art.png"), "cover art.png");
    }

    #[test]
    fn reads_container_and_package() {
        let pkg = read_package(&opf(r#"<itemref idref="c1"/><itemref idref="c2"/>"#), "OEBPS").unwrap();
        assert_eq!(pkg.title, "Test Book");
        assert_eq!(pkg.creator, "Jane Author");
        assert_eq!(pkg.language.as_deref(), Some("en"));
        assert_eq!(pkg.spine, vec!["OEBPS/text/chapter1.xhtml", "OEBPS/text/chapter2.xhtml"]);
    }

    /// quick-xml splits text around an entity reference into several `Text`/`GeneralRef` events;
    /// title/creator must accumulate all of them, not just the first chunk, and a repeated
    /// element (OPF allows more than one `dc:title`) must not overwrite what the first gave.
    #[test]
    fn metadata_accumulates_across_entity_references_and_keeps_only_the_first_element() {
        let xml = br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata>
<dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Fish &amp; Chips</dc:title>
<dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">A second, ignored title</dc:title>
<dc:creator xmlns:dc="http://purl.org/dc/elements/1.1/">Smith &amp; Sons</dc:creator>
</metadata>
<manifest/>
<spine/>
</package>"#;
        let pkg = read_package(xml, "OEBPS").unwrap();
        assert_eq!(pkg.title, "Fish & Chips");
        assert_eq!(pkg.creator, "Smith & Sons");
    }

    #[test]
    fn imports_chapters_in_spine_order_with_metadata_and_image() {
        let doc = import(&sample_package()).unwrap();
        assert_eq!(doc.core.title, "Test Book");
        assert_eq!(doc.core.creator, "Jane Author");
        assert_eq!(doc.styles.default_chr.lang.as_deref(), Some("en"));
        let text = String::from_utf8_lossy(&crate::txt::export(&doc)).into_owned();
        let c1 = text.find("Chapter One").expect("chapter one heading");
        let f1 = text.find("First chapter text").expect("chapter one body");
        let c2 = text.find("Chapter Two").expect("chapter two heading");
        let f2 = text.find("Second chapter").expect("chapter two body");
        assert!(c1 < f1 && f1 < c2 && c2 < f2, "chapters interleaved out of spine order: {text}");
        assert!(!doc.media.is_empty(), "expected the referenced image to be imported into the document's media");
    }

    #[test]
    fn missing_container_is_an_error() {
        let bytes = zip_of(&[("mimetype", b"application/epub+zip")]);
        assert!(import(&bytes).is_err());
    }

    #[test]
    fn not_a_zip_is_an_error() {
        assert!(import(b"not a zip file").is_err());
    }

    /// Truncated mid-element, after the rootfile it names: a lenient parser that only reports
    /// the events it already produced could otherwise call this a well-formed container with no
    /// rootfile missing, and import whatever of the package it still found — a silently partial
    /// document instead of a refused, visibly corrupt one.
    #[test]
    fn container_truncated_after_its_useful_content_is_still_an_error() {
        let truncated = br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>"#;
        let bytes = zip_of(&[("META-INF/container.xml", &truncated[..]), ("OEBPS/content.opf", &opf(""))]);
        assert!(import(&bytes).is_err());
    }

    /// Same, for the package document: truncated after its manifest but before its spine, which
    /// would otherwise import a document with no chapters and no error.
    #[test]
    fn package_document_truncated_mid_spine_is_still_an_error() {
        let truncated = br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Test Book</dc:title></metadata>
<manifest><item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="c1"/>"#;
        let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", &truncated[..])]);
        assert!(import(&bytes).is_err());
    }

    /// A closing tag that doesn't match its open element must not decrement the depth counter
    /// as if it did — otherwise a `</wrong>` closing an unrelated element could reach EOF at
    /// depth zero despite `<container>` never actually closing, defeating the truncation check
    /// above.
    #[test]
    fn mismatched_closing_tag_is_an_error() {
        let mismatched = br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</wrong>"#;
        let bytes = zip_of(&[("META-INF/container.xml", &mismatched[..]), ("OEBPS/content.opf", &opf(""))]);
        assert!(import(&bytes).is_err());
    }

    #[test]
    fn spine_item_missing_from_zip_is_skipped_not_fatal() {
        let bytes = zip_of(&[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", &opf(r#"<itemref idref="c1"/>"#)),
            // chapter1.xhtml deliberately absent
        ]);
        let doc = import(&bytes).unwrap();
        assert_eq!(doc.core.title, "Test Book");
    }

    #[test]
    fn path_traversal_href_cannot_escape_the_archive() {
        assert_eq!(resolve_path("OEBPS/text", "../../../../../../etc/passwd"), "etc/passwd");
    }
}

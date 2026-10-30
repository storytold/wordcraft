//! Objects kept for writing back: charts, SmartArt diagrams and OLE objects.
//!
//! WordCraft draws charts and diagrams and shows an OLE object's picture, but doesn't edit them.
//! So that saving doesn't lose them, the reader keeps each one's own markup (an `a:graphic` or a
//! `w:object`) as a [`wordcraft_doc::graphic::Embedded`] on the object, and every part that markup
//! reaches through relationships (chart, embedded workbook, colours and style; diagram data, layout,
//! quick style, colours and drawing; OLE embedding and its picture…) as opaque bytes in
//! `Document::passthrough`, with their content types and relationships in an
//! [`EmbeddedManifest`](crate::package::EmbeddedManifest). The font table and the fonts it embeds
//! are kept the same way.

use std::collections::HashSet;
use std::sync::Arc;

use wordcraft_doc::graphic::{Embedded, EmbeddedRef};

use super::Reader;
use crate::LossKind;
use crate::package::{
    CUSTOM_XML_ROOTS, ContentTypes, MAX_EMBEDDED_PARTS, MAX_EMBEDDED_RELS, MAX_EMBEDDED_XML, Rel, Rels, UNREADABLE_NAME, manifest_field_ok, rel_is,
    rt, shown,
};
use crate::xml::{El, MAX_DEPTH, NAMESPACES, esc, is_rel_attr};

/// Relationship types (last path segment) the reader models or the writer produces itself, so
/// they are never carried verbatim: carrying them would duplicate or contradict the written part.
/// Types the main document refers to by id (headers, pictures, objects…) are listed too.
pub const HANDLED_RELS: &[&str] = &[
    "officeDocument",
    "core-properties",
    "extended-properties",
    "custom-properties",
    "styles",
    "numbering",
    "settings",
    "theme",
    "fontTable",
    "footnotes",
    "endnotes",
    "comments",
    "commentsExtended",
    "commentsIds",
    "commentsExtensible",
    "customXml",
    "vbaProject",
    "header",
    "footer",
    "image",
    "hyperlink",
    "oleObject",
    "package",
    "chart",
    "chartEx",
    "diagramData",
    "diagramDrawing",
    "diagramLayout",
    "diagramQuickStyle",
    "diagramColors",
    "video",
    "audio",
    "media",
    "control",
    "subDocument",
    "ink",
];

/// Most relationship ids one object's markup may use.
const MAX_IDS: usize = 64;

/// The relationship ids the main document's markup refers to: the ids it names in a
/// relationship attribute (`r:id="rId5"`, VML's `o:relid`) — the same rule the embedded objects
/// use ([`rel_ids`]).
///
/// One pass over the markup the reader has already parsed, not one search per id: the ids are
/// looked up as they are met, so the cost is the size of the document rather than its size
/// times the number of relationships (#12). A name that is not a relationship is ignored, and a
/// name that is not one of the document's relationship ids is not collected. An id that only
/// appears as text is not a reference: the text does not name a part, and treating it as one
/// lost the part on save.
fn referenced_ids<'a>(root: &El, document: &'a Rels) -> HashSet<&'a [u8]> {
    // The ids the loop in `keep_unmodelled` can ask about: the first `MAX_EMBEDDED_RELS`
    // non-external relationships, and no more, so the set stays bounded however many the file
    // carries.
    let ids: HashSet<&[u8]> = document.list.iter().filter(|r| !r.external).take(MAX_EMBEDDED_RELS).map(|r| r.id.as_bytes()).collect();
    let mut found = HashSet::new();
    if ids.is_empty() {
        return found;
    }
    collect_rel_ids(root, &ids, &mut found, 0);
    found
}

/// The ids `e` names in a relationship attribute that are in `ids`, collected into `found`.
///
/// The attribute is a relationship attribute by [`is_rel_attr`]: one in the relationship
/// namespace, VML's `o:relid`, or the unqualified alias of one on an element that carries a
/// relationship — the same attribute the reader resolves a relationship from, so a part the
/// markup names is not carried over a second time.
fn collect_rel_ids<'a>(e: &El, ids: &HashSet<&'a [u8]>, found: &mut HashSet<&'a [u8]>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for (k, v) in &e.attrs {
        if is_rel_attr(&e.name, k)
            && let Some(id) = ids.get(v.as_bytes())
        {
            found.insert(*id);
        }
    }
    for c in e.els() {
        collect_rel_ids(c, ids, found, depth + 1);
    }
}

impl Reader<'_> {
    /// Keep custom XML stores reached from the main document or package, without evaluating
    /// bindings. A bibliography already loaded into the source model belongs to its writer.
    pub(super) fn keep_custom_xml(&mut self, package: &Rels, document: &Rels) {
        let mut roots = String::new();
        for (owner, rels) in [("P", package), ("D", document)] {
            for rel in rels.list.iter().filter(|r| !r.external && rel_is(&r.kind, rt::CUSTOM_XML)).take(MAX_EMBEDDED_RELS) {
                if ![&rel.kind, &rel.target].iter().all(|s| manifest_field_ok(s)) {
                    continue;
                }
                if !self.doc.sources.is_empty() && self.xml(&rel.target).ok().flatten().is_some_and(|r| r.local() == "Sources") {
                    continue;
                }
                // Native ink shares the custom XML relationship type, but its part belongs to the
                // drawing that refers to it: kept with it, or dropped with it (no orphan part).
                let pkg = self.pkg;
                if self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(&rel.target) == Some("application/inkml+xml") {
                    continue;
                }
                if self.keep_part_tree(&rel.target) {
                    roots.push_str(&format!("{owner}\t{}\t{}\n", rel.kind, rel.target));
                }
            }
        }
        if !roots.is_empty() {
            self.doc.passthrough.insert(CUSTOM_XML_ROOTS.into(), Arc::new(roots.into_bytes()));
        }
    }

    /// Keep the parts the package and the main document relate to that WordCraft neither models
    /// nor writes itself (web settings, styles-with-effects, add-in task panes, the thumbnail, a
    /// glossary…), with every part they reach, so saving carries them unchanged. Skipped: the
    /// relationship types the writer produces or the reader models ([`HANDLED_RELS`]) and any
    /// relationship the main document's markup refers to by id (its target goes with the
    /// element, or not at all).
    pub(super) fn keep_unmodelled(&mut self, root: &El, package: &Rels, document: &Rels) {
        // The ids the main document's markup refers to, in one pass over the markup: an id is
        // named in a relationship attribute (`r:id="rId5"`). One scan for all of them, not one
        // search per relationship — a large document with many relationships took minutes to
        // open (#12).
        let referenced = referenced_ids(root, document);
        let mut roots = String::new();
        // One glossary per main document (its first relationship; see `read_glossary`).
        let mut glossary = false;
        for (owner, rels) in [("P", package), ("D", document)] {
            for rel in rels.list.iter().filter(|r| !r.external).take(MAX_EMBEDDED_RELS) {
                let tail = rel.kind.rsplit('/').next().unwrap_or("");
                if HANDLED_RELS.contains(&tail)
                    || (owner == "D" && referenced.contains(rel.id.as_bytes()))
                    || (owner == "D" && tail == "glossaryDocument" && std::mem::replace(&mut glossary, true))
                    || ![&rel.kind, &rel.target].iter().all(|s| manifest_field_ok(s))
                {
                    continue;
                }
                if self.keep_part_tree(&rel.target) {
                    roots.push_str(&format!("{owner}\t{}\t{}\n", rel.kind, rel.target));
                }
            }
        }
        if !roots.is_empty() {
            self.doc.passthrough.insert(crate::package::UNMODELLED_ROOTS.into(), Arc::new(roots.into_bytes()));
        }
    }

    /// Keep `el` (an object's `a:graphic` or `w:object`, read with `rels`) and the parts it refers
    /// to, so the object can be written back. `extra` names relationships the object's parts use
    /// by id although its markup doesn't (a SmartArt drawing, named in the diagram's data part).
    /// `None` when it can't be written back faithfully: a relationship it uses is missing, a part
    /// is missing, or it's too big.
    pub(super) fn keep_embedded(&mut self, el: &El, rels: &Rels, extra: &[String]) -> Option<Arc<Embedded>> {
        let mut ids = Vec::new();
        rel_ids(el, &mut ids, 0)?;
        let mut refs = Vec::new();
        for id in &ids {
            let rel = rels.by_id(id)?;
            if !rel.external && !self.keep_part_tree(&rel.target) {
                return None;
            }
            refs.push(EmbeddedRef { id: id.clone(), kind: rel.kind.clone(), target: rel.target.clone(), external: rel.external });
        }
        for id in extra.iter().filter(|id| !ids.contains(id)) {
            if let Some(rel) = rels.by_id(id).filter(|r| !r.external)
                && self.keep_part_tree(&rel.target)
            {
                refs.push(EmbeddedRef { id: id.clone(), kind: rel.kind.clone(), target: rel.target.clone(), external: false });
            }
        }
        let xml = standalone_xml(el)?;
        Some(Arc::new(Embedded { xml, refs }))
    }

    /// Keep part `root` and every part it reaches through its relationships. False when `root`
    /// itself can't be kept. A part it reaches that can't be kept (missing, past
    /// [`MAX_EMBEDDED_PARTS`], or not a part we carry) and relationships past
    /// [`MAX_EMBEDDED_RELS`] are left out, never related, and noted in the manifest's `dropped`
    /// (see [`crate::incomplete_objects`]). Parts already kept count as reached, so a cycle ends.
    pub(super) fn keep_part_tree(&mut self, root: &str) -> bool {
        if self.embedded.parts.contains_key(root) {
            return true;
        }
        if let Err((kind, why)) = self.keep_part(root) {
            let main = self.main.clone();
            self.embedded.note(&main, kind, format!("relates to {}, {why}; it isn't kept", shown(root)));
            return false;
        }
        let mut todo = vec![root.to_string()];
        while let Some(part) = todo.pop() {
            let mut kept: Vec<Rel> = Vec::new();
            for r in self.pkg.rels(&part).list {
                if kept.len() >= MAX_EMBEDDED_RELS {
                    log::warn!("docx: part {part} has more than {MAX_EMBEDDED_RELS} relationships; the rest are dropped");
                    self.embedded.note(
                        &part,
                        LossKind::RelationshipLimit,
                        format!("has more than {MAX_EMBEDDED_RELS} relationships; the rest aren't kept"),
                    );
                    break;
                }
                if !manifest_field_ok(&r.target) {
                    self.embedded.note(&part, LossKind::NotCarried, format!("relates to {UNREADABLE_NAME}, which can't be kept"));
                    continue;
                }
                if ![&r.id, &r.kind].iter().all(|f| manifest_field_ok(f)) {
                    self.embedded.note(
                        &part,
                        LossKind::NotCarried,
                        "has a relationship with an empty or unreadable id or type, which can't be kept".into(),
                    );
                    continue;
                }
                if kept.iter().any(|k| k.id == r.id) {
                    continue;
                }
                if !r.external && !self.embedded.parts.contains_key(&r.target) {
                    if let Err((kind, why)) = self.keep_part(&r.target) {
                        self.embedded.note(&part, kind, format!("relates to {}, {why}", r.target));
                        continue;
                    }
                    todo.push(r.target.clone());
                }
                kept.push(r);
            }
            if let Some(p) = self.embedded.parts.get_mut(&part) {
                p.rels = kept;
            }
        }
        true
    }

    /// Keep one part's bytes and content type (not its relationships). `Err` says why not.
    fn keep_part(&mut self, part: &str) -> Result<(), (LossKind, String)> {
        let lower = part.to_ascii_lowercase();
        if !manifest_field_ok(part) || lower.ends_with(".rels") || lower == "[content_types].xml" || part.eq_ignore_ascii_case(&self.main) {
            log::warn!("docx: not carrying part {part:?} for an embedded object");
            return Err((LossKind::NotCarried, "which isn't a part an object can carry".into()));
        }
        if self.embedded.parts.len() >= MAX_EMBEDDED_PARTS {
            log::warn!("docx: more than {MAX_EMBEDDED_PARTS} parts for embedded objects; the rest are dropped");
            return Err((LossKind::PartLimit, format!("past the limit of {MAX_EMBEDDED_PARTS} parts kept for embedded objects")));
        }
        let Some(bytes) = self.pkg.get_shared(part) else {
            log::warn!("docx: part {part} of an embedded object is missing");
            return Err((LossKind::MissingPart, "which the file doesn't hold".into()));
        };
        let pkg = self.pkg;
        let ct = self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(part).unwrap_or("application/octet-stream");
        let content_type = if manifest_field_ok(ct) { ct.to_string() } else { "application/octet-stream".to_string() };
        self.doc.passthrough.entry(part.to_string()).or_insert_with(|| bytes.clone());
        self.embedded.parts.insert(part.to_string(), crate::package::EmbeddedPart { content_type, rels: Vec::new() });
        Ok(())
    }
}

/// The relationship ids `e` uses (attributes in the relationships namespace, VML's `o:relid`,
/// and the unqualified alias of one on an element that carries a relationship), in document
/// order. `None` past [`MAX_IDS`].
fn rel_ids(e: &El, out: &mut Vec<String>, depth: usize) -> Option<()> {
    if depth > MAX_DEPTH {
        return Some(());
    }
    for (k, v) in &e.attrs {
        if is_rel_attr(&e.name, k) && !out.contains(v) {
            if out.len() >= MAX_IDS {
                return None;
            }
            out.push(v.clone());
        }
    }
    for c in e.els() {
        rel_ids(c, out, depth + 1)?;
    }
    Some(())
}

/// `e` as a standalone element: its XML with the namespace declarations its canonical prefixes
/// need. `None` when too long, or in a namespace we can't name.
fn standalone_xml(e: &El) -> Option<String> {
    let xml = e.to_xml();
    let rest = xml.get(1 + e.name.len()..).filter(|_| !xml.is_empty())?;
    let mut s = format!("<{}", e.name);
    for (p, u) in NAMESPACES {
        s.push_str(&format!(" xmlns:{p}=\"{}\"", esc(u)));
    }
    s.push_str(rest);
    (s.len() <= MAX_EMBEDDED_XML).then_some(s)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use crate::package::UNMODELLED_ROOTS;

    const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="bin" ContentType="application/octet-stream"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

    /// A package whose main part holds `body` (the XML inside `w:body`), and which relates to
    /// `rels` parts of a type the reader doesn't model, so each one is looked for in the part.
    fn package(body: &str, rels: &[(&str, &str)], parts: &[(&str, &[u8])]) -> Vec<u8> {
        let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
        let mut r = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
        for (id, target) in rels {
            r.push_str(&format!(
                r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/printerSettings" Target="{target}"/>"#
            ));
        }
        r.push_str("</Relationships>");

        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("[Content_Types].xml", opts).unwrap();
        zw.write_all(CONTENT_TYPES.as_bytes()).unwrap();
        zw.start_file("_rels/.rels", opts).unwrap();
        zw.write_all(ROOT_RELS.as_bytes()).unwrap();
        zw.start_file("word/document.xml", opts).unwrap();
        zw.write_all(doc.as_bytes()).unwrap();
        zw.start_file("word/_rels/document.xml.rels", opts).unwrap();
        zw.write_all(r.as_bytes()).unwrap();
        for (name, bytes) in parts {
            zw.start_file(name, opts).unwrap();
            zw.write_all(bytes).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }

    /// The parts the reader carries over unchanged, as one string.
    fn unmodelled_roots(d: &wordcraft_doc::Document) -> String {
        d.passthrough.get(UNMODELLED_ROOTS).map(|v| String::from_utf8_lossy(v).to_string()).unwrap_or_default()
    }

    /// Opening a document costs the size of its main part, not that size times the number of its
    /// relationships (#12). Eight megabytes of main part with a thousand relationships used to
    /// take half a minute; the scan is now one pass over the markup.
    #[test]
    fn a_document_with_many_relationships_opens_in_the_time_its_part_takes() {
        let rels: Vec<(String, String)> = (0..1024).map(|i| (format!("rIdT{i}"), format!("file{i}.bin"))).collect();
        let rels: Vec<(&str, &str)> = rels.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let body = format!(r#"<w:p><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, "a".repeat(8 << 20));
        let bytes = package(&body, &rels, &[]);
        let t = std::time::Instant::now();
        let doc = crate::read(&bytes).unwrap();
        let took = t.elapsed();
        assert_eq!(doc.plain_text(wordcraft_doc::StoryRef::Body).len(), 8 << 20, "the text is there");
        assert!(took.as_secs() < 15, "opening it took {took:?}");
    }

    /// The scan finds the relationships the markup names, so a part the document already refers
    /// to is not carried over a second time (#12).
    #[test]
    fn a_relationship_the_markup_names_is_not_carried_over_again() {
        // The markup names the relationship, so it is not carried over a second time as an
        // unmodelled part. (The part itself survives only if the writer writes it with the
        // element that names it; the reader does not carry it on its own.)
        let named = package(
            r#"<w:p><w:r><w:t>before</w:t></w:r></w:p><w:sectPr><w:printerSettings r:id="rIdDoc1"/></w:sectPr>"#,
            &[("rIdDoc1", "keep.bin")],
            &[("word/keep.bin", b"kept")],
        );
        let roots = unmodelled_roots(&crate::read(&named).unwrap());
        assert!(!roots.contains("keep.bin"), "the part is carried twice: {roots}");

        // A part the markup doesn't name is carried over, as before.
        let unnamed = package(r#"<w:p><w:r><w:t>no mention here</w:t></w:r></w:p>"#, &[("rIdDoc1", "keep.bin")], &[("word/keep.bin", b"kept")]);
        let roots = unmodelled_roots(&crate::read(&unnamed).unwrap());
        assert!(roots.contains("keep.bin"), "the part is dropped: {roots}");
    }

    /// An id the markup only mentions as text does not name a part: the part is carried over,
    /// not dropped because the text looked like a reference.
    #[test]
    fn a_part_the_markup_only_mentions_is_carried_over() {
        let mentioned = package(
            r#"<w:p><w:r><w:t xml:space="preserve">before "rIdDoc1" after</w:t></w:r></w:p>"#,
            &[("rIdDoc1", "keep.bin")],
            &[("word/keep.bin", b"kept")],
        );
        let roots = unmodelled_roots(&crate::read(&mentioned).unwrap());
        assert!(roots.contains("keep.bin"), "the part is dropped: {roots}");
    }

    /// An unqualified attribute is a relationship only where the markup names a relationship
    /// that way: a hyperlink's `id` names the part, a VML shape's `id` is its own.
    #[test]
    fn an_unqualified_attribute_names_a_relationship_only_where_it_is_one() {
        // A hyperlink whose relationship id is written without the prefix: the reader resolves
        // it (the alias `El::attr` accepts), so the part it names is not carried over twice.
        let named = package(
            r#"<w:p><w:hyperlink id="rIdDoc1"><w:r><w:t>link</w:t></w:r></w:hyperlink></w:p>"#,
            &[("rIdDoc1", "keep.bin")],
            &[("word/keep.bin", b"kept")],
        );
        let roots = unmodelled_roots(&crate::read(&named).unwrap());
        assert!(!roots.contains("keep.bin"), "the part is carried twice: {roots}");

        // A VML shape's `id` is its own: it names no part, so the part is carried over.
        let own = package(
            r#"<w:p><w:r><w:pict><v:shape id="rIdDoc1" style="width:10pt;height:10pt"/></w:pict></w:r></w:p>"#,
            &[("rIdDoc1", "keep.bin")],
            &[("word/keep.bin", b"kept")],
        );
        let roots = unmodelled_roots(&crate::read(&own).unwrap());
        assert!(roots.contains("keep.bin"), "the part is dropped: {roots}");
    }
}

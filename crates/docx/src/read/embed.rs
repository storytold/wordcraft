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

use std::sync::Arc;

use wordcraft_doc::graphic::{Embedded, EmbeddedRef};

use super::Reader;
use crate::package::{
    CUSTOM_XML_ROOTS, ContentTypes, MAX_EMBEDDED_PARTS, MAX_EMBEDDED_RELS, MAX_EMBEDDED_XML, Rel, Rels, manifest_field_ok, rel_is, rt,
};
use crate::xml::{El, MAX_DEPTH, NAMESPACES, esc};

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
    pub(super) fn keep_unmodelled(&mut self, package: &Rels, document: &Rels) {
        let main = self.pkg.get(&self.main).map(|b| b.to_vec()).unwrap_or_default();
        let referenced = |id: &str| {
            let needle = format!("\"{id}\"");
            main.windows(needle.len()).any(|w| w == needle.as_bytes())
        };
        let mut roots = String::new();
        // One glossary per main document (its first relationship; see `read_glossary`).
        let mut glossary = false;
        for (owner, rels) in [("P", package), ("D", document)] {
            for rel in rels.list.iter().filter(|r| !r.external).take(MAX_EMBEDDED_RELS) {
                let tail = rel.kind.rsplit('/').next().unwrap_or("");
                if HANDLED_RELS.contains(&tail)
                    || (owner == "D" && referenced(&rel.id))
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
    /// itself can't be kept.
    pub(super) fn keep_part_tree(&mut self, root: &str) -> bool {
        if self.embedded.parts.contains_key(root) {
            return true;
        }
        if !self.keep_part(root) {
            return false;
        }
        let mut todo = vec![root.to_string()];
        while let Some(part) = todo.pop() {
            let mut kept: Vec<Rel> = Vec::new();
            for r in self.pkg.rels(&part).list {
                if kept.len() >= MAX_EMBEDDED_RELS {
                    log::warn!("docx: part {part} has more than {MAX_EMBEDDED_RELS} relationships; the rest are dropped");
                    break;
                }
                if ![&r.id, &r.kind, &r.target].iter().all(|f| manifest_field_ok(f)) || kept.iter().any(|k| k.id == r.id) {
                    continue;
                }
                if !r.external && !self.embedded.parts.contains_key(&r.target) {
                    if !self.keep_part(&r.target) {
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

    /// Keep one part's bytes and content type (not its relationships).
    fn keep_part(&mut self, part: &str) -> bool {
        let lower = part.to_ascii_lowercase();
        if !manifest_field_ok(part) || lower.ends_with(".rels") || lower == "[content_types].xml" || part.eq_ignore_ascii_case(&self.main) {
            log::warn!("docx: not carrying part {part:?} for an embedded object");
            return false;
        }
        if self.embedded.parts.len() >= MAX_EMBEDDED_PARTS {
            log::warn!("docx: more than {MAX_EMBEDDED_PARTS} parts for embedded objects; the rest are dropped");
            return false;
        }
        let Some(bytes) = self.pkg.get(part) else {
            log::warn!("docx: part {part} of an embedded object is missing");
            return false;
        };
        let pkg = self.pkg;
        let ct = self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(part).unwrap_or("application/octet-stream");
        let content_type = if manifest_field_ok(ct) { ct.to_string() } else { "application/octet-stream".to_string() };
        self.doc.passthrough.entry(part.to_string()).or_insert_with(|| Arc::new(bytes.to_vec()));
        self.embedded.parts.insert(part.to_string(), crate::package::EmbeddedPart { content_type, rels: Vec::new() });
        true
    }
}

/// The relationship ids `e` uses (attributes in the relationships namespace, and VML's
/// `o:relid`), in document order. `None` past [`MAX_IDS`].
fn rel_ids(e: &El, out: &mut Vec<String>, depth: usize) -> Option<()> {
    if depth > MAX_DEPTH {
        return Some(());
    }
    for (k, v) in &e.attrs {
        if (k.starts_with("r:") || k == "o:relid") && !out.contains(v) {
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

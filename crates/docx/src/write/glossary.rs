//! Writing the glossary document (ECMA-376 Part 1 §17.12) from `Document::glossary`, once its
//! entries differ from the file's (an untouched glossary is written back verbatim: see
//! `read::glossary`).

use wordcraft_doc::{Document, QUICK_PARTS_GALLERY};

use super::{CT_WML, PartRels, Writer, numbering_xml, push_part, styles_xml};
use crate::package::{GLOSSARY_BEHAVIORS, GLOSSARY_CONTENT_TYPE, GLOSSARY_READ, GLOSSARY_TYPES, glossary_fingerprint, rt};
use crate::xml::{self, W};

/// `values` known to `known`, as `parent` with one `tag` child each (nothing when none are).
fn listed(w: &mut W, parent: &str, tag: &str, values: &[String], known: &[&str]) {
    let mut values = values.iter().filter(|v| known.contains(&v.as_str())).peekable();
    if values.peek().is_none() {
        return;
    }
    w.open(parent, &[]);
    for v in values {
        w.empty(tag, &[("w:val", v)]);
    }
    w.close(parent);
}

/// Whether the glossary differs from the one read from the file (none read: an empty one).
pub(super) fn edited(doc: &Document) -> bool {
    let print = glossary_fingerprint(&doc.glossary);
    match doc.passthrough.get(GLOSSARY_READ) {
        Some(read) => read.as_slice() != print.as_bytes(),
        None => print != glossary_fingerprint(&[]),
    }
}

impl Writer<'_> {
    /// Write the glossary part from the model, with a styles part of its own (the document's
    /// styles), related from the main document's `rels`. Its parts live in `word/`, beside the
    /// main document, so the relationships its stories add resolve as theirs do.
    pub(super) fn glossary(&mut self, entries: &mut Vec<(String, Vec<u8>)>, overrides: &mut Vec<(String, String)>, rels: &mut PartRels) {
        let doc = self.doc;
        if doc.glossary.is_empty() {
            return;
        }
        let names = self.embeds.fresh_set(&["word/glossary", "word/glossaryStyles", "word/glossaryNumbering"], ".xml");
        let [part, styles, numbering] = names.as_slice() else { return };
        let ns = xml::body_ns();
        let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let mut prels = PartRels::default();
        let mut w = W::new();
        w.open("w:glossaryDocument", &ns_refs);
        w.open("w:docParts", &[]);
        for e in &doc.glossary {
            w.open("w:docPart", &[]);
            w.open("w:docPartPr", &[]);
            w.empty("w:name", &[("w:val", &e.name)]);
            if !e.style.is_empty() {
                w.empty("w:style", &[("w:val", &e.style)]);
            }
            w.open("w:category", &[]);
            w.empty("w:name", &[("w:val", if e.category.is_empty() { "General" } else { &e.category })]);
            w.empty("w:gallery", &[("w:val", if e.gallery.is_empty() { QUICK_PARTS_GALLERY } else { &e.gallery })]);
            w.close("w:category");
            listed(&mut w, "w:types", "w:type", &e.types, &GLOSSARY_TYPES);
            listed(&mut w, "w:behaviors", "w:behavior", &e.behaviors, &GLOSSARY_BEHAVIORS);
            if !e.description.is_empty() {
                w.empty("w:description", &[("w:val", &e.description)]);
            }
            if !e.guid.is_empty() {
                w.empty("w:guid", &[("w:val", &e.guid)]);
            }
            w.close("w:docPartPr");
            w.open("w:docPartBody", &[]);
            if e.blocks.is_empty() {
                w.empty("w:p", &[]);
            } else {
                self.blocks(&mut w, &e.blocks, &mut prels, false, 0);
            }
            w.close("w:docPartBody");
            w.close("w:docPart");
        }
        w.close("w:docParts");
        w.close("w:glossaryDocument");
        let file = |p: &str| p.strip_prefix("word/").unwrap_or(p).to_string();
        prels.add(rt::STYLES, &file(styles), false);
        // Lists in entries need their definitions within the glossary's reach.
        let lists = !doc.numbering.nums.is_empty() || !doc.numbering.abstracts.is_empty();
        if lists {
            prels.add(rt::NUMBERING, &file(numbering), false);
        }
        rels.add(rt::GLOSSARY, &file(part), false);
        push_part(entries, overrides, part, w.into_bytes(), GLOSSARY_CONTENT_TYPE, prels);
        push_part(entries, overrides, styles, styles_xml(doc), &format!("{CT_WML}styles+xml"), PartRels::default());
        if lists {
            push_part(entries, overrides, numbering, numbering_xml(doc), &format!("{CT_WML}numbering+xml"), PartRels::default());
        }
    }
}

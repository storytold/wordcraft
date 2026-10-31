//! The glossary document (ECMA-376 Part 1 §17.12): the document's own building blocks (Quick
//! Parts, AutoText…), read into `Document::glossary`.
//!
//! The glossary part and everything it reaches are also kept verbatim (`keep_unmodelled`), with a
//! fingerprint of the entries as read ([`GLOSSARY_READ`]): a save writes the kept part while the
//! entries are unchanged, so whatever the model doesn't hold survives, and writes the glossary from
//! the model once they change.

use std::sync::Arc;

use wordcraft_doc::{Blocks, BuildingBlock};

use super::Reader;
use super::story::StoryCtx;
use crate::package::{
    ContentTypes, GLOSSARY_BEHAVIORS, GLOSSARY_CONTENT_TYPE, GLOSSARY_READ, GLOSSARY_TYPES, MAX_GLOSSARY_ENTRIES, Rels, glossary_fingerprint, rel_is,
    rt,
};
use crate::xml::El;

/// Longest name, gallery, category, description, guid or behaviour value kept (characters).
const MAX_FIELD: usize = 1024;

/// The known values of `parent`'s `tag` children, each once, in file order.
fn listed(parent: Option<&El>, tag: &str, known: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in parent.into_iter().flat_map(|p| p.children(tag)).filter_map(|e| e.attr("w:val")) {
        if known.contains(&v) && !out.iter().any(|o| o == v) {
            out.push(v.to_string());
        }
    }
    out
}

impl Reader<'_> {
    /// Read the glossary the main document relates to (`document`: its relationships). The first
    /// internal glossary relationship counts; one pointing at the main document, a part that isn't a
    /// glossary, or a missing one gives no entries. Entries without a name or a body are skipped.
    pub(super) fn read_glossary(&mut self, document: &Rels) {
        // The first internal one, as `keep_unmodelled` keeps.
        let Some(rel) = document.list.iter().find(|r| !r.external && rel_is(&r.kind, rt::GLOSSARY)) else { return };
        let mut entries = Vec::new();
        if !rel.target.eq_ignore_ascii_case(&self.main) {
            let pkg = self.pkg;
            let ct = self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(&rel.target).map(str::to_string);
            let root = match self.xml(&rel.target) {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("docx: ignoring unreadable glossary {}: {e}", rel.target);
                    None
                }
            };
            // Without content types (a converted package) the root element alone decides.
            if let Some(root) = root.filter(|r| r.name == "w:glossaryDocument" && ct.as_deref().is_none_or(|t| t == GLOSSARY_CONTENT_TYPE)) {
                let rels = self.pkg.rels(&rel.target);
                entries = self.doc_parts(&root, &rels);
            }
        }
        self.doc.glossary = entries;
        let print = glossary_fingerprint(&self.doc.glossary);
        self.doc.passthrough.insert(GLOSSARY_READ.into(), Arc::new(print.into_bytes()));
    }

    /// The building blocks of a `w:glossaryDocument`, read with the glossary's relationships.
    fn doc_parts(&mut self, root: &El, rels: &Rels) -> Vec<BuildingBlock> {
        // The glossary's notes, comments, bookmarks, permissions and moves are its own: what the
        // main document's ids mean doesn't apply to it, and its ids mustn't leak into the body's.
        let footnotes = std::mem::take(&mut self.footnotes);
        let endnotes = std::mem::take(&mut self.endnotes);
        let comments = std::mem::take(&mut self.comment_map);
        let ended = std::mem::take(&mut self.comments_ended);
        let bookmarks = std::mem::take(&mut self.bookmarks);
        let perms = std::mem::take(&mut self.perms);
        let moves = std::mem::take(&mut self.moves);
        let mut out = Vec::new();
        for part in root.child("w:docParts").into_iter().flat_map(|d| d.children("w:docPart")) {
            if out.len() >= MAX_GLOSSARY_ENTRIES {
                log::warn!("docx: glossary has more than {MAX_GLOSSARY_ENTRIES} entries; the rest are dropped");
                break;
            }
            let pr = part.child("w:docPartPr");
            let val = |e: Option<&El>| e.and_then(|e| e.attr("w:val")).unwrap_or("").chars().take(MAX_FIELD).collect::<String>();
            let name = val(pr.and_then(|p| p.child("w:name")));
            let Some(body) = part.child("w:docPartBody").filter(|_| !name.is_empty()) else { continue };
            let category = pr.and_then(|p| p.child("w:category"));
            let mut sc = StoryCtx::default();
            let mut blocks = Blocks::new();
            self.read_blocks(&mut sc, body, rels, &mut blocks, 0);
            self.flush_pending(&mut sc, &mut blocks);
            if blocks.is_empty() {
                continue;
            }
            out.push(BuildingBlock {
                name,
                gallery: val(category.and_then(|c| c.child("w:gallery"))),
                category: val(category.and_then(|c| c.child("w:name"))),
                description: val(pr.and_then(|p| p.child("w:description"))),
                guid: val(pr.and_then(|p| p.child("w:guid"))),
                style: val(pr.and_then(|p| p.child("w:style"))),
                types: listed(pr.and_then(|p| p.child("w:types")), "w:type", &GLOSSARY_TYPES),
                behaviors: listed(pr.and_then(|p| p.child("w:behaviors")), "w:behavior", &GLOSSARY_BEHAVIORS),
                blocks,
            });
        }
        self.footnotes = footnotes;
        self.endnotes = endnotes;
        self.comment_map = comments;
        self.comments_ended = ended;
        self.bookmarks = bookmarks;
        self.perms = perms;
        self.moves = moves;
        out
    }
}

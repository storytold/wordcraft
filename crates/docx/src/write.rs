//! [`Document`] → DOCX.

mod bidi;
mod embed;
mod glossary;
mod settings_merge;
pub(crate) use settings_merge::has_unowned as settings_has_unowned;
mod math;
mod props;
mod sdt;
mod story;
pub(crate) use story::written_toc_span;
mod watermark;
mod wordart;

use std::collections::{BTreeMap, HashMap};

use wordcraft_doc::ParaProps;
use wordcraft_doc::numbering::{Level, LevelSuffix};
use wordcraft_doc::props::VAlign;
use wordcraft_doc::styles::{Style, StyleKind, TablePart};
use wordcraft_doc::{Blocks, Document, PartKind};

use crate::package::{MAX_VBA_RELATED, VBA_PROJECT_PART, VBA_RELATED, rt, zip_entries};
use crate::xml::{self, W};
use crate::{DocxError, Flavor};

mod bibliography;
mod parts;
mod settings;
mod styles;

pub(crate) use bibliography::from_source_type;
pub(crate) use parts::write_parts;
use parts::{content_type_for, push_part, sniff};
use settings::note_id_base;
pub(crate) use settings::{ANONYMOUS_AUTHOR, ANONYMOUS_INITIALS, annotation_author, durable_id_ok};
use styles::{numbering_xml, styles_xml};

/// Relationships of one part.
#[derive(Default)]
pub struct PartRels {
    list: Vec<(String, String, String, bool)>,
    index: HashMap<(String, String, bool), String>,
}

impl PartRels {
    /// Add (or reuse) a relationship; returns its id.
    pub fn add(&mut self, kind: &str, target: &str, external: bool) -> String {
        let key = (kind.to_string(), target.to_string(), external);
        if let Some(id) = self.index.get(&key) {
            return id.clone();
        }
        let id = format!("rId{}", self.list.len() + 1);
        self.list.push((id.clone(), kind.to_string(), target.to_string(), external));
        self.index.insert(key, id.clone());
        id
    }
    /// Add a relationship with a given id (the id a kept part's markup uses); a repeated id is
    /// ignored. Not to be mixed with [`PartRels::add`] in one part.
    pub fn add_with_id(&mut self, id: &str, kind: &str, target: &str, external: bool) {
        if self.list.iter().any(|(i, ..)| i == id) {
            return;
        }
        self.list.push((id.to_string(), kind.to_string(), target.to_string(), external));
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
    pub(crate) fn xml(&self) -> Vec<u8> {
        let mut w = W::new();
        w.open("Relationships", &[("xmlns", "http://schemas.openxmlformats.org/package/2006/relationships")]);
        for (id, kind, target, external) in &self.list {
            if *external {
                w.empty("Relationship", &[("Id", id), ("Type", kind), ("Target", target), ("TargetMode", "External")]);
            } else {
                w.empty("Relationship", &[("Id", id), ("Type", kind), ("Target", target)]);
            }
        }
        w.close("Relationships");
        w.into_bytes()
    }
}

pub(crate) struct Writer<'d> {
    doc: &'d Document,
    /// Numbered display equations written so far (automatic numbers are written as text).
    pub(crate) eq_number: u32,
    /// Media key → file name under `word/media/`.
    media_files: BTreeMap<String, String>,
    bookmarks: HashMap<String, u32>,
    next_rev_id: u32,
    /// Move ranges open in the output: (revision, range `w:id`, destination?).
    open_moves: Vec<(u32, String, bool)>,
    docpr: u32,
    /// Shapes' own drawing ids written so far (connectors refer to them).
    shape_ids: std::collections::HashSet<u32>,
    z: u32,
    /// Note part id → file note id, in first-reference order.
    footnotes: Vec<u32>,
    endnotes: Vec<u32>,
    /// Header/footer part id → (file name, is footer).
    hf: Vec<(u32, String, bool)>,
    /// The headers each slot has so far in body order, with sections' empty slots linked to the
    /// ones before (as layout shows them): where a watermark needs a header of its own.
    linked_headers: wordcraft_doc::section::HeaderSet,
    /// The part id standing for a header written only to hold the watermark (no such part).
    wm_header: u32,
    /// The watermark shape to put at the start of the next top-level paragraph (a header's first).
    pending_watermark: Option<String>,
    /// Paragraphs that need a `w14:paraId` (last paragraph of each comment), by address.
    para_ids: HashMap<usize, String>,
    /// A note reference mark to put at the start of the next paragraph.
    pending_mark: Option<&'static str>,
    /// The note being written (is footnote, part id): its reference to itself is the mark above.
    current_note: Option<(bool, u32)>,
    /// Writing a TOC heading: its TOC field is held back so the entries become the field's result.
    toc_hold_end: bool,
    /// The held TOC field (instruction, locked, props), opened in the first entry as Word does.
    toc_field: Option<(String, bool, bool, wordcraft_doc::props::CharProps)>,
    /// Open the held TOC field at the start of the paragraph being written.
    toc_begin_here: bool,
    /// Close the open TOC field at the end of the paragraph being written.
    toc_end_here: bool,
    /// Byte offsets of the content control markers the next paragraph writes inline.
    inline_controls: Vec<usize>,
    /// Media keys actually referenced by a written drawing.
    used_media: std::collections::BTreeSet<String>,
    /// Bounds writing text boxes inside text boxes (as layout shows them).
    boxes: wordcraft_doc::BoxBudget,
    /// Text box stories already written: a story shown by many shapes is written once, the other
    /// shapes get an empty box, so the file doesn't grow as shapes × story size.
    written_boxes: std::collections::HashSet<u32>,
    /// Charts, diagrams and OLE objects kept from a file: the parts they need.
    embeds: embed::EmbedWriter<'d>,
    /// Linked text boxes: part id → (`wps:txbx`/`wps:linkedTxbx` id, seq; 0 for the box holding
    /// the text).
    txbx_links: HashMap<u32, (u32, u32)>,
}

const CT_WML: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.";

/// Write a `.docx` package.
pub fn write(doc: &Document) -> Result<Vec<u8>, DocxError> {
    write_as(doc, Flavor::Document)
}

/// Write a package of the given flavour (`.docx`, `.docm`, `.dotx`, `.dotm`).
pub fn write_as(doc: &Document, flavor: Flavor) -> Result<Vec<u8>, DocxError> {
    zip_entries(&write_parts(doc, flavor)?)
}

impl Writer<'_> {
    fn assign_media(&mut self) {
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (key, bytes) in &self.doc.media {
            let clean: String = key.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).take(80).collect();
            let has_known_ext = clean.rsplit_once('.').is_some_and(|(s, _)| !s.is_empty()) && content_type_for(&clean) != "application/octet-stream";
            let mut base = if has_known_ext && !clean.starts_with('.') {
                clean
            } else {
                let stem = clean.trim_matches('.');
                let stem = if stem.is_empty() { "image" } else { stem };
                format!("{stem}.{}", sniff(bytes))
            };
            let (stem, ext) = base.rsplit_once('.').map(|(s, e)| (s.to_string(), e.to_string())).unwrap_or((base.clone(), "bin".into()));
            let mut i = 1;
            while used.contains(&base.to_ascii_lowercase()) {
                i += 1;
                base = format!("{stem}_{i}.{ext}");
            }
            used.insert(base.to_ascii_lowercase());
            self.media_files.insert(key.clone(), base);
        }
    }

    fn next_docpr(&mut self) -> String {
        self.docpr = self.docpr.saturating_add(1);
        self.docpr.to_string()
    }

    /// The drawing id of a shape: its own (once; a copy gets a fresh one), else a fresh one.
    fn shape_docpr(&mut self, own: u32) -> String {
        if own != 0 && self.shape_ids.insert(own) { own.to_string() } else { self.next_docpr() }
    }

    fn bookmark_id(&mut self, name: &str) -> String {
        let n = self.bookmarks.len() as u32;
        self.bookmarks.entry(name.to_string()).or_insert(n).to_string()
    }

    fn note_id(&mut self, part: u32, foot: bool) -> String {
        let base = note_id_base(self.doc, foot);
        let list = if foot { &mut self.footnotes } else { &mut self.endnotes };
        let idx = match list.iter().position(|p| *p == part) {
            Some(i) => i,
            None => {
                list.push(part);
                list.len() - 1
            }
        };
        (idx + base).to_string()
    }

    fn hf_rel(&mut self, part: u32, footer: bool, rels: &mut PartRels) -> Option<String> {
        let kind = if part == self.wm_header && !footer { PartKind::Header } else { self.doc.parts.get(&part)?.kind };
        if !matches!(kind, PartKind::Header | PartKind::Footer) {
            return None;
        }
        let file = match self.hf.iter().find(|(p, _, f)| *p == part && *f == footer) {
            Some((_, f, _)) => f.clone(),
            None => {
                let n = self.hf.iter().filter(|(_, _, f)| *f == footer).count() + 1;
                let f = format!("{}{n}.xml", if footer { "footer" } else { "header" });
                self.hf.push((part, f.clone(), footer));
                f
            }
        };
        Some(rels.add(if footer { rt::FOOTER } else { rt::HEADER }, &file, false))
    }

    fn take_para_id(&self, p: &wordcraft_doc::Paragraph) -> Option<String> {
        self.para_ids.get(&(p as *const _ as usize)).cloned()
    }

    /// Whether `p` holds a reference to the note being written.
    fn holds_own_ref(&self, p: &wordcraft_doc::Paragraph) -> bool {
        let Some((foot, id)) = self.current_note else { return false };
        p.objects.iter().any(|o| match o {
            wordcraft_doc::InlineObject::NoteRef { kind, id: nid, .. } => *nid == id && (*kind == wordcraft_doc::para::NoteKind::Footnote) == foot,
            _ => false,
        })
    }

    /// Note stories: the first paragraph starts with the note's own reference mark.
    fn note_blocks(&mut self, w: &mut W, blocks: &Blocks, rels: &mut PartRels, foot: bool, part: u32) {
        self.pending_mark = Some(if foot { "w:footnoteRef" } else { "w:endnoteRef" });
        self.current_note = Some((foot, part));
        self.blocks(w, blocks, rels, false, 0);
        self.current_note = None;
        self.pending_mark = None;
    }
}

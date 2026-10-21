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

/// The parts of the package [`write_as`] zips, in order, `[Content_Types].xml` first: (part name
/// without the leading `/`, bytes).
pub(crate) fn write_parts(doc: &Document, flavor: Flavor) -> Result<Vec<(String, Vec<u8>)>, DocxError> {
    // A range field must be whole in the file: drop markers that lost their partner in editing.
    let balanced;
    let doc = if doc.has_unbalanced_field_ranges() {
        let mut d = doc.clone();
        d.balance_field_ranges();
        balanced = d;
        &balanced
    } else {
        doc
    };
    let mut wr = Writer {
        eq_number: 0,
        doc,
        media_files: BTreeMap::new(),
        bookmarks: HashMap::new(),
        next_rev_id: 0,
        open_moves: Vec::new(),
        // Fresh drawing ids start past the shapes' own (connectors refer to those).
        docpr: doc.next_shape_id().saturating_sub(1),
        shape_ids: std::collections::HashSet::new(),
        z: 251_658_240,
        footnotes: Vec::new(),
        endnotes: Vec::new(),
        hf: Vec::new(),
        linked_headers: Default::default(),
        wm_header: (0..=u32::MAX).rev().find(|id| !doc.parts.contains_key(id)).unwrap_or(u32::MAX),
        pending_watermark: None,
        para_ids: HashMap::new(),
        pending_mark: None,
        current_note: None,
        toc_hold_end: false,
        toc_field: None,
        toc_begin_here: false,
        toc_end_here: false,
        inline_controls: Vec::new(),
        used_media: Default::default(),
        boxes: wordcraft_doc::BoxBudget::default(),
        embeds: embed::EmbedWriter::new(doc),
        txbx_links: doc
            .text_box_chains()
            .iter()
            .enumerate()
            .flat_map(|(n, c)| c.iter().enumerate().map(move |(seq, id)| (*id, (n as u32 + 1, seq as u32))))
            .collect(),
    };
    wr.assign_media();
    wr.embeds.reserve_media(wr.media_files.values());
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut overrides: Vec<(String, String)> = Vec::new();
    let mut rels = PartRels::default();
    let mut root = PartRels::default();

    // Comments get paragraph ids before any story is written.
    let mut comment_paras: Vec<(u32, String)> = Vec::new();
    for (i, (cid, c)) in doc.comments.iter().enumerate() {
        if let Some(part) = doc.parts.get(&c.part)
            && let Some(last) = part.blocks.iter().rev().find_map(|b| b.as_para())
        {
            let pid = format!("{:08X}", 0x1000_0000u32 + i as u32 + 1);
            wr.para_ids.insert(last as *const _ as usize, pid.clone());
            comment_paras.push((*cid, pid));
        }
    }

    // Body.
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:document", &ns_refs);
    let pic = doc.settings.page_picture.as_ref().filter(|p| wr.media_files.contains_key(&p.media) && doc.media.contains_key(&p.media));
    if pic.is_some() || doc.settings.page_color.is_some() {
        let color = doc.settings.page_color.map(|c| c.hex());
        let attrs: Vec<(&str, &str)> = color.iter().map(|c| ("w:color", c.as_str())).collect();
        w.open("w:background", &attrs);
        if let Some(p) = pic
            && let Some(file) = wr.media_files.get(&p.media)
        {
            let rid = rels.add(rt::IMAGE, &format!("media/{file}"), false);
            wr.used_media.insert(p.media.clone());
            w.open("v:background", &[("id", "_x0000_s1025"), ("o:bwmode", "white")]);
            let mut fa: Vec<(&str, &str)> = vec![("r:id", &rid)];
            if !p.fill_type.is_empty() {
                fa.push(("type", &p.fill_type));
            }
            if !p.title.is_empty() {
                fa.push(("o:title", &p.title));
            }
            w.empty("v:fill", &fa);
            w.close("v:background");
        }
        w.close("w:background");
    }
    w.open("w:body", &[]);
    wr.blocks(&mut w, &doc.body, &mut rels, true, 0);
    let last = doc.last_section.clone();
    wr.sectpr(&mut w, &last, &mut rels);
    w.close("w:body");
    w.close("w:document");
    let body = w.into_bytes();

    // Headers and footers (discovered while writing sections).
    let mut done = 0;
    while let Some((id, file, footer)) = wr.hf.get(done).cloned() {
        done += 1;
        let mut prels = PartRels::default();
        let mut w = W::new();
        let tag = if footer { "w:ftr" } else { "w:hdr" };
        w.open(tag, &ns_refs);
        let blocks: Blocks = doc.parts.get(&id).map(|p| p.blocks.clone()).unwrap_or_default();
        // The watermark goes in every header, at the start of its first paragraph.
        wr.pending_watermark = match &doc.settings.watermark {
            Some(wm) if !footer => watermark::run(wm, done, &doc.last_section),
            _ => None,
        };
        wr.blocks(&mut w, &blocks, &mut prels, false, 0);
        // Always taken: the story ends with a top-level paragraph.
        wr.pending_watermark = None;
        w.close(tag);
        push_part(
            &mut entries,
            &mut overrides,
            &format!("word/{file}"),
            w.into_bytes(),
            &format!("{CT_WML}{}+xml", if footer { "footer" } else { "header" }),
            prels,
        );
    }

    // The glossary: kept verbatim while its entries are as read and its parts were kept, else
    // written from the model (before the notes: references its entries hold are collected too).
    if glossary::edited(doc) || (!doc.glossary.is_empty() && !wr.embeds.kept_glossary()) {
        wr.embeds.drop_glossary = true;
        wr.glossary(&mut entries, &mut overrides, &mut rels);
    }

    // Notes (references were collected from the body and headers).
    for foot in [true, false] {
        let seps = if foot { &doc.footnote_separators } else { &doc.endnote_separators };
        if !notes_part(doc, &wr, foot) {
            continue;
        }
        let (root, tag, file) = if foot { ("w:footnotes", "w:footnote", "footnotes.xml") } else { ("w:endnotes", "w:endnote", "endnotes.xml") };
        let mut prels = PartRels::default();
        let mut w = W::new();
        w.open(root, &ns_refs);
        // The separators and continuation notice: the document's own, else the defaults (no
        // notice). Word numbers them -1, 0 and 1.
        for (id, kind, own) in [
            ("-1", "separator", &seps.separator),
            ("0", "continuationSeparator", &seps.continuation_separator),
            ("1", "continuationNotice", &seps.continuation_notice),
        ] {
            match own {
                Some(blocks) => {
                    w.open(tag, &[("w:type", kind), ("w:id", id)]);
                    if blocks.is_empty() {
                        w.empty("w:p", &[]);
                    } else {
                        wr.blocks(&mut w, blocks, &mut prels, false, 0);
                    }
                    w.close(tag);
                }
                None if kind != "continuationNotice" => {
                    w.open(tag, &[("w:type", kind), ("w:id", id)]);
                    w.open("w:p", &[]);
                    w.open("w:pPr", &[]);
                    w.empty("w:spacing", &[("w:after", "0"), ("w:line", "240"), ("w:lineRule", "auto")]);
                    w.close("w:pPr");
                    w.open("w:r", &[]);
                    w.empty(if kind == "separator" { "w:separator" } else { "w:continuationSeparator" }, &[]);
                    w.close("w:r");
                    w.close("w:p");
                    w.close(tag);
                }
                None => {}
            }
        }
        let base = note_id_base(doc, foot);
        let mut k = 0;
        while let Some(pid) = (if foot { &wr.footnotes } else { &wr.endnotes }).get(k).copied() {
            let nid = (k + base).to_string();
            k += 1;
            w.open(tag, &[("w:id", &nid)]);
            let blocks: Blocks = doc.parts.get(&pid).map(|p| p.blocks.clone()).unwrap_or_default();
            wr.note_blocks(&mut w, &blocks, &mut prels, foot, pid);
            w.close(tag);
        }
        w.close(root);
        let kind = if foot { rt::FOOTNOTES } else { rt::ENDNOTES };
        rels.add(kind, file, false);
        push_part(
            &mut entries,
            &mut overrides,
            &format!("word/{file}"),
            w.into_bytes(),
            &format!("{CT_WML}{}+xml", if foot { "footnotes" } else { "endnotes" }),
            prels,
        );
    }

    // Comments.
    if !doc.comments.is_empty() {
        let mut prels = PartRels::default();
        let mut w = W::new();
        w.open("w:comments", &ns_refs);
        for (cid, c) in &doc.comments {
            let id = cid.to_string();
            let author = annotation_author(doc, &c.author);
            let mut a = vec![("w:id", id.as_str()), ("w:author", author)];
            if !c.date.is_empty() && !doc.settings.remove_date_time {
                a.push(("w:date", c.date.as_str()));
            }
            if doc.settings.remove_personal_info {
                a.push(("w:initials", ANONYMOUS_INITIALS));
            } else if !c.initials.is_empty() {
                a.push(("w:initials", c.initials.as_str()));
            }
            w.open("w:comment", &a);
            let blocks: Blocks = doc.parts.get(&c.part).map(|p| p.blocks.clone()).unwrap_or_default();
            wr.blocks(&mut w, &blocks, &mut prels, false, 0);
            w.close("w:comment");
        }
        w.close("w:comments");
        rels.add(rt::COMMENTS, "comments.xml", false);
        push_part(&mut entries, &mut overrides, "word/comments.xml", w.into_bytes(), &format!("{CT_WML}comments+xml"), prels);

        let mut w = W::new();
        w.open(
            "w15:commentsEx",
            &[
                ("xmlns:w15", "http://schemas.microsoft.com/office/word/2012/wordml"),
                ("xmlns:mc", "http://schemas.openxmlformats.org/markup-compatibility/2006"),
            ],
        );
        let pid_of = |cid: u32| comment_paras.iter().find(|(c, _)| *c == cid).map(|(_, p)| p.clone());
        for (cid, c) in &doc.comments {
            let Some(pid) = pid_of(*cid) else { continue };
            let done = if c.resolved { "1" } else { "0" };
            match c.parent.and_then(pid_of) {
                Some(parent) => w.empty("w15:commentEx", &[("w15:paraId", &pid), ("w15:paraIdParent", &parent), ("w15:done", done)]),
                None => w.empty("w15:commentEx", &[("w15:paraId", &pid), ("w15:done", done)]),
            }
        }
        w.close("w15:commentsEx");
        rels.add(rt::COMMENTS_EX, "commentsExtended.xml", false);
        push_part(
            &mut entries,
            &mut overrides,
            "word/commentsExtended.xml",
            w.into_bytes(),
            &format!("{CT_WML}commentsExtended+xml"),
            PartRels::default(),
        );
        write_comment_ids(doc, &comment_paras, &mut entries, &mut overrides, &mut rels);
    }

    rels.add(rt::STYLES, "styles.xml", false);
    push_part(&mut entries, &mut overrides, "word/styles.xml", styles_xml(doc), &format!("{CT_WML}styles+xml"), PartRels::default());
    if !doc.numbering.nums.is_empty() || !doc.numbering.abstracts.is_empty() {
        rels.add(rt::NUMBERING, "numbering.xml", false);
        push_part(&mut entries, &mut overrides, "word/numbering.xml", numbering_xml(doc), &format!("{CT_WML}numbering+xml"), PartRels::default());
    }
    rels.add(rt::SETTINGS, "settings.xml", false);
    // The attached template is an external relationship of the settings part (§17.15.1.6).
    let mut settings_rels = PartRels::default();
    let template = doc.settings.attached_template.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let template_rel = template.map(|t| settings_rels.add(rt::ATTACHED_TEMPLATE, t, true));
    push_part(
        &mut entries,
        &mut overrides,
        "word/settings.xml",
        {
            let generated = settings_xml(doc, notes_part(doc, &wr, true), notes_part(doc, &wr, false), template_rel.as_deref());
            match doc.passthrough.get(crate::package::SETTINGS_ORIG) {
                Some(orig) => settings_merge::merge(orig, &generated).unwrap_or(generated),
                None => generated,
            }
        },
        &format!("{CT_WML}settings+xml"),
        settings_rels,
    );
    rels.add(rt::THEME, "theme/theme1.xml", false);
    push_part(
        &mut entries,
        &mut overrides,
        "word/theme/theme1.xml",
        theme_xml(doc),
        "application/vnd.openxmlformats-officedocument.theme+xml",
        PartRels::default(),
    );

    wr.embeds.font_table(&mut rels);

    // Media referenced by some drawing (orphan parts would make the package suspect).
    let mut media_types: BTreeMap<String, &str> = BTreeMap::new();
    for (key, file) in &wr.media_files {
        if !wr.used_media.contains(key) {
            continue;
        }
        if let Some(bytes) = doc.media.get(key) {
            let ext = file.rsplit('.').next().unwrap_or("bin").to_ascii_lowercase();
            media_types.insert(ext, content_type_for(file));
            entries.push((format!("word/media/{file}"), bytes.to_vec()));
        }
    }

    wr.embeds.custom_xml_rels("D", &mut rels);
    wr.embeds.custom_xml_rels("P", &mut root);
    wr.embeds.unmodelled_rels("D", &mut rels);
    wr.embeds.unmodelled_rels("P", &mut root);
    // Parts of embedded objects and custom XML stores kept from the file.
    for (name, bytes, ct, prels) in std::mem::take(&mut wr.embeds.parts) {
        push_part(&mut entries, &mut overrides, &name, bytes, &ct, prels);
    }

    // The macro project and the parts it relates to (VBA data, signatures…), verbatim. A
    // macro-free package can't hold them (Word drops them too).
    if let Some(vba) = doc.passthrough.get(VBA_PROJECT_PART).filter(|b| !b.is_empty()) {
        if flavor.macros() {
            let mut vba_rels = PartRels::default();
            let manifest = doc.passthrough.get(VBA_RELATED).map(|m| String::from_utf8_lossy(m).into_owned()).unwrap_or_default();
            let mut written: Vec<String> = Vec::new();
            for line in manifest.lines().take(MAX_VBA_RELATED) {
                let mut f = line.split('\t');
                let (Some(kind), Some(path), Some(ct), None) = (f.next(), f.next(), f.next(), f.next()) else { continue };
                let Some(target) = path.strip_prefix("word/").filter(|t| !t.is_empty()) else { continue };
                let Some(bytes) = doc.passthrough.get(path) else { continue };
                let ours = written.iter().any(|w| w.eq_ignore_ascii_case(path));
                // Never shadow a part this writer produces (a hostile relationship may point at one).
                let clash = path.eq_ignore_ascii_case("word/document.xml")
                    || path.eq_ignore_ascii_case(VBA_PROJECT_PART)
                    || path.to_ascii_lowercase().ends_with(".rels")
                    || entries.iter().any(|(n, _)| n.eq_ignore_ascii_case(path));
                if clash && !ours {
                    continue;
                }
                // A relative reference whose first segment has a colon would read as a URI scheme.
                let target = if target.split('/').next().is_some_and(|seg| seg.contains(':')) { format!("./{target}") } else { target.to_string() };
                vba_rels.add(kind, &target, false);
                if !ours {
                    push_part(&mut entries, &mut overrides, path, bytes.to_vec(), ct, PartRels::default());
                    written.push(path.to_string());
                }
            }
            rels.add(rt::VBA_PROJECT, "vbaProject.bin", false);
            push_part(&mut entries, &mut overrides, VBA_PROJECT_PART, vba.to_vec(), "application/vnd.ms-office.vbaProject", vba_rels);
        } else {
            log::warn!("docx: {flavor:?} can't hold macros; the VBA project is left out");
        }
    }

    // Main part goes first in the zip after content types.
    entries.insert(0, ("word/document.xml".into(), body));
    overrides.insert(0, ("/word/document.xml".into(), flavor.main_content_type().into()));
    entries.insert(1, ("word/_rels/document.xml.rels".into(), rels.xml()));

    // Package-level parts.
    root.add(rt::OFFICE_DOC, "word/document.xml", false);
    root.add(rt::CORE, "docProps/core.xml", false);
    root.add(rt::APP, "docProps/app.xml", false);
    entries.push(("docProps/core.xml".into(), core_xml(doc)));
    overrides.push(("/docProps/core.xml".into(), "application/vnd.openxmlformats-package.core-properties+xml".into()));
    entries.push(("docProps/app.xml".into(), app_xml(doc)));
    overrides.push(("/docProps/app.xml".into(), "application/vnd.openxmlformats-officedocument.extended-properties+xml".into()));
    if !doc.custom_props.is_empty() {
        root.add(rt::CUSTOM, "docProps/custom.xml", false);
        entries.push(("docProps/custom.xml".into(), crate::custom::write(&doc.custom_props)));
        overrides.push(("/docProps/custom.xml".into(), "application/vnd.openxmlformats-officedocument.custom-properties+xml".into()));
    }
    if !doc.sources.is_empty() {
        root.add(rt::CUSTOM_XML, "customXml/item1.xml", false);
        let mut item_rels = PartRels::default();
        item_rels.add(rt::CUSTOM_XML_PROPS, "itemProps1.xml", false);
        push_part(
            &mut entries,
            &mut overrides,
            "customXml/item1.xml",
            bibliography_xml(doc),
            "application/vnd.openxmlformats-officedocument.customXml+xml",
            item_rels,
        );
        push_part(
            &mut entries,
            &mut overrides,
            "customXml/itemProps1.xml",
            bibliography_props_xml(),
            "application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
            PartRels::default(),
        );
    }
    entries.insert(0, ("_rels/.rels".into(), root.xml()));

    // Content types.
    let mut w = W::new();
    w.open("Types", &[("xmlns", "http://schemas.openxmlformats.org/package/2006/content-types")]);
    w.empty("Default", &[("Extension", "rels"), ("ContentType", "application/vnd.openxmlformats-package.relationships+xml")]);
    w.empty("Default", &[("Extension", "xml"), ("ContentType", "application/xml")]);
    for (ext, ct) in &media_types {
        if ext != "xml" && ext != "rels" {
            w.empty("Default", &[("Extension", ext), ("ContentType", ct)]);
        }
    }
    for (part, ct) in &overrides {
        w.empty("Override", &[("PartName", part), ("ContentType", ct)]);
    }
    w.close("Types");
    entries.insert(0, ("[Content_Types].xml".into(), w.into_bytes()));
    Ok(entries)
}

fn push_part(entries: &mut Vec<(String, Vec<u8>)>, overrides: &mut Vec<(String, String)>, path: &str, bytes: Vec<u8>, ct: &str, rels: PartRels) {
    if !rels.is_empty() {
        let (dir, file) = path.rsplit_once('/').unwrap_or(("", path));
        entries.push((format!("{dir}/_rels/{file}.rels"), rels.xml()));
    }
    entries.push((path.to_string(), bytes));
    overrides.push((format!("/{path}"), ct.to_string()));
}

/// Content type by file extension (or sniffed name).
fn content_type_for(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// Guess an image extension from magic bytes.
fn sniff(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        "tiff"
    } else if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
        "wmf"
    } else if bytes.get(40..44) == Some(b" EMF") {
        "emf"
    } else if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") {
        "svg"
    } else {
        "bin"
    }
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

/// Built-in names Word keeps in lower case in styles.xml.
fn file_style_name(name: &str) -> String {
    if let Some((f, _)) = crate::read::LOWER_NAMES.iter().find(|(_, ui)| ui.eq_ignore_ascii_case(name)) {
        return (*f).to_string();
    }
    for (pre, f) in [("Heading ", "heading "), ("TOC ", "toc "), ("Index ", "index ")] {
        if let Some(d) = name.strip_prefix(pre)
            && d.len() == 1
            && d.chars().all(|c| c.is_ascii_digit())
        {
            return format!("{f}{d}");
        }
    }
    name.to_string()
}

fn styles_xml(doc: &Document) -> Vec<u8> {
    let s = &doc.styles;
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:styles", &ns_refs);
    w.open("w:docDefaults", &[]);
    w.open("w:rPrDefault", &[]);
    w.open("w:rPr", &[]);
    props::rpr_inner(&mut w, &s.default_chr);
    w.close("w:rPr");
    w.close("w:rPrDefault");
    if s.default_para_present || s.default_para != wordcraft_doc::props::ParaProps::default() {
        w.open("w:pPrDefault", &[]);
        w.open("w:pPr", &[]);
        props::ppr_inner(&mut w, &s.default_para, false, None);
        w.close("w:pPr");
        w.close("w:pPrDefault");
    }
    w.close("w:docDefaults");
    if let Some(latent) = s.latent_styles.as_deref() {
        w.raw(latent);
    }
    // Kinds whose default style the file named (`w:default`); the English ids stand in otherwise.
    let flagged: Vec<StyleKind> = s.styles.iter().filter(|st| st.default).map(|st| st.kind).collect();
    for st in &s.styles {
        style_xml(&mut w, st, &flagged);
    }
    w.close("w:styles");
    w.into_bytes()
}

fn style_xml(w: &mut W, st: &Style, flagged: &[StyleKind]) {
    let ty = match st.kind {
        StyleKind::Paragraph => "paragraph",
        StyleKind::Character => "character",
        StyleKind::Table => "table",
        StyleKind::Numbering => "numbering",
    };
    let english_default = matches!(
        (st.kind, st.id.as_str()),
        (StyleKind::Paragraph, "Normal")
            | (StyleKind::Character, "DefaultParagraphFont")
            | (StyleKind::Table, "TableNormal")
            | (StyleKind::Numbering, "NoList")
    );
    let is_default = st.default || (english_default && !flagged.contains(&st.kind));
    let mut a = vec![("w:type", ty), ("w:styleId", st.id.as_str())];
    if is_default {
        a.push(("w:default", "1"));
    }
    if !st.builtin {
        a.push(("w:customStyle", "1"));
    }
    w.open("w:style", &a);
    w.val("w:name", &file_style_name(if st.name.is_empty() { &st.id } else { &st.name }));
    if let Some(b) = &st.based_on {
        w.val("w:basedOn", b);
    }
    if let Some(b) = &st.next {
        w.val("w:next", b);
    }
    if let Some(b) = &st.linked {
        w.val("w:link", b);
    }
    if let Some(p) = st.priority {
        w.val("w:uiPriority", &p.to_string());
    }
    if st.hidden {
        w.empty("w:semiHidden", &[]);
        w.empty("w:unhideWhenUsed", &[]);
    }
    if st.quick {
        w.empty("w:qFormat", &[]);
    }
    if st.locked {
        w.empty("w:locked", &[]);
    }
    if !st.para.is_empty() && st.kind != StyleKind::Character {
        w.open("w:pPr", &[]);
        props::ppr_inner(w, &st.para, false, None);
        w.close("w:pPr");
    }
    if props::has_rpr(&st.chr) && st.kind != StyleKind::Numbering {
        w.open("w:rPr", &[]);
        props::rpr_inner(w, &st.chr);
        w.close("w:rPr");
    }
    if let Some(t) = &st.table {
        if t.borders.is_some() || t.cell_margins.is_some() || t.band_size.is_some() {
            w.open("w:tblPr", &[]);
            if let Some(n) = t.band_size {
                w.val("w:tblStyleRowBandSize", &n.clamp(1, 1000).to_string());
            }
            if let Some(b) = &t.borders {
                props::borders(w, "w:tblBorders", b, Some("w:insideH"), &[]);
            }
            if let Some(m) = &t.cell_margins {
                props::margins(w, "w:tblCellMar", m);
            }
            w.close("w:tblPr");
        }
        // The conditional formatting parts (ECMA-376 §17.7.6): paragraph and run properties,
        // then cell borders, shading and vertical alignment. The whole table's text formatting
        // is the style's own (above).
        for part in TablePart::ALL {
            let f = t.part(part);
            let ppr = ParaProps { align: f.align, ..ParaProps::default() };
            let has_rpr = props::has_rpr(&f.chr);
            // The whole table's borders are its table borders (`w:tblPr` above).
            let borders = f.borders.filter(|_| part != TablePart::WholeTable);
            let has_tc = f.fill.is_some() || borders.is_some() || f.v_align.is_some();
            if !has_rpr && !has_tc && f.align.is_none() {
                continue;
            }
            w.open("w:tblStylePr", &[("w:type", part.ooxml())]);
            if f.align.is_some() {
                w.open("w:pPr", &[]);
                props::ppr_inner(w, &ppr, false, None);
                w.close("w:pPr");
            }
            if has_rpr {
                w.open("w:rPr", &[]);
                props::rpr_inner(w, &f.chr);
                w.close("w:rPr");
            }
            if has_tc {
                w.open("w:tcPr", &[]);
                if let Some(bs) = borders {
                    props::borders(w, "w:tcBorders", &bs, Some("w:insideH"), &[]);
                }
                if let Some(c) = f.fill {
                    w.empty("w:shd", &[("w:val", "clear"), ("w:color", "auto"), ("w:fill", &c.hex())]);
                }
                if let Some(v) = f.v_align {
                    w.val(
                        "w:vAlign",
                        match v {
                            VAlign::Top => "top",
                            VAlign::Center => "center",
                            VAlign::Bottom => "bottom",
                        },
                    );
                }
                w.close("w:tcPr");
            }
            w.close("w:tblStylePr");
        }
    }
    w.close("w:style");
}

fn numbering_xml(doc: &Document) -> Vec<u8> {
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:numbering", &ns_refs);
    let mut nsids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for a in &doc.numbering.abstracts {
        w.open("w:abstractNum", &[("w:abstractNumId", &a.id.to_string())]);
        // CT_AbstractNum order: nsid, multiLevelType, tmpl, name… A copied definition repeats
        // its source's nsid; only the first keeps it, so Word doesn't take them for one list.
        if let Some(n) = a.nsid.as_deref().filter(|n| nsids.insert(n)) {
            w.val("w:nsid", n);
        }
        let derived = if a.levels.len() > 1 { "multilevel" } else { "singleLevel" };
        w.val("w:multiLevelType", a.multi_level_type.as_deref().unwrap_or(derived));
        if let Some(t) = &a.tmpl {
            w.val("w:tmpl", t);
        }
        if let Some(n) = &a.name {
            w.val("w:name", n);
        }
        for (i, l) in a.levels.iter().take(9).enumerate() {
            write_level(&mut w, i, l);
        }
        w.close("w:abstractNum");
    }
    for n in &doc.numbering.nums {
        w.open("w:num", &[("w:numId", &n.id.to_string())]);
        w.val("w:abstractNumId", &n.abstract_id.to_string());
        for lvl in 0..9u8 {
            let start = n.start_overrides.iter().find(|(l, _)| (*l).min(8) == lvl).map(|(_, s)| *s);
            let level = n.level_overrides.iter().find(|(l, _)| (*l).min(8) == lvl).map(|(_, l)| l);
            if start.is_none() && level.is_none() {
                continue;
            }
            w.open("w:lvlOverride", &[("w:ilvl", &lvl.to_string())]);
            if let Some(start) = start {
                w.val("w:startOverride", &start.to_string());
            }
            if let Some(level) = level {
                write_level(&mut w, lvl as usize, level);
            }
            w.close("w:lvlOverride");
        }
        w.close("w:num");
    }
    w.close("w:numbering");
    w.into_bytes()
}

/// One `w:lvl` (list level `i`).
fn write_level(w: &mut W, i: usize, l: &Level) {
    w.open("w:lvl", &[("w:ilvl", &i.to_string())]);
    w.val("w:start", &l.start.to_string());
    w.val("w:numFmt", l.format.ooxml());
    if !l.restart {
        w.val("w:lvlRestart", "0");
    } else if let Some(k) = l.restart_after.filter(|k| (1..=9).contains(k)) {
        w.val("w:lvlRestart", &k.to_string());
    }
    if let Some(s) = &l.style {
        w.val("w:pStyle", s);
    }
    if l.legal {
        w.empty("w:isLgl", &[]);
    }
    match l.suffix {
        LevelSuffix::Tab => {}
        LevelSuffix::Space => w.val("w:suff", "space"),
        LevelSuffix::Nothing => w.val("w:suff", "nothing"),
    }
    w.val("w:lvlText", &l.text);
    w.val("w:lvlJc", props::align_val(l.align));
    w.open("w:pPr", &[]);
    if let Some(t) = l.tab.filter(|t| t.is_finite()) {
        w.open("w:tabs", &[]);
        w.empty("w:tab", &[("w:val", "num"), ("w:pos", &crate::units::twips(t.clamp(-1584.0, 1584.0)))]);
        w.close("w:tabs");
    }
    let ind = crate::units::twips(l.indent);
    if l.hanging >= 0.0 {
        w.empty("w:ind", &[("w:left", &ind), ("w:hanging", &crate::units::twips(l.hanging))]);
    } else {
        w.empty("w:ind", &[("w:left", &ind), ("w:firstLine", &crate::units::twips(-l.hanging))]);
    }
    w.close("w:pPr");
    props::rpr(w, &l.chr);
    w.close("w:lvl");
}

/// The file id of the first footnote or endnote: after the separators (-1, 0) and the
/// continuation notice (1), when the document has one.
fn note_id_base(doc: &Document, foot: bool) -> usize {
    let seps = if foot { &doc.footnote_separators } else { &doc.endnote_separators };
    if seps.continuation_notice.is_some() { 2 } else { 1 }
}

/// Whether the footnotes (or endnotes) part is written: there are notes, or separators of the
/// document's own to keep.
fn notes_part(doc: &Document, wr: &Writer<'_>, foot: bool) -> bool {
    let (ids, seps) = if foot { (&wr.footnotes, &doc.footnote_separators) } else { (&wr.endnotes, &doc.endnote_separators) };
    !ids.is_empty() || !seps.is_empty()
}

/// `w:settings`, its children in the schema's order (CT_Settings is a sequence). `template_rel`
/// is the id of the attached template's relationship, when there is one.
fn settings_xml(doc: &Document, footnotes: bool, endnotes: bool, template_rel: Option<&str>) -> Vec<u8> {
    let s = &doc.settings;
    let s_notes = (&doc.footnote_separators, &doc.endnote_separators);
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:settings", &ns_refs);
    let zoom = s.zoom.unwrap_or(wordcraft_doc::Zoom { percent: 100, fit: None });
    let percent = zoom.percent.clamp(10, 500).to_string();
    match zoom.fit {
        Some(fit) => {
            let val = match fit {
                wordcraft_doc::ZoomFit::OnePage => "fullPage",
                wordcraft_doc::ZoomFit::PageWidth => "bestFit",
                wordcraft_doc::ZoomFit::TextWidth => "textFit",
            };
            w.empty("w:zoom", &[("w:val", val), ("w:percent", &percent)]);
        }
        None => w.empty("w:zoom", &[("w:percent", &percent)]),
    }
    // §17.15.1.70 / §17.15.1.69, right after `w:zoom` in CT_Settings.
    if s.remove_personal_info {
        w.empty("w:removePersonalInformation", &[]);
    }
    if s.remove_date_time {
        w.empty("w:removeDateAndTime", &[]);
    }
    if s.page_color.is_some() || s.page_picture.is_some() {
        w.empty("w:displayBackgroundShape", &[]);
    }
    if s.mirror_margins {
        w.empty("w:mirrorMargins", &[]);
    }
    if let Some(id) = template_rel {
        w.empty("w:attachedTemplate", &[("r:id", id)]);
    }
    if s.link_styles {
        w.empty("w:linkStyles", &[]);
    }
    if s.track_changes {
        w.empty("w:trackRevisions", &[]);
    }
    if !s.track_moves {
        w.empty("w:doNotTrackMoves", &[]);
    }
    if let Some(p) = &s.protection {
        let mut a: Vec<(&str, &str)> = Vec::new();
        if p != "none" {
            a.push(("w:edit", p));
        }
        if s.protect_formatting {
            a.push(("w:formatting", "1"));
        }
        a.push(("w:enforcement", "1"));
        // Only the hash attributes the reader keeps, each once.
        for name in crate::read::PROTECTION_HASH_ATTRS {
            if let Some((_, v)) = s.protect_hash.iter().find(|(k, _)| k == name) {
                a.push((name, v));
            }
        }
        w.empty("w:documentProtection", &a);
    }
    w.val("w:defaultTabStop", &crate::units::twips(s.default_tab.clamp(1.0, 1584.0)));
    if s.auto_hyphenation {
        w.empty("w:autoHyphenation", &[]);
    }
    // Schema order: consecutiveHyphenLimit, hyphenationZone, doNotHyphenateCaps.
    if s.consecutive_hyphen_limit > 0 {
        w.val("w:consecutiveHyphenLimit", &s.consecutive_hyphen_limit.min(32_767).to_string());
    }
    if s.hyphenation_zone.is_finite() && (s.hyphenation_zone - wordcraft_doc::DEFAULT_HYPHENATION_ZONE).abs() > 0.01 {
        w.val("w:hyphenationZone", &crate::units::twips(s.hyphenation_zone.clamp(0.0, 1584.0)));
    }
    if !s.hyphenate_caps {
        w.empty("w:doNotHyphenateCaps", &[]);
    }
    if s.even_odd_headers {
        w.empty("w:evenAndOddHeaders", &[]);
    }
    for (tag, v) in [("w:drawingGridHorizontalSpacing", s.grid_h), ("w:drawingGridVerticalSpacing", s.grid_v)] {
        if v.is_finite() && (v - wordcraft_doc::DEFAULT_GRID).abs() > 0.01 {
            w.val(tag, &crate::units::twips(v.clamp(0.5, 1584.0)));
        }
    }
    w.val("w:characterSpacingControl", "doNotCompress");
    for (tag, fmt, pr, used, el) in [
        ("w:footnotePr", s.footnote_format, &s.footnote_pr, footnotes, "w:footnote"),
        ("w:endnotePr", s.endnote_format, &s.endnote_pr, endnotes, "w:endnote"),
    ] {
        w.open(tag, &[]);
        story::note_pr_inner(&mut w, &wordcraft_doc::section::NoteProps { num_fmt: Some(fmt), ..*pr });
        if used {
            w.empty(el, &[("w:id", "-1")]);
            w.empty(el, &[("w:id", "0")]);
            let seps = if el == "w:footnote" { &s_notes.0 } else { &s_notes.1 };
            if seps.continuation_notice.is_some() {
                w.empty(el, &[("w:id", "1")]);
            }
        }
        w.close(tag);
    }
    w.open("w:compat", &[]);
    // CT_Compat: the older switches come before the `w:compatSetting`s.
    if s.suppress_break_spacing {
        w.empty("w:suppressSpBfAfterPgBrk", &[]);
    }
    if doc.settings.sum_paragraph_spacing {
        w.empty("w:doNotUseHTMLParagraphAutoSpacing", &[]);
    }
    if s.allow_space_of_same_style_in_table {
        w.empty("w:allowSpaceOfSameStyleInTable", &[]);
    }
    let mode = doc.settings.compat_mode.clamp(11, 15).to_string();
    w.empty("w:compatSetting", &[("w:name", "compatibilityMode"), ("w:uri", "http://schemas.microsoft.com/office/word"), ("w:val", &mode)]);
    w.close("w:compat");
    if let Some(m) = &s.math {
        math::math_pr(&mut w, m);
    }
    w.close("w:settings");
    w.into_bytes()
}

/// Is `d` a usable `w16cid:durableId`: 1–8 hex digits, below 0x7FFFFFFF, not 0?
pub(crate) fn durable_id_ok(d: &str) -> bool {
    d.len() <= 8 && u32::from_str_radix(d, 16).is_ok_and(|v| v != 0 && v < 0x7FFF_FFFF)
}

/// `commentsIds.xml` (paragraph id → durable id) and `commentsExtensible.xml` (durable id → UTC
/// date) for the comments that got paragraph ids. Durable ids read from the file are kept (unique
/// ones); comments without one get the next free id. Word uses them to keep tracking a comment
/// across saves and for its UTC time.
fn write_comment_ids(
    doc: &Document,
    comment_paras: &[(u32, String)],
    entries: &mut Vec<(String, Vec<u8>)>,
    overrides: &mut Vec<(String, String)>,
    rels: &mut PartRels,
) {
    if comment_paras.is_empty() {
        return;
    }
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut next = 0x1000_0000u32;
    let mut rows: Vec<(String, String, Option<String>)> = Vec::new();
    for (cid, pid) in comment_paras {
        let Some(c) = doc.comments.get(cid) else { continue };
        let kept = c.durable_id.as_deref().filter(|d| durable_id_ok(d)).map(|d| format!("{:08X}", u32::from_str_radix(d, 16).unwrap_or(0)));
        let durable = match kept.filter(|d| !used.contains(d)) {
            Some(d) => d,
            None => loop {
                next = next.saturating_add(1);
                let d = format!("{next:08X}");
                if !used.contains(&d) && !doc.comments.values().any(|o| o.durable_id.as_deref() == Some(d.as_str())) {
                    break d;
                }
            },
        };
        used.insert(durable.clone());
        // A local `w:date` with a zone designator is UTC already; otherwise only the file's own.
        // Remove date and time on save (#1062): no comment date here either.
        let utc = if doc.settings.remove_date_time { None } else { c.date_utc.clone().or_else(|| c.date.ends_with('Z').then(|| c.date.clone())) };
        rows.push((pid.clone(), durable, utc));
    }
    let mc = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    let mut w = W::new();
    w.open(
        "w16cid:commentsIds",
        &[("xmlns:mc", mc), ("xmlns:w16cid", "http://schemas.microsoft.com/office/word/2016/wordml/cid"), ("mc:Ignorable", "w16cid")],
    );
    for (pid, durable, _) in &rows {
        w.empty("w16cid:commentId", &[("w16cid:paraId", pid), ("w16cid:durableId", durable)]);
    }
    w.close("w16cid:commentsIds");
    rels.add(rt::COMMENTS_IDS, "commentsIds.xml", false);
    push_part(entries, overrides, "word/commentsIds.xml", w.into_bytes(), &format!("{CT_WML}commentsIds+xml"), PartRels::default());

    let mut w = W::new();
    w.open(
        "w16cex:commentsExtensible",
        &[("xmlns:mc", mc), ("xmlns:w16cex", "http://schemas.microsoft.com/office/word/2018/wordml/cex"), ("mc:Ignorable", "w16cex")],
    );
    for (_, durable, utc) in &rows {
        match utc {
            Some(d) => w.empty("w16cex:commentExtensible", &[("w16cex:durableId", durable), ("w16cex:dateUtc", d)]),
            None => w.empty("w16cex:commentExtensible", &[("w16cex:durableId", durable)]),
        }
    }
    w.close("w16cex:commentsExtensible");
    rels.add(rt::COMMENTS_EXTENSIBLE, "commentsExtensible.xml", false);
    push_part(entries, overrides, "word/commentsExtensible.xml", w.into_bytes(), &format!("{CT_WML}commentsExtensible+xml"), PartRels::default());
}

fn theme_xml(doc: &Document) -> Vec<u8> {
    let s = &doc.settings;
    let names = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];
    let mut w = W::new();
    let name = if s.theme_name.is_empty() { "Craft" } else { s.theme_name.as_str() };
    w.open("a:theme", &[("xmlns:a", "http://schemas.openxmlformats.org/drawingml/2006/main"), ("name", name)]);
    w.open("a:themeElements", &[]);
    w.open("a:clrScheme", &[("name", name)]);
    for (i, n) in names.iter().enumerate() {
        let c = s.theme_colors.get(i).or_else(|| wordcraft_doc::THEME_COLORS.get(i)).copied().unwrap_or_default();
        w.open(&format!("a:{n}"), &[]);
        w.empty("a:srgbClr", &[("val", &c.hex())]);
        w.close(&format!("a:{n}"));
    }
    w.close("a:clrScheme");
    w.open("a:fontScheme", &[("name", name)]);
    for (tag, face) in [("a:majorFont", &s.major_font), ("a:minorFont", &s.minor_font)] {
        w.open(tag, &[]);
        w.empty("a:latin", &[("typeface", face)]);
        w.empty("a:ea", &[("typeface", "")]);
        w.empty("a:cs", &[("typeface", "")]);
        w.close(tag);
    }
    w.close("a:fontScheme");
    // A minimal format scheme (three entries per list, as the schema requires).
    w.open("a:fmtScheme", &[("name", name)]);
    w.open("a:fillStyleLst", &[]);
    for _ in 0..3 {
        w.raw("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    }
    w.close("a:fillStyleLst");
    w.open("a:lnStyleLst", &[]);
    for width in ["6350", "12700", "19050"] {
        w.raw(&format!(
            "<a:ln w=\"{width}\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>"
        ));
    }
    w.close("a:lnStyleLst");
    w.open("a:effectStyleLst", &[]);
    for _ in 0..3 {
        w.raw("<a:effectStyle><a:effectLst/></a:effectStyle>");
    }
    w.close("a:effectStyleLst");
    w.open("a:bgFillStyleLst", &[]);
    for _ in 0..3 {
        w.raw("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    }
    w.close("a:bgFillStyleLst");
    w.close("a:fmtScheme");
    w.close("a:themeElements");
    w.empty("a:objectDefaults", &[]);
    w.empty("a:extraClrSchemeLst", &[]);
    w.close("a:theme");
    w.into_bytes()
}

/// The author the saved file gives a comment or tracked change when the document removes personal
/// information on save (`w:removePersonalInformation`), as Word does.
pub(crate) const ANONYMOUS_AUTHOR: &str = "Author";
const ANONYMOUS_INITIALS: &str = "A";

/// The `w:author` written for an annotation by `author`: "Author" when the document removes
/// personal information on save, "Unknown" when it has none.
pub(crate) fn annotation_author<'a>(doc: &Document, author: &'a str) -> &'a str {
    if doc.settings.remove_personal_info {
        ANONYMOUS_AUTHOR
    } else if author.is_empty() {
        "Unknown"
    } else {
        author
    }
}

fn core_xml(doc: &Document) -> Vec<u8> {
    let c = &doc.core;
    // Remove personal information (§17.15.1.70): no creator or last-modified-by in the file.
    let personal = |v: &String| if doc.settings.remove_personal_info { String::new() } else { v.clone() };
    let (creator, last_modified_by) = (personal(&c.creator), personal(&c.last_modified_by));
    let mut w = W::new();
    w.open(
        "cp:coreProperties",
        &[
            ("xmlns:cp", "http://schemas.openxmlformats.org/package/2006/metadata/core-properties"),
            ("xmlns:dc", "http://purl.org/dc/elements/1.1/"),
            ("xmlns:dcterms", "http://purl.org/dc/terms/"),
            ("xmlns:dcmitype", "http://purl.org/dc/dcmitype/"),
            ("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance"),
        ],
    );
    for (tag, v) in [
        ("dc:title", &c.title),
        ("dc:subject", &c.subject),
        ("dc:creator", &creator),
        ("cp:keywords", &c.keywords),
        ("dc:description", &c.description),
        ("cp:lastModifiedBy", &last_modified_by),
        ("cp:category", &c.category),
    ] {
        if !v.is_empty() {
            w.leaf(tag, &[], v);
        }
    }
    if c.revision > 0 {
        w.leaf("cp:revision", &[], &c.revision.to_string());
    }
    for (tag, v) in [("dcterms:created", &c.created), ("dcterms:modified", &c.modified)] {
        if !v.is_empty() {
            w.leaf(tag, &[("xsi:type", "dcterms:W3CDTF")], v);
        }
    }
    w.close("cp:coreProperties");
    w.into_bytes()
}

fn app_xml(doc: &Document) -> Vec<u8> {
    let mut w = W::new();
    w.open(
        "Properties",
        &[
            ("xmlns", "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"),
            ("xmlns:vt", "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"),
        ],
    );
    w.leaf("Application", &[], "WordCraft");
    w.leaf("Words", &[], &doc.word_count().to_string());
    w.leaf("Paragraphs", &[], &doc.paragraph_count().to_string());
    w.leaf("DocSecurity", &[], "0");
    // Kept properties, except the personal ones when the file asks Word to remove personal
    // information on save.
    let private = doc.settings.remove_personal_info;
    if let Some(kept) = doc.passthrough.get(crate::package::APP_KEPT) {
        for line in String::from_utf8_lossy(kept).lines().take(crate::package::APP_KEPT_PROPS.len()) {
            let Some((name, value)) = line.split_once('\t') else { continue };
            let Some(name) = crate::package::APP_KEPT_PROPS.iter().find(|n| **n == name) else { continue };
            if private && matches!(*name, "Company" | "Manager") {
                continue;
            }
            w.leaf(name, &[], value);
        }
    }
    w.close("Properties");
    w.into_bytes()
}

fn to_source_type(kind: &str) -> &str {
    match kind.to_ascii_lowercase().as_str() {
        "book" => "Book",
        "booksection" | "chapter" => "BookSection",
        "article" | "journal" | "journalarticle" => "JournalArticle",
        "periodical" | "articleinaperiodical" => "ArticleInAPeriodical",
        "website" | "site" | "internetsite" => "InternetSite",
        "documentfrominternetsite" => "DocumentFromInternetSite",
        "electronicsource" => "ElectronicSource",
        "report" => "Report",
        "conference" | "proceedings" | "conferenceproceedings" => "ConferenceProceedings",
        "case" => "Case",
        "art" => "Art",
        "film" => "Film",
        "interview" => "Interview",
        "patent" => "Patent",
        "performance" => "Performance",
        "soundrecording" => "SoundRecording",
        "standard" => "Standard",
        "thesis" => "Thesis",
        "other" | "misc" => "Misc",
        _ => "Book",
    }
}

pub(crate) fn from_source_type(st: &str) -> String {
    match st.trim() {
        "Book" | "book" => "book".into(),
        "BookSection" | "booksection" => "bookSection".into(),
        "JournalArticle" | "journalarticle" | "article" => "article".into(),
        "ArticleInAPeriodical" | "articleinaperiodical" => "periodical".into(),
        "InternetSite" | "internetsite" | "website" => "website".into(),
        "DocumentFromInternetSite" | "documentfrominternetsite" => "website".into(),
        "ElectronicSource" | "electronicsource" => "electronicsource".into(),
        "Report" | "report" => "report".into(),
        "ConferenceProceedings" | "conferenceproceedings" => "conference".into(),
        "Case" | "case" => "case".into(),
        "SoundRecording" | "soundrecording" => "soundrecording".into(),
        "Film" | "film" => "film".into(),
        "Interview" | "interview" => "interview".into(),
        "Patent" | "patent" => "patent".into(),
        "Performance" | "performance" => "performance".into(),
        "Standard" | "standard" => "standard".into(),
        "Thesis" | "thesis" => "thesis".into(),
        "Misc" | "misc" => "other".into(),
        "" => "book".into(),
        other => other.to_ascii_lowercase(),
    }
}

/// The document's citation style, APA when it has none, and a style sheet file name made from
/// it (`\APA.XSL`, `\MLA.XSL`).
fn bib_style(doc: &Document) -> (&str, String) {
    let name = doc.settings.bib_style.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or("APA");
    let file: String = name.chars().filter(char::is_ascii_alphanumeric).collect();
    (name, format!("\\{}.XSL", if file.is_empty() { "APA" } else { &file }))
}

fn write_name_list(w: &mut W, names: &str) {
    w.open("b:NameList", &[]);
    let persons: Vec<_> = names.split(';').map(str::trim).filter(|s| !s.is_empty()).collect();
    for a in persons {
        w.open("b:Person", &[]);
        // Parse "Last, First" or "Last, First Middle"
        let (last, first_middle) = match a.split_once(',') {
            Some((l, f)) => (l.trim(), f.trim()),
            None => (a, ""),
        };
        w.leaf("b:Last", &[], last);
        if !first_middle.is_empty() {
            let parts: Vec<&str> = first_middle.split_whitespace().collect();
            if let Some(first) = parts.first() {
                w.leaf("b:First", &[], first);
            }
            if let Some(slice) = parts.get(1..).filter(|s| !s.is_empty()) {
                let mid = slice.join(" ");
                w.leaf("b:Middle", &[], &mid);
            }
        }
        w.close("b:Person");
    }
    w.close("b:NameList");
}

fn split_accessed_date(s: &str) -> (Option<&str>, Option<&str>, Option<&str>) {
    let s = s.trim();
    if s.is_empty() {
        return (None, None, None);
    }
    let parts: Vec<&str> = if s.contains('-') {
        s.split('-').collect()
    } else if s.contains('/') {
        s.split('/').collect()
    } else {
        Vec::new()
    };
    if let [y, m, d] = parts.as_slice() {
        (Some(y.trim()), Some(m.trim()), Some(d.trim()))
    } else if let [y, m] = parts.as_slice() {
        (Some(y.trim()), Some(m.trim()), None)
    } else {
        (Some(s), None, None)
    }
}

fn bibliography_xml(doc: &Document) -> Vec<u8> {
    let (style_name, selected) = bib_style(doc);
    let mut w = W::new();
    w.open(
        "b:Sources",
        &[
            ("SelectedStyle", selected.as_str()),
            ("StyleName", style_name),
            ("xmlns:b", "http://schemas.openxmlformats.org/officeDocument/2006/bibliography"),
            ("xmlns", "http://schemas.openxmlformats.org/officeDocument/2006/bibliography"),
        ],
    );
    for src in &doc.sources {
        w.open("b:Source", &[]);
        w.leaf("b:Tag", &[], &src.tag);
        w.leaf("b:SourceType", &[], to_source_type(&src.kind));
        // Author & Editor
        if !src.author.is_empty() || !src.editor.is_empty() {
            w.open("b:Author", &[]);
            if !src.author.is_empty() {
                w.open("b:Author", &[]);
                write_name_list(&mut w, &src.author);
                w.close("b:Author");
            }
            if !src.editor.is_empty() {
                w.open("b:Editor", &[]);
                write_name_list(&mut w, &src.editor);
                w.close("b:Editor");
            }
            w.close("b:Author");
        }
        if !src.title.is_empty() {
            w.leaf("b:Title", &[], &src.title);
        }
        if !src.year.is_empty() {
            w.leaf("b:Year", &[], &src.year);
        }
        if !src.month.is_empty() {
            w.leaf("b:Month", &[], &src.month);
        }
        if !src.day.is_empty() {
            w.leaf("b:Day", &[], &src.day);
        }
        if !src.city.is_empty() {
            w.leaf("b:City", &[], &src.city);
        }
        if !src.publisher.is_empty() {
            w.leaf("b:Publisher", &[], &src.publisher);
        }
        if !src.journal.is_empty() {
            w.leaf("b:JournalName", &[], &src.journal);
        }
        if !src.edition.is_empty() {
            w.leaf("b:Edition", &[], &src.edition);
        }
        if !src.volume.is_empty() {
            w.leaf("b:Volume", &[], &src.volume);
        }
        if !src.issue.is_empty() {
            w.leaf("b:Issue", &[], &src.issue);
        }
        if !src.pages.is_empty() {
            w.leaf("b:Pages", &[], &src.pages);
        }
        if !src.url.is_empty() {
            w.leaf("b:URL", &[], &src.url);
        }
        if !src.accessed.is_empty() {
            let (y, m, d) = split_accessed_date(&src.accessed);
            if let Some(y) = y.filter(|s| !s.is_empty()) {
                w.leaf("b:YearAccessed", &[], y);
            }
            if let Some(m) = m.filter(|s| !s.is_empty()) {
                w.leaf("b:MonthAccessed", &[], m);
            }
            if let Some(d) = d.filter(|s| !s.is_empty()) {
                w.leaf("b:DayAccessed", &[], d);
            }
            w.leaf("b:Accessed", &[], &src.accessed);
        }
        if !src.doi.is_empty() {
            w.leaf("b:DOI", &[], &src.doi);
            w.leaf("b:StandardNumber", &[], &src.doi);
        }
        w.close("b:Source");
    }
    w.close("b:Sources");
    w.into_bytes()
}

fn bibliography_props_xml() -> Vec<u8> {
    let mut w = W::new();
    w.open(
        "ds:datastoreItem",
        &[("xmlns:ds", "http://schemas.openxmlformats.org/officeDocument/2006/customXml"), ("ds:itemID", "{3315B9A8-2323-4554-B8B2-1234567890AB}")],
    );
    w.open("ds:schemaRefs", &[]);
    w.empty("ds:schemaRef", &[("ds:uri", "http://schemas.openxmlformats.org/officeDocument/2006/bibliography")]);
    w.close("ds:schemaRefs");
    w.close("ds:datastoreItem");
    w.into_bytes()
}

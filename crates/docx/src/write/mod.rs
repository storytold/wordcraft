//! [`Document`] → DOCX.

mod embed;
mod math;
mod props;
mod story;

use std::collections::{BTreeMap, HashMap};

use wordcraft_doc::numbering::{Level, LevelSuffix};
use wordcraft_doc::styles::{Style, StyleKind};
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
    fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
    fn xml(&self) -> Vec<u8> {
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
    docpr: u32,
    z: u32,
    /// Note part id → file note id, in first-reference order.
    footnotes: Vec<u32>,
    endnotes: Vec<u32>,
    /// Header/footer part id → (file name, is footer).
    hf: Vec<(u32, String, bool)>,
    /// Paragraphs that need a `w14:paraId` (last paragraph of each comment), by address.
    para_ids: HashMap<usize, String>,
    /// A note reference mark to put at the start of the next paragraph.
    pending_mark: Option<&'static str>,
    /// The note being written (is footnote, part id): its reference to itself is the mark above.
    current_note: Option<(bool, u32)>,
    /// Writing a TOC heading: its TOC field is held back so the entries become the field's result.
    toc_hold_end: bool,
    /// The held TOC field (instruction, locked, props), opened in the first entry as Word does.
    toc_field: Option<(String, bool, wordcraft_doc::props::CharProps)>,
    /// Open the held TOC field at the start of the paragraph being written.
    toc_begin_here: bool,
    /// Close the open TOC field at the end of the paragraph being written.
    toc_end_here: bool,
    /// Media keys actually referenced by a written drawing.
    used_media: std::collections::BTreeSet<String>,
    /// Bounds writing text boxes inside text boxes (as layout shows them).
    boxes: wordcraft_doc::BoxBudget,
    /// Charts, diagrams and OLE objects kept from a file: the parts they need.
    embeds: embed::EmbedWriter<'d>,
}

const CT_WML: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.";

/// Write a `.docx` package.
pub fn write(doc: &Document) -> Result<Vec<u8>, DocxError> {
    write_as(doc, Flavor::Document)
}

/// Write a package of the given flavour (`.docx`, `.docm`, `.dotx`, `.dotm`).
pub fn write_as(doc: &Document, flavor: Flavor) -> Result<Vec<u8>, DocxError> {
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
        docpr: 0,
        z: 251_658_240,
        footnotes: Vec::new(),
        endnotes: Vec::new(),
        hf: Vec::new(),
        para_ids: HashMap::new(),
        pending_mark: None,
        current_note: None,
        toc_hold_end: false,
        toc_field: None,
        toc_begin_here: false,
        toc_end_here: false,
        used_media: Default::default(),
        boxes: wordcraft_doc::BoxBudget::default(),
        embeds: embed::EmbedWriter::new(doc),
    };
    wr.assign_media();
    wr.embeds.reserve_media(wr.media_files.values());
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut overrides: Vec<(String, String)> = Vec::new();
    let mut rels = PartRels::default();

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
    if let Some(c) = doc.settings.page_color {
        w.empty("w:background", &[("w:color", &c.hex())]);
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
        wr.blocks(&mut w, &blocks, &mut prels, false, 0);
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

    // Notes (references were collected from the body and headers).
    for foot in [true, false] {
        let ids = if foot { wr.footnotes.clone() } else { wr.endnotes.clone() };
        if ids.is_empty() {
            continue;
        }
        let (root, tag, file) = if foot { ("w:footnotes", "w:footnote", "footnotes.xml") } else { ("w:endnotes", "w:endnote", "endnotes.xml") };
        let mut prels = PartRels::default();
        let mut w = W::new();
        w.open(root, &ns_refs);
        for (id, kind) in [("-1", "separator"), ("0", "continuationSeparator")] {
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
        let mut k = 0;
        while let Some(pid) = (if foot { &wr.footnotes } else { &wr.endnotes }).get(k).copied() {
            k += 1;
            let nid = k.to_string();
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
            let author = if c.author.is_empty() { "Unknown" } else { c.author.as_str() };
            let mut a = vec![("w:id", id.as_str()), ("w:author", author)];
            if !c.date.is_empty() {
                a.push(("w:date", c.date.as_str()));
            }
            if !c.initials.is_empty() {
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
    }

    rels.add(rt::STYLES, "styles.xml", false);
    push_part(&mut entries, &mut overrides, "word/styles.xml", styles_xml(doc), &format!("{CT_WML}styles+xml"), PartRels::default());
    if !doc.numbering.nums.is_empty() || !doc.numbering.abstracts.is_empty() {
        rels.add(rt::NUMBERING, "numbering.xml", false);
        push_part(&mut entries, &mut overrides, "word/numbering.xml", numbering_xml(doc), &format!("{CT_WML}numbering+xml"), PartRels::default());
    }
    rels.add(rt::SETTINGS, "settings.xml", false);
    push_part(
        &mut entries,
        &mut overrides,
        "word/settings.xml",
        settings_xml(doc, !wr.footnotes.is_empty(), !wr.endnotes.is_empty()),
        &format!("{CT_WML}settings+xml"),
        PartRels::default(),
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

    // Parts of charts, diagrams and OLE objects kept from a file, as the objects written need them.
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
    let mut root = PartRels::default();
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

    zip_entries(&entries)
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
        self.docpr += 1;
        self.docpr.to_string()
    }

    fn bookmark_id(&mut self, name: &str) -> String {
        let n = self.bookmarks.len() as u32;
        self.bookmarks.entry(name.to_string()).or_insert(n).to_string()
    }

    fn note_id(&mut self, part: u32, foot: bool) -> String {
        let list = if foot { &mut self.footnotes } else { &mut self.endnotes };
        let idx = match list.iter().position(|p| *p == part) {
            Some(i) => i,
            None => {
                list.push(part);
                list.len() - 1
            }
        };
        (idx + 1).to_string()
    }

    fn hf_rel(&mut self, part: u32, footer: bool, rels: &mut PartRels) -> Option<String> {
        let kind = self.doc.parts.get(&part)?.kind;
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
    w.open("w:pPrDefault", &[]);
    w.open("w:pPr", &[]);
    props::ppr_inner(&mut w, &s.default_para, false, None);
    w.close("w:pPr");
    w.close("w:pPrDefault");
    w.close("w:docDefaults");
    for st in &s.styles {
        style_xml(&mut w, st);
    }
    w.close("w:styles");
    w.into_bytes()
}

fn style_xml(w: &mut W, st: &Style) {
    let ty = match st.kind {
        StyleKind::Paragraph => "paragraph",
        StyleKind::Character => "character",
        StyleKind::Table => "table",
        StyleKind::Numbering => "numbering",
    };
    let is_default = matches!(
        (st.kind, st.id.as_str()),
        (StyleKind::Paragraph, "Normal")
            | (StyleKind::Character, "DefaultParagraphFont")
            | (StyleKind::Table, "TableNormal")
            | (StyleKind::Numbering, "NoList")
    );
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
        // One conditional formatting region (ECMA-376 §17.7.6): run properties, then cell borders
        // and shading.
        let cond = |w: &mut W,
                    ty: &str,
                    chr: &wordcraft_doc::CharProps,
                    fill: Option<wordcraft_doc::Rgb>,
                    borders: Option<wordcraft_doc::props::Borders>| {
            if !props::has_rpr(chr) && fill.is_none() && borders.is_none() {
                return;
            }
            w.open("w:tblStylePr", &[("w:type", ty)]);
            if props::has_rpr(chr) {
                w.open("w:rPr", &[]);
                props::rpr_inner(w, chr);
                w.close("w:rPr");
            }
            if fill.is_some() || borders.is_some() {
                w.open("w:tcPr", &[]);
                if let Some(bs) = &borders {
                    props::borders(w, "w:tcBorders", bs, Some("w:insideH"), &[]);
                }
                if let Some(f) = fill {
                    w.empty("w:shd", &[("w:val", "clear"), ("w:color", "auto"), ("w:fill", &f.hex())]);
                }
                w.close("w:tcPr");
            }
            w.close("w:tblStylePr");
        };
        cond(w, "wholeTable", &wordcraft_doc::CharProps::default(), t.fill, None);
        cond(w, "firstRow", &t.header_chr, t.header_fill, t.header_borders);
        // The total row's borders, its top edge falling back to the built-in top rule.
        let total_borders = match (t.total_borders, t.total_border_top) {
            (Some(mut b), top) => {
                b.top = b.top.or(top);
                Some(b)
            }
            (None, top) => top.map(|b| wordcraft_doc::props::Borders { top: Some(b), ..Default::default() }),
        };
        cond(w, "lastRow", &t.total_chr, t.total_fill, total_borders);
        cond(w, "firstCol", &t.first_col_chr, t.first_col_fill, t.first_col_borders);
        cond(w, "lastCol", &t.last_col_chr, t.last_col_fill, t.last_col_borders);
        cond(w, "band1Vert", &t.col_band_chr, t.col_band_fill, t.col_band_borders);
        cond(w, "band1Horz", &t.band_chr, t.band_fill, t.band_borders);
    }
    w.close("w:style");
}

fn numbering_xml(doc: &Document) -> Vec<u8> {
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:numbering", &ns_refs);
    for a in &doc.numbering.abstracts {
        w.open("w:abstractNum", &[("w:abstractNumId", &a.id.to_string())]);
        w.val("w:multiLevelType", if a.levels.len() > 1 { "multilevel" } else { "singleLevel" });
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

fn settings_xml(doc: &Document, footnotes: bool, endnotes: bool) -> Vec<u8> {
    let s = &doc.settings;
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:settings", &ns_refs);
    w.empty("w:zoom", &[("w:percent", "100")]);
    if s.page_color.is_some() {
        w.empty("w:displayBackgroundShape", &[]);
    }
    if s.mirror_margins {
        w.empty("w:mirrorMargins", &[]);
    }
    if s.track_changes {
        w.empty("w:trackRevisions", &[]);
    }
    if let Some(p) = &s.protection {
        w.empty("w:documentProtection", &[("w:edit", p), ("w:enforcement", "1")]);
    }
    w.val("w:defaultTabStop", &crate::units::twips(s.default_tab.clamp(1.0, 1584.0)));
    if s.auto_hyphenation {
        w.empty("w:autoHyphenation", &[]);
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
    for (tag, fmt, used, el) in
        [("w:footnotePr", s.footnote_format.clone(), footnotes, "w:footnote"), ("w:endnotePr", s.endnote_format.clone(), endnotes, "w:endnote")]
    {
        w.open(tag, &[]);
        w.val("w:numFmt", fmt.ooxml());
        if used {
            w.empty(el, &[("w:id", "-1")]);
            w.empty(el, &[("w:id", "0")]);
        }
        w.close(tag);
    }
    w.open("w:compat", &[]);
    let mode = doc.settings.compat_mode.clamp(11, 15).to_string();
    w.empty("w:compatSetting", &[("w:name", "compatibilityMode"), ("w:uri", "http://schemas.microsoft.com/office/word"), ("w:val", &mode)]);
    w.close("w:compat");
    if let Some(m) = &s.math {
        math::math_pr(&mut w, m);
    }
    w.close("w:settings");
    w.into_bytes()
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

fn core_xml(doc: &Document) -> Vec<u8> {
    let c = &doc.core;
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
        ("dc:creator", &c.creator),
        ("cp:keywords", &c.keywords),
        ("dc:description", &c.description),
        ("cp:lastModifiedBy", &c.last_modified_by),
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
    w.close("Properties");
    w.into_bytes()
}

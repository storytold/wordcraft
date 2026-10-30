//! DOCX → [`Document`].

pub(crate) mod chart;
pub(crate) mod diagram;
mod drawing_color;
mod embed;
mod freeform;
mod glossary;
pub(crate) mod math;
mod props;
mod sdt;
mod shape_style;
mod story;
mod wordart;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use wordcraft_doc::control::{ControlEvent, ControlKind, control_events, pair_events};
use wordcraft_doc::graphic::{Graphic, GraphicKind};
use wordcraft_doc::numbering::{AbstractNum, Level, LevelSuffix, Num};
use wordcraft_doc::para::NoteKind;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::props::Rgb;
use wordcraft_doc::props::VAlign;
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::styles::{PartFormat, Style, StyleKind, StyleSheet, TablePart, TableStyleParts};
use wordcraft_doc::{
    Block, Blocks, Comment, Document, InlineObject, PagePicture, Paragraph, PartKind, Revision, RevisionKind, Zoom, ZoomFit, para_block,
};

use crate::DocxError;
use crate::bibliography::{MAX_SOURCES, Seen};
use crate::package::{ContentTypes, EMBEDDED_PARTS, EmbeddedManifest, MAX_VBA_RELATED, Package, Rels, VBA_PROJECT_PART, VBA_RELATED, rel_is, rt};
use crate::units::{flag, int, on_off, tw, u32_of};
use crate::xml::El;
pub(crate) use props::PropCtx;
use story::StoryCtx;

mod meta;
mod parts;
mod styles;
mod toc;

pub(crate) use meta::PROTECTION_HASH_ATTRS;
pub(crate) use styles::LOWER_NAMES;
use toc::{is_toc, toc_control_to_toc, toc_field_to_heading, unstyle_toc_links};

/// Most fonts read from a font table, and the longest name (hostile files).
const MAX_TABLE_FONTS: usize = 2000;
const MAX_FONT_NAME: usize = 256;

/// Most parts (headers, notes, comments, text boxes) we create from one file.
const MAX_PARTS: usize = 50_000;
/// Deepest chain of headers/footers whose own sections reference further headers/footers.
const MAX_HF_DEPTH: usize = 4;
/// Most graphic work (items plus path segments, see `story::graphic_work`) built for one file's
/// charts and diagrams; later ones are left empty.
const MAX_GRAPHIC_WORK: usize = 2_000_000;
/// Custom XML parts looked at for bibliography sources.
const MAX_BIBLIOGRAPHY_PARTS: usize = 64;

/// Longest attached-template path we keep (bytes); a longer one is not a real path.
const MAX_TEMPLATE_PATH: usize = 32 * 1024;

pub(crate) struct Reader<'p> {
    pkg: &'p Package,
    pub doc: Document,
    pub pc: PropCtx,
    revs: HashMap<(u8, String, String, Option<String>), u32>,
    /// Move ranges (`w:moveFromRangeStart` …) seen so far.
    pub moves: MoveCtx,
    media_by_path: HashMap<String, String>,
    hf_by_path: HashMap<String, u32>,
    /// Header/footer parts being read right now (see [`MAX_HF_DEPTH`]).
    hf_depth: usize,
    pub footnotes: HashMap<i64, u32>,
    pub endnotes: HashMap<i64, u32>,
    /// The note being read (kind, part id): its `w:footnoteRef` / `w:endnoteRef` mark is a
    /// reference to itself.
    current_note: Option<(NoteKind, u32)>,
    /// Reading a footnote or endnote separator or continuation notice (where the `w:separator`
    /// and `w:continuationSeparator` run elements belong).
    in_note_sep: bool,
    pub comment_map: HashMap<String, u32>,
    pub comments_ended: HashSet<u32>,
    pub bookmarks: HashMap<String, String>,
    /// Restrict Editing exception ranges (`w:permStart`): the file's id → ours.
    pub perms: HashMap<String, u32>,
    /// Chart and diagram parts by path, parsed once.
    graphic_parts: HashMap<String, Option<Arc<El>>>,
    /// Charts and diagrams by (kind, part path, width bits, height bits), built once.
    graphics: HashMap<(GraphicKind, String, u32, u32), Arc<Graphic>>,
    /// Graphic work left before charts and diagrams are left empty (see [`MAX_GRAPHIC_WORK`]).
    graphic_budget: usize,
    /// Drawing ids connectors are glued to (`a:stCxn`, `a:endCxn`), and whether any shape's id
    /// was kept while reading (those not glued to are dropped at the end).
    pub glued: HashSet<u32>,
    pub shape_ids: bool,
    /// The main document part's name.
    main: String,
    /// The package's content types, read when first needed.
    content_types: Option<ContentTypes>,
    /// Parts kept for charts, diagrams and OLE objects (see `embed`).
    embedded: EmbeddedManifest,
    /// Linked text boxes as read: (`wps:txbx/@id` or `wps:linkedTxbx/@id`, `seq` (0 for the box
    /// holding the text), text box part). See [`link_text_boxes`].
    pub txbx_links: Vec<(u32, u32, u32)>,
    /// The theme's fill styles, which shapes' style references point to.
    pub theme_fills: shape_style::ThemeFills,
    /// The fill of the group being read (what its members' `a:grpFill` stands for).
    pub group_fill: Option<wordcraft_doc::fill::Fill>,
}

/// The package path that internal relationship `id` points at, if it has type `kind` (an `rt`
/// constant): a crafted id can't make a chart of some other, possibly huge, part.
fn part_of(rels: &Rels, id: &str, kind: &str) -> Option<String> {
    rels.by_id(id).filter(|r| !r.external && rel_is(&r.kind, kind)).map(|r| r.target.clone())
}

/// Read a `.docx` package.
pub fn read(bytes: &[u8]) -> Result<Document, DocxError> {
    // A password-protected document is a compound file, not a zip: say so rather than failing
    // as a broken zip (`read_with_password` opens it).
    if crate::is_encrypted(bytes) {
        return crate::read_with_password(bytes, None);
    }
    read_package(bytes)
}

/// [`read`] for bytes known not to be an encrypted package.
pub(crate) fn read_package(bytes: &[u8]) -> Result<Document, DocxError> {
    read_opened(&Package::open(bytes)?)
}

/// Read a package whose parts are already unpacked (from a zip, a Flat OPC file or a converted
/// Word 2003 XML document).
pub(crate) fn read_opened(pkg: &Package) -> Result<Document, DocxError> {
    let root_rels = pkg.rels("");
    let main = root_rels.by_type(rt::OFFICE_DOC).filter(|r| !r.external).map(|r| r.target.clone()).unwrap_or_else(|| "word/document.xml".to_string());
    let Some(root) = pkg.xml_strict(&main)? else {
        return Err(DocxError::MissingPart(main));
    };
    if root.name != "w:document" {
        return Err(DocxError::NotWord(format!("root element is {}", root.name)));
    }
    let rels = pkg.rels(&main);
    let mut doc = Document::new();
    doc.body.clear();
    let mut r = Reader {
        pkg,
        doc,
        pc: PropCtx::default(),
        revs: HashMap::new(),
        moves: MoveCtx::default(),
        media_by_path: HashMap::new(),
        hf_by_path: HashMap::new(),
        hf_depth: 0,
        footnotes: HashMap::new(),
        endnotes: HashMap::new(),
        current_note: None,
        in_note_sep: false,
        comment_map: HashMap::new(),
        comments_ended: HashSet::new(),
        bookmarks: HashMap::new(),
        perms: HashMap::new(),
        graphic_parts: HashMap::new(),
        graphics: HashMap::new(),
        graphic_budget: MAX_GRAPHIC_WORK,
        glued: HashSet::new(),
        shape_ids: false,
        main: main.clone(),
        content_types: None,
        embedded: EmbeddedManifest::default(),
        txbx_links: Vec::new(),
        theme_fills: Default::default(),
        group_fill: None,
    };
    r.pc.major_font = r.doc.settings.major_font.clone();
    r.pc.minor_font = r.doc.settings.minor_font.clone();
    let part = |t: &str| rels.by_type(t).filter(|x| !x.external).map(|x| x.target.clone());

    // The font table and the fonts embedded through it ride along as opaque parts.
    if let Some(p) = part(rt::FONT_TABLE) {
        r.keep_part_tree(&p);
        r.read_font_families(&p);
    }
    if let Some(p) = part(rt::THEME) {
        r.read_theme(&p);
    }
    // Secondary parts are optional: a broken one is skipped (logged), not fatal.
    let lenient = |what: &str, res: Result<(), DocxError>| {
        if let Err(e) = res {
            log::warn!("docx: ignoring unreadable {what}: {e}");
        }
    };
    // Styles come before settings, but their theme colours already follow its colour mapping, and
    // their `…Bidi` theme fonts its theme font language.
    if let Some(p) = part(rt::SETTINGS) {
        r.read_color_mapping(&p);
    }
    if let Some(p) = part(rt::STYLES) {
        lenient("styles", r.read_styles(&p));
    }
    if let Some(p) = part(rt::NUMBERING) {
        lenient("numbering", r.read_numbering(&p));
    }
    // Without a stated mode (or settings at all) Word lays a document out as Word 2007 did.
    r.doc.settings.compat_mode = wordcraft_doc::LEGACY_COMPAT_MODE;
    if let Some(p) = part(rt::SETTINGS) {
        lenient("settings", r.read_settings(&p));
    }
    if let Some(p) = part(rt::FOOTNOTES) {
        lenient("footnotes", r.read_notes(&p, true));
    }
    if let Some(p) = part(rt::ENDNOTES) {
        lenient("endnotes", r.read_notes(&p, false));
    }
    if let Some(p) = part(rt::COMMENTS) {
        let ex = part(rt::COMMENTS_EX);
        let ids = part(rt::COMMENTS_IDS);
        let cex = part(rt::COMMENTS_EXTENSIBLE);
        lenient("comments", r.read_comments(&p, ex.as_deref(), ids.as_deref(), cex.as_deref()));
    }
    // Bibliography sources live in a custom XML data store (`b:Sources`), related from the
    // package root or the main part; Word's own files sometimes leave the relationship out, so
    // the conventional `customXml/itemN.xml` names are tried too. Each part is read at most once.
    let mut bib_parts: Vec<String> = Vec::new();
    for rel in root_rels.list.iter().chain(rels.list.iter()) {
        if rel_is(&rel.kind, rt::CUSTOM_XML) && !rel.external && !bib_parts.contains(&rel.target) {
            bib_parts.push(rel.target.clone());
            if bib_parts.len() >= MAX_BIBLIOGRAPHY_PARTS {
                break;
            }
        }
    }
    for i in 1..=16 {
        let path = format!("customXml/item{i}.xml");
        if bib_parts.len() < MAX_BIBLIOGRAPHY_PARTS && pkg.get(&path).is_some() && !bib_parts.contains(&path) {
            bib_parts.push(path);
        }
    }
    let mut seen_sources = Seen::default();
    for path in &bib_parts {
        if r.doc.sources.len() >= MAX_SOURCES {
            break;
        }
        lenient("customXml bibliography", r.read_bibliography(path, &mut seen_sources));
    }

    if let Some(bg) = root.child("w:background").and_then(|b| b.attr("w:color")).and_then(Rgb::parse) {
        r.doc.settings.page_color = Some(bg);
    }
    // A picture or texture fill: `w:background/v:background/v:fill` with a relationship id.
    if let Some(fill) = root.child("w:background").and_then(|b| b.child("v:background")).and_then(|b| b.child("v:fill")) {
        let rid = fill.attr("r:id").or_else(|| fill.attr("r:embed")).or_else(|| fill.attr("o:relid"));
        if let Some(media) = rid.and_then(|id| r.media_for(&rels, id)) {
            r.doc.settings.page_picture = Some(PagePicture {
                media,
                fill_type: fill.attr("type").unwrap_or("").chars().take(32).collect(),
                title: fill.attr("o:title").unwrap_or("").chars().take(255).collect(),
            });
        }
    }
    if let Some(body) = root.child("w:body") {
        let mut sc = StoryCtx::default();
        let mut blocks = Blocks::new();
        r.read_blocks(&mut sc, body, &rels, &mut blocks, 0);
        r.flush_pending(&mut sc, &mut blocks);
        r.doc.body = blocks;
        unstyle_toc_links(&mut r.doc);
        toc_control_to_toc(&mut r.doc.body);
        toc_field_to_heading(&mut r.doc.body);
        if let Some(s) = body.child("w:sectPr") {
            r.doc.last_section = r.read_section(s, &rels);
        }
    }
    // A watermark in the box Word's Design › Watermark gives it for the page keeps no box of its
    // own: it follows the page, as when it was made.
    let (pw, ph) = (r.doc.last_section.page_w, r.doc.last_section.page_h);
    if let Some(wm) = r.doc.settings.watermark.as_mut()
        && let Some([w, h]) = wm.size
    {
        let [dw, dh] = wordcraft_doc::Watermark { size: None, ..wm.clone() }.frame(pw, ph);
        if (w - dw).abs() < 0.06 && (h - dh).abs() < 0.06 {
            wm.size = None;
        }
    }

    let core = root_rels.by_type(rt::CORE).map(|x| x.target.clone()).unwrap_or_else(|| "docProps/core.xml".into());
    r.read_core(&core);
    if let Some(app) = root_rels.by_type(rt::APP).map(|x| x.target.clone()) {
        r.read_app(&app);
    }
    if let Some(c) = root_rels.by_type(rt::CUSTOM)
        && let Ok(Some(x)) = r.xml(&c.target)
    {
        r.doc.custom_props = crate::custom::read(&x);
    }
    // A macro project and every part it relates to (VBA data, signatures…) ride along as opaque
    // bytes, never parsed or run, so a .docm/.dotm saved again carries exactly what the file had.
    // Only the relationships and content types are read.
    if let Some(v) = part(rt::VBA_PROJECT)
        && let Some(b) = pkg.get_shared(&v).filter(|b| !b.is_empty())
    {
        r.doc.passthrough.insert(VBA_PROJECT_PART.into(), b.clone());
        r.read_vba_related(&v);
    }
    r.read_glossary(&rels);
    r.keep_custom_xml(&root_rels, &rels);
    r.keep_unmodelled(&root, &root_rels, &rels);
    if !r.embedded.parts.is_empty() || !r.embedded.dropped.is_empty() {
        let manifest = r.embedded.to_bytes();
        r.doc.passthrough.insert(EMBEDDED_PARTS.into(), Arc::new(manifest));
    }
    r.finish_moves();
    let links = std::mem::take(&mut r.txbx_links);
    link_text_boxes(&mut r.doc, links);
    if r.shape_ids {
        keep_glued_ids(&mut r.doc, &r.glued);
    }
    let mut doc = r.doc;
    doc.ensure_nonempty();
    Ok(doc)
}

/// Most move ranges open at once, and most move names tracked, per file.
const MAX_OPEN_MOVES: usize = 64;
const MAX_MOVES: usize = 100_000;

/// The move ranges of a file (ECMA-376 §17.13.5.22–27): a `w:moveFrom` / `w:moveTo` is one end
/// of a move when it sits inside a range of that kind; the range's name links both ends.
#[derive(Default)]
pub(crate) struct MoveCtx {
    /// Open source and destination ranges: (`w:id`, `w:name`).
    open: [Vec<(String, String)>; 2],
    /// Per name: a source range started / ended, a destination range started / ended (bits 0–3).
    seen: HashMap<String, u8>,
    /// The range of each kind that ended last in the paragraph being read (for its mark).
    pub closed_here: [Option<String>; 2],
}

impl Reader<'_> {
    /// A `w:moveFromRangeStart/End` or `w:moveToRangeStart/End`.
    pub fn move_range(&mut self, e: &El) {
        let (side, start) = match e.name.as_str() {
            "w:moveFromRangeStart" => (0, true),
            "w:moveFromRangeEnd" => (0, false),
            "w:moveToRangeStart" => (1, true),
            "w:moveToRangeEnd" => (1, false),
            _ => return,
        };
        let id = e.attr("w:id").unwrap_or("").chars().take(64).collect::<String>();
        let m = &mut self.moves;
        let Some(open) = m.open.get_mut(side) else { return };
        if start {
            let name = e.attr("w:name").unwrap_or("").chars().take(256).collect::<String>();
            if name.is_empty() || open.len() >= MAX_OPEN_MOVES || (m.seen.len() >= MAX_MOVES && !m.seen.contains_key(&name)) {
                return;
            }
            *m.seen.entry(name.clone()).or_default() |= 1 << (side * 2);
            open.push((id, name));
        } else if let Some(i) = open.iter().rposition(|(x, _)| *x == id) {
            let (_, name) = open.remove(i);
            *m.seen.entry(name.clone()).or_default() |= 2 << (side * 2);
            if let Some(c) = m.closed_here.get_mut(side) {
                *c = Some(name);
            }
        }
    }

    /// The revision of a `w:moveFrom` (`to` false) or `w:moveTo`: one end of the innermost open
    /// range of its kind, else (no range) a plain deletion or insertion.
    pub fn move_revision(&mut self, to: bool, e: &El, mark: bool) -> u32 {
        let side = usize::from(to);
        let open = self.moves.open.get(side).and_then(|o| o.last()).map(|(_, n)| n.clone());
        // A paragraph mark comes after its paragraph's text: a range that ended in it counts.
        let name = if mark { open.or_else(|| self.moves.closed_here.get(side).cloned().flatten()) } else { open };
        let kind = if to { RevisionKind::Insert } else { RevisionKind::Delete };
        self.revision_named(kind, e, name)
    }

    /// Moves missing an end (a source or destination range, or its text) are plain deletions
    /// and insertions.
    fn finish_moves(&mut self) {
        let mut ends: HashMap<&str, u8> = HashMap::new();
        for r in &self.doc.revisions {
            if let Some(n) = r.move_name.as_deref() {
                *ends.entry(n).or_default() |= if r.kind == RevisionKind::Delete { 1 } else { 2 };
            }
        }
        let ok: HashSet<String> =
            ends.into_iter().filter(|(n, e)| *e == 3 && self.moves.seen.get(*n) == Some(&0b1111)).map(|(n, _)| n.to_string()).collect();
        for r in &mut self.doc.revisions {
            if r.move_name.as_ref().is_some_and(|n| !ok.contains(n)) {
                r.move_name = None;
            }
        }
    }
}

/// Link the text boxes read as (id, seq, part) into chains (see [`Reader::txbx_links`]): each
/// id's boxes in `seq` order after the box holding the text. A repeated seq, a chain without its
/// first box or more than [`wordcraft_doc::MAX_LINKED_BOXES`] boxes: the extra boxes stay apart.
fn link_text_boxes(doc: &mut Document, mut links: Vec<(u32, u32, u32)>) {
    links.sort_by_key(|l| (l.0, l.1));
    links.dedup_by_key(|l| (l.0, l.1));
    for group in links.chunk_by(|a, b| a.0 == b.0) {
        if group.first().is_none_or(|l| l.1 != 0) {
            continue;
        }
        let chain: Vec<u32> = group.iter().take(wordcraft_doc::MAX_LINKED_BOXES).map(|l| l.2).collect();
        for pair in chain.windows(2) {
            if let [a, b] = pair
                && let Some(p) = doc.parts.get_mut(a)
            {
                p.body.next = Some(*b);
            }
        }
    }
}

/// Drop the drawing ids read with shapes, except those connectors are glued to (so a shape
/// without connectors reads back as it was written).
fn keep_glued_ids(doc: &mut Document, glued: &HashSet<u32>) {
    let has_id = |o: &InlineObject| match o {
        InlineObject::Shape { extra, .. } => extra.id != 0,
        InlineObject::Group { children, .. } => children.iter().any(|c| matches!(&c.obj, InlineObject::Shape { extra, .. } if extra.id != 0)),
        _ => false,
    };
    let clear = |o: &mut InlineObject| {
        if let InlineObject::Shape { extra, .. } = o
            && !glued.contains(&extra.id)
        {
            extra.id = 0;
        }
    };
    doc.objects_mut(&has_id, &mut |o| match o {
        InlineObject::Group { children, .. } => children.iter_mut().for_each(|c| clear(&mut c.obj)),
        o => clear(o),
    });
}

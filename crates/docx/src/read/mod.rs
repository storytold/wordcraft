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
use crate::package::{ContentTypes, EMBEDDED_PARTS, EmbeddedManifest, MAX_VBA_RELATED, Package, Rels, VBA_PROJECT_PART, VBA_RELATED, rel_is, rt};
use crate::units::{flag, int, on_off, tw, u32_of};
use crate::xml::El;
pub(crate) use props::PropCtx;
use story::StoryCtx;

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
/// Bibliography sources kept from a file (Word's own source manager holds far fewer).
const MAX_SOURCES: usize = 10_000;
/// Custom XML parts looked at for bibliography sources.
const MAX_BIBLIOGRAPHY_PARTS: usize = 64;
/// Characters kept of a bibliography's citation style name.
const MAX_STYLE_NAME: usize = 64;

/// Bibliography sources already read: tags, and (title, author) of untagged ones.
#[derive(Default)]
struct BibSeen {
    tags: HashSet<String>,
    untagged: HashSet<(String, String)>,
}

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
    let Some(root) = pkg.xml(&main)? else {
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
    // Styles come before settings, but their theme colours already follow its colour mapping.
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
    let mut seen_sources = BibSeen::default();
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
        toc_field_to_heading(&mut r.doc.body);
        if let Some(s) = body.child("w:sectPr") {
            r.doc.last_section = r.read_section(s, &rels);
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
        && let Some(b) = pkg.get(&v).filter(|b| !b.is_empty())
    {
        r.doc.passthrough.insert(VBA_PROJECT_PART.into(), Arc::new(b.to_vec()));
        r.read_vba_related(&v);
    }
    r.read_glossary(&rels);
    r.keep_custom_xml(&root_rels, &rels);
    r.keep_unmodelled(&root_rels, &rels);
    if !r.embedded.parts.is_empty() {
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

/// Word draws a table of contents' entry links without the Hyperlink character style (no blue,
/// no underline) though the entries carry it (measured in Word for Windows: a Hyperlink-styled run
/// in a hyperlink inside a TOC field's result is plain; one outside a hyperlink, another style,
/// direct formatting, or a hyperlink in another field keep their look). The style is dropped from
/// those runs so they look the same here; the links stay.
fn unstyle_toc_links(doc: &mut Document) {
    for f in doc.field_ranges(wordcraft_doc::StoryRef::Body) {
        if is_toc(&f.instr) {
            let _ = doc.format_range(&f.start, &f.end, &|c| {
                if c.link.is_some() && c.style.as_deref() == Some("Hyperlink") {
                    c.style = None;
                }
            });
        }
    }
}

/// Normalize only a complete TOC range that the engine's heading/entry representation can
/// reproduce exactly. Other imported TOCs keep their explicit boundaries, independent of styles.
fn toc_field_to_heading(body: &mut Blocks) {
    for i in 1..body.len() {
        let heading = body[i - 1].as_para().filter(|p| p.props.style.as_deref() == Some("TOCHeading"));
        let has_toc = |o: &InlineObject| matches!(o, InlineObject::Field { instr, .. } if is_toc(instr));
        if heading.is_none_or(|h| h.objects.iter().any(has_toc)) {
            continue;
        }
        let Some((field, props, end)) = body[i].as_para().and_then(|p| {
            let (field, end) = match p.object_at(0) {
                Some(InlineObject::FieldStart { instr, locked, dirty }) if is_toc(instr) => (
                    InlineObject::Field {
                        instr: instr.clone(),
                        result: String::new(),
                        locked: *locked,
                        dirty: *dirty,
                        code: false,
                        form: String::new(),
                    },
                    Some(toc_range_end(body, i)?),
                ),
                Some(f @ InlineObject::Field { result, .. }) if result.is_empty() && has_toc(f) => (f.clone(), None),
                _ => return None,
            };
            Some((field, p.props_of_char(0).clone(), end))
        }) else {
            continue;
        };
        // Delete the end first: the two markers may occupy the same paragraph.
        if let Some(Block::Para(p)) = end.and_then(|end| body.get_mut(end)).map(Arc::make_mut) {
            let off = p.len().saturating_sub(OBJ.len_utf8());
            let _ = p.delete(off, p.len());
        }
        if let Some(Block::Para(p)) = body.get_mut(i).map(Arc::make_mut) {
            let _ = p.delete(0, OBJ.len_utf8());
        }
        if let Some(Block::Para(h)) = body.get_mut(i - 1).map(Arc::make_mut) {
            let end = h.len();
            let _ = h.insert_object(end, field, &props);
        }
    }
}

fn is_toc(instr: &str) -> bool {
    instr.split_whitespace().next().is_some_and(|k| k.eq_ignore_ascii_case("TOC"))
}

/// The actual matching end must also be the end the styled writer would choose: at the end
/// of the last contiguous TOC entry, with no section break or trailing content outside it.
fn toc_range_end(body: &Blocks, start: usize) -> Option<usize> {
    let entry =
        |p: &wordcraft_doc::Paragraph| p.section.is_none() && p.props.style.as_deref().is_some_and(|s| s.starts_with("TOC") && s != "TOCHeading");
    let mut depth = 0_usize;
    for (i, b) in body.iter().enumerate().skip(start) {
        let p = b.as_para().filter(|p| entry(p))?;
        for (off, o) in p.object_offsets().into_iter().zip(&p.objects) {
            match o {
                InlineObject::FieldStart { .. } => depth = depth.checked_add(1)?,
                InlineObject::FieldEnd => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        let following_entry = body.get(i + 1).and_then(|b| b.as_para()).is_some_and(entry);
                        return (off.checked_add(OBJ.len_utf8()) == Some(p.len()) && !following_entry).then_some(i);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Built-in style names Word stores in lower case, and how the UI shows them.
pub(crate) const LOWER_NAMES: &[(&str, &str)] = &[
    ("caption", "Caption"),
    ("header", "Header"),
    ("footer", "Footer"),
    ("footnote text", "Footnote Text"),
    ("footnote reference", "Footnote Reference"),
    ("endnote text", "Endnote Text"),
    ("endnote reference", "Endnote Reference"),
    ("annotation text", "Annotation Text"),
    ("annotation reference", "Annotation Reference"),
    ("annotation subject", "Annotation Subject"),
    ("table of figures", "Table of Figures"),
    ("normal indent", "Normal Indent"),
    ("line number", "Line Number"),
    ("page number", "Page Number"),
    ("index heading", "Index Heading"),
    ("envelope address", "Envelope Address"),
    ("envelope return", "Envelope Return"),
    ("toa heading", "TOA Heading"),
    ("table of authorities", "Table of Authorities"),
];

/// File style name → display name.
pub(crate) fn ui_style_name(n: &str) -> String {
    if let Some((_, ui)) = LOWER_NAMES.iter().find(|(f, _)| *f == n) {
        return (*ui).to_string();
    }
    for (pre, ui) in [("heading ", "Heading "), ("toc ", "TOC "), ("index ", "Index ")] {
        if let Some(d) = n.strip_prefix(pre)
            && d.len() == 1
            && d.chars().all(|c| c.is_ascii_digit())
        {
            return format!("{ui}{d}");
        }
    }
    n.to_string()
}

impl Reader<'_> {
    /// Keep the parts the VBA project at `project` relates to, with their relationship types and
    /// content types (see [`VBA_RELATED`]). Parts outside `word/`, relationship parts, the project
    /// itself and missing targets are skipped (logged); at most [`MAX_VBA_RELATED`] are kept.
    fn read_vba_related(&mut self, project: &str) {
        let pkg = self.pkg;
        let types = self.content_types.get_or_insert_with(|| ContentTypes::read(pkg));
        let mut manifest = String::new();
        let mut kept = 0usize;
        for rel in self.pkg.rels(project).list.iter().filter(|r| !r.external) {
            // Part names compare case-insensitively (OPC): `/WORD/x` names the part the writer
            // stores as `word/x`.
            let target = rel.target.as_str();
            let path = match (target.get(..5), target.get(5..)) {
                (Some(dir), Some(rest)) if dir.eq_ignore_ascii_case("word/") => format!("word/{rest}"),
                _ => target.to_string(),
            };
            let path = path.as_str();
            let ct = types.of(path).unwrap_or("application/octet-stream");
            let usable = path.strip_prefix("word/").is_some_and(|t| !t.is_empty())
                && !path.to_ascii_lowercase().ends_with(".rels")
                && !path.eq_ignore_ascii_case(project)
                && !path.eq_ignore_ascii_case(VBA_PROJECT_PART)
                && ![rel.kind.as_str(), path, ct].iter().any(|f| f.is_empty() || f.contains(['\t', '\r', '\n']));
            if !usable {
                log::warn!("docx: not carrying VBA-related part {path:?}");
                continue;
            }
            let Some(bytes) = self.pkg.get(path) else {
                log::warn!("docx: VBA-related part {path} is missing; keeping the project without it");
                continue;
            };
            if kept >= MAX_VBA_RELATED {
                log::warn!("docx: more than {MAX_VBA_RELATED} VBA-related parts; the rest are dropped");
                break;
            }
            kept += 1;
            manifest.push_str(&format!("{}\t{path}\t{ct}\n", rel.kind));
            self.doc.passthrough.entry(path.to_string()).or_insert_with(|| Arc::new(bytes.to_vec()));
        }
        if !manifest.is_empty() {
            self.doc.passthrough.insert(VBA_RELATED.into(), Arc::new(manifest.into_bytes()));
        }
    }

    /// The fonts the font table classes as roman (`w:font/w:family w:val="roman"`).
    fn read_font_families(&mut self, path: &str) {
        let Ok(Some(table)) = self.xml(path) else { return };
        self.doc.settings.roman_fonts = table
            .children("w:font")
            .filter(|f| f.child("w:family").and_then(|v| v.attr("w:val")) == Some("roman"))
            .filter_map(|f| f.attr("w:name"))
            .filter(|n| !n.is_empty() && n.len() <= MAX_FONT_NAME)
            .take(MAX_TABLE_FONTS)
            .map(str::to_string)
            .collect();
    }

    fn xml(&self, path: &str) -> Result<Option<El>, DocxError> {
        self.pkg.xml(path)
    }

    pub fn revision(&mut self, kind: RevisionKind, e: &El) -> u32 {
        self.revision_named(kind, e, None)
    }

    fn revision_named(&mut self, kind: RevisionKind, e: &El, move_name: Option<String>) -> u32 {
        let author = e.attr("w:author").unwrap_or("").chars().take(256).collect::<String>();
        let date = e.attr("w:date").unwrap_or("").chars().take(64).collect::<String>();
        let k = match kind {
            RevisionKind::Insert => 0,
            RevisionKind::Delete => 1,
            RevisionKind::Format => 2,
        };
        let key = (k, author.clone(), date.clone(), move_name.clone());
        if let Some(i) = self.revs.get(&key) {
            return *i;
        }
        let i = self.doc.revisions.len() as u32;
        self.doc.revisions.push(Revision { kind, author, date, move_name });
        self.revs.insert(key, i);
        i
    }

    /// Media key for an image relationship (loads the bytes once).
    pub fn media_for(&mut self, rels: &Rels, id: &str) -> Option<String> {
        let rel = rels.by_id(id)?;
        if rel.external {
            return None;
        }
        if let Some(k) = self.media_by_path.get(&rel.target) {
            return Some(k.clone());
        }
        let bytes = self.pkg.get(&rel.target)?;
        let base: String =
            rel.target.rsplit('/').next().unwrap_or("image").chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).collect();
        let base = if base.is_empty() || base.starts_with('.') { format!("image{base}") } else { base };
        let mut key = base.clone();
        let mut n = 1;
        while self.doc.media.contains_key(&key) {
            n += 1;
            key = match base.rsplit_once('.') {
                Some((stem, ext)) => format!("{stem}_{n}.{ext}"),
                None => format!("{base}_{n}"),
            };
        }
        self.doc.media.insert(key.clone(), Arc::new(bytes.to_vec()));
        self.media_by_path.insert(rel.target.clone(), key.clone());
        Some(key)
    }

    /// A chart or diagram part, parsed once however often it's referenced (`None` when unreadable).
    fn graphic_part(&mut self, path: &str) -> Option<Arc<El>> {
        if let Some(p) = self.graphic_parts.get(path) {
            return p.clone();
        }
        let p = self.xml(path).ok().flatten().map(Arc::new);
        self.graphic_parts.insert(path.to_string(), p.clone());
        p
    }

    pub fn load_header_footer(&mut self, path: &str, footer: bool) -> Option<u32> {
        if let Some(id) = self.hf_by_path.get(path) {
            return Some(*id);
        }
        if self.doc.parts.len() >= MAX_PARTS || self.hf_depth >= MAX_HF_DEPTH {
            return None;
        }
        // Reserve the id first so a (malformed) self-reference can't recurse.
        let kind = if footer { PartKind::Footer } else { PartKind::Header };
        let id = self.doc.add_part(kind, Blocks::new());
        self.hf_by_path.insert(path.to_string(), id);
        let root = match self.xml(path) {
            Ok(Some(r)) => r,
            _ => return Some(id),
        };
        let rels = self.pkg.rels(path);
        let mut sc = StoryCtx::default();
        sc.in_hf = true;
        let mut blocks = Blocks::new();
        self.hf_depth += 1;
        self.read_blocks(&mut sc, &root, &rels, &mut blocks, 0);
        self.hf_depth = self.hf_depth.saturating_sub(1);
        self.flush_pending(&mut sc, &mut blocks);
        if let Some(p) = self.doc.parts.get_mut(&id)
            && !blocks.is_empty()
        {
            p.blocks = blocks;
        }
        Some(id)
    }

    fn read_theme(&mut self, path: &str) {
        let Ok(Some(t)) = self.xml(path) else { return };
        if let Some(n) = t.attr("name").filter(|n| !n.is_empty()) {
            self.doc.settings.theme_name = n.to_string();
        }
        let Some(el) = t.child("a:themeElements") else { return };
        if let Some(cs) = el.child("a:clrScheme") {
            let names = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];
            let mut colors = self.doc.settings.theme_colors.clone();
            colors.resize(12, Rgb::BLACK);
            for (i, n) in names.iter().enumerate() {
                let Some(c) = cs.child(&format!("a:{n}")) else { continue };
                let rgb = c
                    .child("a:srgbClr")
                    .and_then(|x| x.attr("val"))
                    .and_then(Rgb::parse)
                    .or_else(|| c.child("a:sysClr").and_then(|x| x.attr("lastClr")).and_then(Rgb::parse));
                if let (Some(rgb), Some(slot)) = (rgb, colors.get_mut(i)) {
                    *slot = rgb;
                }
            }
            self.pc.theme_colors = colors.clone();
            self.doc.settings.theme_colors = colors;
        }
        if let Some(fs) = el.child("a:fontScheme") {
            let face =
                |n: &str| fs.child(n).and_then(|f| f.child("a:latin")).and_then(|l| l.attr("typeface")).filter(|t| !t.is_empty()).map(str::to_string);
            if let Some(f) = face("a:majorFont") {
                self.doc.settings.major_font = f;
            }
            if let Some(f) = face("a:minorFont") {
                self.doc.settings.minor_font = f;
            }
            let cs = |n: &str| fs.child(n).and_then(|f| f.child("a:cs")).and_then(|c| c.attr("typeface")).unwrap_or_default().to_string();
            self.pc.major_cs = cs("a:majorFont");
            self.pc.minor_cs = cs("a:minorFont");
        }
        self.pc.major_font = self.doc.settings.major_font.clone();
        self.pc.minor_font = self.doc.settings.minor_font.clone();
        self.pc.theme = self.doc.settings.theme_colors.clone();
    }

    /// `w:settings/w:clrSchemeMapping`: which scheme colour text1, background1… stand for.
    fn read_color_mapping(&mut self, path: &str) {
        let Ok(Some(root)) = self.xml(path) else { return };
        let Some(m) = root.child("w:clrSchemeMapping") else { return };
        let names = [
            "dark1",
            "light1",
            "dark2",
            "light2",
            "accent1",
            "accent2",
            "accent3",
            "accent4",
            "accent5",
            "accent6",
            "hyperlink",
            "followedHyperlink",
        ];
        let attrs = [
            "w:t1",
            "w:bg1",
            "w:t2",
            "w:bg2",
            "w:accent1",
            "w:accent2",
            "w:accent3",
            "w:accent4",
            "w:accent5",
            "w:accent6",
            "w:hyperlink",
            "w:followedHyperlink",
        ];
        let mut map = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        for (slot, a) in map.iter_mut().zip(attrs) {
            if let Some(i) = m.attr(a).and_then(|v| names.iter().position(|n| *n == v)) {
                *slot = i;
            }
        }
        self.pc.color_map = Some(map);
    }

    fn read_styles(&mut self, path: &str) -> Result<(), DocxError> {
        /// Largest `w:latentStyles` kept (Word's has ~380 exceptions, ~40 KB).
        const MAX_LATENT_STYLES_XML: usize = 1 << 20;
        let Some(root) = self.xml(path)? else { return Ok(()) };
        // The default paragraph style must be called `Normal` in our model.
        let has_normal = root.children("w:style").any(|s| s.attr("w:styleId") == Some("Normal"));
        if !has_normal
            && let Some(def) = root
                .children("w:style")
                .find(|s| s.attr("w:type") == Some("paragraph") && s.attr("w:default").is_some_and(|v| v == "1" || v == "true"))
            && let Some(id) = def.attr("w:styleId")
        {
            self.pc.alias.insert(id.to_string(), "Normal".into());
        }
        let mut sheet = StyleSheet::empty();
        sheet.default_para_present = root.child("w:docDefaults").and_then(|dd| dd.child("w:pPrDefault")).is_some();
        if let Some(dd) = root.child("w:docDefaults") {
            if let Some(r) = dd.child("w:rPrDefault").and_then(|d| d.child("w:rPr")) {
                sheet.default_chr = self.pc.rpr(r);
            }
            if let Some(p) = dd.child("w:pPrDefault").and_then(|d| d.child("w:pPr")) {
                sheet.default_para = self.pc.ppr(p).0;
            }
        }
        if sheet.default_chr.size.is_none() {
            sheet.default_chr.size = Some(10.0);
        }
        // Built-in styles' behaviour (gallery, priority, visibility), written back as is.
        sheet.latent_styles = root.child("w:latentStyles").map(|l| l.to_xml()).filter(|x| x.len() <= MAX_LATENT_STYLES_XML);
        for s in root.children("w:style").take(10_000) {
            let Some(raw_id) = s.attr("w:styleId").filter(|i| !i.is_empty()) else { continue };
            let id = self.pc.style_id(raw_id);
            if sheet.get(&id).is_some() {
                continue;
            }
            let kind = match s.attr("w:type").unwrap_or("paragraph") {
                "character" => StyleKind::Character,
                "table" => StyleKind::Table,
                "numbering" => StyleKind::Numbering,
                _ => StyleKind::Paragraph,
            };
            let name = s.child_val("w:name").map(ui_style_name).unwrap_or_else(|| id.clone());
            let r = |n: &str| s.child_val(n).filter(|v| !v.is_empty()).map(|v| self.pc.style_id(v));
            let mut st = Style {
                id: id.clone(),
                name,
                kind,
                based_on: r("w:basedOn").filter(|b| *b != id),
                next: r("w:next"),
                linked: r("w:link"),
                priority: s.child_val("w:uiPriority").and_then(u32_of),
                quick: flag(s, "w:qFormat").unwrap_or(false),
                hidden: flag(s, "w:semiHidden").unwrap_or(false) || flag(s, "w:hidden").unwrap_or(false),
                builtin: !s.attr("w:customStyle").is_some_and(|v| v == "1" || v == "true"),
                locked: flag(s, "w:locked").unwrap_or(false),
                // Only where the writer couldn't tell from the id (it flags the English ids).
                default: s.attr("w:default").is_some_and(|v| v == "1" || v == "true")
                    && !matches!(
                        (kind, id.as_str()),
                        (StyleKind::Paragraph, "Normal")
                            | (StyleKind::Character, "DefaultParagraphFont")
                            | (StyleKind::Table, "TableNormal")
                            | (StyleKind::Numbering, "NoList")
                    ),
                ..Default::default()
            };
            if let Some(p) = s.child("w:pPr") {
                st.para = self.pc.ppr(p).0;
            }
            if let Some(rp) = s.child("w:rPr") {
                st.chr = self.pc.rpr(rp);
            }
            if kind == StyleKind::Table {
                let parts = self.table_style_parts(s, &mut st);
                st.table = (parts != TableStyleParts::default()).then_some(parts);
            }
            sheet.styles.push(st);
        }
        // Styles that give only a list level take the list from the style they are based on
        // (Heading 3: level 3 of Heading 2's outline list).
        let fixed: Vec<Option<wordcraft_doc::props::NumRef>> = sheet
            .styles
            .iter()
            .map(|s| match s.para.numbering {
                Some(n) if n.num == props::LEVEL_ONLY => {
                    let base = s.based_on.as_deref().and_then(|b| props::chain_list(&sheet, b));
                    base.map(|num| wordcraft_doc::props::NumRef { num, level: n.level })
                }
                other => other,
            })
            .collect();
        for (s, n) in sheet.styles.iter_mut().zip(fixed) {
            s.para.numbering = n;
        }
        if sheet.default_para.numbering.is_some_and(|n| n.num == props::LEVEL_ONLY) {
            sheet.default_para.numbering = None;
        }
        // A producer that never writes `w:qFormat` (LibreOffice) still gets Word's usual gallery.
        if !sheet.styles.iter().any(|s| s.quick) {
            sheet.add_default_gallery();
        }
        self.doc.styles = sheet;
        Ok(())
    }

    /// A table style's table-level formatting and conditional formats. The `wholeTable` region
    /// applies to every cell, so its paragraph and run properties go to the style's own (`st`).
    fn table_style_parts(&self, s: &El, st: &mut Style) -> TableStyleParts {
        let mut t = TableStyleParts::default();
        if let Some(tp) = s.child("w:tblPr") {
            if let Some(b) = tp.child("w:tblBorders") {
                t.borders = Some(props::borders(b));
            }
            t.fill = tp.child("w:shd").and_then(|e| self.pc.shd_fill(e));
            t.cell_margins = tp.child("w:tblCellMar").map(props::margins);
            t.band_size = tp.child_val("w:tblStyleRowBandSize").and_then(u32_of).filter(|n| *n > 0).map(|n| n.min(1000));
        }
        for c in s.children("w:tblStylePr").take(64) {
            let Some(part) = c.attr("w:type").and_then(TablePart::from_ooxml) else { continue };
            let tc = c.child("w:tcPr");
            let ppr = c.child("w:pPr").map(|p| self.pc.ppr(p).0);
            let f = PartFormat {
                chr: c.child("w:rPr").map(|r| self.pc.rpr(r)).unwrap_or_default(),
                fill: tc.and_then(|p| p.child("w:shd")).and_then(|e| self.pc.shd_fill(e)),
                borders: tc.and_then(|p| p.child("w:tcBorders")).map(props::borders),
                align: ppr.as_ref().and_then(|p| p.align),
                v_align: tc.and_then(|p| p.child_val("w:vAlign")).map(|v| match v {
                    "center" => VAlign::Center,
                    "bottom" => VAlign::Bottom,
                    _ => VAlign::Top,
                }),
            };
            if part == TablePart::WholeTable {
                // The whole table's paragraph and run properties are the style's own; its
                // borders the table borders.
                if let Some(p) = ppr {
                    st.para.overlay(&p);
                }
                st.chr.overlay(&f.chr);
                if f.fill.is_some() {
                    t.fill = f.fill;
                }
                if let Some(b) = c.child("w:tblPr").and_then(|p| p.child("w:tblBorders")) {
                    t.borders = Some(props::borders(b));
                }
                if f.v_align.is_some() {
                    t.regions.insert(part, PartFormat { v_align: f.v_align, ..PartFormat::default() });
                }
            } else {
                t.set_part(part, f);
            }
        }
        t
    }

    fn read_numbering(&mut self, path: &str) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let mut abstracts = Vec::new();
        let mut links: Vec<(u32, String)> = Vec::new();
        for a in root.children("w:abstractNum").take(10_000) {
            let Some(id) = a.attr("w:abstractNumId").and_then(u32_of) else { continue };
            let mut levels: Vec<Level> = (0..9).map(default_level).collect();
            for l in a.children("w:lvl") {
                let i = l.attr("w:ilvl").and_then(u32_of).unwrap_or(0) as usize;
                if let Some(slot) = levels.get_mut(i) {
                    *slot = self.level(l, i);
                }
            }
            if let Some(link) = a.child_val("w:numStyleLink") {
                links.push((id, link.to_string()));
            }
            let hex = |v: &str| v.len() == 8 && v.bytes().all(|b| b.is_ascii_hexdigit());
            abstracts.push(AbstractNum {
                id,
                name: a.child_val("w:name").map(str::to_string),
                levels,
                nsid: a.child_val("w:nsid").filter(|v| hex(v)).map(str::to_ascii_uppercase),
                tmpl: a.child_val("w:tmpl").filter(|v| hex(v)).map(str::to_ascii_uppercase),
                multi_level_type: a
                    .child_val("w:multiLevelType")
                    // `multilevel` is what the writer derives for the nine levels read here.
                    .filter(|v| matches!(*v, "singleLevel" | "hybridMultilevel"))
                    .map(str::to_string),
            });
        }
        let mut nums = Vec::new();
        for n in root.children("w:num").take(10_000) {
            let (Some(id), Some(abs)) = (n.attr("w:numId").and_then(u32_of), n.child_val("w:abstractNumId").and_then(u32_of)) else { continue };
            let (mut start_overrides, mut level_overrides) = (Vec::new(), Vec::new());
            for o in n.children("w:lvlOverride").take(9) {
                let lvl = o.attr("w:ilvl").and_then(u32_of).unwrap_or(0).min(8) as u8;
                if let Some(s) = o.child_val("w:startOverride").and_then(u32_of) {
                    start_overrides.push((lvl, s));
                } else if let Some(s) = o.child("w:lvl").and_then(|l| l.child_val("w:start")).and_then(u32_of) {
                    start_overrides.push((lvl, s));
                }
                // A whole level definition replaces the abstract list's for this list.
                if let Some(l) = o.child("w:lvl") {
                    level_overrides.push((lvl, self.level(l, lvl as usize)));
                }
            }
            nums.push(Num { id, abstract_id: abs, start_overrides, level_overrides });
        }
        // Abstracts that only link to a numbering style take that style's list levels.
        for (aid, style) in links {
            let num = self.doc.styles.get(&self.pc.style_id(&style)).and_then(|s| s.para.numbering).map(|n| n.num);
            let src = num.and_then(|n| nums.iter().find(|x: &&Num| x.id == n)).map(|x| x.abstract_id);
            let levels = src.filter(|s| *s != aid).and_then(|s| abstracts.iter().find(|a: &&AbstractNum| a.id == s)).map(|a| a.levels.clone());
            if let (Some(levels), Some(a)) = (levels, abstracts.iter_mut().find(|a| a.id == aid)) {
                a.levels = levels;
            }
        }
        self.doc.numbering.abstracts = abstracts;
        self.doc.numbering.nums = nums;
        Ok(())
    }

    fn level(&self, l: &El, i: usize) -> Level {
        let mut lv = default_level(i);
        if let Some(s) = l.child_val("w:start").and_then(u32_of) {
            lv.start = s;
        }
        if let Some(f) = l.child_val("w:numFmt") {
            lv.format = NumFormat::from_ooxml(f);
        }
        lv.text = l.child_val("w:lvlText").map(|t| t.chars().take(64).collect()).unwrap_or_default();
        if let Some(j) = l.child_val("w:lvlJc") {
            lv.align = props::align(j);
        }
        if let Some(ind) = l.child("w:pPr").and_then(|p| p.child("w:ind")) {
            if let Some(v) = tw(ind, "w:left").or_else(|| tw(ind, "w:start")) {
                lv.indent = v;
            }
            if let Some(h) = tw(ind, "w:hanging") {
                lv.hanging = h;
            } else if let Some(f) = tw(ind, "w:firstLine") {
                lv.hanging = -f;
            }
        }
        if let Some(r) = l.child("w:rPr") {
            lv.chr = self.pc.rpr(r);
        }
        lv.suffix = match l.child_val("w:suff") {
            Some("space") => LevelSuffix::Space,
            Some("nothing") => LevelSuffix::Nothing,
            _ => LevelSuffix::Tab,
        };
        lv.legal = flag(l, "w:isLgl").unwrap_or(false);
        let restart = l.child_val("w:lvlRestart").and_then(int);
        lv.restart = restart.is_none_or(|v| v != 0);
        lv.restart_after = restart.filter(|v| (1..=9).contains(v)).map(|v| v as u8);
        // The tab stop after the number ("num" tab in the level's paragraph properties).
        lv.tab = l
            .child("w:pPr")
            .and_then(|p| p.child("w:tabs"))
            .and_then(|t| t.children("w:tab").find(|t| t.attr("w:val") == Some("num")))
            .and_then(|t| tw(t, "w:pos"))
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(-1584.0, 1584.0));
        lv.style = l.child_val("w:pStyle").filter(|s| !s.is_empty()).map(|s| self.pc.style_id(s));
        lv
    }

    fn read_settings(&mut self, path: &str) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        if let Some(b) = self.pkg.get(path).filter(|b| crate::write::settings_has_unowned(b)) {
            self.doc.passthrough.insert(crate::package::SETTINGS_ORIG.into(), Arc::new(b.to_vec()));
        }
        let rels = self.pkg.rels(path);
        let s = &mut self.doc.settings;
        for k in root.els() {
            match k.name.as_str() {
                // §17.15.1.6: the template, by a relationship (usually external) of the settings part.
                "w:attachedTemplate" => {
                    s.attached_template = k
                        .attr("r:id")
                        .and_then(|id| rels.by_id(id))
                        .map(|r| r.target.trim().to_string())
                        .filter(|t| !t.is_empty() && t.len() <= MAX_TEMPLATE_PATH);
                }
                "w:linkStyles" => s.link_styles = on_off(k),
                "w:removePersonalInformation" => s.remove_personal_info = on_off(k),
                "w:removeDateAndTime" => s.remove_date_time = on_off(k),
                "w:compat" => {
                    s.allow_space_of_same_style_in_table = k.child("w:allowSpaceOfSameStyleInTable").is_some_and(on_off);
                    // Only Word's own setting: another `w:uri` names a different application's.
                    let mode = k
                        .children("w:compatSetting")
                        .find(|c| {
                            c.attr("w:name") == Some("compatibilityMode")
                                && c.attr("w:uri").is_none_or(|u| u == "http://schemas.microsoft.com/office/word")
                        })
                        .and_then(|c| c.attr("w:val"));
                    if let Some(m) = mode.and_then(u32_of) {
                        s.compat_mode = m.clamp(11, 99);
                    }
                    s.sum_paragraph_spacing = k.children("w:doNotUseHTMLParagraphAutoSpacing").any(on_off);
                }
                "w:trackRevisions" => s.track_changes = on_off(k),
                "w:defaultTabStop" => {
                    if let Some(v) = tw(k, "w:val").filter(|v| *v > 0.0) {
                        s.default_tab = v.clamp(1.0, 1584.0);
                    }
                }
                "w:evenAndOddHeaders" => s.even_odd_headers = on_off(k),
                "w:drawingGridHorizontalSpacing" | "w:drawingGridVerticalSpacing" => {
                    if let Some(v) = tw(k, "w:val").filter(|v| *v > 0.0) {
                        let v = v.clamp(0.5, 1584.0);
                        if k.name == "w:drawingGridHorizontalSpacing" { s.grid_h = v } else { s.grid_v = v }
                    }
                }
                "m:mathPr" => s.math = Some(math::read_math_pr(k)),
                "w:mirrorMargins" => s.mirror_margins = on_off(k),
                "w:autoHyphenation" => s.auto_hyphenation = on_off(k),
                "w:doNotHyphenateCaps" => s.hyphenate_caps = !on_off(k),
                "w:hyphenationZone" => {
                    if let Some(v) = tw(k, "w:val").filter(|v| v.is_finite() && *v >= 0.0) {
                        s.hyphenation_zone = v.min(1584.0);
                    }
                }
                "w:consecutiveHyphenLimit" => {
                    if let Some(v) = k.attr("w:val").and_then(u32_of) {
                        s.consecutive_hyphen_limit = v.min(32_767);
                    }
                }
                "w:footnotePr" => {
                    if let Some(f) = k.child_val("w:numFmt") {
                        s.footnote_format = NumFormat::from_ooxml(f);
                    }
                    s.footnote_pr = props::note_pr(k, false);
                }
                "w:endnotePr" => {
                    if let Some(f) = k.child_val("w:numFmt") {
                        s.endnote_format = NumFormat::from_ooxml(f);
                    }
                    s.endnote_pr = props::note_pr(k, false);
                }
                "w:documentProtection" => read_protection(k, s),
                "w:zoom" => {
                    let percent = k.attr("w:percent").and_then(|v| u32_of(v.trim_end_matches('%'))).unwrap_or(100).clamp(10, 500);
                    let fit = match k.attr("w:val") {
                        Some("fullPage") => Some(ZoomFit::OnePage),
                        Some("bestFit") => Some(ZoomFit::PageWidth),
                        Some("textFit") => Some(ZoomFit::TextWidth),
                        _ => None,
                    };
                    s.zoom = Some(Zoom { percent, fit }).filter(|z| z.percent != 100 || z.fit.is_some());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn read_notes(&mut self, path: &str, foot: bool) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let rels = self.pkg.rels(path);
        let tag = if foot { "w:footnote" } else { "w:endnote" };
        for n in root.children(tag) {
            match n.attr("w:type") {
                None | Some("normal") => {}
                Some(t @ ("separator" | "continuationSeparator" | "continuationNotice")) => {
                    self.read_note_sep(n, &rels, foot, t);
                    continue;
                }
                Some(_) => continue,
            }
            let Some(fid) = n.attr("w:id").and_then(int) else { continue };
            if self.doc.parts.len() >= MAX_PARTS {
                break;
            }
            // Reserve the id first: the note's own reference mark points at it.
            let (pk, kind) = if foot { (PartKind::Footnote, NoteKind::Footnote) } else { (PartKind::Endnote, NoteKind::Endnote) };
            let id = self.doc.add_part(pk, Blocks::new());
            let mut sc = StoryCtx::default();
            let mut blocks = Blocks::new();
            self.current_note = Some((kind, id));
            self.read_blocks(&mut sc, n, &rels, &mut blocks, 0);
            self.flush_pending(&mut sc, &mut blocks);
            self.current_note = None;
            if let Some(p) = self.doc.parts.get_mut(&id)
                && !blocks.is_empty()
            {
                p.blocks = blocks;
            }
            if foot {
                self.footnotes.insert(fid, id);
            } else {
                self.endnotes.insert(fid, id);
            }
        }
        Ok(())
    }

    /// A document's own separator, continuation separator or continuation notice (`kind`, the
    /// note's `w:type`) for its footnotes or endnotes. The first of each kind counts.
    fn read_note_sep(&mut self, n: &El, rels: &Rels, foot: bool, kind: &str) {
        let seps = if foot { &self.doc.footnote_separators } else { &self.doc.endnote_separators };
        let taken = match kind {
            "separator" => seps.separator.is_some(),
            "continuationSeparator" => seps.continuation_separator.is_some(),
            _ => seps.continuation_notice.is_some(),
        };
        if taken {
            return;
        }
        let mut sc = StoryCtx::default();
        let mut blocks = Blocks::new();
        self.in_note_sep = true;
        self.read_blocks(&mut sc, n, rels, &mut blocks, 0);
        self.flush_pending(&mut sc, &mut blocks);
        self.in_note_sep = false;
        if blocks.is_empty() {
            blocks.push(para_block(Paragraph::new()));
        }
        let seps = if foot { &mut self.doc.footnote_separators } else { &mut self.doc.endnote_separators };
        let slot = match kind {
            "separator" => &mut seps.separator,
            "continuationSeparator" => &mut seps.continuation_separator,
            _ => &mut seps.continuation_notice,
        };
        *slot = Some(blocks);
    }

    /// The extended properties a save carries over ([`crate::package::APP_KEPT_PROPS`]).
    fn read_app(&mut self, path: &str) {
        let Ok(Some(root)) = self.xml(path) else { return };
        let mut kept = String::new();
        for e in root.els() {
            let name = e.local();
            let value = e.text();
            if crate::package::APP_KEPT_PROPS.contains(&name) && value.len() <= 2048 && !value.contains(['\t', '\n']) {
                kept.push_str(&format!("{name}\t{value}\n"));
            }
        }
        if !kept.is_empty() {
            self.doc.passthrough.insert(crate::package::APP_KEPT.into(), Arc::new(kept.into_bytes()));
        }
    }

    fn read_comments(&mut self, path: &str, ext: Option<&str>, ids: Option<&str>, cex: Option<&str>) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        let rels = self.pkg.rels(path);
        let mut by_para: HashMap<String, u32> = HashMap::new();
        let mut next_id = 0u32;
        let mut entries: BTreeMap<u32, Comment> = BTreeMap::new();
        for c in root.children("w:comment") {
            if self.doc.parts.len() >= MAX_PARTS {
                break;
            }
            let file_id = c.attr("w:id").unwrap_or("").to_string();
            if self.comment_map.contains_key(&file_id) {
                continue;
            }
            let id = match file_id.parse::<u32>() {
                Ok(v) if !entries.contains_key(&v) => v,
                _ => {
                    while entries.contains_key(&next_id) || self.comment_map.values().any(|v| *v == next_id) {
                        next_id += 1;
                    }
                    next_id
                }
            };
            let mut sc = StoryCtx::default();
            let mut blocks = Blocks::new();
            self.read_blocks(&mut sc, c, &rels, &mut blocks, 0);
            self.flush_pending(&mut sc, &mut blocks);
            let part = self.doc.add_part(PartKind::Comment, blocks);
            if let Some(pid) = c.children("w:p").last().and_then(|p| p.attr("w14:paraId")) {
                by_para.insert(pid.to_string(), id);
            }
            entries.insert(
                id,
                Comment {
                    author: c.attr("w:author").unwrap_or("").to_string(),
                    initials: c.attr("w:initials").unwrap_or("").to_string(),
                    date: c.attr("w:date").unwrap_or("").to_string(),
                    parent: None,
                    resolved: false,
                    part,
                    ..Default::default()
                },
            );
            self.comment_map.insert(file_id, id);
        }
        if let Some(ext) = ext
            && let Ok(Some(x)) = self.xml(ext)
        {
            for e in x.els().filter(|e| e.local() == "commentEx") {
                let Some(cid) = e.attr("w15:paraId").and_then(|p| by_para.get(p)).copied() else { continue };
                let parent = e.attr("w15:paraIdParent").and_then(|p| by_para.get(p)).copied().filter(|p| *p != cid);
                let done = e.attr("w15:done").is_some_and(|v| v == "1" || v == "true");
                if let Some(c) = entries.get_mut(&cid) {
                    c.parent = parent;
                    c.resolved = done;
                }
            }
        }
        // Durable ids (by paragraph id) and UTC dates (by durable id), which the writer
        // carries over (`write_comment_ids`). Their namespaces (w16cid, w16cex) have no canonical
        // prefix here, so attributes are found by local name.
        let local = |e: &crate::xml::El, name: &str| -> Option<String> {
            e.attrs.iter().find(|(k, _)| k.rsplit(':').next() == Some(name)).map(|(_, v)| v.clone())
        };
        let mut by_durable: HashMap<String, u32> = HashMap::new();
        if let Some(ids) = ids
            && let Ok(Some(x)) = self.xml(ids)
        {
            for e in x.els().filter(|e| e.local() == "commentId") {
                let Some(cid) = local(e, "paraId").and_then(|p| by_para.get(&p).copied()) else { continue };
                let Some(durable) = local(e, "durableId").filter(|d| crate::write::durable_id_ok(d)) else { continue };
                if let Some(c) = entries.get_mut(&cid) {
                    c.durable_id = Some(durable.to_ascii_uppercase());
                    by_durable.insert(durable.to_ascii_uppercase(), cid);
                }
            }
        }
        if let Some(cex) = cex
            && let Ok(Some(x)) = self.xml(cex)
        {
            for e in x.els().filter(|e| e.local() == "commentExtensible") {
                let Some(cid) = local(e, "durableId").and_then(|d| by_durable.get(&d.to_ascii_uppercase()).copied()) else { continue };
                let date = local(e, "dateUtc").filter(|d| d.len() <= 40 && d.chars().all(|c| c.is_ascii_alphanumeric() || ":-.+".contains(c)));
                if let (Some(c), Some(date)) = (entries.get_mut(&cid), date) {
                    c.date_utc = Some(date);
                }
            }
        }
        self.doc.comments = entries;
        Ok(())
    }

    /// Sources from a `b:Sources` custom XML part (other custom XML is left alone); at most
    /// [`MAX_SOURCES`] in all, and the document's citation style from `StyleName`.
    fn read_bibliography(&mut self, path: &str, seen: &mut BibSeen) -> Result<(), DocxError> {
        let Some(root) = self.xml(path)? else { return Ok(()) };
        if root.local() != "Sources" {
            return Ok(());
        }
        if self.doc.settings.bib_style.is_none()
            && let Some(name) = root.attrs.iter().find(|(k, _)| k.rsplit(':').next() == Some("StyleName")).map(|(_, v)| v.trim())
            && !name.is_empty()
        {
            self.doc.settings.bib_style = Some(name.chars().take(MAX_STYLE_NAME).collect());
        }
        let child_text = |el: &El, local: &str| el.els().find(|e| e.local() == local).map(|e| e.text()).unwrap_or_default();
        let parse_persons = |parent: &El| -> String {
            let mut names = Vec::new();
            let person_els: Vec<&El> = {
                let in_lists: Vec<&El> =
                    parent.els().filter(|e| e.local() == "NameList").flat_map(|n| n.els().filter(|e| e.local() == "Person")).collect();
                if in_lists.is_empty() { parent.els().filter(|e| e.local() == "Person").collect() } else { in_lists }
            };
            for name_el in person_els {
                let last = child_text(name_el, "Last");
                let first = child_text(name_el, "First");
                let middle = child_text(name_el, "Middle");
                let name = if !first.is_empty() && !middle.is_empty() {
                    format!("{last}, {first} {middle}")
                } else if !first.is_empty() {
                    format!("{last}, {first}")
                } else {
                    last
                };
                if !name.is_empty() {
                    names.push(name);
                }
            }
            names.join("; ")
        };

        for src_el in root.els().filter(|e| e.local() == "Source") {
            if self.doc.sources.len() >= MAX_SOURCES {
                log::warn!("docx: more than {MAX_SOURCES} bibliography sources; the rest are dropped");
                break;
            }
            let tag = child_text(src_el, "Tag");
            let raw_kind = child_text(src_el, "SourceType");
            let kind = crate::write::from_source_type(&raw_kind);
            let (author, editor) = match src_el.els().find(|e| e.local() == "Author") {
                Some(a) => {
                    let auth =
                        a.els().find(|e| e.local() == "Author").map(parse_persons).filter(|s| !s.is_empty()).unwrap_or_else(|| parse_persons(a));
                    let ed = a.els().find(|e| e.local() == "Editor").map(parse_persons).unwrap_or_default();
                    (auth, ed)
                }
                None => (String::new(), String::new()),
            };
            let editor = if editor.is_empty() { src_el.els().find(|e| e.local() == "Editor").map(parse_persons).unwrap_or_default() } else { editor };
            let title = child_text(src_el, "Title");
            let year = child_text(src_el, "Year");
            let month = child_text(src_el, "Month");
            let day = child_text(src_el, "Day");
            let city = child_text(src_el, "City");
            let publisher = child_text(src_el, "Publisher");
            let journal = child_text(src_el, "JournalName");
            let edition = child_text(src_el, "Edition");
            let volume = child_text(src_el, "Volume");
            let issue = child_text(src_el, "Issue");
            let pages = child_text(src_el, "Pages");
            let url = child_text(src_el, "URL");
            let accessed = match child_text(src_el, "Accessed") {
                a if !a.is_empty() => a,
                _ => {
                    let y = child_text(src_el, "YearAccessed");
                    let m = child_text(src_el, "MonthAccessed");
                    let d = child_text(src_el, "DayAccessed");
                    if !y.is_empty() && !m.is_empty() && !d.is_empty() {
                        format!("{y}-{m}-{d}")
                    } else if !y.is_empty() && !m.is_empty() {
                        format!("{y}-{m}")
                    } else {
                        y
                    }
                }
            };
            let doi = match child_text(src_el, "DOI") {
                d if !d.is_empty() => d,
                _ => child_text(src_el, "StandardNumber"),
            };

            // Keep sources with some content, once each (by tag, or title and author untagged).
            if tag.is_empty() && title.is_empty() && author.is_empty() && editor.is_empty() {
                continue;
            }
            let fresh = if tag.is_empty() {
                seen.untagged.insert((title.clone(), if author.is_empty() { editor.clone() } else { author.clone() }))
            } else {
                seen.tags.insert(tag.clone())
            };
            if fresh {
                self.doc.sources.push(wordcraft_doc::Source {
                    tag,
                    kind,
                    author,
                    title,
                    year,
                    city,
                    publisher,
                    journal,
                    volume,
                    pages,
                    url,
                    editor,
                    edition,
                    issue,
                    month,
                    day,
                    accessed,
                    doi,
                });
            }
        }
        Ok(())
    }

    fn read_core(&mut self, path: &str) {
        let Ok(Some(root)) = self.xml(path) else { return };
        let c = &mut self.doc.core;
        for k in root.els() {
            let t = k.text().trim().to_string();
            match k.name.as_str() {
                "dc:title" => c.title = t,
                "dc:subject" => c.subject = t,
                "dc:creator" => c.creator = t,
                "cp:keywords" => c.keywords = t,
                "dc:description" => c.description = t,
                "cp:lastModifiedBy" => c.last_modified_by = t,
                "cp:revision" => c.revision = u32_of(&t).unwrap_or(0),
                "dcterms:created" => c.created = t,
                "dcterms:modified" => c.modified = t,
                "cp:category" => c.category = t,
                _ => {}
            }
        }
    }
}

/// A level as the file defines it before its own elements: no indents (a level without `w:ind`
/// leaves the label at the paragraph's left edge, as Word shows it), no text.
fn default_level(_i: usize) -> Level {
    Level { indent: 0.0, hanging: 0.0, text: String::new(), ..Level::default() }
}

/// The password hash attributes of `w:documentProtection` that are kept (ECMA-376 §17.15.1.29:
/// the current `algorithmName`/`hashValue`/`saltValue`/`spinCount`, and the older transitional
/// `crypt*`/`hash`/`salt` set).
pub(crate) const PROTECTION_HASH_ATTRS: [&str; 16] = [
    "w:algorithmName",
    "w:hashValue",
    "w:saltValue",
    "w:spinCount",
    "w:cryptProviderType",
    "w:cryptAlgorithmClass",
    "w:cryptAlgorithmType",
    "w:cryptAlgorithmSid",
    "w:cryptSpinCount",
    "w:cryptProvider",
    "w:algIdExt",
    "w:algIdExtSource",
    "w:cryptProviderTypeExt",
    "w:cryptProviderTypeExtSource",
    "w:hash",
    "w:salt",
];

/// Restrict Editing (`w:documentProtection`, §17.15.1.29): the editing restriction, whether
/// formatting is limited to unlocked styles, and the password hash. Only enforced protection is
/// kept; a hostile attribute value can't be longer than a hash needs.
fn read_protection(k: &El, s: &mut wordcraft_doc::Settings) {
    let on = |v: &str| !matches!(v, "0" | "false" | "off");
    if !k.attr("w:enforcement").is_some_and(on) {
        return;
    }
    let edit = k.attr("w:edit").filter(|e| matches!(*e, "readOnly" | "comments" | "trackedChanges" | "forms"));
    let formatting = k.attr("w:formatting").is_some_and(on);
    if edit.is_none() && !formatting {
        return;
    }
    s.protection = Some(edit.unwrap_or("none").to_string());
    s.protect_formatting = formatting;
    s.protect_hash = PROTECTION_HASH_ATTRS
        .iter()
        .filter_map(|n| k.attr(n).filter(|v| !v.is_empty() && v.len() <= 1024).map(|v| (n.to_string(), v.to_string())))
        .collect();
}

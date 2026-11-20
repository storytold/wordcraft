//! Formatting and table edits for [`crate::write_back`]: the document before and after editing
//! (both as WordCraft documents, the "before" one read from the original's conversion) are
//! compared paragraph by paragraph (by their source marks) and table by table.
//!
//! - A character or paragraph shape that changed becomes a new hh:charPr / hh:paraPr: a copy of
//!   the original shape with only the changed properties set (every other Hancom property kept),
//!   added to the header with a new id.
//! - A table whose structure, borders or fill changed is written again whole (with the shapes it
//!   needs added to the header); a new table goes after the paragraph before it.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::HashMap;

use wordcraft_doc::resolve::ResolvedChar;
use wordcraft_doc::table::Table;
use wordcraft_doc::{Block, Document, InlineObject, Paragraph, TextColor};

use crate::header::LANGS;
use crate::hwpx_out::{CharKey, FillKey, ParaKey, Tables, char_pr_xml, fill_xml, para_key, para_pr_xml, underline_shape};
use crate::writer::{Edit, Mark, Node, apply, parse_mark, parse_spans, tag_attrs};

/// Deepest nesting followed (hostile documents).
const MAX_DEPTH: usize = 32;

/// A source paragraph as WordCraft shows it.
pub(crate) struct ParaInfo {
    /// The paragraph's text as the DOCX reading of write-back sees it, with each character's
    /// formatting.
    pub chars: Vec<(char, ResolvedChar)>,
    pub para: ParaKey,
    /// The paragraph's own pictures, in order (not those of its text boxes).
    pub pics: Vec<Pic>,
}

/// A picture of a paragraph.
#[derive(Clone, Debug)]
pub(crate) struct Pic {
    pub obj: InlineObject,
    pub w: f32,
    pub h: f32,
    /// A hash of the image bytes: the same picture, or another one.
    pub data: u64,
    /// A hash of everything else but the size (crop, rotation, position, wrapping, effects): a
    /// picture whose settings changed is written again whole, not only resized.
    pub settings: u64,
    /// Characters of text before it in the paragraph, and after it.
    pub text_before: usize,
    pub text_after: usize,
}

impl Pic {
    /// The same picture at the same size.
    pub fn same(&self, o: &Pic) -> bool {
        self.data == o.data && self.settings == o.settings && (self.w - o.w).abs() < 0.05 && (self.h - o.h).abs() < 0.05
    }
}

/// A table and where it is.
pub(crate) struct TableInfo<'d> {
    /// The source mark of its first cell's first paragraph (none: a new table).
    pub mark: Option<Mark>,
    pub table: &'d Table,
    /// What comes before it in the body (for a new table): a source paragraph, or (`true`) the
    /// table whose first mark that is.
    pub prev: Option<(Mark, bool)>,
    /// New body paragraphs between `prev` and it (a new table goes after the new paragraphs
    /// that come before it, which are placed after `prev` too).
    pub new_before: usize,
    pub top: bool,
    /// Tables around it (a table inside a table shares its first mark with the outer one).
    pub depth: usize,
    /// Tables inside it around the paragraph of its mark (a table whose first cell starts
    /// with a table: 1), so the original table is found from that paragraph.
    pub inside: usize,
    /// The source marks of its cells' own paragraphs, so a table whose first row was deleted
    /// (its first mark another now) is still found among the original's.
    pub marks: Vec<Mark>,
}

/// A document's source paragraphs and tables.
pub(crate) struct DocMarks<'d> {
    pub paras: HashMap<Mark, ParaInfo>,
    pub tables: Vec<TableInfo<'d>>,
    /// The pictures of each body paragraph without a source mark that has any, in order.
    pub new_pics: Vec<Vec<Pic>>,
}

/// The formatting that write-back writes (what a character shape holds).
fn fmt_key(c: &ResolvedChar) -> impl PartialEq + '_ {
    (
        (&c.font, &c.font_ea, c.size.to_bits(), c.bold, c.italic, c.underline),
        (c.strike, c.double_strike, c.color, c.highlight, c.shading, c.vert_align),
    )
}

pub(crate) fn same_format(a: &ResolvedChar, b: &ResolvedChar) -> bool {
    fmt_key(a) == fmt_key(b)
}

fn first_mark(p: &Paragraph) -> Option<Mark> {
    p.objects.iter().find_map(|o| match o {
        InlineObject::BookmarkStart { name } => parse_mark(name).filter(|m| m.0 == "hp"),
        _ => None,
    })
}

/// The first source mark in a table's cells, tables inside them included, and how many of
/// those tables are around its paragraph.
fn table_mark(t: &Table, depth: usize) -> Option<(Mark, usize)> {
    if depth > MAX_DEPTH {
        return None;
    }
    t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| c.blocks.iter()).find_map(|b| match &**b {
        Block::Para(p) => first_mark(p).map(|m| (m, 0)),
        Block::Table(inner) => table_mark(inner, depth + 1).map(|(m, n)| (m, n + 1)),
    })
}

/// The source marks of the paragraphs in table `t`'s own cells (not those of tables inside).
fn cell_marks(t: &Table) -> Vec<Mark> {
    t.rows
        .iter()
        .flat_map(|r| &r.cells)
        .flat_map(|c| c.blocks.iter())
        .filter_map(|b| match &**b {
            Block::Para(p) => first_mark(p),
            Block::Table(_) => None,
        })
        .collect()
}

fn para_info(doc: &Document, p: &Paragraph) -> ParaInfo {
    let style = p.props.style.as_deref();
    let rp = doc.styles.resolve_para(&p.props);
    let mark_size = doc.styles.resolve_char(style, &p.mark).size;
    let mut chars = Vec::new();
    let mut pos = 0usize;
    for run in &p.runs {
        let end = pos.saturating_add(run.len).min(p.text.len());
        let text = p.text.get(pos..end).unwrap_or("");
        pos = end;
        if run.props.del.is_some() {
            continue;
        }
        let rc = doc.styles.resolve_char(style, &run.props);
        for ch in text.chars() {
            let ch = match ch {
                wordcraft_doc::para::OBJ | wordcraft_doc::para::PAGE_BREAK | wordcraft_doc::para::COLUMN_BREAK | wordcraft_doc::para::SOFT_HYPHEN => {
                    continue;
                }
                wordcraft_doc::para::NB_HYPHEN => '-',
                c => c,
            };
            chars.push((ch, rc.clone()));
        }
    }
    ParaInfo { chars, para: para_key(&rp, mark_size), pics: pics(doc, p) }
}

/// The pictures of paragraph `p` (tracked deletions left out).
fn pics(doc: &Document, p: &Paragraph) -> Vec<Pic> {
    use std::hash::{Hash, Hasher};
    let deleted = |off: usize| {
        let mut pos = 0usize;
        p.runs.iter().any(|r| {
            let start = pos;
            pos = pos.saturating_add(r.len);
            r.props.del.is_some() && (start..pos).contains(&off)
        })
    };
    let mut out = Vec::new();
    for (off, o) in p.objects_with_offsets() {
        // pictures, drawings and groups (a drawing's own description stands for its image)
        let (w, h) = match o {
            InlineObject::Image { w, h, .. } | InlineObject::Shape { w, h, .. } | InlineObject::Group { w, h, .. } => (w, h),
            _ => continue,
        };
        if deleted(off) {
            continue;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        match o {
            InlineObject::Image { media, .. } => doc.media.get(media).map(|d| d.as_slice()).unwrap_or(&[]).hash(&mut hasher),
            // its description without its size: resizing it is not replacing it
            _ => {
                let mut sizeless = o.clone();
                if let InlineObject::Shape { w, h, .. } | InlineObject::Group { w, h, .. } = &mut sizeless {
                    (*w, *h) = (0.0, 0.0);
                }
                format!("{sizeless:?}").hash(&mut hasher)
            }
        }
        // a picture's crop, placement and effects (a drawing's are in its `data` already); not
        // its media key, which names the same image differently from one reading to the next
        let mut settings = std::collections::hash_map::DefaultHasher::new();
        if let InlineObject::Image { float, crop, fx, .. } = o {
            format!("{float:?}{crop:?}{fx:?}").hash(&mut settings);
        }
        let count = |t: &str| t.chars().filter(|&c| c != wordcraft_doc::para::OBJ).count();
        let (text_before, text_after) = (count(p.text.get(..off).unwrap_or("")), count(p.text.get(off..).unwrap_or("")));
        out.push(Pic { obj: o.clone(), w: *w, h: *h, data: hasher.finish(), settings: settings.finish(), text_before, text_after });
    }
    out
}

/// Has paragraph `p` a source mark of any kind?
fn any_mark(p: &Paragraph) -> bool {
    p.objects.iter().any(|o| matches!(o, InlineObject::BookmarkStart { name } if parse_mark(name).is_some()))
}

struct Walk<'d> {
    doc: &'d Document,
    out: DocMarks<'d>,
    prev: Option<(Mark, bool)>,
    /// New body paragraphs since `prev`.
    new_since: usize,
    /// Walking the body (not headers, notes or text boxes).
    body: bool,
}

impl<'d> Walk<'d> {
    fn blocks(&mut self, blocks: &'d [std::sync::Arc<Block>], top: bool, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for b in blocks {
            match &**b {
                Block::Para(p) => {
                    if let Some(m) = first_mark(p) {
                        self.out.paras.entry(m).or_insert_with(|| para_info(self.doc, p));
                        if top {
                            self.prev = Some((m, false));
                            self.new_since = 0;
                        }
                    } else if top && self.body && !any_mark(p) {
                        self.new_since += 1;
                        let pics = pics(self.doc, p);
                        if !pics.is_empty() {
                            self.out.new_pics.push(pics);
                        }
                    }
                }
                Block::Table(t) => {
                    let found = table_mark(t, 0);
                    let (mark, inside) = (found.map(|f| f.0), found.map_or(0, |f| f.1));
                    let marks = cell_marks(t);
                    self.out.tables.push(TableInfo { mark, table: t, prev: self.prev, new_before: self.new_since, top, depth, inside, marks });
                    if top && let Some(m) = mark {
                        self.prev = Some((m, true));
                        self.new_since = 0;
                    }
                    for row in &t.rows {
                        for cell in &row.cells {
                            self.blocks(&cell.blocks, false, depth + 1);
                        }
                    }
                }
            }
        }
    }
}

/// The source paragraphs and tables of a document (body, cells, headers, notes, text boxes).
pub(crate) fn collect(doc: &Document) -> DocMarks<'_> {
    let mut w = Walk { doc, out: DocMarks { paras: HashMap::new(), tables: Vec::new(), new_pics: Vec::new() }, prev: None, new_since: 0, body: true };
    w.blocks(&doc.body, true, 0);
    w.body = false;
    for part in doc.parts.values() {
        w.blocks(&part.blocks, false, 1);
    }
    w.out
}

/// A table's structure and look (what a cell edit does not change): what the table writer
/// writes, so a change to any of it rewrites the table (column widths, row heights, cell
/// margins and alignment, the table's alignment and margins too).
pub(crate) fn table_signature(t: &Table) -> String {
    let rows: Vec<String> = t
        .rows
        .iter()
        .map(|r| {
            let cells: Vec<String> = r
                .cells
                .iter()
                .map(|c| {
                    let p = &c.props;
                    format!("{}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}", p.span, p.vmerge, p.shading, p.borders, p.width, p.valign, p.margins)
                })
                .collect();
            format!("{:?}/{:?}/{}:{cells:?}", r.props.height, r.props.height_rule, r.props.header)
        })
        .collect();
    let tp = &t.props;
    format!("{:?}|{rows:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}", t.grid, tp.borders, tp.shading, tp.align, tp.indent, tp.cell_margins, tp.width)
}

// ==== the header part ===========================================================================

/// `name="value"` set in a start tag (added before its end when missing).
fn set_attr(tag: &str, name: &str, value: &str) -> String {
    // the value's span whichever quote it stands in (`name='1'` too)
    if let Some((_, from, to)) = crate::writer::attr_spans(tag).into_iter().find(|a| a.0 == name) {
        return format!("{}{value}{}", tag.get(..from).unwrap_or(""), tag.get(to..).unwrap_or(""));
    }
    let end = if tag.ends_with("/>") { tag.len().saturating_sub(2) } else { tag.len().saturating_sub(1) };
    format!("{} {name}=\"{value}\"{}", tag.get(..end).unwrap_or(""), tag.get(end..).unwrap_or(""))
}

fn local(n: &Node) -> &str {
    n.name.split_once(':').map(|(_, l)| l).unwrap_or(&n.name)
}

/// The header part being extended with new shapes.
pub(crate) struct HeaderEdit {
    text: String,
    nodes: Vec<Node>,
    /// The prefix the part uses for the head namespace ("hh").
    hh: String,
    next: HashMap<&'static str, i64>,
    added: HashMap<&'static str, Vec<String>>,
    /// Fonts per language (HANGUL, LATIN, ... order) as (id, name), existing then added.
    fonts: [Vec<(i64, String)>; 7],
    fonts_added: [Vec<String>; 7],
    cache: HashMap<String, String>,
    /// The changes each cached paragraph variant could not write.
    para_unwritten: HashMap<String, Vec<&'static str>>,
    /// Each added shape: (element, new id, the id of the shape it copies; none for a generated
    /// one). Writing to HWP copies the source's record.
    pub(crate) sources: Vec<(&'static str, i64, Option<String>)>,
}

/// (element, its list element)
const LISTS: [(&str, &str); 3] = [("charPr", "charProperties"), ("paraPr", "paraProperties"), ("borderFill", "borderFills")];

impl HeaderEdit {
    pub(crate) fn new(text: String) -> HeaderEdit {
        let nodes = parse_spans(&text);
        let hh =
            nodes.iter().find(|n| local(n) == "head").and_then(|n| n.name.split_once(':')).map(|(p, _)| p.to_string()).unwrap_or_else(|| "hh".into());
        let mut next = HashMap::new();
        for (el, _) in LISTS {
            let max = nodes
                .iter()
                .filter(|n| local(n) == el)
                .filter_map(|n| tag_attrs(text.get(n.start..n.tag_end).unwrap_or("")).get("id").and_then(|v| v.parse::<i64>().ok()))
                .max()
                .unwrap_or(0);
            next.insert(el, max + 1);
        }
        let mut fonts: [Vec<(i64, String)>; 7] = Default::default();
        for n in nodes.iter().filter(|n| local(n) == "fontface") {
            let attrs = tag_attrs(text.get(n.start..n.tag_end).unwrap_or(""));
            let Some(k) = attrs.get("lang").and_then(|l| LANGS.iter().position(|x| x.eq_ignore_ascii_case(l))) else { continue };
            let mut list: Vec<(i64, String)> = n
                .children
                .iter()
                .filter_map(|&c| nodes.get(c))
                .filter(|c| local(c) == "font")
                .map(|c| {
                    let a = tag_attrs(text.get(c.start..c.tag_end).unwrap_or(""));
                    (a.get("id").and_then(|v| v.parse().ok()).unwrap_or(0), a.get("face").cloned().unwrap_or_default())
                })
                .collect();
            list.sort();
            if let Some(slot) = fonts.get_mut(k) {
                *slot = list;
            }
        }
        HeaderEdit {
            text,
            nodes,
            hh,
            next,
            added: HashMap::new(),
            fonts,
            fonts_added: Default::default(),
            cache: HashMap::new(),
            para_unwritten: HashMap::new(),
            sources: Vec::new(),
        }
    }

    fn element(&self, el: &str, id: &str) -> Option<String> {
        self.nodes
            .iter()
            .filter(|n| local(n) == el)
            .find(|n| tag_attrs(self.text.get(n.start..n.tag_end).unwrap_or("")).get("id").map(String::as_str) == Some(id))
            .and_then(|n| {
                let raw = self.text.get(n.start..n.end)?;
                // one written empty (`<hh:charPr .../>`) is opened, so the children a variant
                // adds go inside it, not after it
                Some(match raw.strip_suffix("/>").filter(|_| n.end == n.tag_end) {
                    Some(open) => format!("{}></{}>", open.trim_end(), n.name),
                    None => raw.to_string(),
                })
            })
    }

    fn take_id(&mut self, el: &'static str) -> i64 {
        let n = self.next.entry(el).or_insert(1);
        let id = *n;
        *n += 1;
        id
    }

    /// [`Self::take_id`] for a generated shape.
    fn take_generated(&mut self, el: &'static str) -> i64 {
        let id = self.take_id(el);
        self.sources.push((el, id, None));
        id
    }

    fn add(&mut self, el: &'static str, xml: String) {
        self.added.entry(el).or_default().push(xml);
    }

    /// The id of font `name` for language `lang`, added when the header has none.
    /// The id of font `name` in language `lang`'s list: its own (ids as the file gives them, which
    /// need not run 0, 1, 2 ...), or a new one above every id there.
    fn font_id(&mut self, lang: usize, name: &str) -> usize {
        let Some(list) = self.fonts.get_mut(lang) else { return 0 };
        if let Some((id, _)) = list.iter().find(|f| f.1 == name) {
            return usize::try_from(*id).unwrap_or(0);
        }
        let id = list.iter().map(|f| f.0).max().map_or(0, |m| m.saturating_add(1));
        list.push((id, name.to_string()));
        let id = usize::try_from(id).unwrap_or(0);
        if let Some(a) = self.fonts_added.get_mut(lang) {
            a.push(format!("<{}:font id=\"{id}\" face={} type=\"TTF\" isEmbedded=\"0\"/>", self.hh, crate::docx::attr(name)));
        }
        id
    }

    /// A character shape like `id` with the formatting that changed from `before` to `after`;
    /// its new id.
    pub(crate) fn char_variant(&mut self, id: &str, before: &ResolvedChar, after: &ResolvedChar) -> Option<String> {
        let east = |c: &ResolvedChar| c.font_ea.clone().unwrap_or_else(|| c.font.clone());
        let color = |c: TextColor| match c {
            TextColor::Rgb(c) => format!("#{}", c.hex()),
            TextColor::Auto => "#000000".into(),
        };
        let shade = |c: &ResolvedChar| c.highlight.or(c.shading).map(|c| format!("#{}", c.hex())).unwrap_or_else(|| "none".into());
        // the patch compares each value before and after, so both go into the key: two runs of
        // one shape whose formatting before differs (bold from a paragraph style) make different
        // patches
        let fields = |c: &ResolvedChar| {
            format!(
                "{:?}|{:?}|{}|{}|{}|{:?}|{}|{}|{}|{}|{:?}",
                c.font,
                east(c),
                c.size,
                c.bold,
                c.italic,
                c.underline,
                c.strike,
                c.double_strike,
                color(c.color),
                shade(c),
                c.vert_align
            )
        };
        let key = format!("c{id}|{}|{}", fields(before), fields(after));
        if let Some(n) = self.cache.get(&key) {
            return Some(n.clone());
        }
        let raw = self.element("charPr", id)?;
        let nodes = parse_spans(&raw);
        let root = nodes.first()?;
        let new_id = self.take_id("charPr");
        self.sources.push(("charPr", new_id, Some(id.to_string())));
        let mut tag = raw.get(root.start..root.tag_end)?.to_string();
        tag = set_attr(&tag, "id", &new_id.to_string());
        if before.size != after.size {
            tag = set_attr(&tag, "height", &crate::hwpx_out::hu(after.size).to_string());
        }
        if before.color != after.color {
            tag = set_attr(&tag, "textColor", &color(after.color));
        }
        if shade(before) != shade(after) {
            tag = set_attr(&tag, "shadeColor", &shade(after));
        }
        let mut edits: Vec<Edit> = vec![(root.start, root.tag_end, tag)];
        let child = |name: &str| nodes.iter().skip(1).find(|n| local(n) == name);
        let hh = self.hh.clone();
        // fonts
        let latin_changed = before.font != after.font;
        let east_changed = east(before) != east(after);
        if latin_changed || east_changed {
            match child("fontRef") {
                Some(fr) => {
                    let mut t = raw.get(fr.start..fr.tag_end).unwrap_or("").to_string();
                    for (k, a) in LANGS.iter().enumerate() {
                        let is_east = matches!(k, 0 | 2 | 3);
                        if (is_east && east_changed) || (!is_east && latin_changed) {
                            let name = if is_east { east(after) } else { after.font.clone() };
                            let fid = self.font_id(k, &name);
                            t = set_attr(&t, a, &fid.to_string());
                        }
                    }
                    edits.push((fr.start, fr.tag_end, t));
                }
                None => {
                    // A shape naming no fonts gets a fontRef, every slot the font the run shows
                    // now (the East Asian one for Hangul, hanja and Japanese): skipping it would
                    // count the change as written while the text kept its old font.
                    let slots: String = LANGS
                        .iter()
                        .enumerate()
                        .map(|(k, a)| {
                            let name = if matches!(k, 0 | 2 | 3) { east(after) } else { after.font.clone() };
                            format!(" {a}=\"{}\"", self.font_id(k, &name))
                        })
                        .collect();
                    edits.push((root.tag_end, root.tag_end, format!("<{hh}:fontRef{slots}/>")));
                }
            }
        }
        let close = root.inner_end;
        let toggle = |name: &str, was: bool, now: bool, edits: &mut Vec<Edit>| {
            if was == now {
                return;
            }
            match (now, child(name)) {
                (true, None) => edits.push((close, close, format!("<{hh}:{name}/>"))),
                (false, Some(n)) => edits.push((n.start, n.end, String::new())),
                _ => {}
            }
        };
        toggle("bold", before.bold, after.bold, &mut edits);
        toggle("italic", before.italic, after.italic, &mut edits);
        toggle(
            "supscript",
            before.vert_align == wordcraft_doc::props::VertAlign::Superscript,
            after.vert_align == wordcraft_doc::props::VertAlign::Superscript,
            &mut edits,
        );
        toggle(
            "subscript",
            before.vert_align == wordcraft_doc::props::VertAlign::Subscript,
            after.vert_align == wordcraft_doc::props::VertAlign::Subscript,
            &mut edits,
        );
        if before.underline != after.underline {
            let (kind, shape) = match underline_shape(after.underline) {
                Some(s) => ("BOTTOM", s),
                None => ("NONE", "SOLID"),
            };
            match child("underline") {
                Some(u) => {
                    let t = set_attr(&set_attr(raw.get(u.start..u.tag_end).unwrap_or(""), "type", kind), "shape", shape);
                    edits.push((u.start, u.tag_end, t));
                }
                None => edits.push((close, close, format!("<{hh}:underline type=\"{kind}\" shape=\"{shape}\" color=\"#000000\"/>"))),
            }
        }
        if (before.strike, before.double_strike) != (after.strike, after.double_strike) {
            let shape = if after.double_strike {
                "DOUBLE_SLIM"
            } else if after.strike {
                "SOLID"
            } else {
                "NONE"
            };
            match child("strikeout") {
                Some(s) => edits.push((s.start, s.tag_end, set_attr(raw.get(s.start..s.tag_end).unwrap_or(""), "shape", shape))),
                None => edits.push((close, close, format!("<{hh}:strikeout shape=\"{shape}\" color=\"#000000\"/>"))),
            }
        }
        self.add("charPr", apply(&raw, &edits));
        let id = new_id.to_string();
        self.cache.insert(key, id.clone());
        Some(id)
    }

    /// A paragraph shape like `id` with what changed from `before` to `after`: its new id, and
    /// the changes it could not write (margins the source shape has no element for).
    pub(crate) fn para_variant(&mut self, id: &str, before: &ParaKey, after: &ParaKey) -> Option<(String, Vec<&'static str>)> {
        // the patch writes what differs from `before`: two paragraphs of one shape that resolve
        // apart (a margin in characters at two font sizes) make different patches
        let key = format!("p{id}|{before:?}|{after:?}");
        if let Some(n) = self.cache.get(&key) {
            return Some((n.clone(), self.para_unwritten.get(&key).cloned().unwrap_or_default()));
        }
        let raw = self.element("paraPr", id)?;
        let nodes = parse_spans(&raw);
        let root = nodes.first()?;
        let new_id = self.take_id("paraPr");
        self.sources.push(("paraPr", new_id, Some(id.to_string())));
        let mut edits: Vec<Edit> = vec![(root.start, root.tag_end, set_attr(raw.get(root.start..root.tag_end)?, "id", &new_id.to_string()))];
        // values in an hp:default branch are doubled (HwpUnitChar, see xml.rs)
        let in_default = |mut i: usize| {
            let mut guard = 0;
            while let Some(p) = nodes.get(i).and_then(|n| n.parent) {
                if nodes.get(p).is_some_and(|n| local(n) == "default") {
                    return true;
                }
                i = p;
                guard += 1;
                if guard > 64 {
                    break;
                }
            }
            false
        };
        for (i, n) in nodes.iter().enumerate() {
            let tag = raw.get(n.start..n.tag_end).unwrap_or("");
            let scale = if in_default(i) { 2 } else { 1 };
            // a margin is written in HWPUNIT, and says so: one given in characters
            // (unit="CHAR") would otherwise be read as that many characters
            let margin = |v: i64| Some(set_attr(&set_attr(tag, "value", &(v * scale).to_string()), "unit", "HWPUNIT"));
            let set = match local(n) {
                "align" if before.align != after.align => Some(set_attr(tag, "horizontal", after.align)),
                "intent" if before.indent != after.indent => margin(after.indent),
                "left" if before.left != after.left => margin(after.left),
                "right" if before.right != after.right => margin(after.right),
                "prev" if before.before != after.before => margin(after.before),
                "next" if before.after != after.after => margin(after.after),
                "lineSpacing" if (before.ls_type, before.ls_value) != (after.ls_type, after.ls_value) => {
                    let v = if after.ls_type == "PERCENT" { after.ls_value } else { after.ls_value * scale };
                    Some(set_attr(&set_attr(tag, "type", after.ls_type), "value", &v.to_string()))
                }
                "breakSetting" => {
                    // keep with next, keep lines together, page break before, widow and orphan
                    // control, and whether Korean words may break
                    let flags = |k: &ParaKey| (k.keep_next, k.keep_lines, k.page_break_before, k.widow, k.break_words);
                    (flags(before) != flags(after)).then(|| {
                        let b = |v: bool| if v { "1" } else { "0" };
                        let words = if after.break_words { "BREAK_WORD" } else { "KEEP_WORD" };
                        let t = set_attr(tag, "keepWithNext", b(after.keep_next));
                        let t = set_attr(&t, "keepLines", b(after.keep_lines));
                        let t = set_attr(&t, "pageBreakBefore", b(after.page_break_before));
                        let t = set_attr(&t, "widowOrphan", b(after.widow));
                        set_attr(&t, "breakNonLatinWord", words)
                    })
                }
                _ => None,
            };
            if let Some(t) = set {
                edits.push((n.start, n.tag_end, t));
            }
        }
        // A change whose element the source shape lacks: alignment, break settings and line
        // spacing are added (each before the first child that follows it in a paragraph
        // shape); a margin, whose elements take another namespace, is reported instead.
        let has = |name: &str| nodes.iter().any(|n| local(n) == name);
        const ORDER: [&str; 7] = ["align", "heading", "breakSetting", "margin", "lineSpacing", "border", "autoSpacing"];
        let spot = |name: &str| {
            let rank = |n: &str| ORDER.iter().position(|o| *o == n);
            let me = rank(name);
            nodes.iter().filter(|n| n.parent == Some(0)).find(|n| rank(local(n)) > me).map_or(root.inner_end, |n| n.start)
        };
        let hh = self.hh.clone();
        if before.align != after.align && !has("align") {
            let at = spot("align");
            edits.push((at, at, format!("<{hh}:align horizontal=\"{}\" vertical=\"BASELINE\"/>", after.align)));
        }
        let flags = |k: &ParaKey| (k.keep_next, k.keep_lines, k.page_break_before, k.widow, k.break_words);
        if flags(before) != flags(after) && !has("breakSetting") {
            let b = |v: bool| if v { "1" } else { "0" };
            let at = spot("breakSetting");
            edits.push((
                at,
                at,
                format!(
                    "<{hh}:breakSetting breakLatinWord=\"KEEP_WORD\" breakNonLatinWord=\"{}\" widowOrphan=\"{}\" keepWithNext=\"{}\" keepLines=\"{}\" pageBreakBefore=\"{}\" lineWrap=\"BREAK\"/>",
                    if after.break_words { "BREAK_WORD" } else { "KEEP_WORD" },
                    b(after.widow),
                    b(after.keep_next),
                    b(after.keep_lines),
                    b(after.page_break_before)
                ),
            ));
        }
        if (before.ls_type, before.ls_value) != (after.ls_type, after.ls_value) && !has("lineSpacing") {
            let at = spot("lineSpacing");
            edits.push((at, at, format!("<{hh}:lineSpacing type=\"{}\" value=\"{}\" unit=\"HWPUNIT\"/>", after.ls_type, after.ls_value)));
        }
        let unwritten: Vec<&'static str> = [
            ("indent", "intent", before.indent != after.indent),
            ("left indent", "left", before.left != after.left),
            ("right indent", "right", before.right != after.right),
            ("spacing before", "prev", before.before != after.before),
            ("spacing after", "next", before.after != after.after),
        ]
        .into_iter()
        .filter(|&(_, el, changed)| changed && !has(el))
        .map(|(what, _, _)| what)
        .collect();
        self.add("paraPr", apply(&raw, &edits));
        let id = new_id.to_string();
        self.cache.insert(key.clone(), id.clone());
        self.para_unwritten.insert(key, unwritten.clone());
        Some((id, unwritten))
    }

    /// Add the shapes a generated table uses; the new ids of its character shapes, paragraph
    /// shapes and border fills (border fill `k` is at index `k - 1`).
    pub(crate) fn add_generated(&mut self, t: &Tables) -> (Vec<String>, Vec<String>, Vec<String>) {
        let none_id = self.generated("borderFill", |id| fill_xml(id, &FillKey::none()));
        let mut fills = vec![none_id.clone()];
        for k in &t.fills {
            fills.push(self.generated("borderFill", |id| fill_xml(id, k)));
        }
        let none = format!("borderFillIDRef=\"{none_id}\"");
        let mut chars = Vec::new();
        for k in &t.char_order {
            let mut k: CharKey = k.clone();
            for (lang, f) in k.fonts.iter_mut().enumerate() {
                let name = t.fonts.get(lang).and_then(|l| l.get(*f)).cloned().unwrap_or_default();
                *f = self.font_id(lang, &name);
            }
            chars.push(self.generated("charPr", |id| char_pr_xml(id, &k).replace("borderFillIDRef=\"1\"", &none)));
        }
        let mut paras = Vec::new();
        for k in &t.para_order {
            paras.push(self.generated("paraPr", |id| para_pr_xml(id, k).replace("borderFillIDRef=\"1\"", &none)));
        }
        (chars, paras, fills)
    }

    /// The id of a generated shape `xml(id)`; one already added with the same content (every
    /// rewritten table makes its own list) is used again, so the header doesn't fill with copies.
    fn generated(&mut self, el: &'static str, xml: impl Fn(usize) -> String) -> String {
        let key = format!("g{el}|{}", xml(0));
        if let Some(id) = self.cache.get(&key) {
            return id.clone();
        }
        let id = self.take_generated(el);
        let text = self.rename(&xml(id as usize));
        self.add(el, text);
        let id = id.to_string();
        self.cache.insert(key, id.clone());
        id
    }

    /// Generated XML with this header's prefix for the head namespace.
    fn rename(&self, xml: &str) -> String {
        if self.hh == "hh" { xml.to_string() } else { xml.replace("<hh:", &format!("<{}:", self.hh)).replace("</hh:", &format!("</{}:", self.hh)) }
    }

    /// The header with the added shapes and fonts, or None when nothing was added.
    pub(crate) fn finish(&self) -> Option<String> {
        if self.added.values().all(Vec::is_empty) && self.fonts_added.iter().all(Vec::is_empty) {
            return None;
        }
        let hh = &self.hh;
        let mut edits: Vec<Edit> = Vec::new();
        // lists the header doesn't have (one with no shape lists at all): made, so the new
        // shapes' definitions are there for the ids written
        let mut made: Vec<(&str, String)> = Vec::new();
        for (el, list) in LISTS {
            let Some(new) = self.added.get(el).filter(|v| !v.is_empty()) else { continue };
            let Some(n) = self.nodes.iter().find(|n| local(n) == list) else {
                made.push((list, format!("<{hh}:{list} itemCnt=\"{}\">{}</{hh}:{list}>", new.len(), new.concat())));
                continue;
            };
            let count = n.children.iter().filter_map(|&c| self.nodes.get(c)).filter(|c| local(c) == el).count() + new.len();
            let tag = set_attr(self.text.get(n.start..n.tag_end).unwrap_or(""), "itemCnt", &count.to_string());
            add_children(n, tag, new.concat(), &mut edits);
        }
        let mut have_face = [false; 7];
        for n in self.nodes.iter().filter(|n| local(n) == "fontface") {
            let attrs = tag_attrs(self.text.get(n.start..n.tag_end).unwrap_or(""));
            let Some(k) = attrs.get("lang").and_then(|l| LANGS.iter().position(|x| x.eq_ignore_ascii_case(l))) else { continue };
            if let Some(h) = have_face.get_mut(k) {
                *h = true;
            }
            let Some(new) = self.fonts_added.get(k).filter(|v| !v.is_empty()) else { continue };
            let total = self.fonts.get(k).map(Vec::len).unwrap_or(0);
            add_children(n, set_attr(self.text.get(n.start..n.tag_end).unwrap_or(""), "fontCnt", &total.to_string()), new.concat(), &mut edits);
        }
        // font lists of languages the header has none of
        let faces: Vec<String> = LANGS
            .iter()
            .enumerate()
            .filter(|(k, _)| !have_face.get(*k).copied().unwrap_or(true))
            .filter_map(|(k, lang)| {
                let new = self.fonts_added.get(k).filter(|v| !v.is_empty())?;
                let lang = lang.to_ascii_uppercase();
                Some(format!("<{hh}:fontface lang=\"{lang}\" fontCnt=\"{}\">{}</{hh}:fontface>", new.len(), new.concat()))
            })
            .collect();
        if !faces.is_empty() {
            match self.nodes.iter().find(|n| local(n) == "fontfaces") {
                Some(n) => {
                    let count = n.children.iter().filter_map(|&c| self.nodes.get(c)).filter(|c| local(c) == "fontface").count() + faces.len();
                    add_children(
                        n,
                        set_attr(self.text.get(n.start..n.tag_end).unwrap_or(""), "itemCnt", &count.to_string()),
                        faces.concat(),
                        &mut edits,
                    );
                }
                None => made.push(("fontfaces", format!("<{hh}:fontfaces itemCnt=\"{}\">{}</{hh}:fontfaces>", faces.len(), faces.concat()))),
            }
        }
        if !made.is_empty() {
            self.place_lists(&mut made, &mut edits);
        }
        Some(apply(&self.text, &edits))
    }

    /// Lists `made` put into the header's refList, each before the first list the format gives
    /// after it; a header without a refList gets one (after hh:beginNum), and a head written
    /// empty (`<hh:head .../>`) is opened for it.
    fn place_lists(&self, made: &mut [(&str, String)], edits: &mut Vec<Edit>) {
        const ORDER: [&str; 9] =
            ["fontfaces", "borderFills", "charProperties", "tabProperties", "numberings", "bullets", "paraProperties", "styles", "memoProperties"];
        let rank = |name: &str| ORDER.iter().position(|o| *o == name).unwrap_or(ORDER.len());
        made.sort_by_key(|(name, _)| rank(name));
        let hh = &self.hh;
        let Some(head) = self.nodes.iter().find(|n| local(n) == "head") else { return };
        match self.nodes.iter().find(|n| local(n) == "refList") {
            Some(r) if self.text.get(r.start..r.tag_end).is_some_and(|t| t.ends_with("/>")) => {
                // an empty refList written closed: opened for the lists
                let lists: String = made.iter().map(|(_, xml)| xml.as_str()).collect();
                add_children(r, self.text.get(r.start..r.tag_end).unwrap_or("").to_string(), lists, edits);
            }
            Some(r) => {
                let kids: Vec<&Node> = r.children.iter().filter_map(|&c| self.nodes.get(c)).collect();
                for (name, xml) in made.iter() {
                    let at = kids.iter().find(|k| rank(local(k)) > rank(name)).map_or(r.inner_end, |k| k.start);
                    edits.push((at, at, xml.clone()));
                }
            }
            None => {
                let lists: String = made.iter().map(|(_, xml)| xml.as_str()).collect();
                let ref_list = format!("<{hh}:refList>{lists}</{hh}:refList>");
                let tag = self.text.get(head.start..head.tag_end).unwrap_or("");
                if let Some(open) = tag.strip_suffix("/>") {
                    let name = &head.name;
                    edits.push((head.start, head.tag_end, format!("{}>{ref_list}</{name}>", open.trim_end())));
                } else {
                    let after =
                        head.children.iter().filter_map(|&c| self.nodes.get(c)).find(|k| local(k) == "beginNum").map_or(head.tag_end, |k| k.end);
                    edits.push((after, after, ref_list));
                }
            }
        }
    }
}

/// Children `xml` added at the end of element `n`, its start tag now `tag`. An element written
/// closed (`<hh:charProperties itemCnt="0"/>`) is opened for them: they'd otherwise go after it,
/// as its siblings.
fn add_children(n: &Node, tag: String, xml: String, edits: &mut Vec<Edit>) {
    match tag.strip_suffix("/>") {
        Some(open) => edits.push((n.start, n.tag_end, format!("{}>{xml}</{}>", open.trim_end(), n.name))),
        None => {
            edits.push((n.start, n.tag_end, tag));
            edits.push((n.inner_end, n.inner_end, xml));
        }
    }
}

/// Generated table XML with its shape ids turned into the header's. Only tags are remapped: the
/// same spelling in a cell's text (quotes are not escaped there) is the text, kept as it is.
pub(crate) fn remap_ids(xml: &str, chars: &[String], paras: &[String], fills: &[String]) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        out.push_str(rest.get(..lt).unwrap_or(""));
        // generated attribute values escape '>', so the first one ends the tag
        let len = rest.get(lt..).and_then(|t| t.find('>')).map_or(rest.len() - lt, |g| g + 1);
        out.push_str(&remap_tag(rest.get(lt..lt + len).unwrap_or(""), chars, paras, fills));
        rest = rest.get(lt + len..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// One tag of [`remap_ids`] with its shape ids turned into the header's.
fn remap_tag(tag: &str, chars: &[String], paras: &[String], fills: &[String]) -> String {
    let mut out = String::with_capacity(tag.len());
    let mut rest = tag;
    loop {
        let next = ["charPrIDRef=\"", "paraPrIDRef=\"", "borderFillIDRef=\""].iter().filter_map(|p| rest.find(p).map(|i| (i, *p))).min();
        let Some((i, pat)) = next else { break };
        let vs = i + pat.len();
        out.push_str(rest.get(..vs).unwrap_or(""));
        let after = rest.get(vs..).unwrap_or("");
        let close = after.find('"').unwrap_or(0);
        let old = after.get(..close).unwrap_or("");
        let n: usize = old.parse().unwrap_or(0);
        let new = match pat {
            "charPrIDRef=\"" => chars.get(n),
            "paraPrIDRef=\"" => paras.get(n),
            _ => fills.get(n.saturating_sub(1)),
        };
        out.push_str(new.map(String::as_str).unwrap_or(old));
        rest = after.get(close..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::HeaderEdit;
    use crate::hwpx_out::{FillKey, Tables};

    #[test]
    fn tables_rewritten_together_share_their_generated_shapes() {
        let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:borderFills itemCnt="1"><hh:borderFill id="1"/></hh:borderFills></hh:refList></hh:head>"#;
        let mut h = HeaderEdit::new(head.to_string());
        let mut t = Tables::default();
        t.fills.push(FillKey::none());
        let first = h.add_generated(&t);
        let second = h.add_generated(&t);
        assert_eq!(first, second);
        let out = h.finish().unwrap();
        assert_eq!(out.matches("<hh:borderFill id=").count(), 2, "{out}");
        assert!(out.contains(r#"itemCnt="2""#), "{out}");
    }

    /// Keep with next, keep lines, page break before and widow control turned on make a
    /// paragraph shape that says so. Before, the new shape kept the old break settings.
    #[test]
    fn a_paragraph_variant_carries_its_break_settings() {
        let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:paraProperties itemCnt="1"><hh:paraPr id="1"><hh:align horizontal="LEFT"/><hh:breakSetting breakLatinWord="KEEP_WORD" breakNonLatinWord="KEEP_WORD" widowOrphan="0" keepWithNext="0" keepLines="0" pageBreakBefore="0" lineWrap="BREAK"/></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#;
        let doc = wordcraft_doc::Document::from_text("x");
        let before = crate::hwpx_out::para_key(&doc.styles.resolve_para(&wordcraft_doc::ParaProps::default()), 10.0);
        let after = crate::hwpx_out::ParaKey { keep_next: true, keep_lines: true, page_break_before: true, widow: true, ..before.clone() };
        let mut h = HeaderEdit::new(head.to_string());
        let id = h.para_variant("1", &before, &after).unwrap().0;
        let out = h.finish().unwrap();
        let shape = &out[out.find(&format!("<hh:paraPr id=\"{id}\"")).unwrap()..];
        let setting = &shape[shape.find("<hh:breakSetting").unwrap()..];
        let setting = &setting[..setting.find("/>").unwrap()];
        for attr in [r#"keepWithNext="1""#, r#"keepLines="1""#, r#"pageBreakBefore="1""#, r#"widowOrphan="1""#] {
            assert!(setting.contains(attr), "{attr} in {setting}");
        }
    }

    /// A margin given in characters (unit="CHAR") and changed is written in HWPUNIT and says so.
    /// Before, the value was HWPUNIT but the unit stayed CHAR: a 20 pt margin read as 2000
    /// characters' worth.
    #[test]
    fn a_changed_margin_says_its_unit() {
        let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:paraProperties itemCnt="1"><hh:paraPr id="1"><hh:margin><hc:intent value="0" unit="CHAR"/><hc:left value="2" unit="CHAR"/><hc:right value="0" unit="HWPUNIT"/></hh:margin></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#;
        let doc = wordcraft_doc::Document::from_text("x");
        let before = crate::hwpx_out::para_key(&doc.styles.resolve_para(&wordcraft_doc::ParaProps::default()), 10.0);
        let after = crate::hwpx_out::ParaKey { left: 2000, ..before.clone() };
        let mut h = HeaderEdit::new(head.to_string());
        let id = h.para_variant("1", &before, &after).unwrap().0;
        let out = h.finish().unwrap();
        let shape = &out[out.find(&format!("<hh:paraPr id=\"{id}\"")).unwrap()..];
        let left = &shape[shape.find("<hc:left").unwrap()..];
        let left = &left[..left.find("/>").unwrap()];
        assert!(left.contains(r#"value="2000""#) && left.contains(r#"unit="HWPUNIT""#), "{left}");
        let intent = &shape[shape.find("<hc:intent").unwrap()..];
        assert!(intent[..intent.find("/>").unwrap()].contains(r#"unit="CHAR""#), "an unchanged margin keeps its unit");
    }

    /// Column widths, row heights and cell alignment are part of a table's signature: changing
    /// only them rewrites the table. Before, only the column count was, and the edit was lost.
    #[test]
    fn a_table_signature_includes_widths_and_heights() {
        let t = wordcraft_doc::Table::new(2, 2, 200.0);
        let base = super::table_signature(&t);
        let mut wider = t.clone();
        wider.grid = vec![150.0, 50.0];
        assert_ne!(super::table_signature(&wider), base, "column widths");
        let mut taller = t.clone();
        if let Some(r) = taller.rows.first_mut() {
            r.props.height = Some(40.0);
        }
        assert_ne!(super::table_signature(&taller), base, "row height");
        let mut aligned = t.clone();
        aligned.props.align = Some(wordcraft_doc::Align::Center);
        assert_ne!(super::table_signature(&aligned), base, "table alignment");
    }
    /// A font list whose ids skip a number (0 and 2): the existing font keeps its own id, and a
    /// new one takes an id above them all. Before, ids were positions: the font at id 2 read as 1,
    /// and a new font took 2, the id of a font runs already used.
    #[test]
    fn font_ids_are_the_files_own() {
        let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:fontfaces itemCnt="1"><hh:fontface lang="HANGUL" fontCnt="2"><hh:font id="0" face="A" type="TTF"/><hh:font id="2" face="B" type="TTF"/></hh:fontface></hh:fontfaces></hh:refList></hh:head>"#;
        let mut h = HeaderEdit::new(head.to_string());
        let hangul = crate::header::LANGS.iter().position(|l| l.eq_ignore_ascii_case("HANGUL")).unwrap();
        assert_eq!(h.font_id(hangul, "B"), 2);
        assert_eq!(h.font_id(hangul, "A"), 0);
        assert_eq!(h.font_id(hangul, "C"), 3);
        let out = h.finish().unwrap();
        assert!(out.contains(r#"<hh:font id="3" face="C""#), "{out}");
        assert!(out.contains(r#"fontCnt="3""#), "{out}");
    }
    /// Two runs of one character shape, one already bold (from its paragraph style) and one not,
    /// both made bold: each gets its own variant, and the one that wasn't bold has the bold edit.
    /// Before, the cache key held only the formatting after, and the second run took the first
    /// one's variant, with no bold.
    #[test]
    fn char_variants_are_keyed_by_the_formatting_before_too() {
        let head = r##"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:charProperties itemCnt="1"><hh:charPr id="1" height="1000" textColor="#000000"></hh:charPr></hh:charProperties></hh:refList></hh:head>"##;
        let doc = wordcraft_doc::Document::from_text("x");
        let plain = doc.styles.resolve_char(None, &wordcraft_doc::CharProps::default());
        let bold = wordcraft_doc::resolve::ResolvedChar { bold: true, ..plain.clone() };
        let mut h = HeaderEdit::new(head.to_string());
        let from_bold = h.char_variant("1", &bold, &bold);
        let from_plain = h.char_variant("1", &plain, &bold).unwrap();
        assert_ne!(from_bold.as_deref(), Some(from_plain.as_str()));
        let out = h.finish().unwrap();
        let shape = &out[out.find(&format!("<hh:charPr id=\"{from_plain}\"")).unwrap()..];
        assert!(shape[..shape.find("</hh:charPr>").unwrap()].contains("<hh:bold"), "{shape}");
    }
    /// Two paragraphs of one shape with a margin of one character, at 20 and 10 pt (20 and 10 pt
    /// of margin), both given a 20 pt margin: the second writes its margin, the first needs no
    /// change there. Before, the cache key held only the result, and the 10 pt paragraph took the
    /// first one's variant, which kept the margin in characters (10 pt there).
    #[test]
    fn para_variants_are_keyed_by_the_formatting_before_too() {
        let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:paraProperties itemCnt="1"><hh:paraPr id="1"><hh:align horizontal="LEFT"/><hh:margin><hc:left value="1" unit="CHAR"/></hh:margin></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#;
        let doc = wordcraft_doc::Document::from_text("x");
        let base = crate::hwpx_out::para_key(&doc.styles.resolve_para(&wordcraft_doc::ParaProps::default()), 10.0);
        let at20 = crate::hwpx_out::ParaKey { left: 2000, ..base.clone() };
        let at10 = crate::hwpx_out::ParaKey { left: 1000, ..base.clone() };
        let centred = |k: &crate::hwpx_out::ParaKey| crate::hwpx_out::ParaKey { align: "CENTER", left: 2000, ..k.clone() };
        let mut h = HeaderEdit::new(head.to_string());
        let first = h.para_variant("1", &at20, &centred(&at20)).unwrap().0;
        let second = h.para_variant("1", &at10, &centred(&at10)).unwrap().0;
        assert_ne!(first, second);
        let out = h.finish().unwrap();
        let shape = &out[out.find(&format!("<hh:paraPr id=\"{second}\"")).unwrap()..];
        let left = &shape[shape.find("<hc:left").unwrap()..];
        assert!(left[..left.find("/>").unwrap()].contains(r#"value="2000""#), "{left}");
    }
    /// A shape list written closed (`<hh:borderFills itemCnt="0"/>`) is opened for the shapes
    /// added to it, so they are its children. Before, they went after it, outside the list.
    #[test]
    fn shapes_go_into_a_list_written_closed() {
        let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:borderFills itemCnt="0"/></hh:refList></hh:head>"#;
        let mut h = HeaderEdit::new(head.to_string());
        let mut t = Tables::default();
        t.fills.push(FillKey::none());
        h.add_generated(&t);
        let out = h.finish().unwrap();
        let root = crate::xml::parse(out.as_bytes(), false).unwrap();
        let list = root.iter("hh:borderFills").next().expect("the list");
        assert!(list.findall("hh:borderFill").count() >= 1, "{out}");
        assert_eq!(list.get("itemCnt"), Some(list.findall("hh:borderFill").count().to_string().as_str()), "{out}");
    }
    /// A character shape naming no fonts (no fontRef) whose font changes gets a fontRef with
    /// the new font in its slot. Before, the change was skipped but counted, and the text kept
    /// its font.
    #[test]
    fn a_font_change_without_a_font_ref_is_written() {
        let head = r##"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:fontfaces itemCnt="2"><hh:fontface lang="HANGUL" fontCnt="1"><hh:font id="0" face="A" type="TTF"/></hh:fontface><hh:fontface lang="LATIN" fontCnt="1"><hh:font id="0" face="A" type="TTF"/></hh:fontface></hh:fontfaces><hh:charProperties itemCnt="1"><hh:charPr id="1" height="1000" textColor="#000000"><hh:ratio hangul="100"/></hh:charPr></hh:charProperties></hh:refList></hh:head>"##;
        let doc = wordcraft_doc::Document::from_text("x");
        let before = wordcraft_doc::resolve::ResolvedChar {
            font: "A".into(),
            font_ea: Some("A".into()),
            ..doc.styles.resolve_char(None, &wordcraft_doc::CharProps::default())
        };
        let after = wordcraft_doc::resolve::ResolvedChar { font: "B".into(), ..before.clone() };
        let mut h = HeaderEdit::new(head.to_string());
        let id = h.char_variant("1", &before, &after).unwrap();
        let out = h.finish().unwrap();
        let root = crate::xml::parse(out.as_bytes(), false).unwrap();
        let cp = root.iter("hh:charPr").find(|c| c.get("id") == Some(id.as_str())).expect("the variant");
        let fr = cp.find("hh:fontRef").expect("a fontRef");
        let latin = root.iter("hh:fontface").find(|f| f.get("lang") == Some("LATIN")).unwrap();
        let face = latin.iter("hh:font").find(|f| f.get("id") == fr.get("latin")).and_then(|f| f.get("face"));
        assert_eq!(face, Some("B"), "{out}");
    }
    /// A character shape written empty (`<hh:charPr .../>`) made bold gets its bold inside the
    /// variant. Before, the bold went after the empty tag, outside the shape.
    #[test]
    fn an_empty_char_shape_takes_its_new_children_inside() {
        let head = r##"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:charProperties itemCnt="1"><hh:charPr id="1" height="1000" textColor="#000000"/></hh:charProperties></hh:refList></hh:head>"##;
        let doc = wordcraft_doc::Document::from_text("x");
        let plain = doc.styles.resolve_char(None, &wordcraft_doc::CharProps::default());
        let bold = wordcraft_doc::resolve::ResolvedChar { bold: true, ..plain.clone() };
        let mut h = HeaderEdit::new(head.to_string());
        let id = h.char_variant("1", &plain, &bold).unwrap();
        let out = h.finish().unwrap();
        let root = crate::xml::parse(out.as_bytes(), false).unwrap();
        let cp = root.iter("hh:charPr").find(|c| c.get("id") == Some(id.as_str())).expect("the variant");
        assert!(cp.find("hh:bold").is_some(), "{out}");
    }

    /// A paragraph shape without break settings or alignment whose paragraph keeps with the
    /// next and is centred gets both elements; a margin it has no element for is reported.
    /// Before, the new shape lacked them and the change was counted as written.
    #[test]
    fn a_paragraph_variant_adds_elements_its_source_lacks() {
        let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:paraProperties itemCnt="1"><hh:paraPr id="1"><hh:heading type="NONE" idRef="0" level="0"/></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#;
        let doc = wordcraft_doc::Document::from_text("x");
        let before = crate::hwpx_out::para_key(&doc.styles.resolve_para(&wordcraft_doc::ParaProps::default()), 10.0);
        let after = crate::hwpx_out::ParaKey { keep_next: true, align: "CENTER", left: 2000, ..before.clone() };
        let mut h = HeaderEdit::new(head.to_string());
        let (id, unwritten) = h.para_variant("1", &before, &after).unwrap();
        assert_eq!(unwritten, vec!["left indent"]);
        let out = h.finish().unwrap();
        let root = crate::xml::parse(out.as_bytes(), false).unwrap();
        let pp = root.iter("hh:paraPr").find(|c| c.get("id") == Some(id.as_str())).expect("the variant");
        assert_eq!(pp.find("hh:breakSetting").and_then(|b| b.get("keepWithNext")), Some("1"), "{out}");
        assert_eq!(pp.find("hh:align").and_then(|b| b.get("horizontal")), Some("CENTER"), "{out}");
        let names: Vec<&str> = pp.children.iter().map(|c| c.tag.as_str()).collect();
        assert_eq!(names, ["hh:align", "hh:heading", "hh:breakSetting"], "in a paragraph shape's order");
    }
}

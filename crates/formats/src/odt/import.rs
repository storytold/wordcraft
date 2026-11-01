use super::body::Body;
use super::limits::{MAX_ENTRY, MAX_SECTIONS};
use super::styles::{Styles, column_sep, columns_of};
use super::xml::{Attrs, Ev, events, get, length, local};
use super::{Columns, Cursor, Document, FBlock, Flow, HashMap, Meta, Para, Read, SectionProps, model, page_parts};

pub(super) fn read_entry(z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
    let f = z.by_name(name).ok()?;
    if f.size() > MAX_ENTRY {
        return None;
    }
    let mut out = Vec::new();
    f.take(MAX_ENTRY).read_to_end(&mut out).ok()?;
    Some(out)
}

/// The pictures a package's frames point at, within one byte budget. Each entry is inflated
/// at most once, and only when the size it declares fits in what is left; the bytes inflated
/// (even from an entry bigger than it declares) and every later copy are charged, so repeated
/// references and lying entries can't go past the budget, and nothing is inflated after it.
pub(super) struct Pictures {
    /// Entry name → its bytes (`None`: missing, unreadable or over the budget; not retried).
    seen: HashMap<String, Option<Vec<u8>>>,
    /// Bytes still allowed.
    pub(super) room: u64,
}

impl Pictures {
    pub(super) fn new(budget: u64) -> Self {
        Pictures { seen: HashMap::new(), room: budget }
    }

    /// The bytes of entry `name`, or `None` when it can't be read or doesn't fit.
    pub(super) fn get(&mut self, z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
        if let Some(hit) = self.seen.get(name) {
            let d = hit.as_ref().filter(|d| d.len() as u64 <= self.room)?;
            self.room = self.room.saturating_sub(d.len() as u64);
            return Some(d.clone());
        }
        let d = self.inflate(z, name);
        self.seen.insert(name.to_string(), d.clone());
        d
    }

    /// Inflate entry `name`, charging the bytes inflated; `None` when it doesn't fit or fails.
    fn inflate(&mut self, z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
        let room = self.room;
        if room == 0 {
            return None;
        }
        let f = z.by_name(name).ok()?;
        if f.size() > room {
            return None;
        }
        let mut out = Vec::new();
        let read = f.take(room.saturating_add(1)).read_to_end(&mut out);
        self.room = room.saturating_sub(out.len() as u64);
        (read.is_ok() && out.len() as u64 <= room).then_some(out)
    }
}

pub(super) fn read_meta(xml: &[u8]) -> Meta {
    let mut m = Meta::default();
    let mut cur: Option<String> = None;
    let mut keywords: Vec<String> = Vec::new();
    let mut creator = String::new();
    let mut mod_date = String::new();
    events(xml, |ev| match ev {
        Ev::Start(q, _) => cur = Some(q),
        Ev::End(_) => cur = None,
        Ev::Text(t) => match cur.as_deref() {
            Some("dc:title") => m.title.push_str(&t),
            Some("dc:subject") => m.subject.push_str(&t),
            Some("dc:description") => m.description.push_str(&t),
            Some("meta:initial-creator") => m.author.push_str(&t),
            Some("dc:creator") => creator.push_str(&t),
            Some("meta:keyword") => keywords.push(t),
            Some("meta:creation-date") => m.created.push_str(&t),
            Some("dc:date") => mod_date.push_str(&t),
            _ => {}
        },
    });
    if m.author.is_empty() {
        m.author = creator;
    }
    if m.created.is_empty() {
        m.created = mod_date;
    }
    m.keywords = keywords.join(", ");
    let created = m.created.trim();
    if !created.is_empty() {
        let has_tz =
            created.ends_with(['Z', 'z']) || created.contains('+') || created.split_once('T').map(|(_, time)| time.contains('-')).unwrap_or(false);
        if !has_tz {
            let base = created.split('.').next().unwrap_or("");
            m.created = format!("{base}Z");
        } else {
            m.created = created.to_string();
        }
    } else {
        m.created.clear();
    }
    m
}

/// Give the document's style for paragraph kind `kind` the look the file gives its own: its text
/// formatting, spacing, alignment and keep with next replace WordCraft's (a heading the file
/// leaves black isn't blue). Its outline level and other settings stay.
fn apply_look(doc: &mut Document, kind: model::Kind, look: &super::styles::ParaLook) {
    let id = match kind {
        model::Kind::Heading(n) => format!("Heading{}", n.clamp(1, 6)),
        model::Kind::Title => "Title".into(),
        model::Kind::Quote => "Quote".into(),
        model::Kind::Code => model::CODE_STYLE.into(),
        model::Kind::Normal | model::Kind::Rule => return,
    };
    let Some(st) = doc.styles.get_mut(&id) else { return };
    st.chr = model::char_props(&look.fmt);
    let sp = &look.spacing;
    let p = &mut st.para;
    (p.space_before, p.space_after, p.line_spacing) = (sp.before, sp.after, sp.line);
    (p.indent_left, p.indent_right, p.indent_first) = (sp.left, sp.right, sp.first);
    p.align = look.align;
    p.keep_next = look.keep_next.then_some(true);
}

/// The boolean configuration item `name` of `settings.xml` (ODF 1.3 §3.10), when it has one.
fn setting(xml: &[u8], name: &str) -> Option<bool> {
    let mut inside = false;
    let mut value = None;
    events(xml, |ev| match ev {
        Ev::Start(q, a) => inside = q == "config:config-item" && get(&a, "config:name") == Some(name) && value.is_none(),
        Ev::Text(t) if inside => value = Some(t.trim() == "true"),
        _ => inside = false,
    });
    value
}

/// Parse an `.odt` package into the flow model.
pub fn parse(bytes: &[u8]) -> Result<Flow, String> {
    parse_full(bytes).map(|(flow, _)| flow)
}

/// [`parse`], with the last section's properties (the others are on the paragraphs that end them).
fn parse_full(bytes: &[u8]) -> Result<(Flow, SectionProps), String> {
    parse_tracked(bytes).map(|p| (p.flow, p.last))
}

/// What [`parse_tracked`] reads besides the flow.
struct Parsed {
    flow: Flow,
    /// The last section's properties.
    last: SectionProps,
    /// Whether change tracking is on (`None`: the document has no tracked changes).
    track: Option<bool>,
    /// The document's default font.
    font: Option<String>,
    /// How the file's heading, title, quotation and preformatted styles look.
    looks: Vec<(model::Kind, super::styles::ParaLook)>,
    /// The document's default paragraph spacing.
    spacing: model::Spacing,
    /// Whether the document kerns pairs of letters by default.
    kern: bool,
    /// The default tab interval, points.
    default_tab: Option<f32>,
    parts: page_parts::Parts,
}

/// [`parse_full`], plus what the document holds outside the flow.
fn parse_tracked(bytes: &[u8]) -> Result<Parsed, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("ODT: not a valid package ({e})"))?;
    if let Some(mt) = read_entry(&mut z, "mimetype") {
        let mt = String::from_utf8_lossy(&mt);
        if !mt.trim().starts_with("application/vnd.oasis.opendocument.text") {
            return Err(format!("ODT: not a text document ({})", mt.trim()));
        }
    }
    let content = read_entry(&mut z, "content.xml").ok_or("ODT: content.xml is missing")?;
    let mut styles = Styles::default();
    let mut pages = Pages::default();
    let styles_xml = read_entry(&mut z, "styles.xml");
    if let Some(s) = &styles_xml {
        styles.read(s, false);
        pages = read_pages(s);
    }
    styles.read(&content, true);
    styles.tabs_from_indent = read_entry(&mut z, "settings.xml").map(|s| setting(&s, "TabsRelativeToIndent")).unwrap_or(None).unwrap_or(true);
    let meta = read_entry(&mut z, "meta.xml").map(|m| read_meta(&m)).unwrap_or_default();
    let mut pictures = Pictures::new(MAX_ENTRY);
    let mut fetch = |path: &str| -> Option<Vec<u8>> {
        // A frame may point at any entry, as often as it likes: `Pictures` keeps that bounded.
        pictures.get(&mut z, path)
    };
    let parts = styles_xml.as_deref().map(|x| page_parts::read(x, &styles, &mut pages, &mut fetch)).unwrap_or_default();
    let mut body = Body::new(&styles, &pages, &mut fetch);
    events(&content, |ev| body.feed(ev));
    let mut blocks = body.take_blocks();
    // An open section with nothing in it (after a column section at the end) isn't one: the
    // section before it ends the document.
    let mut last = body.sec;
    if blocks.len() <= body.sec_from
        && let Some(FBlock::Para(p)) = blocks.last_mut()
        && let Some(s) = p.section.take()
    {
        last = *s;
    }
    if blocks.is_empty() {
        blocks.push(FBlock::Para(Para::default()));
    }
    let track = body.track;
    Ok(Parsed {
        flow: Flow { blocks, meta },
        last,
        track,
        font: styles.doc_font(),
        spacing: styles.doc_spacing(),
        looks: styles.kind_looks(),
        kern: styles.default_kern,
        default_tab: styles.default_tab,
        parts,
    })
}

/// Parse an `.odt` package into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    model::begin_import();
    let Parsed { flow, last, track, font, spacing, looks, kern, default_tab, parts } = parse_tracked(bytes)?;
    let mut doc = model::to_doc(&flow);
    if font.is_some() {
        doc.styles.default_chr.font = font;
    }
    doc.styles.default_chr.kern = kern.then_some(1.0);
    if let Some(t) = default_tab {
        doc.settings.default_tab = t;
    }
    let d = &mut doc.styles.default_para;
    (d.space_before, d.space_after, d.line_spacing) = (spacing.before, spacing.after, spacing.line);
    (d.indent_left, d.indent_right, d.indent_first) = (spacing.left, spacing.right, spacing.first);
    for (kind, look) in &looks {
        apply_look(&mut doc, *kind, look);
    }
    doc.last_section = last;
    page_parts::add(&mut doc, parts);
    if let Some(t) = track {
        doc.settings.track_changes = t;
    }
    Ok(doc)
}

/// The page setup of each master page in `styles.xml` (ODF 1.3 §16.9), and of the first one,
/// which pages use until a paragraph names another.
#[derive(Default)]
pub(super) struct Pages {
    pub(super) masters: HashMap<String, SectionProps>,
    pub(super) first: Option<SectionProps>,
    pub(super) first_name: Option<String>,
}

/// A page layout's header or footer area (`style:header-style`/`style:footer-style`, ODF 1.3
/// §16.6–16.7): its minimum height and its spacing from the text, in points.
#[derive(Clone, Copy, Default)]
struct Band {
    height: f32,
    spacing: f32,
}

/// A page layout read from `styles.xml`: its properties, columns, and header and footer areas.
#[derive(Default)]
struct Layout {
    attrs: Attrs,
    columns: Option<Columns>,
    header: Band,
    footer: Band,
}

fn read_pages(xml: &[u8]) -> Pages {
    let mut layouts: HashMap<String, Layout> = HashMap::new();
    let mut cur: Option<String> = None;
    // Inside a layout's footer style (else its header style).
    let mut in_footer = false;
    // Master pages: name, layout, whether a header and a footer show.
    let mut masters: Vec<(String, String, bool, bool)> = Vec::new();
    let mut in_master = false;
    events(xml, |ev| match ev {
        Ev::Start(q, a) => match local(&q) {
            "page-layout" if q.starts_with("style:") => cur = get(&a, "style:name").map(String::from),
            "header-style" => in_footer = false,
            "footer-style" => in_footer = true,
            "header-footer-properties" => {
                if let Some(l) = cur.as_ref().and_then(|n| layouts.get_mut(n)) {
                    let len = |k: &str| get(&a, k).and_then(length).filter(|v| *v >= 0.0).map(|v| v.min(1584.0));
                    let band = if in_footer { &mut l.footer } else { &mut l.header };
                    band.height = len("fo:min-height").or_else(|| len("svg:height")).unwrap_or(0.0);
                    band.spacing = len(if in_footer { "fo:margin-top" } else { "fo:margin-bottom" }).unwrap_or(0.0);
                }
            }
            "page-layout-properties" => {
                if let Some(n) = &cur
                    && layouts.len() < MAX_SECTIONS
                {
                    layouts.insert(n.clone(), Layout { attrs: a, ..Default::default() });
                }
            }
            "columns" if q.starts_with("style:") => {
                if let Some(l) = cur.as_ref().and_then(|n| layouts.get_mut(n)) {
                    l.columns = Some(columns_of(&a));
                }
            }
            "column-sep" => {
                if let Some(c) = cur.as_ref().and_then(|n| layouts.get_mut(n)).and_then(|l| l.columns.as_mut()) {
                    c.separator = column_sep(&a);
                }
            }
            "master-page" if masters.len() < MAX_SECTIONS => {
                in_master = false;
                if let (Some(n), Some(l)) = (get(&a, "style:name"), get(&a, "style:page-layout-name")) {
                    masters.push((n.to_string(), l.to_string(), false, false));
                    in_master = true;
                }
            }
            "header" | "footer" if in_master && q.starts_with("style:") && get(&a, "style:display") != Some("false") => {
                if let Some(m) = masters.last_mut() {
                    if local(&q) == "header" {
                        m.2 = true;
                    } else {
                        m.3 = true;
                    }
                }
            }
            _ => {}
        },
        Ev::End(q) => match local(&q) {
            "page-layout" => cur = None,
            "master-page" => in_master = false,
            _ => {}
        },
        Ev::Text(_) => {}
    });
    let mut pages = Pages::default();
    for (name, layout, header, footer) in masters {
        let Some(Layout { attrs: a, columns: cols, header: hb, footer: fb }) = layouts.get(&layout) else { continue };
        let mut sec = SectionProps::default();
        apply_layout(a, &mut sec);
        // ODF starts a header at the page margin and the text after the header's band; Word
        // measures the header's distance and the text's margin from the edge. The band's height
        // takes in its spacing (as LibreOffice draws it: a 2 cm header with 1 cm spacing puts
        // the text 2 cm below the margin), so the spacing counts only past the height.
        let limit = sec.page_w.max(sec.page_h);
        if header {
            sec.header = sec.margin_top;
            sec.margin_top = (sec.margin_top + hb.height.max(hb.spacing)).min(limit);
        }
        if footer {
            sec.footer = sec.margin_bottom;
            sec.margin_bottom = (sec.margin_bottom + fb.height.max(fb.spacing)).min(limit);
        }
        // A one-column layout keeps the default columns (and their gap).
        if let Some(c) = cols.as_ref().filter(|c| c.count > 1) {
            sec.columns = c.clone();
        }
        if pages.first.is_none() {
            pages.first = Some(sec.clone());
            pages.first_name = Some(name.clone());
        }
        pages.masters.entry(name).or_insert(sec);
    }
    pages
}

/// Replace `sec` with the page setup of the first master page in `styles.xml`, if any.
#[cfg(test)]
pub(super) fn apply_page_layout(xml: &[u8], sec: &mut SectionProps) {
    if let Some(first) = read_pages(xml).first {
        *sec = first;
    }
}

/// A `style:page-layout-properties` element's page size, orientation and margins into `sec`.
fn apply_layout(a: &Attrs, sec: &mut SectionProps) {
    let len = |k: &str| get(a, k).and_then(length).filter(|v| *v >= 0.0);
    // Like the DOCX reader: a page is at least half an inch; at most 22 inches (Word's limit), so a hostile
    // `1e30pt` can't reach layout.
    let page = |k: &str| len(k).filter(|v| *v >= 36.0).map(|v| v.min(1584.0));
    if let (Some(w), Some(h)) = (page("fo:page-width"), page("fo:page-height")) {
        sec.page_w = w;
        sec.page_h = h;
    }
    sec.landscape = match get(a, "style:print-orientation") {
        Some("landscape") => true,
        Some("portrait") => false,
        _ => sec.page_w > sec.page_h,
    };
    for (k, slot) in [
        ("fo:margin-top", &mut sec.margin_top),
        ("fo:margin-bottom", &mut sec.margin_bottom),
        ("fo:margin-left", &mut sec.margin_left),
        ("fo:margin-right", &mut sec.margin_right),
    ] {
        if let Some(v) = len(k) {
            // No margin is larger than the page's longer side (as `w:pgMar` in the DOCX reader).
            *slot = v.min(sec.page_w.max(sec.page_h));
        }
    }
}

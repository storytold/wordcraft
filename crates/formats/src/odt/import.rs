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

/// Every paragraph of `blocks`, in table cells, notes and comments too.
fn each_para(blocks: &mut [FBlock], depth: usize, f: &mut dyn FnMut(&mut Para)) {
    if depth > model::MAX_DEPTH {
        return;
    }
    for b in blocks {
        match b {
            FBlock::Para(p) => {
                f(p);
                for i in &mut p.inlines {
                    match i {
                        model::Inline::Note { blocks, .. } => each_para(blocks, depth + 1, f),
                        model::Inline::Comment { comment, .. } => each_para(&mut comment.blocks, depth + 1, f),
                        _ => {}
                    }
                }
            }
            FBlock::Table(t) => {
                for c in t.rows.iter_mut().flatten() {
                    each_para(&mut c.blocks, depth + 1, f);
                }
            }
        }
    }
}

/// The custom character styles the text of `blocks` names (by ODF name, in [`model::Fmt::extra`])
/// with their custom parents, parents first, each with its document style id, added to `found`;
/// the text then names them by id. `para_ids` are the paragraph styles' ids, which a character
/// style's id doesn't take.
fn custom_char_styles(blocks: &mut [FBlock], styles: &Styles, para_ids: &[String], found: &mut Vec<(String, String)>) {
    let id_for = |odt: &str| {
        let id = super::styles::style_id(odt);
        if para_ids.contains(&id) { format!("{id}Char") } else { id }
    };
    each_para(blocks, 0, &mut |p| {
        for i in &mut p.inlines {
            let fmt = match i {
                model::Inline::Text(_, f) | model::Inline::Field { fmt: f, .. } | model::Inline::Ruby { fmt: f, .. } => f,
                _ => continue,
            };
            let Some(odt) = fmt.extra.as_mut().and_then(|x| x.style.take()) else { continue };
            let mut chain: Vec<String> = Vec::new();
            let mut cur = Some(odt.clone());
            while let Some(n) = cur.take() {
                if found.iter().any(|(o, _)| *o == n) || chain.contains(&n) || chain.len() >= 16 {
                    break;
                }
                cur = styles.named_text(&n).and_then(|(_, parent, _)| parent);
                chain.push(n);
            }
            for n in chain.into_iter().rev() {
                let id = id_for(&n);
                found.push((n, id));
            }
            if let Some(x) = fmt.extra.as_mut() {
                x.style = found.iter().find(|(o, _)| *o == odt).map(|(_, id)| id.clone());
            }
        }
    });
}

/// Add the custom character styles `found` (ODF name, document id; parents first) built from
/// `defs` (name, parent's ODF name, own formatting), alongside.
fn add_char_styles(doc: &mut Document, found: &[(String, String)], defs: &[(String, Option<String>, model::Fmt)]) {
    for ((_, id), (name, parent, fmt)) in found.iter().zip(defs) {
        let based_on = parent.as_ref().and_then(|p| found.iter().find(|(o, _)| o == p).map(|(_, id)| id.clone())).filter(|b| b != id);
        doc.styles.upsert(wordcraft_doc::styles::Style {
            id: id.clone(),
            name: name.clone(),
            kind: wordcraft_doc::styles::StyleKind::Character,
            based_on,
            chr: model::char_props(fmt),
            ..Default::default()
        });
    }
}

/// The custom styles the paragraphs of `blocks` name (by ODF name, [`super::styles::ParaLook::named`])
/// with their parents, parents first; the paragraphs then name them by document style id.
/// `odt` holds the styles already found (in other parts), which aren't added again.
fn custom_styles(blocks: &mut [FBlock], styles: &Styles, odt: Vec<String>) -> (Vec<String>, Vec<super::styles::NamedStyle>) {
    let mut odt = odt;
    let mut defs: Vec<super::styles::NamedStyle> = Vec::new();
    each_para(blocks, 0, &mut |p| {
        let Some(name) = p.extra.as_mut().and_then(|x| x.style.take()) else { return };
        // The style and its custom parents, parents first, each once (a loop ends the chain).
        let mut chain: Vec<(String, super::styles::NamedStyle)> = Vec::new();
        let mut cur = Some(name.clone());
        while let Some(n) = cur.take() {
            if odt.contains(&n) || chain.iter().any(|(m, _)| *m == n) || chain.len() >= 16 {
                break;
            }
            let Some(def) = styles.named_style(&n) else { break };
            if let super::styles::Parent::Named(p) = &def.parent {
                cur = Some(p.clone());
            }
            chain.push((n, def));
        }
        for (n, def) in chain.into_iter().rev() {
            odt.push(n);
            defs.push(def);
        }
        // A custom style, or an index style WordCraft has under its own id (TOC 1…).
        if (odt.contains(&name) || super::writer::generating_style(&name).is_some())
            && let Some(x) = p.extra.as_mut()
        {
            x.style = Some(super::styles::style_id(&name));
        }
    });
    (odt, defs)
}

/// Add the custom styles `named` (ODF names `odt`, parents first) to the document: each with its
/// name, the style it is based on and its own formatting. A style WordCraft has under the same id
/// takes the file's definition.
fn add_custom_styles(doc: &mut Document, named: &[super::styles::NamedStyle], odt: &[String], tabs_from_indent: bool) {
    use super::styles::{Parent, style_id};
    for (def, name) in named.iter().zip(odt) {
        let id = style_id(name);
        let based_on = match &def.parent {
            Parent::Normal => "Normal".to_string(),
            Parent::Kind(model::Kind::Heading(n)) => format!("Heading{}", (*n).clamp(1, 6)),
            Parent::Kind(model::Kind::Title) => "Title".into(),
            Parent::Kind(model::Kind::Quote) => "Quote".into(),
            Parent::Kind(model::Kind::Code) if doc.styles.get(model::CODE_STYLE).is_some() => model::CODE_STYLE.into(),
            Parent::Kind(_) => "Normal".into(),
            Parent::Named(p) => style_id(p),
        };
        let look = &def.look;
        let sp = &look.spacing;
        let para = wordcraft_doc::props::ParaProps {
            space_before: sp.before,
            space_after: sp.after,
            line_spacing: sp.line,
            indent_left: sp.left,
            indent_right: sp.right,
            indent_first: sp.first,
            align: look.align,
            keep_next: look.keep_next.then_some(true),
            borders: look.borders,
            shading: look.shading,
            // LibreOffice measures tabs from the indent (`TabsRelativeToIndent`); Word from the
            // margin.
            tabs: look.tabs.clone().map(|mut tabs| {
                if tabs_from_indent {
                    let indent = sp.left.unwrap_or(0.0);
                    for t in &mut tabs {
                        t.pos = (t.pos + indent).clamp(-1584.0, 1584.0);
                    }
                }
                tabs
            }),
            contextual_spacing: look.contextual,
            bidi: look.rtl.filter(|r| *r),
            ..Default::default()
        };
        let name = super::writer::generating_style(name).map_or_else(|| def.name.clone(), |(_, n)| n);
        let style = wordcraft_doc::styles::Style {
            id: id.clone(),
            name,
            kind: wordcraft_doc::styles::StyleKind::Paragraph,
            based_on: (based_on != id).then_some(based_on),
            para,
            chr: model::char_props(&look.fmt),
            ..Default::default()
        };
        doc.styles.upsert(style);
    }
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
    if look.contextual.is_some() {
        p.contextual_spacing = look.contextual;
    }
}

/// Number the headings as the outline style `levels` does (ODF 1.3 §16.34): one list whose
/// levels carry the outline's formats, each linked to its heading style, which numbers its
/// paragraphs through it, as Word's numbered headings do. Levels without a number are left so.
fn number_headings(doc: &mut Document, levels: &[super::styles::ListLevel]) {
    use wordcraft_doc::numbering::{AbstractNum, ListKind, Num, levels_for};
    use wordcraft_doc::props::NumRef;
    let numbered: Vec<bool> = (0..6).map(|i| levels.get(i).is_some_and(|l| l.ordered)).collect();
    if !numbered.contains(&true) {
        return;
    }
    let mut out = levels_for(ListKind::Numbered);
    for (i, level) in out.iter_mut().enumerate() {
        let src = levels.get(i).copied().unwrap_or_default();
        // Headings sit where the outline level puts them, else at the margin; their number is
        // followed by a tab.
        (level.indent, level.hanging) = src.label.and_then(|l| l.position()).unwrap_or((0.0, 0.0));
        if let Some(a) = src.label.and_then(|l| l.align) {
            level.align = a;
        }
        match src.label.filter(|_| src.ordered) {
            Some(l) => {
                level.format = l.format;
                level.text = l.text(i as u8);
                level.start = src.start;
            }
            None => {
                level.format = wordcraft_doc::section::NumFormat::None;
                level.text = String::new();
            }
        }
        if i < 6 {
            level.style = Some(format!("Heading{}", i + 1));
        }
    }
    let numbering = &mut doc.numbering;
    let aid = numbering.abstracts.iter().map(|a| a.id + 1).max().unwrap_or(0);
    numbering.abstracts.push(AbstractNum { id: aid, name: None, levels: out, ..Default::default() });
    let nid = numbering.nums.iter().map(|n| n.id + 1).max().unwrap_or(1).max(1);
    numbering.nums.push(Num { id: nid, abstract_id: aid, ..Default::default() });
    for (i, on) in numbered.iter().enumerate() {
        if *on && let Some(st) = doc.styles.get_mut(&format!("Heading{}", i + 1)) {
            st.para.numbering = Some(NumRef { num: nid, level: i as u8 });
        }
    }
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
    /// The custom paragraph styles the paragraphs use, parents included, parents first.
    named: Vec<super::styles::NamedStyle>,
    /// Their ODF names, alongside.
    named_odt: Vec<String>,
    /// The custom character styles the text uses: ODF name and document id, parents first.
    char_styles: Vec<(String, String)>,
    /// Their names, parents and own formatting, alongside.
    char_defs: Vec<(String, Option<String>, model::Fmt)>,
    /// The file's `settings.xml`, kept for the next save.
    settings: Option<Vec<u8>>,
    /// The heading numbering (`text:outline-style`), level by level.
    outline: Option<Vec<super::styles::ListLevel>>,
    /// How the file's heading, title, quotation and preformatted styles look.
    looks: Vec<(model::Kind, super::styles::ParaLook)>,
    /// The document's default paragraph spacing.
    spacing: model::Spacing,
    /// Whether the document kerns pairs of letters by default.
    kern: bool,
    /// The document's default Asian and complex-script fonts and languages.
    scripts: wordcraft_doc::props::CharProps,
    /// The default tab interval, points.
    default_tab: Option<f32>,
    /// Tab stops are measured from the indent (LibreOffice's `TabsRelativeToIndent`).
    tabs_from_indent: bool,
    /// The page settings that are the document's.
    page_doc: PageDoc,
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
    let settings = read_entry(&mut z, "settings.xml");
    styles.tabs_from_indent = settings.as_deref().and_then(|s| setting(s, "TabsRelativeToIndent")).unwrap_or(true);
    let meta = read_entry(&mut z, "meta.xml").map(|m| read_meta(&m)).unwrap_or_default();
    let entries: Vec<String> = z.file_names().take(100_000).map(str::to_string).collect();
    let manifest = read_entry(&mut z, "META-INF/manifest.xml").map(|m| super::objects::manifest(&m)).unwrap_or_default();
    let mut pictures = Pictures::new(MAX_ENTRY);
    let mut fetch = |path: &str| -> Option<Vec<u8>> {
        // A frame may point at any entry, as often as it likes: `Pictures` keeps that bounded.
        pictures.get(&mut z, path)
    };
    let objects = (entries.as_slice(), &manifest);
    let mut parts = styles_xml.as_deref().map(|x| page_parts::read(x, &styles, &mut pages, &mut fetch, objects)).unwrap_or_default();
    let mut body = Body::new(&styles, &pages, &mut fetch);
    (body.entries, body.manifest) = (objects.0, Some(objects.1));
    events(&content, |ev| body.feed(ev));
    let mut blocks = body.take_blocks();
    let (mut named_odt, mut named) = custom_styles(&mut blocks, &styles, Vec::new());
    for b in parts.blocks_mut() {
        let (o, n) = custom_styles(b, &styles, named_odt);
        named_odt = o;
        named.extend(n);
    }
    let para_ids: Vec<String> = named_odt.iter().map(|n| super::styles::style_id(n)).collect();
    let mut char_styles = Vec::new();
    custom_char_styles(&mut blocks, &styles, &para_ids, &mut char_styles);
    for b in parts.blocks_mut() {
        custom_char_styles(b, &styles, &para_ids, &mut char_styles);
    }
    let char_defs = char_styles.iter().filter_map(|(o, _)| styles.named_text(o)).collect();
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
        named,
        named_odt,
        char_styles,
        char_defs,
        settings: settings.filter(|s| s.len() <= super::package::MAX_SETTINGS),
        outline: styles.outline.clone(),
        kern: styles.default_kern,
        scripts: styles.default_scripts.clone(),
        default_tab: styles.default_tab,
        tabs_from_indent: styles.tabs_from_indent,
        page_doc: pages.doc.clone(),
        parts,
    })
}

/// Parse an `.odt` package into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    model::begin_import();
    let Parsed {
        flow,
        last,
        track,
        font,
        spacing,
        looks,
        named,
        named_odt,
        char_styles,
        char_defs,
        settings,
        outline,
        kern,
        scripts,
        default_tab,
        tabs_from_indent,
        page_doc,
        parts,
    } = parse_tracked(bytes)?;
    let mut doc = model::to_doc(&flow);
    if font.is_some() {
        doc.styles.default_chr.font = font;
    }
    doc.styles.default_chr.kern = kern.then_some(1.0);
    doc.styles.default_chr.overlay(&scripts);
    if let Some(t) = default_tab {
        doc.settings.default_tab = t;
    }
    doc.settings.page_color = page_doc.page_color;
    if let Some((fmt, props)) = page_doc.notes[0] {
        (doc.settings.footnote_format, doc.settings.footnote_pr) = (fmt, props);
    }
    if let Some((fmt, props)) = page_doc.notes[1] {
        (doc.settings.endnote_format, doc.settings.endnote_pr) = (fmt, props);
    }
    let d = &mut doc.styles.default_para;
    (d.space_before, d.space_after, d.line_spacing) = (spacing.before, spacing.after, spacing.line);
    (d.indent_left, d.indent_right, d.indent_first) = (spacing.left, spacing.right, spacing.first);
    for (kind, look) in &looks {
        apply_look(&mut doc, *kind, look);
    }
    if let Some(s) = settings {
        doc.passthrough.insert(super::package::SETTINGS_PART.into(), std::sync::Arc::new(s));
    }
    add_custom_styles(&mut doc, &named, &named_odt, tabs_from_indent);
    add_char_styles(&mut doc, &char_styles, &char_defs);
    if let Some(levels) = &outline {
        number_headings(&mut doc, levels);
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
    /// What the document's first page style and its note and line numbering settings say for
    /// the whole document.
    pub(super) doc: PageDoc,
}

/// Page settings that are the document's in WordCraft (and Word): the page colour (the first
/// page style's background), footnote and endnote numbering (`text:notes-configuration`).
#[derive(Clone, Default)]
pub(super) struct PageDoc {
    pub(super) page_color: Option<wordcraft_doc::props::Rgb>,
    /// Footnotes', then endnotes', number format and options.
    pub(super) notes: [Option<(wordcraft_doc::section::NumFormat, wordcraft_doc::section::NoteProps)>; 2],
}

/// A `text:notes-configuration` (ODF 1.3 §16.29): which notes it is for (endnotes: true), their
/// number format, first number (ODF counts from 0), restart, and footnotes at the page's foot
/// or the document's end.
fn notes_config(a: &Attrs) -> (bool, wordcraft_doc::section::NumFormat, wordcraft_doc::section::NoteProps) {
    use wordcraft_doc::section::{MAX_NOTE_START, NotePos, NoteProps, NoteRestart};
    let endnote = get(a, "text:note-class").is_some_and(|c| c.trim() == "endnote");
    let fmt = super::styles::num_format(get(a, "style:num-format").unwrap_or("1"));
    let num_start =
        get(a, "text:start-value").and_then(|v| v.trim().parse::<u64>().ok()).map(|v| v.saturating_add(1).min(u64::from(MAX_NOTE_START)) as u32);
    let num_restart = get(a, "text:start-numbering-at").map(|v| match v.trim() {
        "page" => NoteRestart::EachPage,
        "chapter" => NoteRestart::EachSect,
        _ => NoteRestart::Continuous,
    });
    let pos = match (endnote, get(a, "text:footnotes-position").map(str::trim)) {
        (false, Some("document")) => Some(NotePos::DocEnd),
        (false, Some("page")) => Some(NotePos::PageBottom),
        _ => None,
    };
    (endnote, fmt, NoteProps { pos, num_fmt: None, num_start, num_restart })
}

/// A `text:linenumbering-configuration` (ODF 1.3 §16.31) that numbers lines: every how many,
/// how far from the text, restarting on each page or not.
fn line_numbering(a: &Attrs) -> Option<wordcraft_doc::section::LineNumbering> {
    use wordcraft_doc::section::{LineNumberRestart, LineNumbering};
    // On unless it says otherwise (§19.845).
    if get(a, "text:number-lines").is_some_and(|v| v.trim() == "false") {
        return None;
    }
    let count_by = get(a, "text:increment").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 100);
    let distance = get(a, "text:offset").and_then(length).filter(|v| v.is_finite()).map_or(0.0, |v| v.clamp(0.0, 1584.0));
    let restart =
        if get(a, "text:restart-on-page").is_some_and(|v| v.trim() == "true") { LineNumberRestart::Page } else { LineNumberRestart::Continuous };
    Some(LineNumbering { count_by, start: 1, distance, restart })
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
    let mut doc = PageDoc::default();
    let mut lines = None;
    events(xml, |ev| match ev {
        Ev::Start(q, a) => match local(&q) {
            "notes-configuration" if q.starts_with("text:") => {
                let (endnote, fmt, props) = notes_config(&a);
                if let Some(slot) = doc.notes.get_mut(usize::from(endnote)) {
                    *slot = Some((fmt, props));
                }
            }
            "linenumbering-configuration" if q.starts_with("text:") => lines = line_numbering(&a),
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
    let mut pages = Pages { doc, ..Default::default() };
    for (name, layout, header, footer) in masters {
        let Some(Layout { attrs: a, columns: cols, header: hb, footer: fb }) = layouts.get(&layout) else { continue };
        let mut sec = SectionProps::default();
        apply_layout(a, &mut sec);
        sec.line_numbers.clone_from(&lines);
        if pages.first.is_none() {
            pages.doc.page_color = get(a, "fo:background-color").and_then(model::parse_color);
        }
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
    // Page numbers' format (§19.500 `style:num-format`; an empty one shows the numbers anyway).
    if let Some(f) = get(a, "style:num-format").filter(|f| !f.trim().is_empty()) {
        sec.page_num_format = super::styles::num_format(f);
    }
    // Page borders (§20.183 `fo:border`, §20.210 `fo:padding`: the space inside them).
    let side = |s: &str| {
        let b = get(a, &format!("fo:border-{s}")).or_else(|| get(a, "fo:border")).and_then(super::styles::parse_border)?;
        let pad = get(a, &format!("fo:padding-{s}")).or_else(|| get(a, "fo:padding")).and_then(length).filter(|v| v.is_finite());
        (b.style != wordcraft_doc::props::BorderStyle::None)
            .then(|| wordcraft_doc::props::Border { space: pad.map_or(0.0, |p| p.clamp(0.0, 31.0)), ..b })
    };
    // ODF's border is at the margin, the text inside its padding; WordCraft measures the border
    // from the page edge and the text margin from the edge too.
    let limit = sec.page_w.max(sec.page_h);
    let inset = |b: Option<wordcraft_doc::props::Border>, margin: &mut f32| {
        b.map(|b| {
            let edge = *margin;
            *margin = (edge + b.width + b.space).min(limit);
            wordcraft_doc::props::Border { space: edge, ..b }
        })
    };
    let borders = wordcraft_doc::props::Borders {
        top: inset(side("top"), &mut sec.margin_top),
        bottom: inset(side("bottom"), &mut sec.margin_bottom),
        left: inset(side("left"), &mut sec.margin_left),
        right: inset(side("right"), &mut sec.margin_right),
        ..Default::default()
    };
    if borders.any_visible() {
        sec.page_borders = Some(borders);
    }
}
